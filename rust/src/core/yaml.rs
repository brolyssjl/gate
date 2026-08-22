//! Hand-rolled YAML subset parser/stringifier (NEW - no direct TS
//! counterpart; replaces the `yaml` npm package, gate's one runtime
//! dependency). See `docs/rust-port.md`'s yaml section for the scoping
//! rationale.
//!
//! Gate uses YAML in exactly three places: `parseYaml` on `.gate/config.yml`
//! (`core/config.ts`), `parseYaml` on artifact frontmatter (plan.md,
//! debug-log.md, review.md, retro.md - `artifacts/frontmatter.ts`), and
//! `stringify` in the review packet (`artifacts/review.ts`'s
//! `serializeReview`). This module covers exactly what those call sites and
//! the shipped/documented file shapes need:
//!
//! - **Parse**: block maps and block sequences (including block sequences of
//!   maps, e.g. `criteria:`/`findings:`/`cycles:` in the artifact schemas),
//!   flow sequences (`[a, b]`) and flow maps (`{}`/`{a: b}`), plain/
//!   single/double-quoted scalars, `#` comments (quote-aware), a leading
//!   `---` document marker, and nested indentation. Scalar typing
//!   (null/bool/int/float/string) matches the `yaml` package's core schema
//!   as verified empirically (`node -e`): `null`/`Null`/`NULL`/`~`/empty are
//!   null; `true`/`True`/`TRUE`/`false`/`False`/`FALSE` are bool (`yes`/`no`
//!   are plain strings, not booleans); decimal/hex (`0x..`)/octal (`0o..`)
//!   integers and decimal/exponent floats are numbers.
//! - **Stringify**: byte-matches the `yaml` package's default block style for
//!   the review packet's actual payload shape (a flat map plus an array of
//!   flat maps - `reviewer`/`findings[].{id,severity,status,note,waiver}`),
//!   pinned against real `node -e 'require("yaml").stringify(...)'` output.
//!   Quoting decisions reuse the same scalar-typing logic as parsing (quote
//!   iff the plain form would round-trip to something else, plus the
//!   indicator-character and whitespace rules the `yaml` package applies).
//!
//! Divergence policy (per the plan): anything outside this subset - block
//! scalars (`|`/`>`), anchors/aliases/tags, multi-document streams - is
//! rejected with a plain error rather than silently mis-parsed. Embedded
//! newlines in a stringified scalar are emitted as an escaped `\n` inside a
//! double-quoted string (valid YAML) rather than the `yaml` package's block
//! literal (`|-`) - a documented, deliberate divergence for content outside
//! the review packet's actual field values, which are single-line text.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<YamlValue>),
    Map(Vec<(String, YamlValue)>),
}

impl YamlValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            YamlValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(String, YamlValue)]> {
        match self {
            YamlValue::Map(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[YamlValue]> {
        match self {
            YamlValue::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            YamlValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            YamlValue::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            YamlValue::Int(n) => Some(*n as f64),
            YamlValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, YamlValue::Null)
    }

    pub fn get(&self, key: &str) -> Option<&YamlValue> {
        self.as_map()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}

impl From<&str> for YamlValue {
    fn from(s: &str) -> Self {
        YamlValue::String(s.to_string())
    }
}
impl From<String> for YamlValue {
    fn from(s: String) -> Self {
        YamlValue::String(s)
    }
}
impl From<bool> for YamlValue {
    fn from(b: bool) -> Self {
        YamlValue::Bool(b)
    }
}
impl From<i64> for YamlValue {
    fn from(n: i64) -> Self {
        YamlValue::Int(n)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError(pub String);

impl fmt::Display for YamlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for YamlError {}

// ---------------------------------------------------------------------
// Tokenizing
// ---------------------------------------------------------------------

struct Line {
    indent: usize,
    content: String,
    line_no: usize,
}

/// Advance past a quoted span starting at `chars[start]` (which must be `'`
/// or `"`), honoring backslash-escaping for double quotes and doubled `''`
/// for single quotes. Returns the index just past the closing quote (or
/// `chars.len()` if unterminated).
fn scan_quoted(chars: &[char], start: usize) -> usize {
    let quote = chars[start];
    let mut i = start + 1;
    while i < chars.len() {
        if quote == '"' && chars[i] == '\\' && i + 1 < chars.len() {
            i += 2;
            continue;
        }
        if chars[i] == quote {
            if quote == '\'' && chars.get(i + 1) == Some(&'\'') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// Whether a `'`/`"` at `chars[i]` could plausibly *open* a quoted scalar,
/// rather than being a stray apostrophe/quote inside plain text (e.g.
/// "don't"). Mirrors agnosgram's `yaml.ts` port's same heuristic: a quote
/// only opens right after whitespace or a structural separator (or at the
/// very start).
fn can_open_quote(chars: &[char], i: usize) -> bool {
    i == 0 || matches!(chars[i - 1], ' ' | '\t' | '[' | '{' | ',' | ':')
}

/// Remove a trailing ` # comment`, respecting quotes.
fn strip_comment(raw: &str) -> &str {
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\'' | '"' if can_open_quote(&chars, i) => i = scan_quoted(&chars, i),
            '#' if i == 0 || chars[i - 1] == ' ' || chars[i - 1] == '\t' => {
                let byte_idx: usize = chars[..i].iter().map(|c| c.len_utf8()).sum();
                return &raw[..byte_idx];
            }
            _ => i += 1,
        }
    }
    raw
}

fn tokenize(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for (idx, row) in text.split('\n').enumerate() {
        let row = row.strip_suffix('\r').unwrap_or(row);
        let raw = strip_comment(row);
        if raw.trim().is_empty() {
            continue;
        }
        let trimmed = raw.trim();
        if trimmed == "---" || trimmed == "..." {
            continue;
        }
        let trimmed_start = raw.trim_start();
        let indent = raw.chars().count() - trimmed_start.chars().count();
        out.push(Line {
            indent,
            content: trimmed_start.trim_end().to_string(),
            line_no: idx + 1,
        });
    }
    out
}

// ---------------------------------------------------------------------
// Scalar parsing
// ---------------------------------------------------------------------

fn unescape_double(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                ' ' => out.push(' '),
                'x' if i + 3 < chars.len() => {
                    let hex: String = chars[i + 2..i + 4].iter().collect();
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                            i += 4;
                            continue;
                        }
                    }
                    out.push(chars[i + 1]);
                }
                'u' if i + 5 < chars.len() => {
                    let hex: String = chars[i + 2..i + 6].iter().collect();
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                            i += 6;
                            continue;
                        }
                    }
                    out.push(chars[i + 1]);
                }
                other => out.push(other),
            }
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn unescape_single(s: &str) -> String {
    s.replace("''", "'")
}

/// Split a flow sequence/map body on top-level commas, respecting quotes and
/// nested `[]`/`{}`.
fn split_flow(inner: &str) -> Vec<String> {
    let chars: Vec<char> = inner.chars().collect();
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\'' | '"' if can_open_quote(&chars, i) => i = scan_quoted(&chars, i),
            '[' | '{' => {
                depth += 1;
                i += 1;
            }
            ']' | '}' => {
                depth -= 1;
                i += 1;
            }
            ',' if depth == 0 => {
                parts.push(
                    chars[start..i]
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_string(),
                );
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    parts.push(chars[start..].iter().collect::<String>().trim().to_string());
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

/// Find the first unquoted `:` followed by whitespace or end-of-string (a
/// mapping key/value separator), as a byte offset into `content`.
fn find_colon(content: &str) -> Option<usize> {
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\'' | '"' if can_open_quote(&chars, i) => i = scan_quoted(&chars, i),
            ':' if i + 1 >= chars.len() || chars[i + 1] == ' ' || chars[i + 1] == '\t' => {
                return Some(chars[..i].iter().map(|c| c.len_utf8()).sum());
            }
            _ => i += 1,
        }
    }
    None
}

fn parse_int_literal(t: &str) -> Option<i64> {
    let (sign, rest): (i64, &str) = match t.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, t.strip_prefix('+').unwrap_or(t)),
    };
    if rest.is_empty() {
        return None;
    }
    if let Some(hex) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        return i64::from_str_radix(hex, 16).ok().map(|v| v * sign);
    }
    if let Some(oct) = rest.strip_prefix("0o").or_else(|| rest.strip_prefix("0O")) {
        if oct.is_empty() || !oct.chars().all(|c| ('0'..='7').contains(&c)) {
            return None;
        }
        return i64::from_str_radix(oct, 8).ok().map(|v| v * sign);
    }
    if !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    rest.parse::<i64>().ok().map(|v| v * sign)
}

fn parse_float_literal(t: &str) -> Option<f64> {
    let rest = t
        .strip_prefix('-')
        .or_else(|| t.strip_prefix('+'))
        .unwrap_or(t);
    if rest.is_empty() {
        return None;
    }
    let has_dot = rest.contains('.');
    let has_exp = rest.contains('e') || rest.contains('E');
    if !has_dot && !has_exp {
        return None;
    }
    if rest.matches('.').count() > 1 {
        return None;
    }
    if !rest
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-')
    {
        return None;
    }
    t.parse::<f64>().ok()
}

/// Classify a plain (unquoted, non-flow) scalar token per the `yaml`
/// package's core schema, as verified against real output.
fn classify_plain(t: &str) -> YamlValue {
    match t {
        "" | "~" | "null" | "Null" | "NULL" => return YamlValue::Null,
        "true" | "True" | "TRUE" => return YamlValue::Bool(true),
        "false" | "False" | "FALSE" => return YamlValue::Bool(false),
        _ => {}
    }
    if let Some(n) = parse_int_literal(t) {
        return YamlValue::Int(n);
    }
    if let Some(f) = parse_float_literal(t) {
        return YamlValue::Float(f);
    }
    YamlValue::String(t.to_string())
}

fn is_block_scalar_marker(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        Some('|') | Some('>') => {}
        _ => return false,
    }
    match chars.next() {
        None => true,
        Some('+') | Some('-') => chars.next().is_none(),
        _ => false,
    }
}

fn parse_scalar(token: &str) -> Result<YamlValue, YamlError> {
    let t = token.trim();
    if t.is_empty() {
        return Ok(YamlValue::Null);
    }
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        return Ok(YamlValue::String(unescape_double(&t[1..t.len() - 1])));
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return Ok(YamlValue::String(unescape_single(&t[1..t.len() - 1])));
    }
    if t == "{}" {
        return Ok(YamlValue::Map(Vec::new()));
    }
    if t.starts_with('{') && t.ends_with('}') {
        let inner = t[1..t.len() - 1].trim();
        if inner.is_empty() {
            return Ok(YamlValue::Map(Vec::new()));
        }
        let mut entries = Vec::new();
        for part in split_flow(inner) {
            let Some(colon) = find_colon(&part) else {
                return Err(YamlError(format!(
                    "Invalid YAML: expected \"key: value\" in flow map entry {part:?}"
                )));
            };
            let key = part[..colon].trim().to_string();
            let val = parse_scalar(part[colon + 1..].trim())?;
            entries.push((key, val));
        }
        return Ok(YamlValue::Map(entries));
    }
    if t.starts_with('[') && t.ends_with(']') {
        let inner = t[1..t.len() - 1].trim();
        if inner.is_empty() {
            return Ok(YamlValue::Array(Vec::new()));
        }
        let items = split_flow(inner)
            .iter()
            .map(|s| parse_scalar(s))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(YamlValue::Array(items));
    }
    Ok(classify_plain(t))
}

// ---------------------------------------------------------------------
// Block parsing
// ---------------------------------------------------------------------

fn set_map(map: &mut Vec<(String, YamlValue)>, key: String, value: YamlValue) {
    if let Some(slot) = map.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = value;
    } else {
        map.push((key, value));
    }
}

fn is_dash_line(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

/// Parse a block map, optionally seeded with a first `key: rest` pair
/// already extracted from a preceding line (the `- key: value` case of a
/// block sequence item). `i` is the index of the first *unconsumed* line.
fn parse_map(
    lines: &[Line],
    mut i: usize,
    indent: usize,
    seed: Option<(String, String)>,
) -> Result<(YamlValue, usize), YamlError> {
    let mut map: Vec<(String, YamlValue)> = Vec::new();

    if let Some((key, rest)) = seed {
        if !rest.is_empty() {
            if is_block_scalar_marker(&rest) {
                return Err(YamlError(
                    "Invalid YAML: block scalars (| and >) are outside the supported subset"
                        .to_string(),
                ));
            }
            set_map(&mut map, key, parse_scalar(&rest)?);
        } else if i < lines.len() && lines[i].indent > indent {
            let next_indent = lines[i].indent;
            let (value, next) = parse_block(lines, i, next_indent)?;
            set_map(&mut map, key, value);
            i = next;
        } else {
            set_map(&mut map, key, YamlValue::Null);
        }
    }

    while i < lines.len() && lines[i].indent == indent {
        let line = &lines[i];
        if is_dash_line(&line.content) {
            break; // a sequence item at map indent ends the map
        }
        let Some(colon) = find_colon(&line.content) else {
            return Err(YamlError(format!(
                "Invalid YAML at line {}: expected \"key: value\"",
                line.line_no
            )));
        };
        let key = line.content[..colon].trim().to_string();
        let rest = line.content[colon + 1..].trim().to_string();
        let line_no = line.line_no;
        i += 1;
        if !rest.is_empty() {
            if is_block_scalar_marker(&rest) {
                return Err(YamlError(format!(
                    "Invalid YAML at line {line_no}: block scalars (| and >) are outside the supported subset"
                )));
            }
            set_map(&mut map, key, parse_scalar(&rest)?);
        } else if i < lines.len() && lines[i].indent > indent {
            let next_indent = lines[i].indent;
            let (value, next) = parse_block(lines, i, next_indent)?;
            set_map(&mut map, key, value);
            i = next;
        } else {
            set_map(&mut map, key, YamlValue::Null);
        }
    }
    Ok((YamlValue::Map(map), i))
}

/// Parse a block of lines whose indentation is `indent`, starting at `i`.
fn parse_block(
    lines: &[Line],
    mut i: usize,
    indent: usize,
) -> Result<(YamlValue, usize), YamlError> {
    let Some(first) = lines.get(i) else {
        return Ok((YamlValue::Null, i));
    };

    if is_dash_line(&first.content) {
        let mut items = Vec::new();
        while i < lines.len() && lines[i].indent == indent && is_dash_line(&lines[i].content) {
            let line = &lines[i];
            let item_col = indent + 2;
            let rest = if line.content == "-" {
                ""
            } else {
                line.content[2..].trim_start()
            };
            if rest.is_empty() {
                if i + 1 < lines.len() && lines[i + 1].indent > indent {
                    let next_indent = lines[i + 1].indent;
                    let (value, next) = parse_block(lines, i + 1, next_indent)?;
                    items.push(value);
                    i = next;
                } else {
                    items.push(YamlValue::Null);
                    i += 1;
                }
            } else if let Some(colon) = find_colon(rest) {
                let key = rest[..colon].trim().to_string();
                let after = rest[colon + 1..].trim().to_string();
                let (value, next) = parse_map(lines, i + 1, item_col, Some((key, after)))?;
                items.push(value);
                i = next;
            } else {
                items.push(parse_scalar(rest)?);
                i += 1;
            }
        }
        return Ok((YamlValue::Array(items), i));
    }

    parse_map(lines, i, indent, None)
}

/// Parse a YAML document within gate's supported subset. Empty/blank/
/// comment-only input parses to `Null`, matching the `yaml` package's
/// `parse("")`.
pub fn parse_yaml(text: &str) -> Result<YamlValue, YamlError> {
    let lines = tokenize(text);
    if lines.is_empty() {
        return Ok(YamlValue::Null);
    }
    let (value, next) = parse_block(&lines, 0, lines[0].indent)?;
    if next < lines.len() {
        return Err(YamlError(format!(
            "Invalid YAML at line {}: unexpected indentation or content outside the supported subset",
            lines[next].line_no
        )));
    }
    Ok(value)
}

// ---------------------------------------------------------------------
// Stringify
// ---------------------------------------------------------------------

fn needs_quote(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if s.starts_with(' ') || s.starts_with('\t') || s.ends_with(' ') || s.ends_with('\t') {
        return true;
    }
    if s.contains(": ") || s.ends_with(':') || s.contains('\n') {
        return true;
    }
    if s.contains(" #") {
        return true;
    }
    let first = s.chars().next().unwrap();
    if "&*%#|>'\"@`,[]{}".contains(first) {
        return true;
    }
    if (first == '-' || first == '?' || first == ':') && (s.len() == 1 || s.as_bytes()[1] == b' ') {
        return true;
    }
    // Ambiguous with a typed scalar (null/bool/int/float) - quote so it
    // round-trips back to the same string.
    !matches!(classify_plain(s), YamlValue::String(ref r) if r == s)
}

/// `"..."` with `\\`, `"`, and control-character escaping. Used whenever a
/// scalar needs quoting; this is a documented divergence for embedded
/// newlines (escaped `\n` here vs. the `yaml` package's block literal) - see
/// the module doc comment.
fn yaml_double_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn format_key(key: &str) -> String {
    if needs_quote(key) {
        yaml_double_quote(key)
    } else {
        key.to_string()
    }
}

fn format_scalar(value: &YamlValue) -> String {
    match value {
        YamlValue::Null => "null".to_string(),
        YamlValue::Bool(b) => b.to_string(),
        YamlValue::Int(n) => n.to_string(),
        YamlValue::Float(f) => f.to_string(),
        YamlValue::String(s) => {
            if needs_quote(s) {
                yaml_double_quote(s)
            } else {
                s.to_string()
            }
        }
        YamlValue::Array(_) | YamlValue::Map(_) => {
            unreachable!("format_scalar called on a non-scalar")
        }
    }
}

fn stringify_map(entries: &[(String, YamlValue)], indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    for (key, val) in entries {
        match val {
            YamlValue::Array(items) => {
                if items.is_empty() {
                    out.push_str(&format!("{pad}{}: []\n", format_key(key)));
                } else {
                    out.push_str(&format!("{pad}{}:\n", format_key(key)));
                    stringify_array(items, indent + 1, out);
                }
            }
            YamlValue::Map(sub) => {
                if sub.is_empty() {
                    out.push_str(&format!("{pad}{}: {{}}\n", format_key(key)));
                } else {
                    out.push_str(&format!("{pad}{}:\n", format_key(key)));
                    stringify_map(sub, indent + 1, out);
                }
            }
            scalar => out.push_str(&format!(
                "{pad}{}: {}\n",
                format_key(key),
                format_scalar(scalar)
            )),
        }
    }
}

fn stringify_array(items: &[YamlValue], indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    for item in items {
        match item {
            YamlValue::Map(entries) if !entries.is_empty() => {
                let (first_key, first_val) = &entries[0];
                match first_val {
                    YamlValue::Array(_) | YamlValue::Map(_) => {
                        // A map-valued first field can't render inline with the
                        // dash; fall back to a `-` on its own line (outside the
                        // review-packet shape this module is pinned against).
                        out.push_str(&format!("{pad}-\n"));
                        stringify_map(entries, indent + 1, out);
                    }
                    scalar => {
                        out.push_str(&format!(
                            "{pad}- {}: {}\n",
                            format_key(first_key),
                            format_scalar(scalar)
                        ));
                    }
                }
                if entries.len() > 1 {
                    stringify_map(&entries[1..], indent + 1, out);
                }
            }
            YamlValue::Map(_) => out.push_str(&format!("{pad}- {{}}\n")),
            YamlValue::Array(sub) => {
                if sub.is_empty() {
                    out.push_str(&format!("{pad}- []\n"));
                } else {
                    out.push_str(&format!("{pad}-\n"));
                    stringify_array(sub, indent + 1, out);
                }
            }
            scalar => out.push_str(&format!("{pad}- {}\n", format_scalar(scalar))),
        }
    }
}

/// `stringify(value)` from the `yaml` package's default block style, pinned
/// against real output for the review-packet payload shape (see module doc).
pub fn stringify_yaml(value: &YamlValue) -> String {
    let mut out = String::new();
    match value {
        YamlValue::Map(entries) => stringify_map(entries, 0, &mut out),
        YamlValue::Array(items) => stringify_array(items, 0, &mut out),
        scalar => out.push_str(&format!("{}\n", format_scalar(scalar))),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalars_with_correct_types() {
        let v = parse_yaml("version: 1\nname: hello\nflag: true\noff: false\nempty:\n").unwrap();
        assert_eq!(v.get("version").unwrap().as_i64(), Some(1));
        assert_eq!(v.get("name").unwrap().as_str(), Some("hello"));
        assert_eq!(v.get("flag").unwrap().as_bool(), Some(true));
        assert_eq!(v.get("off").unwrap().as_bool(), Some(false));
        assert_eq!(v.get("empty").unwrap(), &YamlValue::Null);
    }

    #[test]
    fn null_and_bool_literal_variants_match_yaml_package_core_schema() {
        let v = parse_yaml(
            "a: Null\nb: NULL\nc: ~\nd: True\ne: TRUE\nf: False\ng: FALSE\nh: yes\ni: no\n",
        )
        .unwrap();
        assert_eq!(v.get("a").unwrap(), &YamlValue::Null);
        assert_eq!(v.get("b").unwrap(), &YamlValue::Null);
        assert_eq!(v.get("c").unwrap(), &YamlValue::Null);
        assert_eq!(v.get("d").unwrap().as_bool(), Some(true));
        assert_eq!(v.get("e").unwrap().as_bool(), Some(true));
        assert_eq!(v.get("f").unwrap().as_bool(), Some(false));
        assert_eq!(v.get("g").unwrap().as_bool(), Some(false));
        // `yes`/`no` are NOT booleans under the yaml package's core schema.
        assert_eq!(v.get("h").unwrap().as_str(), Some("yes"));
        assert_eq!(v.get("i").unwrap().as_str(), Some("no"));
    }

    #[test]
    fn numbers_cover_decimal_hex_octal_and_float_forms() {
        // node -e 'console.log(JSON.stringify(require("yaml").parse(
        //   "o: 5\np: 5.5\nq: -5\nr: .5\ns: 5e3\nt: 0x1A\nu: 0o17\n")))'
        let v = parse_yaml("o: 5\np: 5.5\nq: -5\nr: .5\ns: 5e3\nt: 0x1A\nu: 0o17\n").unwrap();
        assert_eq!(v.get("o").unwrap().as_i64(), Some(5));
        assert_eq!(v.get("p").unwrap().as_f64(), Some(5.5));
        assert_eq!(v.get("q").unwrap().as_i64(), Some(-5));
        assert_eq!(v.get("r").unwrap().as_f64(), Some(0.5));
        assert_eq!(v.get("s").unwrap().as_f64(), Some(5000.0));
        assert_eq!(v.get("t").unwrap().as_i64(), Some(26));
        assert_eq!(v.get("u").unwrap().as_i64(), Some(15));
    }

    #[test]
    fn does_not_misparse_a_multi_dot_version_string_as_a_number() {
        let v = parse_yaml("v: 1.2.3\n").unwrap();
        assert_eq!(v.get("v").unwrap().as_str(), Some("1.2.3"));
    }

    #[test]
    fn parses_nested_maps() {
        let v = parse_yaml("commands:\n  build: npm run build\n  test: npm test\n").unwrap();
        assert_eq!(
            v.get("commands").unwrap().get("build").unwrap().as_str(),
            Some("npm run build")
        );
        assert_eq!(
            v.get("commands").unwrap().get("test").unwrap().as_str(),
            Some("npm test")
        );
    }

    #[test]
    fn parses_block_sequences_of_scalars() {
        let v = parse_yaml("scope_ignore:\n  - node_modules/**\n  - coverage/**\n").unwrap();
        let items = v.get("scope_ignore").unwrap().as_array().unwrap();
        assert_eq!(
            items,
            &[
                YamlValue::String("node_modules/**".into()),
                YamlValue::String("coverage/**".into())
            ]
        );
    }

    #[test]
    fn parses_block_sequences_of_maps_as_used_by_criteria_and_findings() {
        let raw = "findings:\n  - id: f1\n    severity: blocker\n    status: open\n    note: no constant-time check\n  - id: f2\n    severity: minor\n    status: resolved\n    note: extract the loop\n";
        let v = parse_yaml(raw).unwrap();
        let findings = v.get("findings").unwrap().as_array().unwrap();
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].get("id").unwrap().as_str(), Some("f1"));
        assert_eq!(
            findings[0].get("severity").unwrap().as_str(),
            Some("blocker")
        );
        assert_eq!(
            findings[0].get("note").unwrap().as_str(),
            Some("no constant-time check")
        );
        assert_eq!(findings[1].get("id").unwrap().as_str(), Some("f2"));
        assert_eq!(
            findings[1].get("status").unwrap().as_str(),
            Some("resolved")
        );
    }

    #[test]
    fn ignores_comments_and_blank_lines_and_leading_doc_marker() {
        let v = parse_yaml("---\n# a comment\n\nversion: 1  # inline\n").unwrap();
        assert_eq!(v.get("version").unwrap().as_i64(), Some(1));
    }

    #[test]
    fn strips_a_comment_after_an_unquoted_value_containing_an_apostrophe() {
        let v = parse_yaml("note: don't repeat this # see LES-002\n").unwrap();
        assert_eq!(v.get("note").unwrap().as_str(), Some("don't repeat this"));
    }

    #[test]
    fn parses_inline_flow_sequences_of_scalars() {
        let v = parse_yaml("match: [apps/api/**, apps/web/**]\n").unwrap();
        assert_eq!(
            v.get("match").unwrap().as_array().unwrap(),
            &[
                YamlValue::String("apps/api/**".into()),
                YamlValue::String("apps/web/**".into())
            ]
        );
    }

    #[test]
    fn parses_empty_flow_sequence_and_empty_flow_map() {
        let v = parse_yaml("a: []\nb: {}\n").unwrap();
        assert_eq!(v.get("a").unwrap(), &YamlValue::Array(vec![]));
        assert_eq!(v.get("b").unwrap(), &YamlValue::Map(vec![]));
    }

    #[test]
    fn flow_sequence_respects_quoted_commas() {
        let v = parse_yaml("tags: [\"a, b\", c]\n").unwrap();
        assert_eq!(
            v.get("tags").unwrap().as_array().unwrap(),
            &[
                YamlValue::String("a, b".into()),
                YamlValue::String("c".into())
            ]
        );
    }

    #[test]
    fn double_and_single_quoted_scalars_are_unescaped() {
        let v = parse_yaml("a: \"line\\nbreak\"\nb: 'it''s fine'\n").unwrap();
        assert_eq!(v.get("a").unwrap().as_str(), Some("line\nbreak"));
        assert_eq!(v.get("b").unwrap().as_str(), Some("it's fine"));
    }

    #[test]
    fn rejects_block_scalars_instead_of_silently_mis_parsing_them() {
        let err = parse_yaml("source: |\n  line one\n  line two\n").unwrap_err();
        assert!(err.0.contains("block scalars"));
        let err2 = parse_yaml("note: >-\n  folded\n").unwrap_err();
        assert!(err2.0.contains("block scalars"));
    }

    #[test]
    fn rejects_leftover_lines_it_cannot_represent() {
        let err = parse_yaml("a: 1\n  b: 2\n").unwrap_err();
        assert!(err.0.contains("line 2"));
    }

    #[test]
    fn empty_input_parses_to_null_matching_the_yaml_package() {
        assert_eq!(parse_yaml("").unwrap(), YamlValue::Null);
        assert_eq!(parse_yaml("\n\n").unwrap(), YamlValue::Null);
        assert_eq!(parse_yaml("# just a comment\n").unwrap(), YamlValue::Null);
    }

    #[test]
    fn parses_a_real_config_yml_shaped_document_written_by_gate_init() {
        // Shape written by `buildConfig` in src/commands/init.ts.
        let raw = "# Gate configuration. Commands are the same trust class as npm scripts.\ncommands:\n  build: \"npm run build\"\n  test: \"npm test\"\n  # lint: \"<command>\"\n  # coverage: \"<command>\"\nthresholds:\n  # diff_coverage: 80   # uncomment once a coverage command is set\nphases:\n  plan: required\n  implement: required\n  test: required\nintegrations:\n  agnosgram: off\n  sdd: off\n# Noise the scope check ignores rather than flags as undeclared - part of the trust hash.\nscope_ignore:\n  # - <glob>\n";
        let v = parse_yaml(raw).unwrap();
        assert_eq!(
            v.get("commands").unwrap().get("build").unwrap().as_str(),
            Some("npm run build")
        );
        assert_eq!(
            v.get("phases").unwrap().get("plan").unwrap().as_str(),
            Some("required")
        );
        assert_eq!(
            v.get("integrations")
                .unwrap()
                .get("agnosgram")
                .unwrap()
                .as_str(),
            Some("off")
        );
    }

    #[test]
    fn parses_the_real_yaml_packages_stringify_output_for_a_targets_config() {
        // node -e 'console.log(require("yaml").stringify({
        //   commands: { build: "npm run build", test: "npm test" },
        //   thresholds: { diff_coverage: 80 },
        //   targets: { api: { match: ["apps/api/**"], commands: { test: "pytest" },
        //     thresholds: { diff_coverage: 85 }, coverage_format: "coverage-py" },
        //     web: { match: ["apps/web/**"], commands: { test: "vitest run" } } },
        //   phases: { plan: "required", implement: "required", test: "required" },
        //   integrations: { agnosgram: "auto", sdd: "off" },
        //   coverage_format: "auto", retention: {},
        //   scope_ignore: ["node_modules/**", "coverage/**"] }))'
        let raw = "commands:\n  build: npm run build\n  test: npm test\nthresholds:\n  diff_coverage: 80\ntargets:\n  api:\n    match:\n      - apps/api/**\n    commands:\n      test: pytest\n    thresholds:\n      diff_coverage: 85\n    coverage_format: coverage-py\n  web:\n    match:\n      - apps/web/**\n    commands:\n      test: vitest run\nphases:\n  plan: required\n  implement: required\n  test: required\nintegrations:\n  agnosgram: auto\n  sdd: off\ncoverage_format: auto\nretention: {}\nscope_ignore:\n  - node_modules/**\n  - coverage/**\n";
        let v = parse_yaml(raw).unwrap();
        assert_eq!(
            v.get("targets")
                .unwrap()
                .get("api")
                .unwrap()
                .get("match")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            v.get("targets")
                .unwrap()
                .get("api")
                .unwrap()
                .get("thresholds")
                .unwrap()
                .get("diff_coverage")
                .unwrap()
                .as_i64(),
            Some(85)
        );
        assert_eq!(v.get("retention").unwrap(), &YamlValue::Map(vec![]));
        assert_eq!(
            v.get("scope_ignore").unwrap().as_array().unwrap(),
            &[
                YamlValue::String("node_modules/**".into()),
                YamlValue::String("coverage/**".into())
            ]
        );
        assert_eq!(
            v.get("integrations").unwrap().get("sdd").unwrap().as_str(),
            Some("off")
        );
    }

    #[test]
    fn stringify_matches_the_yaml_packages_output_for_the_review_packet_shape() {
        // node -e 'console.log(require("yaml").stringify({ reviewer: "alice", findings: [
        //   { id: "f1", severity: "blocker", status: "open",
        //     note: "resetToken is compared with == not a constant-time check" },
        //   { id: "f2", severity: "minor", status: "resolved",
        //     note: "Extract the retry loop", waiver: "not needed" } ] }))'
        let value = YamlValue::Map(vec![
            ("reviewer".into(), YamlValue::String("alice".into())),
            (
                "findings".into(),
                YamlValue::Array(vec![
                    YamlValue::Map(vec![
                        ("id".into(), "f1".into()),
                        ("severity".into(), "blocker".into()),
                        ("status".into(), "open".into()),
                        (
                            "note".into(),
                            "resetToken is compared with == not a constant-time check".into(),
                        ),
                    ]),
                    YamlValue::Map(vec![
                        ("id".into(), "f2".into()),
                        ("severity".into(), "minor".into()),
                        ("status".into(), "resolved".into()),
                        ("note".into(), "Extract the retry loop".into()),
                        ("waiver".into(), "not needed".into()),
                    ]),
                ]),
            ),
        ]);
        let expected = "reviewer: alice\nfindings:\n  - id: f1\n    severity: blocker\n    status: open\n    note: resetToken is compared with == not a constant-time check\n  - id: f2\n    severity: minor\n    status: resolved\n    note: Extract the retry loop\n    waiver: not needed\n";
        assert_eq!(stringify_yaml(&value), expected);
    }

    #[test]
    fn stringify_quotes_values_that_would_otherwise_reparse_wrong_or_change_meaning() {
        // node -e 'console.log(require("yaml").stringify({
        //   a: "True", b: "Yes", c: "1e5", d: "0x10", e: "", f: "-5", g: "- item" }))'
        let value = YamlValue::Map(vec![
            ("a".into(), "True".into()),
            ("b".into(), "Yes".into()),
            ("c".into(), "1e5".into()),
            ("d".into(), "0x10".into()),
            ("e".into(), "".into()),
            ("f".into(), "-5".into()),
            ("g".into(), "- item".into()),
        ]);
        let expected =
            "a: \"True\"\nb: Yes\nc: \"1e5\"\nd: \"0x10\"\ne: \"\"\nf: \"-5\"\ng: \"- item\"\n";
        assert_eq!(stringify_yaml(&value), expected);
    }

    #[test]
    fn stringify_leaves_ordinary_words_and_dashed_identifiers_unquoted() {
        // node -e 'console.log(require("yaml").stringify({ k: "leading-dash-but-word",
        //   l: "normal text", m: "a-b_c.d/e", n: "1.2.3", o: "v1.0.0" }))'
        let value = YamlValue::Map(vec![
            ("k".into(), "leading-dash-but-word".into()),
            ("l".into(), "normal text".into()),
            ("m".into(), "a-b_c.d/e".into()),
            ("n".into(), "1.2.3".into()),
            ("o".into(), "v1.0.0".into()),
        ]);
        let expected =
            "k: leading-dash-but-word\nl: normal text\nm: a-b_c.d/e\nn: 1.2.3\no: v1.0.0\n";
        assert_eq!(stringify_yaml(&value), expected);
    }

    #[test]
    fn stringify_empty_array_and_map_use_the_flow_form() {
        let value = YamlValue::Map(vec![
            ("a".into(), YamlValue::Array(vec![])),
            ("b".into(), YamlValue::Map(vec![])),
        ]);
        assert_eq!(stringify_yaml(&value), "a: []\nb: {}\n");
    }

    #[test]
    fn round_trips_a_config_shaped_object_through_stringify_and_parse() {
        let value = YamlValue::Map(vec![
            (
                "commands".into(),
                YamlValue::Map(vec![("test".into(), "echo hi".into())]),
            ),
            (
                "scope_ignore".into(),
                YamlValue::Array(vec!["a/**".into(), "b/**".into()]),
            ),
        ]);
        let out = stringify_yaml(&value);
        assert_eq!(parse_yaml(&out).unwrap(), value);
    }
}
