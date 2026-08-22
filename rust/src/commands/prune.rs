//! Port of `src/commands/prune.ts`.
//!
//! `gate prune` - archive finished runs past the retention window.
//! Candidates are every non-active run except the currently active one.
//! Keeps the most recently updated `--keep` (default 10, or
//! `retention.keep` in config.yml); with `--days`, a candidate must *also*
//! be older than that many days to be pruned. `--dry-run` reports what
//! would be pruned without touching disk. A pruned run's summary is
//! written to `.gate/archive/<id>.json` before its folder is deleted, so
//! `gate report` keeps working on a pruned run.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::args::{parse_args, Flags};
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::commands::report::{build_report_data, parse_iso_millis_or, report_data_to_json};
use crate::core::config::load_config;
use crate::core::current::list_active_branches;
use crate::core::json::{self, Value};
use crate::core::paths::{archive_path, gate_paths, run_paths};
use crate::core::run::{read_run, RunStatus};

const DEFAULT_KEEP: i64 = 10;

struct Candidate {
    id: String,
    updated_at: String,
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Port of `parseIntFlag`: `--flag` given as a bare boolean (no value)
/// mirrors TS's `Number(true) -> NaN` rejection; a string value is parsed
/// permissively (empty string as `0`, matching `Number("")`).
fn parse_int_flag(flags: &Flags, name: &str, flag_label: &str) -> Result<i64, UserError> {
    let Some(s) = flags.str(name) else {
        return Err(UserError::usage(format!(
            "{flag_label} expects a non-negative integer, got \"true\""
        )));
    };
    let trimmed = s.trim();
    let n: f64 = if trimmed.is_empty() {
        0.0
    } else {
        match trimmed.parse() {
            Ok(n) => n,
            Err(_) => {
                return Err(UserError::usage(format!(
                    "{flag_label} expects a non-negative integer, got \"{s}\""
                )))
            }
        }
    };
    if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
        return Err(UserError::usage(format!(
            "{flag_label} expects a non-negative integer, got \"{s}\""
        )));
    }
    Ok(n as i64)
}

fn int_flag_or_default(
    flags: &Flags,
    name: &str,
    fallback: i64,
    flag_label: &str,
) -> Result<i64, UserError> {
    if !flags.is_present(name) {
        return Ok(fallback);
    }
    parse_int_flag(flags, name, flag_label)
}

fn list_candidates(root: &Path, protected_ids: &HashSet<String>) -> Vec<Candidate> {
    let runs_dir = gate_paths(root).runs;
    if !runs_dir.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&runs_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if protected_ids.contains(&id) {
            continue;
        }
        if !run_paths(root, &id).run_json.exists() {
            continue;
        }
        let Ok(run) = read_run(root, &id) else {
            continue; // unreadable run.json - leave it alone, don't guess
        };
        if run.status == RunStatus::Active {
            continue;
        }
        out.push(Candidate {
            id,
            updated_at: run.updated_at,
        });
    }
    out
}

/// Newest `keep` candidates are always spared; the rest are pruned unless
/// `days` excludes them.
fn select_for_prune(candidates: Vec<Candidate>, keep: i64, days: Option<i64>) -> Vec<Candidate> {
    let mut sorted = candidates;
    sorted.sort_by(|a, b| {
        let a_at = parse_iso_millis_or(&a.updated_at, 0);
        let b_at = parse_iso_millis_or(&b.updated_at, 0);
        match b_at.cmp(&a_at) {
            std::cmp::Ordering::Equal => a.id.cmp(&b.id),
            other => other,
        }
    });
    let skip_n = keep.max(0) as usize;
    let beyond_keep: Vec<Candidate> = sorted.into_iter().skip(skip_n).collect();
    let Some(days) = days else {
        return beyond_keep;
    };
    let cutoff = now_millis() - days * 24 * 60 * 60 * 1000;
    beyond_keep
        .into_iter()
        .filter(|c| parse_iso_millis_or(&c.updated_at, 0) < cutoff)
        .collect()
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    // See `check::run`'s comment (commands/check.rs): prepend a placeholder
    // command token so `parse_args` never misreads a leading flag/
    // positional as the command.
    let mut full = vec!["prune".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = require_root()?;
    let config = load_config(&root)?;

    let keep = int_flag_or_default(
        &args.flags,
        "keep",
        config.retention.keep.unwrap_or(DEFAULT_KEEP),
        "--keep",
    )?;
    let days = if args.flags.is_present("days") {
        Some(parse_int_flag(&args.flags, "days", "--days")?)
    } else {
        config.retention.days
    };
    let dry_run = args.flags.is_true("dry-run");

    // Every run any branch's current.json entry points at is protected,
    // regardless of its own `status` field.
    let mapped_ids: HashSet<String> = list_active_branches(&root)?
        .into_iter()
        .map(|(_, run_id)| run_id)
        .collect();
    let candidates = list_candidates(&root, &mapped_ids);
    let to_prune = select_for_prune(candidates, keep, days);

    if dry_run {
        let ids: Vec<String> = to_prune.iter().map(|c| c.id.clone()).collect();
        let human = if !ids.is_empty() {
            format!(
                "Would prune {} run(s):\n{}",
                ids.len(),
                ids.iter()
                    .map(|id| format!("  {id}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        } else {
            "Nothing to prune.".to_string()
        };
        let mut data = Value::object();
        data.insert("dryRun", true);
        data.insert("pruned", ids);
        return emit(&human, &data, &args.flags);
    }

    let archive_dir = gate_paths(&root).archive;
    fs::create_dir_all(&archive_dir).map_err(|e| UserError::new(e.to_string()))?;
    let mut pruned: Vec<String> = Vec::new();
    for c in &to_prune {
        let run = read_run(&root, &c.id)?;
        let data = build_report_data(&root, &run);
        let text = json::stringify_pretty(&report_data_to_json(&data)) + "\n";
        fs::write(archive_path(&root, &c.id), text).map_err(|e| UserError::new(e.to_string()))?;
        let _ = fs::remove_dir_all(run_paths(&root, &c.id).dir);
        pruned.push(c.id.clone());
    }

    let human = if !pruned.is_empty() {
        format!(
            "Pruned {} run(s), archived to {}:\n{}",
            pruned.len(),
            archive_dir.display(),
            pruned
                .iter()
                .map(|id| format!("  {id}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    } else {
        "Nothing to prune.".to_string()
    };
    let mut data = Value::object();
    data.insert("dryRun", false);
    data.insert("pruned", pruned);
    emit(&human, &data, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, updated_at: &str) -> Candidate {
        Candidate {
            id: id.to_string(),
            updated_at: updated_at.to_string(),
        }
    }

    #[test]
    fn select_for_prune_keeps_the_newest_n_and_prunes_the_rest() {
        let candidates = vec![
            candidate("run-0", "2026-01-05T00:00:00.000Z"),
            candidate("run-1", "2026-01-04T00:00:00.000Z"),
            candidate("run-2", "2026-01-03T00:00:00.000Z"),
            candidate("run-3", "2026-01-02T00:00:00.000Z"),
            candidate("run-4", "2026-01-01T00:00:00.000Z"),
        ];
        let pruned = select_for_prune(candidates, 2, None);
        let ids: Vec<&str> = pruned.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["run-2", "run-3", "run-4"]);
    }

    #[test]
    fn select_for_prune_breaks_ties_by_id() {
        let candidates = vec![
            candidate("b", "2026-01-01T00:00:00.000Z"),
            candidate("a", "2026-01-01T00:00:00.000Z"),
        ];
        let pruned = select_for_prune(candidates, 0, None);
        let ids: Vec<&str> = pruned.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    /// Days-since-epoch -> (year, month, day) (Howard Hinnant's
    /// `civil_from_days`; duplicated in test scope purely to build ISO
    /// fixtures relative to "now" - see `core/run.rs`/`commands/report.rs`
    /// for the production copies this mirrors).
    fn civil_from_days(z: i64) -> (i64, u32, u32) {
        let z = z + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = (z - era * 146097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = if m <= 2 { y + 1 } else { y };
        (y, m, d)
    }

    fn ms_to_iso(ms: i64) -> String {
        let secs = ms.div_euclid(1000);
        let days = secs.div_euclid(86400);
        let secs_of_day = secs.rem_euclid(86400);
        let (y, m, d) = civil_from_days(days);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
            secs_of_day / 3600,
            (secs_of_day % 3600) / 60,
            secs_of_day % 60
        )
    }

    #[test]
    fn select_for_prune_days_additionally_requires_age() {
        let now = now_millis();
        let recent = now - 24 * 60 * 60 * 1000;
        let old = now - 40 * 24 * 60 * 60 * 1000;
        let candidates = vec![
            candidate("recent", &ms_to_iso(recent)),
            candidate("old", &ms_to_iso(old)),
        ];
        let pruned = select_for_prune(candidates, 0, Some(30));
        let ids: Vec<&str> = pruned.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["old"]);
    }

    #[test]
    fn parse_int_flag_accepts_a_plain_digit_string() {
        let flags = parse_args(&["cmd".to_string(), "--keep".to_string(), "5".to_string()]).flags;
        assert_eq!(parse_int_flag(&flags, "keep", "--keep").unwrap(), 5);
    }

    #[test]
    fn parse_int_flag_rejects_a_bare_boolean_flag() {
        let flags = parse_args(&["cmd".to_string(), "--keep".to_string()]).flags;
        assert!(parse_int_flag(&flags, "keep", "--keep").is_err());
    }

    #[test]
    fn parse_int_flag_rejects_negative_and_non_integer_values() {
        let flags = parse_args(&["cmd".to_string(), "--keep".to_string(), "-1".to_string()]).flags;
        assert!(parse_int_flag(&flags, "keep", "--keep").is_err());
        let flags2 =
            parse_args(&["cmd".to_string(), "--keep".to_string(), "2.5".to_string()]).flags;
        assert!(parse_int_flag(&flags2, "keep", "--keep").is_err());
    }
}
