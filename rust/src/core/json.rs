//! Hand-rolled JSON value type, emitter, and parser (no TS counterpart - see
//! `docs/rust-port.md`). Zero external crates, mirroring the TS
//! implementation's zero runtime dependencies. Ported wholesale from
//! agnosgram's `rust/src/core/json.rs` (same contract, shared-code
//! duplication across the two tools is accepted by the roadmap).
//!
//! The emitter must byte-match `JSON.stringify(value, null, 2)` (gate's
//! `run.json`, `--json` output, coverage/test-report shapes): 2-space
//! indent, `": "` after keys, insertion-ordered object keys, minimal escaping
//! exactly as JS does it, integers without a decimal point. The parser
//! accepts what `JSON.parse` accepts for the shapes this CLI reads (test
//! reports, coverage JSON, advise reports).

use std::fmt;

/// A JSON value. Objects are a `Vec` of key/value pairs (never a `HashMap`)
/// so key order is preserved exactly as inserted, matching `JSON.stringify`'s
/// insertion-order guarantee for string keys.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn object() -> Value {
        Value::Object(Vec::new())
    }

    pub fn array() -> Value {
        Value::Array(Vec::new())
    }

    /// Insert/replace a key in an object value. Panics if `self` is not an
    /// `Object` - a programmer error, not a runtime condition.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) -> &mut Self {
        let Value::Object(entries) = self else {
            panic!("Value::insert called on a non-object Value");
        };
        let key = key.into();
        let value = value.into();
        if let Some(slot) = entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            entries.push((key, value));
        }
        self
    }

    pub fn push(&mut self, value: impl Into<Value>) -> &mut Self {
        let Value::Array(items) = self else {
            panic!("Value::push called on a non-array Value");
        };
        items.push(value.into());
        self
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            Value::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Int(n)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Int(n as i64)
    }
}
impl From<usize> for Value {
    fn from(n: usize) -> Self {
        Value::Int(n as i64)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Float(n)
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(items: Vec<T>) -> Self {
        Value::Array(items.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Value> + Clone> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(v) => v.into(),
            None => Value::Null,
        }
    }
}

/// Escape one string exactly as `JSON.stringify` does: `"` `\\` `\b` `\f`
/// `\n` `\r` `\t` as their two-char escapes, other control chars (< 0x20) as
/// `\u00XX` (lowercase hex), non-ASCII passed through verbatim. Rust's `char`
/// is always a valid Unicode scalar value, so the "lone surrogate" case
/// `JSON.stringify` also has to handle for JS's UTF-16 strings cannot arise
/// here - there is no ill-formed `String` to encode.
fn escape_json_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub(crate) fn format_float(f: f64) -> String {
    if f.is_nan() || f.is_infinite() {
        return "null".to_string();
    }
    if f == 0.0 {
        // Rust prints "-0" for negative zero; `(-0).toString()` (and hence
        // `JSON.stringify(-0)`) is "0" in JS.
        return "0".to_string();
    }
    format!("{f}")
}

fn write_value(value: &Value, indent: usize, depth: usize, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Float(f) => out.push_str(&format_float(*f)),
        Value::String(s) => escape_json_string(s, out),
        Value::Array(items) => write_array(items, indent, depth, out),
        Value::Object(entries) => write_object(entries, indent, depth, out),
    }
}

fn write_array(items: &[Value], indent: usize, depth: usize, out: &mut String) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    let pad = " ".repeat(indent * (depth + 1));
    let close_pad = " ".repeat(indent * depth);
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        out.push('\n');
        out.push_str(&pad);
        write_value(item, indent, depth + 1, out);
        if i + 1 < items.len() {
            out.push(',');
        }
    }
    out.push('\n');
    out.push_str(&close_pad);
    out.push(']');
}

fn write_object(entries: &[(String, Value)], indent: usize, depth: usize, out: &mut String) {
    if entries.is_empty() {
        out.push_str("{}");
        return;
    }
    let pad = " ".repeat(indent * (depth + 1));
    let close_pad = " ".repeat(indent * depth);
    out.push('{');
    for (i, (key, val)) in entries.iter().enumerate() {
        out.push('\n');
        out.push_str(&pad);
        escape_json_string(key, out);
        out.push_str(": ");
        write_value(val, indent, depth + 1, out);
        if i + 1 < entries.len() {
            out.push(',');
        }
    }
    out.push('\n');
    out.push_str(&close_pad);
    out.push('}');
}

/// `JSON.stringify(value, null, 2)`.
pub fn stringify_pretty(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, 2, 0, &mut out);
    out
}

fn write_compact(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Float(f) => out.push_str(&format_float(*f)),
        Value::String(s) => escape_json_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_compact(item, out);
            }
            out.push(']');
        }
        Value::Object(entries) => {
            out.push('{');
            for (i, (key, val)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                escape_json_string(key, out);
                out.push(':');
                write_compact(val, out);
            }
            out.push('}');
        }
    }
}

/// `JSON.stringify(value)` with no indentation argument - gate's one use
/// site is `core/config.ts`'s `commandsBlockHashSource`, which hashes a
/// compact, deterministic byte string rather than a pretty-printed one.
pub fn stringify_compact(value: &Value) -> String {
    let mut out = String::new();
    write_compact(value, &mut out);
    out
}

/// A `JSON.parse` failure. Message text is descriptive, not a byte-exact port
/// of V8's parser errors - the plan requires only that parsing *accept* what
/// `JSON.parse` accepts for the shapes this CLI reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ParseError {}

struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    src: &'a str,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser {
            chars: src.chars().collect(),
            pos: 0,
            src,
        }
    }

    fn err(&self, msg: &str) -> ParseError {
        ParseError(format!("{msg} at position {} in {:?}", self.pos, self.src))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, c: char) -> Result<(), ParseError> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected '{c}'")))
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => Ok(Value::String(self.parse_string()?)),
            Some('t') => self.parse_literal("true", Value::Bool(true)),
            Some('f') => self.parse_literal("false", Value::Bool(false)),
            Some('n') => self.parse_literal("null", Value::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            _ => Err(self.err("unexpected end of input or invalid token")),
        }
    }

    fn parse_literal(&mut self, lit: &str, value: Value) -> Result<Value, ParseError> {
        for expect in lit.chars() {
            if self.bump() != Some(expect) {
                return Err(self.err(&format!("expected literal '{lit}'")));
            }
        }
        Ok(value)
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        self.expect('{')?;
        let mut entries: Vec<(String, Value)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Value::Object(entries));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err(self.err("expected a string key"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(':')?;
            let value = self.parse_value()?;
            if let Some(slot) = entries.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = value;
            } else {
                entries.push((key, value));
            }
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some('}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
        Ok(Value::Object(entries))
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some(']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let c = self.bump().ok_or_else(|| self.err("unterminated string"))?;
            match c {
                '"' => break,
                '\\' => {
                    let esc = self.bump().ok_or_else(|| self.err("unterminated escape"))?;
                    match esc {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let cp = self.parse_hex4()?;
                            // Combine a UTF-16 surrogate pair when present;
                            // otherwise take the code unit as a scalar value.
                            if (0xD800..=0xDBFF).contains(&cp) {
                                if self.peek() == Some('\\')
                                    && self.chars.get(self.pos + 1) == Some(&'u')
                                {
                                    let save = self.pos;
                                    self.pos += 2;
                                    let low = self.parse_hex4()?;
                                    if (0xDC00..=0xDFFF).contains(&low) {
                                        let combined =
                                            0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                                        if let Some(ch) = char::from_u32(combined) {
                                            out.push(ch);
                                            continue;
                                        }
                                    }
                                    self.pos = save;
                                }
                                out.push('\u{FFFD}');
                            } else if let Some(ch) = char::from_u32(cp) {
                                out.push(ch);
                            } else {
                                out.push('\u{FFFD}');
                            }
                        }
                        other => return Err(self.err(&format!("invalid escape '\\{other}'"))),
                    }
                }
                c => out.push(c),
            }
        }
        Ok(out)
    }

    fn parse_hex4(&mut self) -> Result<u32, ParseError> {
        let mut cp = 0u32;
        for _ in 0..4 {
            let c = self
                .bump()
                .ok_or_else(|| self.err("truncated \\u escape"))?;
            let digit = c
                .to_digit(16)
                .ok_or_else(|| self.err("invalid hex digit in \\u escape"))?;
            cp = cp * 16 + digit;
        }
        Ok(cp)
    }

    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        let mut is_float = false;
        if self.peek() == Some('.') {
            is_float = true;
            self.pos += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        if text.is_empty() || text == "-" {
            return Err(self.err("invalid number"));
        }
        if is_float {
            text.parse::<f64>()
                .map(Value::Float)
                .map_err(|_| self.err("invalid number"))
        } else {
            match text.parse::<i64>() {
                Ok(n) => Ok(Value::Int(n)),
                // A whole number too large for i64 (or malformed) still
                // parses fine as JS's f64-backed number type would.
                Err(_) => text
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| self.err("invalid number")),
            }
        }
    }
}

/// Parse a JSON document, accepting what `JSON.parse` accepts (leading/
/// trailing whitespace, any JSON value as the top level, no trailing
/// comments or commas).
pub fn parse(text: &str) -> Result<Value, ParseError> {
    let mut parser = Parser::new(text);
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.chars.len() {
        return Err(parser.err("unexpected trailing content"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_empty_object_and_array_inline() {
        assert_eq!(stringify_pretty(&Value::object()), "{}");
        assert_eq!(stringify_pretty(&Value::array()), "[]");
    }

    #[test]
    fn emits_nested_structures_with_two_space_indent() {
        let mut obj = Value::object();
        let mut inner = Value::object();
        inner.insert("b", 1i64);
        obj.insert("a", inner);
        let arr: Value = vec![Value::Int(1), Value::Int(2)].into();
        obj.insert("list", arr);
        assert_eq!(
            stringify_pretty(&obj),
            "{\n  \"a\": {\n    \"b\": 1\n  },\n  \"list\": [\n    1,\n    2\n  ]\n}"
        );
    }

    #[test]
    fn escapes_quotes_backslashes_and_newlines() {
        let v = Value::String("a\"b\\c\nd".to_string());
        assert_eq!(stringify_pretty(&v), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn escapes_control_chars_as_u00xx() {
        let v = Value::String("\u{1}\u{1f}".to_string());
        assert_eq!(stringify_pretty(&v), "\"\\u0001\\u001f\"");
    }

    #[test]
    fn passes_non_ascii_through_verbatim() {
        let v = Value::String("héllo 日本語".to_string());
        assert_eq!(stringify_pretty(&v), "\"héllo 日本語\"");
    }

    #[test]
    fn integers_have_no_decimal_point() {
        assert_eq!(stringify_pretty(&Value::Int(42)), "42");
        assert_eq!(stringify_pretty(&Value::Int(-7)), "-7");
    }

    #[test]
    fn floats_format_like_js_number_tostring() {
        assert_eq!(stringify_pretty(&Value::Float(1.5)), "1.5");
        assert_eq!(stringify_pretty(&Value::Float(0.1)), "0.1");
        assert_eq!(stringify_pretty(&Value::Float(-0.0)), "0");
    }

    #[test]
    fn array_of_objects_matches_json_stringify_shape() {
        let mut a = Value::object();
        a.insert("id", "LES-001");
        let mut b = Value::object();
        b.insert("id", "LES-002");
        let arr = Value::Array(vec![a, b]);
        assert_eq!(
            stringify_pretty(&arr),
            "[\n  {\n    \"id\": \"LES-001\"\n  },\n  {\n    \"id\": \"LES-002\"\n  }\n]"
        );
    }

    #[test]
    fn parses_scalars_arrays_and_objects() {
        assert_eq!(parse("null").unwrap(), Value::Null);
        assert_eq!(parse("true").unwrap(), Value::Bool(true));
        assert_eq!(parse("false").unwrap(), Value::Bool(false));
        assert_eq!(parse("42").unwrap(), Value::Int(42));
        assert_eq!(parse("-3.5").unwrap(), Value::Float(-3.5));
        assert_eq!(parse("\"hi\"").unwrap(), Value::String("hi".to_string()));
        assert_eq!(
            parse("[1,2,3]").unwrap(),
            Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );
        let obj = parse("{\"a\": 1, \"b\": [true, null]}").unwrap();
        assert_eq!(
            obj,
            Value::Object(vec![
                ("a".to_string(), Value::Int(1)),
                (
                    "b".to_string(),
                    Value::Array(vec![Value::Bool(true), Value::Null])
                ),
            ])
        );
    }

    #[test]
    fn parses_escaped_strings() {
        let v = parse("\"a\\nb\\t\\\"c\\\"\"").unwrap();
        assert_eq!(v, Value::String("a\nb\t\"c\"".to_string()));
    }

    #[test]
    fn round_trips_through_stringify_and_parse() {
        let mut obj = Value::object();
        obj.insert("id", "LES-001");
        obj.insert("count", 3i64);
        obj.insert("ratio", 0.5f64);
        obj.insert(
            "tags",
            Value::Array(vec![Value::String("a".into()), Value::String("b".into())]),
        );
        let text = stringify_pretty(&obj);
        assert_eq!(parse(&text).unwrap(), obj);
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse("{}x").is_err());
    }

    #[test]
    fn rejects_incomplete_input() {
        assert!(parse("{\"a\":").is_err());
    }

    #[test]
    fn stringify_compact_matches_json_stringify_with_no_indent_arg() {
        // node -e 'console.log(JSON.stringify({ commands: { test: "x" }, targets: {} }))'
        let mut commands = Value::object();
        commands.insert("test", "x");
        let mut obj = Value::object();
        obj.insert("commands", commands);
        obj.insert("targets", Value::object());
        assert_eq!(
            stringify_compact(&obj),
            "{\"commands\":{\"test\":\"x\"},\"targets\":{}}"
        );

        // node -e 'console.log(JSON.stringify({ a: [1,2], b: {} }))'
        let mut obj2 = Value::object();
        obj2.insert("a", vec![1i64, 2i64]);
        obj2.insert("b", Value::object());
        assert_eq!(stringify_compact(&obj2), "{\"a\":[1,2],\"b\":{}}");
    }
}
