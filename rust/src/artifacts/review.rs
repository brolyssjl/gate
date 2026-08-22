//! Port of `src/artifacts/review.ts` (`parseReviewFile`, `parseReview`,
//! `serializeReview`, `blockingFindings`, `unjustifiedWaivers`). Ported in
//! wave 3 (see `docs/rust-port.md`), built on `core::yaml::stringify_yaml`
//! (wave 1, already pinned against this exact review-packet payload shape -
//! see its module doc comment and tests).

use std::fs;
use std::path::Path;

use crate::artifacts::frontmatter::{split_frontmatter, FrontmatterResult};
use crate::core::yaml::{stringify_yaml, YamlValue};

/// Severity ordering matters: `blocker` and `major` are *load-bearing* - the
/// REVIEW gate refuses to advance while any is still open. `minor`/`nit` are
/// advisory and never block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Blocker,
    Major,
    Minor,
    Nit,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Blocker => "blocker",
            Severity::Major => "major",
            Severity::Minor => "minor",
            Severity::Nit => "nit",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<Severity> {
        match value {
            "blocker" => Some(Severity::Blocker),
            "major" => Some(Severity::Major),
            "minor" => Some(Severity::Minor),
            "nit" => Some(Severity::Nit),
            _ => None,
        }
    }
}

pub const SEVERITIES: &str = "blocker/major/minor/nit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingStatus {
    Open,
    Resolved,
    Waived,
}

impl FindingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FindingStatus::Open => "open",
            FindingStatus::Resolved => "resolved",
            FindingStatus::Waived => "waived",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<FindingStatus> {
        match value {
            "open" => Some(FindingStatus::Open),
            "resolved" => Some(FindingStatus::Resolved),
            "waived" => Some(FindingStatus::Waived),
            _ => None,
        }
    }
}

pub const STATUSES: &str = "open/resolved/waived";

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub status: FindingStatus,
    pub note: String,
    /// Human waiver rationale; required when status is `waived`.
    pub waiver: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    /// Reviewer identity as recorded in the findings file (advisory; run.json
    /// is authoritative).
    pub reviewer: String,
    pub findings: Vec<Finding>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewParse {
    pub review: Option<Review>,
    pub errors: Vec<String>,
}

pub fn parse_review_file(path: &Path) -> ReviewParse {
    if !path.exists() {
        return ReviewParse {
            review: None,
            errors: vec!["review.md does not exist".to_string()],
        };
    }
    match fs::read_to_string(path) {
        Ok(raw) => parse_review(&raw),
        Err(_) => ReviewParse {
            review: None,
            errors: vec!["review.md does not exist".to_string()],
        },
    }
}

fn get_str(data: &YamlValue, key: &str) -> String {
    data.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn parse_review(raw: &str) -> ReviewParse {
    let split = match split_frontmatter(raw, "review.md") {
        FrontmatterResult::Err(errors) => {
            return ReviewParse {
                review: None,
                errors,
            };
        }
        FrontmatterResult::Ok(s) => s,
    };
    let data = &split.data;
    let mut errors: Vec<String> = Vec::new();

    let reviewer = get_str(data, "reviewer");

    let mut findings: Vec<Finding> = Vec::new();
    let raw_findings: &[YamlValue] = match data.get("findings") {
        Some(YamlValue::Array(items)) => items.as_slice(),
        _ => &[],
    };
    let mut seen_ids: Vec<String> = Vec::new();
    for (i, f) in raw_findings.iter().enumerate() {
        if f.as_map().is_none() {
            errors.push(format!(
                "findings[{i}] must be a mapping with id, severity, status"
            ));
            continue;
        }
        let id = get_str(f, "id");
        let severity_raw = get_str(f, "severity");
        let status_raw = get_str(f, "status");
        let note = get_str(f, "note");
        let waiver = get_str(f, "waiver");
        if id.is_empty() {
            errors.push(format!("findings[{i}].id is required"));
        } else if seen_ids.contains(&id) {
            errors.push(format!("findings[{i}].id \"{id}\" is duplicated"));
        } else {
            seen_ids.push(id.clone());
        }
        let severity = Severity::from_str_opt(&severity_raw);
        if severity.is_none() {
            let label = if id.is_empty() {
                i.to_string()
            } else {
                id.clone()
            };
            errors.push(format!(
                "findings[{label}].severity must be one of {SEVERITIES}"
            ));
        }
        let status = FindingStatus::from_str_opt(&status_raw);
        if status.is_none() {
            let label = if id.is_empty() {
                i.to_string()
            } else {
                id.clone()
            };
            errors.push(format!(
                "findings[{label}].status must be one of {STATUSES}"
            ));
        }
        if !id.is_empty() {
            if let (Some(severity), Some(status)) = (severity, status) {
                findings.push(Finding {
                    id,
                    severity,
                    status,
                    note,
                    waiver,
                });
            }
        }
    }

    if !errors.is_empty() {
        return ReviewParse {
            review: None,
            errors,
        };
    }
    ReviewParse {
        review: Some(Review {
            reviewer,
            findings,
            body: split.body,
        }),
        errors: Vec::new(),
    }
}

/// Findings that block advancement: blocker/major that are still open.
pub fn blocking_findings(review: &Review) -> Vec<&Finding> {
    review
        .findings
        .iter()
        .filter(|f| {
            matches!(f.severity, Severity::Blocker | Severity::Major)
                && f.status == FindingStatus::Open
        })
        .collect()
}

/// Waived blocker/major findings missing the required human rationale.
pub fn unjustified_waivers(review: &Review) -> Vec<&Finding> {
    review
        .findings
        .iter()
        .filter(|f| {
            matches!(f.severity, Severity::Blocker | Severity::Major)
                && f.status == FindingStatus::Waived
                && f.waiver.is_empty()
        })
        .collect()
}

/// Render review.md from a reviewer + findings list (Milestone 4: `gate
/// review --human` writes this after walking the rubric interactively) - the
/// same shape the REVIEW gate parses via `parse_review_file`, so a
/// human-recorded review satisfies it exactly like an agent-edited one.
pub fn serialize_review(reviewer: &str, findings: &[Finding]) -> String {
    let mut findings_value = Vec::with_capacity(findings.len());
    for f in findings {
        let mut entries = vec![
            ("id".to_string(), YamlValue::String(f.id.clone())),
            (
                "severity".to_string(),
                YamlValue::String(f.severity.as_str().to_string()),
            ),
            (
                "status".to_string(),
                YamlValue::String(f.status.as_str().to_string()),
            ),
            ("note".to_string(), YamlValue::String(f.note.clone())),
        ];
        if f.status == FindingStatus::Waived {
            entries.push(("waiver".to_string(), YamlValue::String(f.waiver.clone())));
        }
        findings_value.push(YamlValue::Map(entries));
    }
    let value = YamlValue::Map(vec![
        (
            "reviewer".to_string(),
            YamlValue::String(reviewer.to_string()),
        ),
        ("findings".to_string(), YamlValue::Array(findings_value)),
    ]);
    let frontmatter = stringify_yaml(&value);
    let frontmatter = frontmatter.trim_end();
    format!("---\n{frontmatter}\n---\n\n# Review\n\nRecorded via `gate review --human`.\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_review() {
        let raw = "---\nreviewer: rev-1\nfindings:\n  - id: f1\n    severity: blocker\n    status: open\n    note: bad\n---\n# Review";
        let ReviewParse { review, errors } = parse_review(raw);
        assert_eq!(errors, Vec::<String>::new());
        let review = review.unwrap();
        assert_eq!(review.reviewer, "rev-1");
        assert_eq!(review.findings.len(), 1);
        assert_eq!(review.findings[0].severity, Severity::Blocker);
    }

    #[test]
    fn blocking_findings_returns_only_open_blocker_or_major() {
        let raw = "---\nreviewer: r\nfindings:\n  - id: f1\n    severity: blocker\n    status: open\n    note: a\n  - id: f2\n    severity: minor\n    status: open\n    note: b\n  - id: f3\n    severity: major\n    status: resolved\n    note: c\n---\n";
        let review = parse_review(raw).review.unwrap();
        let blocking = blocking_findings(&review);
        assert_eq!(blocking.len(), 1);
        assert_eq!(blocking[0].id, "f1");
    }

    #[test]
    fn unjustified_waivers_flags_a_waived_major_with_no_rationale() {
        let raw = "---\nreviewer: r\nfindings:\n  - id: f1\n    severity: major\n    status: waived\n    note: n\n---\n";
        let review = parse_review(raw).review.unwrap();
        assert_eq!(unjustified_waivers(&review).len(), 1);
    }

    #[test]
    fn a_waived_major_with_a_rationale_is_not_unjustified() {
        let raw = "---\nreviewer: r\nfindings:\n  - id: f1\n    severity: major\n    status: waived\n    note: n\n    waiver: \"accepted for now\"\n---\n";
        let review = parse_review(raw).review.unwrap();
        assert_eq!(unjustified_waivers(&review).len(), 0);
    }

    #[test]
    fn rejects_an_unknown_severity_or_status() {
        let raw = "---\nreviewer: r\nfindings:\n  - id: f1\n    severity: catastrophic\n    status: open\n    note: n\n---\n";
        let ReviewParse { errors, .. } = parse_review(raw);
        assert!(errors.iter().any(|e| e.contains("severity must be one of")));
    }

    #[test]
    fn serialize_review_matches_the_real_yaml_packages_output() {
        // node --input-type=module -e 'import { serializeReview } from
        // "./dist/artifacts/review.js"; console.log(JSON.stringify(
        // serializeReview("alice", [...])))'
        let findings = vec![
            Finding {
                id: "f1".to_string(),
                severity: Severity::Blocker,
                status: FindingStatus::Open,
                note: "resetToken is compared with == not a constant-time check".to_string(),
                waiver: String::new(),
            },
            Finding {
                id: "f2".to_string(),
                severity: Severity::Minor,
                status: FindingStatus::Resolved,
                note: "Extract the retry loop".to_string(),
                waiver: String::new(),
            },
            Finding {
                id: "f3".to_string(),
                severity: Severity::Major,
                status: FindingStatus::Waived,
                note: "n".to_string(),
                waiver: "accepted for now".to_string(),
            },
        ];
        let out = serialize_review("alice", &findings);
        let expected = "---\nreviewer: alice\nfindings:\n  - id: f1\n    severity: blocker\n    status: open\n    note: resetToken is compared with == not a constant-time check\n  - id: f2\n    severity: minor\n    status: resolved\n    note: Extract the retry loop\n  - id: f3\n    severity: major\n    status: waived\n    note: n\n    waiver: accepted for now\n---\n\n# Review\n\nRecorded via `gate review --human`.\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn serialize_review_round_trips_through_parse_review() {
        let findings = vec![Finding {
            id: "f1".to_string(),
            severity: Severity::Nit,
            status: FindingStatus::Resolved,
            note: "cosmetic".to_string(),
            waiver: String::new(),
        }];
        let out = serialize_review("bob", &findings);
        let review = parse_review(&out).review.unwrap();
        assert_eq!(review.reviewer, "bob");
        assert_eq!(review.findings, findings);
    }

    #[test]
    fn parse_review_file_reports_missing_file() {
        let ReviewParse { review, errors } = parse_review_file(Path::new("/nonexistent/review.md"));
        assert!(review.is_none());
        assert_eq!(errors, vec!["review.md does not exist".to_string()]);
    }
}
