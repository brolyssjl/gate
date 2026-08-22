//! Port of `src/artifacts/debugLog.ts` (`parseDebugLogFile`, `parseDebugLog`,
//! `DebugLog`, `DebugCycle`, `isCycleComplete`). Ported in wave 3 (see
//! `docs/rust-port.md`).

use std::fs;
use std::path::Path;

use crate::artifacts::frontmatter::{split_frontmatter, FrontmatterResult};
use crate::core::yaml::YamlValue;

/// A single pass of the debugging protocol: reproduce -> hypothesize ->
/// predict -> test -> conclude. A cycle is *complete* only when every field
/// is filled in and `status: complete`; the DEBUG gate requires at least one
/// complete cycle so the agent can't claim a fix without recording how it
/// got there.
#[derive(Debug, Clone, PartialEq)]
pub struct DebugCycle {
    pub hypothesis: String,
    pub prediction: String,
    pub experiment: String,
    pub observation: String,
    pub conclusion: String,
    pub status: CycleStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleStatus {
    Complete,
    InProgress,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DebugLog {
    /// The failing test this session set out to fix (substring of its title).
    pub triggering_test: String,
    /// Whether the bug was reproduced before diagnosing it.
    pub reproduced: bool,
    pub cycles: Vec<DebugCycle>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DebugLogParse {
    pub log: Option<DebugLog>,
    pub errors: Vec<String>,
}

const CYCLE_FIELDS: [&str; 5] = [
    "hypothesis",
    "prediction",
    "experiment",
    "observation",
    "conclusion",
];

pub fn parse_debug_log_file(path: &Path) -> DebugLogParse {
    if !path.exists() {
        return DebugLogParse {
            log: None,
            errors: vec!["debug-log.md does not exist".to_string()],
        };
    }
    match fs::read_to_string(path) {
        Ok(raw) => parse_debug_log(&raw),
        Err(_) => DebugLogParse {
            log: None,
            errors: vec!["debug-log.md does not exist".to_string()],
        },
    }
}

fn get_str(data: &YamlValue, key: &str) -> String {
    data.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub fn parse_debug_log(raw: &str) -> DebugLogParse {
    let split = match split_frontmatter(raw, "debug-log.md") {
        FrontmatterResult::Err(errors) => {
            return DebugLogParse { log: None, errors };
        }
        FrontmatterResult::Ok(s) => s,
    };
    let data = &split.data;
    let mut errors: Vec<String> = Vec::new();

    let triggering_test = get_str(data, "triggering_test");
    if triggering_test.is_empty() {
        errors
            .push("`triggering_test` is required (name/substring of the failing test)".to_string());
    }

    let reproduced = matches!(data.get("reproduced"), Some(YamlValue::Bool(true)));

    let mut cycles: Vec<DebugCycle> = Vec::new();
    let raw_cycles: &[YamlValue] = match data.get("cycles") {
        Some(YamlValue::Array(items)) => items.as_slice(),
        _ => &[],
    };
    if raw_cycles.is_empty() {
        errors.push("`cycles` must list at least one debugging cycle".to_string());
    }
    for (i, c) in raw_cycles.iter().enumerate() {
        if c.as_map().is_none() {
            errors.push(format!("cycles[{i}] must be a mapping"));
            continue;
        }
        let status = if matches!(c.get("status").and_then(|v| v.as_str()), Some("complete")) {
            CycleStatus::Complete
        } else {
            CycleStatus::InProgress
        };
        cycles.push(DebugCycle {
            hypothesis: get_str(c, CYCLE_FIELDS[0]),
            prediction: get_str(c, CYCLE_FIELDS[1]),
            experiment: get_str(c, CYCLE_FIELDS[2]),
            observation: get_str(c, CYCLE_FIELDS[3]),
            conclusion: get_str(c, CYCLE_FIELDS[4]),
            status,
        });
    }

    if !errors.is_empty() {
        return DebugLogParse { log: None, errors };
    }
    DebugLogParse {
        log: Some(DebugLog {
            triggering_test,
            reproduced,
            cycles,
            body: split.body,
        }),
        errors: Vec::new(),
    }
}

/// A cycle counts as complete when every protocol field is filled and status
/// is complete.
pub fn is_cycle_complete(cycle: &DebugCycle) -> bool {
    cycle.status == CycleStatus::Complete
        && !cycle.hypothesis.is_empty()
        && !cycle.prediction.is_empty()
        && !cycle.experiment.is_empty()
        && !cycle.observation.is_empty()
        && !cycle.conclusion.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "---\ntriggering_test: \"greets by name\"\nreproduced: true\ncycles:\n  - hypothesis: \"units mismatch\"\n    prediction: \"logs show a 1000x gap\"\n    experiment: \"logged both sides\"\n    observation: \"off by 1000x\"\n    conclusion: \"confirmed units bug\"\n    status: complete\n---\n# Debug log";

    #[test]
    fn parses_a_well_formed_debug_log() {
        let DebugLogParse { log, errors } = parse_debug_log(GOOD);
        assert_eq!(errors, Vec::<String>::new());
        let log = log.unwrap();
        assert_eq!(log.triggering_test, "greets by name");
        assert!(log.reproduced);
        assert_eq!(log.cycles.len(), 1);
        assert!(is_cycle_complete(&log.cycles[0]));
    }

    #[test]
    fn requires_triggering_test_and_at_least_one_cycle() {
        let DebugLogParse { errors, .. } =
            parse_debug_log("---\nreproduced: true\ncycles: []\n---\n");
        assert!(errors.iter().any(|e| e.contains("triggering_test")));
        assert!(errors.iter().any(|e| e.contains("cycles")));
    }

    #[test]
    fn an_in_progress_cycle_is_not_complete() {
        let raw = GOOD.replace("status: complete", "status: in-progress");
        let log = parse_debug_log(&raw).log.unwrap();
        assert!(!is_cycle_complete(&log.cycles[0]));
    }

    #[test]
    fn a_cycle_missing_a_field_is_not_complete_even_if_marked_complete() {
        let raw = "---\ntriggering_test: t\nreproduced: true\ncycles:\n  - hypothesis: \"h\"\n    prediction: \"\"\n    experiment: \"e\"\n    observation: \"o\"\n    conclusion: \"c\"\n    status: complete\n---\n";
        let log = parse_debug_log(raw).log.unwrap();
        assert!(!is_cycle_complete(&log.cycles[0]));
    }

    #[test]
    fn reproduced_defaults_to_false_for_anything_but_a_literal_true() {
        let raw = GOOD.replace("reproduced: true", "reproduced: \"yes\"");
        let log = parse_debug_log(&raw).log.unwrap();
        assert!(!log.reproduced);
    }

    #[test]
    fn parse_debug_log_file_reports_missing_file() {
        let DebugLogParse { log, errors } =
            parse_debug_log_file(Path::new("/nonexistent/debug-log.md"));
        assert!(log.is_none());
        assert_eq!(errors, vec!["debug-log.md does not exist".to_string()]);
    }
}
