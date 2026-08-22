//! Port of `src/integrations/advise.ts` (`adviseReportPath`,
//! `parseAdviseReport`, `loadAdviseReport`). Ported in wave 3 (see
//! `docs/rust-port.md`).
//!
//! Parser for Agnosgram's `agnosgram_advise` report schema (pinned
//! cross-project contract, owned by Agnosgram). Agnosgram writes the report
//! to `<plan>.advise.json` by default. Gate only *reads* this file - it
//! never shells out to `agnosgram advise` itself (integrations are
//! advisory, never load-bearing): a missing or unparseable report is
//! treated as "no report", never as an error.

use std::fs;
use std::path::Path;

use crate::core::json::{self, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct AdviseContradiction {
    pub record_id: String,
    pub kind: String,
    pub severity: String,
    pub plan_excerpt: String,
    pub record_excerpt: String,
    pub confidence: String,
    pub last_verified: String,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdviseReport {
    pub version: i64,
    pub plan: String,
    pub generated: String,
    pub checked_ids: Vec<String>,
    pub contradictions: Vec<AdviseContradiction>,
    pub clear: bool,
}

/// Where `agnosgram advise` writes its report for a given plan.md path.
pub fn advise_report_path(plan_path: &str) -> String {
    format!("{plan_path}.advise.json")
}

/// `v` as a string if it's one, else `""` - the report's fields are all
/// optional/untrusted input.
fn str(v: Option<&Value>) -> String {
    v.and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Int(n) => Some(*n),
        Value::Float(f) => Some(*f as i64),
        _ => None,
    }
}

/// Parse the pinned `agnosgram_advise` schema. Tolerant of extra fields
/// (forward compatibility); returns `None` on anything that doesn't look
/// like a report at all - Gate treats that identically to a missing file.
pub fn parse_advise_report(raw: &str) -> Option<AdviseReport> {
    let data = json::parse(raw).ok()?;
    let obj = data.as_object()?;
    let version = obj
        .iter()
        .find(|(k, _)| k == "agnosgram_advise")
        .map(|(_, v)| v)
        .and_then(as_i64)?;

    let mut contradictions = Vec::new();
    if let Some(Value::Array(items)) = data.get("contradictions") {
        for c in items {
            let Some(_) = c.as_object() else { continue };
            let record_id = str(c.get("record_id"));
            if record_id.is_empty() {
                continue;
            }
            contradictions.push(AdviseContradiction {
                record_id,
                kind: str(c.get("kind")),
                severity: str(c.get("severity")),
                plan_excerpt: str(c.get("plan_excerpt")),
                record_excerpt: str(c.get("record_excerpt")),
                confidence: str(c.get("confidence")),
                last_verified: str(c.get("last_verified")),
                explanation: str(c.get("explanation")),
            });
        }
    }

    let checked_ids = match data.get("checked_ids") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    };

    Some(AdviseReport {
        version,
        plan: str(data.get("plan")),
        generated: str(data.get("generated")),
        checked_ids,
        contradictions,
        clear: matches!(data.get("clear"), Some(Value::Bool(true))),
    })
}

/// Load and parse the advise report for `plan_path`; `None` when absent or
/// unparseable ("no report").
pub fn load_advise_report(plan_path: &str) -> Option<AdviseReport> {
    let path = advise_report_path(plan_path);
    if !Path::new(&path).exists() {
        return None;
    }
    let raw = fs::read_to_string(&path).ok()?;
    parse_advise_report(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{"agnosgram_advise":1,"plan":".gate/runs/r1/plan.md","generated":"2026-07-27","checked_ids":["LES-001","LES-002"],"contradictions":[{"record_id":"LES-002","kind":"empirical","severity":"blocker","plan_excerpt":"retry on 500","record_excerpt":"retries caused double-charges","confidence":"high","last_verified":"2026-07-20","explanation":"the plan reintroduces a fix that already failed"}],"clear":false}"#;

    #[test]
    fn parses_a_well_formed_report() {
        let report = parse_advise_report(VALID).unwrap();
        assert_eq!(report.version, 1);
        assert_eq!(report.contradictions.len(), 1);
        assert_eq!(report.contradictions[0].record_id, "LES-002");
        assert!(!report.clear);
    }

    #[test]
    fn tolerates_unknown_extra_fields() {
        let mut value = json::parse(VALID).unwrap();
        if let Value::Object(entries) = &mut value {
            entries.push((
                "future_field".to_string(),
                Value::String("whatever".to_string()),
            ));
        }
        let with_extra = json::stringify_compact(&value);
        let report = parse_advise_report(&with_extra).unwrap();
        assert_eq!(report.contradictions.len(), 1);
    }

    #[test]
    fn returns_none_for_json_missing_the_version_tag() {
        assert!(parse_advise_report(r#"{"contradictions":[]}"#).is_none());
    }

    #[test]
    fn returns_none_for_unparseable_json() {
        assert!(parse_advise_report("not json").is_none());
    }

    #[test]
    fn returns_a_report_for_a_clear_report_with_no_contradictions_array_required() {
        let report = parse_advise_report(r#"{"agnosgram_advise":1,"clear":true}"#).unwrap();
        assert!(report.clear);
        assert_eq!(report.contradictions, Vec::<AdviseContradiction>::new());
    }

    #[test]
    fn derives_the_report_path_as_plan_dot_advise_dot_json() {
        assert_eq!(
            advise_report_path(".gate/runs/r1/plan.md"),
            ".gate/runs/r1/plan.md.advise.json"
        );
    }

    #[test]
    fn load_advise_report_returns_none_when_the_file_does_not_exist() {
        assert!(load_advise_report("/nonexistent/plan.md").is_none());
    }
}
