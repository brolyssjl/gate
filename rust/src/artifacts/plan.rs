//! Port of `src/artifacts/plan.ts` (`parsePlanFile`, `parsePlan`, `Plan`,
//! `Criterion`, `hashPlan`, `hashPlanFile`). Ported in wave 3 (see
//! `docs/rust-port.md`). `core::targets::resolve_display_targets` depends
//! on `parse_plan_file` and was deferred until this module landed - see the
//! TODO at the bottom of `core/targets.rs` (now filled in).

use std::fs;
use std::path::Path;

use crate::artifacts::frontmatter::{split_frontmatter, FrontmatterResult};
use crate::core::sha256::sha256_prefixed;
use crate::core::yaml::YamlValue;

#[derive(Debug, Clone, PartialEq)]
pub struct Criterion {
    pub id: String,
    pub text: String,
    /// How the criterion is verified. Mechanically meaningful prefixes:
    ///   `test: <substring>` - a named test whose title contains <substring>
    ///                          must run and pass (checked by the TEST gate).
    ///   `manual`            - verified by a human; the TEST gate accepts it
    ///                          as-is.
    /// Any other value is treated as free-form and satisfies "checkable"
    /// only by being non-empty (a verification method was declared).
    pub verify: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub goal: String,
    /// Repo-relative path to an SDD spec this plan cites, e.g.
    /// "openspec/changes/x/spec.md".
    pub spec: Option<String>,
    pub files: Vec<String>,
    pub out_of_scope: Vec<String>,
    pub criteria: Vec<Criterion>,
    pub risks: Vec<String>,
    /// Record ids (e.g. "LES-002") from an agnosgram advise report the
    /// author has acknowledged.
    pub acknowledgments: Vec<String>,
    /// Free-form markdown body (may cite an SDD spec path).
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanParse {
    pub plan: Option<Plan>,
    pub errors: Vec<String>,
}

pub fn parse_plan_file(path: &Path) -> PlanParse {
    if !path.exists() {
        return PlanParse {
            plan: None,
            errors: vec!["plan.md does not exist".to_string()],
        };
    }
    match fs::read_to_string(path) {
        Ok(raw) => parse_plan(&raw),
        Err(_) => PlanParse {
            plan: None,
            errors: vec!["plan.md does not exist".to_string()],
        },
    }
}

fn as_string_array(v: Option<&YamlValue>) -> Vec<String> {
    let Some(YamlValue::Array(items)) = v else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|x| x.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn get_str(data: &YamlValue, key: &str) -> String {
    data.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn parse_plan(raw: &str) -> PlanParse {
    let split = match split_frontmatter(raw, "plan.md") {
        FrontmatterResult::Err(errors) => {
            return PlanParse { plan: None, errors };
        }
        FrontmatterResult::Ok(s) => s,
    };
    let data = &split.data;
    let mut errors: Vec<String> = Vec::new();

    let goal = get_str(data, "goal");
    if goal.is_empty() {
        errors.push("`goal` is required and must be a non-empty string".to_string());
    }

    let spec_raw = get_str(data, "spec");
    let spec = if spec_raw.is_empty() {
        None
    } else {
        Some(spec_raw)
    };

    let files = as_string_array(data.get("files"));
    if files.is_empty() {
        errors.push("`files` must list at least one path or glob".to_string());
    }

    let out_of_scope = as_string_array(data.get("out_of_scope"));
    let risks = as_string_array(data.get("risks"));
    let acknowledgments = as_string_array(data.get("acknowledgments"));

    let mut criteria: Vec<Criterion> = Vec::new();
    let raw_criteria: &[YamlValue] = match data.get("criteria") {
        Some(YamlValue::Array(items)) => items.as_slice(),
        _ => &[],
    };
    if raw_criteria.is_empty() {
        errors.push("`criteria` must list at least one acceptance criterion".to_string());
    }
    let mut seen_ids: Vec<String> = Vec::new();
    for (i, c) in raw_criteria.iter().enumerate() {
        let Some(_map) = c.as_map() else {
            errors.push(format!(
                "criteria[{i}] must be a mapping with id, text, verify"
            ));
            continue;
        };
        let id = get_str(c, "id");
        let text = get_str(c, "text");
        let verify = get_str(c, "verify");
        if id.is_empty() {
            errors.push(format!("criteria[{i}].id is required"));
        } else if seen_ids.contains(&id) {
            errors.push(format!("criteria[{i}].id \"{id}\" is duplicated"));
        } else {
            seen_ids.push(id.clone());
        }
        if text.is_empty() {
            errors.push(format!("criteria[{i}].text is required"));
        }
        // "each criterion is checkable" is enforced mechanically as: a
        // verification method must be declared (non-empty `verify`).
        if verify.is_empty() {
            let label = if id.is_empty() {
                i.to_string()
            } else {
                id.clone()
            };
            errors.push(format!(
                "criteria[{label}].verify is required (how is this checked?)"
            ));
        }
        if !id.is_empty() && !text.is_empty() && !verify.is_empty() {
            criteria.push(Criterion { id, text, verify });
        }
    }

    if !errors.is_empty() {
        return PlanParse { plan: None, errors };
    }
    PlanParse {
        plan: Some(Plan {
            goal,
            spec,
            files,
            out_of_scope,
            criteria,
            risks,
            acknowledgments,
            body: split.body,
        }),
        errors: Vec::new(),
    }
}

/// Content hash of a plan, binding an approval to the exact plan it approved.
pub fn hash_plan(raw: &str) -> String {
    sha256_prefixed(raw.as_bytes())
}

pub fn hash_plan_file(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    Some(hash_plan(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "---\ngoal: Do the thing.\nfiles:\n  - src/**\ncriteria:\n  - id: c1\n    text: \"does the thing\"\n    verify: \"test: does the thing\"\n---\n# Plan\nbody";

    #[test]
    fn parses_a_valid_plan() {
        let PlanParse { plan, errors } = parse_plan(VALID);
        assert_eq!(errors, Vec::<String>::new());
        let plan = plan.unwrap();
        assert_eq!(plan.goal, "Do the thing.");
        assert_eq!(plan.criteria.len(), 1);
        assert_eq!(plan.files, vec!["src/**".to_string()]);
    }

    #[test]
    fn rejects_missing_frontmatter() {
        let PlanParse { plan, errors } = parse_plan("# just markdown");
        assert!(plan.is_none());
        assert!(errors[0].contains("frontmatter"));
    }

    #[test]
    fn requires_goal_files_and_at_least_one_criterion() {
        let PlanParse { errors, .. } = parse_plan("---\napproved: true\n---\n");
        assert!(errors.iter().any(|e| e.contains("goal")));
        assert!(errors.iter().any(|e| e.contains("files")));
        assert!(errors.iter().any(|e| e.contains("criteria")));
    }

    #[test]
    fn requires_each_criterion_to_declare_a_verify_method() {
        let raw = "---\ngoal: g\nfiles: [a.ts]\ncriteria:\n  - id: c1\n    text: \"x\"\n---";
        let PlanParse { errors, .. } = parse_plan(raw);
        assert!(errors.iter().any(|e| e.contains("verify is required")));
    }

    #[test]
    fn rejects_duplicate_criterion_ids() {
        let raw = "---\ngoal: g\nfiles: [a.ts]\ncriteria:\n  - id: c1\n    text: x\n    verify: manual\n  - id: c1\n    text: y\n    verify: manual\n---";
        let PlanParse { errors, .. } = parse_plan(raw);
        assert!(errors.iter().any(|e| e.contains("duplicated")));
    }

    #[test]
    fn computes_a_content_hash_that_changes_with_the_plan() {
        assert_eq!(hash_plan(VALID), hash_plan(VALID));
        assert_ne!(hash_plan(VALID), hash_plan(&format!("{VALID}\nextra")));
    }

    #[test]
    fn parses_an_optional_spec_citation_defaulting_to_none() {
        assert_eq!(parse_plan(VALID).plan.unwrap().spec, None);
        let with_spec = VALID.replace(
            "goal: Do the thing.",
            "goal: Do the thing.\nspec: openspec/changes/x/spec.md",
        );
        assert_eq!(
            parse_plan(&with_spec).plan.unwrap().spec,
            Some("openspec/changes/x/spec.md".to_string())
        );
    }

    #[test]
    fn hash_plan_matches_a_real_ts_binary_output() {
        // node dist/cli.js init && node dist/cli.js start "My First Run" in a
        // fresh temp git repo, then `hashPlan(readFileSync(plan.md))`:
        let raw = "---\ngoal:\nspec:\nfiles:\n  -\nout_of_scope: []\ncriteria:\n  - id: c1\n    text:\n    verify: \"test: \"\nrisks: []\nacknowledgments: []\n---\n\n# Plan: My First Run\n\n";
        assert_eq!(
            hash_plan(raw),
            "sha256:a33070eaa6dc874c4ed4254a8b757a95d8097077cb840db9b50640e1d2b78a01"
        );
    }

    #[test]
    fn parse_plan_file_reports_missing_file() {
        let PlanParse { plan, errors } = parse_plan_file(Path::new("/nonexistent/plan.md"));
        assert!(plan.is_none());
        assert_eq!(errors, vec!["plan.md does not exist".to_string()]);
    }
}
