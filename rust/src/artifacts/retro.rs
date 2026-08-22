//! Port of `src/artifacts/retro.ts` (`parseRetroFile`, `parseRetro`,
//! `RETRO_TEMPLATE`, `hasSubstance`). Ported in wave 3 (see
//! `docs/rust-port.md`).

use std::fs;
use std::path::Path;

use crate::artifacts::frontmatter::{split_frontmatter, FrontmatterResult};
use crate::core::yaml::YamlValue;

/// The RETRO phase asks three questions (proposal §3): what broke, what to
/// avoid next time, what convention emerged. All three lists may be empty
/// individually - `has_substance` is the gate's real bar: at least one
/// answer across the three, so a run can't coast through on an untouched
/// scaffold.
#[derive(Debug, Clone, PartialEq)]
pub struct RetroLog {
    pub broke: Vec<String>,
    pub avoid: Vec<String>,
    pub conventions: Vec<String>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetroParse {
    pub retro: Option<RetroLog>,
    pub errors: Vec<String>,
}

/// The scaffold `gate start` writes for a run whose profile walks through
/// RETRO, and that `advance()` also writes when a pre-Milestone-3 run
/// (started before RETRO scaffolding existed) transitions into RETRO and
/// finds no retro.md waiting - otherwise that run stalls forever with no way
/// to satisfy the RETRO gate. `%TITLE%` is replaced by the caller.
pub const RETRO_TEMPLATE: &str =
    "---\nbroke: []\navoid: []\nconventions: []\n---\n\n# Retro: %TITLE%\n\n";

pub fn parse_retro_file(path: &Path) -> RetroParse {
    if !path.exists() {
        return RetroParse {
            retro: None,
            errors: vec!["retro.md does not exist".to_string()],
        };
    }
    match fs::read_to_string(path) {
        Ok(raw) => parse_retro(&raw),
        Err(_) => RetroParse {
            retro: None,
            errors: vec!["retro.md does not exist".to_string()],
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

pub fn parse_retro(raw: &str) -> RetroParse {
    let split = match split_frontmatter(raw, "retro.md") {
        FrontmatterResult::Err(errors) => {
            return RetroParse {
                retro: None,
                errors,
            };
        }
        FrontmatterResult::Ok(s) => s,
    };
    let data = &split.data;

    let broke = as_string_array(data.get("broke"));
    let avoid = as_string_array(data.get("avoid"));
    let conventions = as_string_array(data.get("conventions"));

    RetroParse {
        retro: Some(RetroLog {
            broke,
            avoid,
            conventions,
            body: split.body,
        }),
        errors: Vec::new(),
    }
}

/// At least one of the three questions was actually answered.
pub fn has_substance(retro: &RetroLog) -> bool {
    !retro.broke.is_empty() || !retro.avoid.is_empty() || !retro.conventions.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_retro() {
        let raw =
            "---\nbroke: []\navoid:\n  - \"Do not skip reproduce\"\nconventions: []\n---\n# Retro";
        let RetroParse { retro, errors } = parse_retro(raw);
        assert_eq!(errors, Vec::<String>::new());
        let retro = retro.unwrap();
        assert_eq!(retro.avoid, vec!["Do not skip reproduce".to_string()]);
        assert!(has_substance(&retro));
    }

    #[test]
    fn an_all_empty_retro_has_no_substance() {
        let raw = "---\nbroke: []\navoid: []\nconventions: []\n---\n# Retro";
        let retro = parse_retro(raw).retro.unwrap();
        assert!(!has_substance(&retro));
    }

    #[test]
    fn rejects_missing_frontmatter() {
        let RetroParse { retro, errors } = parse_retro("# just markdown");
        assert!(retro.is_none());
        assert!(errors[0].contains("frontmatter"));
    }

    #[test]
    fn parse_retro_file_reports_missing_file() {
        let RetroParse { retro, errors } = parse_retro_file(Path::new("/nonexistent/retro.md"));
        assert!(retro.is_none());
        assert_eq!(errors, vec!["retro.md does not exist".to_string()]);
    }

    #[test]
    fn retro_template_has_the_expected_scaffold_shape() {
        assert!(RETRO_TEMPLATE.starts_with("---\nbroke: []\n"));
        assert!(RETRO_TEMPLATE.contains("%TITLE%"));
    }
}
