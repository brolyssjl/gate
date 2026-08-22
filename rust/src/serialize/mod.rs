//! Port of `src/serialize/index.ts`: the structured-output format facade.

pub mod toon;

use crate::core::json::{stringify_pretty, Value};
use toon::encode_toon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Toon,
}

pub fn is_format(value: &str) -> bool {
    value == "json" || value == "toon"
}

pub fn parse_format(value: &str) -> Option<Format> {
    match value {
        "json" => Some(Format::Json),
        "toon" => Some(Format::Toon),
        _ => None,
    }
}

/// Render agent-facing structured output. JSON is the default and the only
/// format CI should rely on. TOON is opt-in for token savings on uniform,
/// tabular payloads (findings/lessons lists) and worse on small objects, so
/// it is never the default (kickoff decision).
pub fn serialize(value: &Value, format: Format) -> String {
    match format {
        Format::Toon => encode_toon(value),
        Format::Json => stringify_pretty(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_selected_via_the_serializer_facade() {
        let mut v = Value::object();
        v.insert("n", 1i64);
        assert_eq!(serialize(&v, Format::Toon), "n: 1");
        assert_eq!(serialize(&v, Format::Json), "{\n  \"n\": 1\n}");
    }

    #[test]
    fn is_format_recognizes_json_and_toon_only() {
        assert!(is_format("json"));
        assert!(is_format("toon"));
        assert!(!is_format("yaml"));
    }
}
