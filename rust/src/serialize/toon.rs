//! Port of `src/serialize/toon.ts`: minimal TOON (Token-Oriented Object
//! Notation) encoder.
//!
//! Scope (kickoff decision "uniform arrays only"): the token win comes
//! entirely from rendering uniform arrays of flat objects as a header +
//! comma rows. Other shapes fall back to a readable, indented, YAML-like
//! form. This is an encoder only; Gate never needs to parse TOON back (JSON
//! remains the durable format).
//!
//! Operates on `core::json::Value` (gate's structured-output value type)
//! rather than a separate JSON type - see `serialize/mod.rs`.

use crate::core::json::{format_float, Value};

const INDENT: &str = "  ";

pub fn encode_toon(value: &Value) -> String {
    if is_primitive(value) {
        return encode_primitive(value);
    }
    match value {
        Value::Array(items) => encode_array("", items, 0).trim_start().to_string(),
        Value::Object(entries) => encode_object(entries, 0),
        _ => unreachable!(),
    }
}

fn is_primitive(v: &Value) -> bool {
    !matches!(v, Value::Array(_) | Value::Object(_))
}

fn encode_object(obj: &[(String, Value)], depth: usize) -> String {
    let pad = INDENT.repeat(depth);
    let mut lines: Vec<String> = Vec::new();
    for (key, val) in obj {
        if is_primitive(val) {
            lines.push(format!(
                "{pad}{}: {}",
                encode_key(key),
                encode_primitive(val)
            ));
        } else if let Value::Array(items) = val {
            lines.push(encode_array(key, items, depth));
        } else if let Value::Object(sub) = val {
            lines.push(format!("{pad}{}:", encode_key(key)));
            lines.push(encode_object(sub, depth + 1));
        }
    }
    lines.join("\n")
}

fn encode_array(key: &str, arr: &[Value], depth: usize) -> String {
    let pad = INDENT.repeat(depth);
    let label = if key.is_empty() {
        String::new()
    } else {
        encode_key(key)
    };

    if arr.is_empty() {
        return format!("{pad}{label}[0]:");
    }

    if arr.iter().all(is_primitive) {
        let joined = arr
            .iter()
            .map(encode_primitive)
            .collect::<Vec<_>>()
            .join(",");
        return format!("{pad}{label}[{}]: {joined}", arr.len());
    }

    if let Some(table) = as_uniform_table(arr) {
        let header = format!(
            "{pad}{label}[{}]{{{}}}:",
            arr.len(),
            table
                .fields
                .iter()
                .map(|f| encode_key(f))
                .collect::<Vec<_>>()
                .join(",")
        );
        let rows: Vec<String> = table
            .rows
            .iter()
            .map(|row| {
                format!(
                    "{}{}",
                    INDENT.repeat(depth + 1),
                    row.iter()
                        .map(|v| encode_primitive(v))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect();
        let mut out = vec![header];
        out.extend(rows);
        return out.join("\n");
    }

    let header = format!("{pad}{label}[{}]:", arr.len());
    let items: Vec<String> = arr
        .iter()
        .map(|item| {
            let body = if is_primitive(item) {
                encode_primitive(item)
            } else if let Value::Array(sub) = item {
                encode_array("", sub, depth + 2).trim_start().to_string()
            } else if let Value::Object(sub) = item {
                format!("\n{}", encode_object(sub, depth + 2))
            } else {
                unreachable!()
            };
            let body = body.strip_prefix('\n').unwrap_or(&body);
            format!("{}- {body}", INDENT.repeat(depth + 1))
        })
        .collect();
    let mut out = vec![header];
    out.extend(items);
    out.join("\n")
}

struct Table<'a> {
    fields: Vec<&'a str>,
    rows: Vec<Vec<&'a Value>>,
}

fn as_uniform_table(arr: &[Value]) -> Option<Table<'_>> {
    let first = arr.first()?;
    let Value::Object(first_entries) = first else {
        return None;
    };
    let fields: Vec<&str> = first_entries.iter().map(|(k, _)| k.as_str()).collect();
    if fields.is_empty() {
        return None;
    }
    let mut rows = Vec::with_capacity(arr.len());
    for item in arr {
        let Value::Object(entries) = item else {
            return None;
        };
        if entries.len() != fields.len() {
            return None;
        }
        let mut row = Vec::with_capacity(fields.len());
        for f in &fields {
            let cell = entries.iter().find(|(k, _)| k == f).map(|(_, v)| v)?;
            if !is_primitive(cell) {
                return None; // nested values disqualify the table form
            }
            row.push(cell);
        }
        rows.push(row);
    }
    Some(Table { fields, rows })
}

fn encode_primitive(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => {
            if f.is_finite() {
                format_float(*f)
            } else {
                "null".to_string()
            }
        }
        Value::String(s) => {
            if needs_quote(s) {
                quote(s)
            } else {
                s.clone()
            }
        }
        _ => "null".to_string(), // non-primitive cannot reach here in practice
    }
}

fn encode_key(key: &str) -> String {
    if needs_quote(key) {
        quote(key)
    } else {
        key.to_string()
    }
}

fn needs_quote(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if s.chars().any(|c| ",:{}[]\"\n".contains(c)) {
        return true;
    }
    if s.trim() != s {
        return true;
    }
    if matches!(s, "null" | "true" | "false") {
        return true;
    }
    let rest = s.strip_prefix('-').unwrap_or(s);
    if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return true;
    }
    false
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(pairs: Vec<(&str, Value)>) -> Value {
        Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn encodes_primitives() {
        assert_eq!(encode_toon(&Value::Int(42)), "42");
        assert_eq!(encode_toon(&Value::Bool(true)), "true");
        assert_eq!(encode_toon(&Value::Null), "null");
    }

    #[test]
    fn encodes_a_flat_object_as_key_value_lines() {
        let v = obj(vec![("phase", "PLAN".into()), ("ok", Value::Bool(false))]);
        assert_eq!(encode_toon(&v), "phase: PLAN\nok: false");
    }

    #[test]
    fn renders_a_uniform_array_of_flat_objects_as_a_table() {
        let v = obj(vec![(
            "checks",
            Value::Array(vec![
                obj(vec![("name", "a".into()), ("ok", Value::Bool(true))]),
                obj(vec![("name", "b".into()), ("ok", Value::Bool(false))]),
            ]),
        )]);
        assert_eq!(encode_toon(&v), "checks[2]{name,ok}:\n  a,true\n  b,false");
    }

    #[test]
    fn inlines_a_primitive_array() {
        let v = obj(vec![(
            "files",
            Value::Array(vec!["a.ts".into(), "b.ts".into()]),
        )]);
        assert_eq!(encode_toon(&v), "files[2]: a.ts,b.ts");
    }

    #[test]
    fn renders_an_empty_array_with_a_zero_count() {
        let v = obj(vec![("risks", Value::Array(vec![]))]);
        assert_eq!(encode_toon(&v), "risks[0]:");
    }

    #[test]
    fn falls_back_to_list_form_for_non_uniform_arrays() {
        let v = obj(vec![(
            "items",
            Value::Array(vec![
                obj(vec![("a", Value::Int(1))]),
                obj(vec![("a", Value::Int(1)), ("b", Value::Int(2))]),
            ]),
        )]);
        let out = encode_toon(&v);
        assert!(out.contains("items[2]:"));
        assert!(out.contains("- "));
    }

    #[test]
    fn quotes_values_containing_separators() {
        let v = obj(vec![("msg", "a, b: c".into())]);
        assert_eq!(encode_toon(&v), "msg: \"a, b: c\"");
    }
}
