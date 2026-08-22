//! Port of `src/gates/testReport.ts` (`parseTestReport`, `NormalizedReport`).
//! Ported in wave 3 (see `docs/rust-port.md`).

use crate::core::json::{self, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

impl TestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestStatus::Passed => "passed",
            TestStatus::Failed => "failed",
            TestStatus::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedTest {
    pub name: String,
    pub status: TestStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedReport {
    pub tests: Vec<NormalizedTest>,
    pub skipped: usize,
}

fn map_status(s: Option<&str>) -> TestStatus {
    match s {
        Some("passed") => TestStatus::Passed,
        Some("failed") => TestStatus::Failed,
        Some("pending") | Some("skipped") | Some("todo") | Some("disabled") => TestStatus::Skipped,
        _ => TestStatus::Failed,
    }
}

/// Extract a JSON object/array from possibly-noisy output. Test runners
/// invoked through `npm test` prefix the report with banner lines (`>
/// pkg@1 test`), so a plain parse of stdout fails. Try the whole string,
/// then the outermost brace/bracket slice, then each line.
fn extract_json(raw: &str) -> Option<Value> {
    let t = raw.trim();
    let mut attempts: Vec<&str> = vec![t];

    let obj_start = t.find('{');
    let obj_end = t.rfind('}');
    if let (Some(s), Some(e)) = (obj_start, obj_end) {
        if e > s {
            attempts.push(&t[s..=e]);
        }
    }
    let arr_start = t.find('[');
    let arr_end = t.rfind(']');
    if let (Some(s), Some(e)) = (arr_start, arr_end) {
        if e > s {
            attempts.push(&t[s..=e]);
        }
    }
    for line in t.split('\n') {
        let l = line.trim();
        if l.starts_with('{') || l.starts_with('[') {
            attempts.push(l);
        }
    }

    for candidate in attempts {
        if let Ok(v) = json::parse(candidate) {
            return Some(v);
        }
    }
    None
}

/// Normalize a test report from either the jest/vitest JSON shape or the
/// generic contract `{ tests: [{ name, status }] }`. Returns `None` if
/// unparseable.
pub fn parse_test_report(raw: &str) -> Option<NormalizedReport> {
    let data = extract_json(raw)?;
    data.as_object()?; // must be an object, not e.g. a bare array or scalar

    // jest / vitest --reporter=json
    if let Some(Value::Array(suites)) = data.get("testResults") {
        let mut tests = Vec::new();
        for suite in suites {
            if let Some(Value::Array(assertions)) = suite.get("assertionResults") {
                for a in assertions {
                    let name = match a.get("fullName").and_then(|v| v.as_str()) {
                        Some(s) if !s.trim().is_empty() => s.to_string(),
                        _ => a
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    };
                    let status = map_status(a.get("status").and_then(|v| v.as_str()));
                    tests.push(NormalizedTest { name, status });
                }
            }
        }
        let skipped = tests
            .iter()
            .filter(|t| t.status == TestStatus::Skipped)
            .count();
        return Some(NormalizedReport { tests, skipped });
    }

    // generic contract
    if let Some(Value::Array(items)) = data.get("tests") {
        let mut tests = Vec::new();
        for t in items {
            let Some(name) = t.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let status = map_status(t.get("status").and_then(|v| v.as_str()));
            tests.push(NormalizedTest {
                name: name.to_string(),
                status,
            });
        }
        let skipped = tests
            .iter()
            .filter(|t| t.status == TestStatus::Skipped)
            .count();
        return Some(NormalizedReport { tests, skipped });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_generic_contract() {
        let raw = r#"{"tests":[{"name":"a","status":"passed"},{"name":"b","status":"failed"}]}"#;
        let report = parse_test_report(raw).unwrap();
        assert_eq!(report.tests.len(), 2);
        assert_eq!(report.tests[0].status, TestStatus::Passed);
        assert_eq!(report.tests[1].status, TestStatus::Failed);
        assert_eq!(report.skipped, 0);
    }

    #[test]
    fn parses_the_jest_vitest_shape() {
        let raw = r#"{"testResults":[{"assertionResults":[{"fullName":"suite > does a thing","status":"passed"},{"title":"pending one","status":"pending"}]}]}"#;
        let report = parse_test_report(raw).unwrap();
        assert_eq!(report.tests.len(), 2);
        assert_eq!(report.tests[0].name, "suite > does a thing");
        assert_eq!(report.tests[1].name, "pending one");
        assert_eq!(report.tests[1].status, TestStatus::Skipped);
        assert_eq!(report.skipped, 1);
    }

    #[test]
    fn maps_todo_and_disabled_to_skipped() {
        let raw = r#"{"tests":[{"name":"a","status":"todo"},{"name":"b","status":"disabled"}]}"#;
        let report = parse_test_report(raw).unwrap();
        assert_eq!(report.skipped, 2);
    }

    #[test]
    fn maps_an_unknown_status_to_failed() {
        let raw = r#"{"tests":[{"name":"a","status":"weird"}]}"#;
        let report = parse_test_report(raw).unwrap();
        assert_eq!(report.tests[0].status, TestStatus::Failed);
    }

    #[test]
    fn extracts_json_from_noisy_npm_banner_output() {
        let raw = "> pkg@1 test\n> node test.js\n\n{\"tests\":[{\"name\":\"a\",\"status\":\"passed\"}]}\n";
        let report = parse_test_report(raw).unwrap();
        assert_eq!(report.tests.len(), 1);
    }

    #[test]
    fn returns_none_for_unparseable_input() {
        assert!(parse_test_report("not json at all").is_none());
    }

    #[test]
    fn returns_none_when_neither_known_shape_matches() {
        assert!(parse_test_report(r#"{"unrelated":true}"#).is_none());
    }
}
