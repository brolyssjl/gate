//! Port of `src/commands/report.ts`.
//!
//! `gate report [runId]` - a per-run summary: how long each phase took, how
//! many gate attempts failed, and any review findings. Read-only; it never
//! runs a gate or a command. Defaults to the active run, else the most
//! recently updated live one. Falls back to the newest archived summary
//! (`.gate/archive/<id>.json`) when `gate prune` has already removed every
//! live run folder.

use std::fs;
use std::path::Path;

use crate::artifacts::review::{parse_review_file, FindingStatus, Severity};
use crate::cli::args::parse_args;
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::core::current::{read_current_run_id, resolve_branch_key, BranchKeyResolution};
use crate::core::json::{self, Value};
use crate::core::paths::{archive_path, gate_paths, run_paths, validate_run_id};
use crate::core::run::{read_run, HistoryEntry, HistoryEvent, Run, RunStatus};
use crate::core::state_machine::Phase;

#[derive(Debug, Clone, PartialEq)]
pub struct PhaseReport {
    pub phase: Phase,
    pub seconds: i64,
    pub gate_failures: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FindingsSummary {
    pub total: i64,
    pub blocker: i64,
    pub major: i64,
    pub minor: i64,
    pub nit: i64,
    pub open: i64,
    pub resolved: i64,
    pub waived: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverrideOut {
    pub phase: Phase,
    pub reason: String,
    pub at: String,
    pub by: Option<String>,
}

/// Pure summary of a run: everything `gate report` prints, with no I/O
/// beyond what the caller already did. Shared by the live path
/// (`cmd::run`) and `gate prune`, which persists exactly this shape to
/// `.gate/archive/<id>.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportData {
    pub id: String,
    pub title: String,
    pub profile: String,
    pub phase: Phase,
    pub status: RunStatus,
    pub total_seconds: i64,
    pub gate_failures: i64,
    pub phases: Vec<PhaseReport>,
    pub findings: Option<FindingsSummary>,
    pub overrides: Vec<OverrideOut>,
    pub artifacts: Vec<String>,
}

/// Inverse of `civil_from_days`: (year, month, day) -> days-since-epoch.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (m as i64 + if m > 2 { -3 } else { 9 }) + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// `Date.parse` for the one shape every `at`/`updatedAt` string in this
/// codebase actually takes: `now_iso()`'s
/// `YYYY-MM-DDTHH:MM:SS[.mmm]Z`. Returns milliseconds since the epoch, or
/// `None` on anything that doesn't parse (mirrors `Date.parse`'s `NaN` as
/// an absent value rather than a panic).
pub(crate) fn parse_iso_millis(s: &str) -> Option<i64> {
    if s.len() < 20 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    if s.as_bytes().get(4) != Some(&b'-') || s.as_bytes().get(7) != Some(&b'-') {
        return None;
    }
    let month: u32 = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    if s.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let min: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;
    let millis: i64 = if s.as_bytes().get(19) == Some(&b'.') {
        let end = s[20..].find('Z').map(|i| 20 + i).unwrap_or(s.len());
        let mut digits = s.get(20..end)?.to_string();
        while digits.len() < 3 {
            digits.push('0');
        }
        digits.truncate(3);
        digits.parse().ok()?
    } else {
        0
    };
    let days = days_from_civil(year, month, day);
    Some(days * 86_400_000 + hour * 3_600_000 + min * 60_000 + sec * 1000 + millis)
}

pub(crate) fn parse_iso_millis_or(s: &str, fallback: i64) -> i64 {
    parse_iso_millis(s).unwrap_or(fallback)
}

/// Per-phase wall-clock, summed across re-entries, plus failed-attempt
/// counts.
pub fn phase_reports(history: &[HistoryEntry]) -> Vec<PhaseReport> {
    let mut seconds: Vec<(Phase, f64)> = Vec::new();
    let mut failures: Vec<(Phase, i64)> = Vec::new();
    let mut order: Vec<Phase> = Vec::new();
    let last_at = history.last().map(|h| h.at.clone());

    for (i, entry) in history.iter().enumerate() {
        if !order.contains(&entry.phase) {
            order.push(entry.phase);
        }
        if entry.event == HistoryEvent::Failed {
            match failures.iter_mut().find(|(p, _)| *p == entry.phase) {
                Some((_, c)) => *c += 1,
                None => failures.push((entry.phase, 1)),
            }
        }
        if entry.event == HistoryEvent::Entered {
            // A phase spans from its `entered` event to the next phase
            // entry, or to the last recorded event while it is still the
            // current phase.
            let mut end = last_at.clone().unwrap_or_else(|| entry.at.clone());
            for later in &history[i + 1..] {
                if later.event == HistoryEvent::Entered {
                    end = later.at.clone();
                    break;
                }
            }
            let delta = match (parse_iso_millis(&end), parse_iso_millis(&entry.at)) {
                (Some(e), Some(s)) => Some((e - s) as f64 / 1000.0),
                _ => None,
            };
            let slot = seconds.iter_mut().find(|(p, _)| *p == entry.phase);
            match delta {
                Some(d) if d.is_finite() && d > 0.0 => match slot {
                    Some((_, s)) => *s += d,
                    None => seconds.push((entry.phase, d)),
                },
                _ => {
                    if slot.is_none() {
                        seconds.push((entry.phase, 0.0));
                    }
                }
            }
        }
    }

    order
        .into_iter()
        .map(|phase| {
            let secs = seconds
                .iter()
                .find(|(p, _)| *p == phase)
                .map(|(_, s)| *s)
                .unwrap_or(0.0);
            let fails = failures
                .iter()
                .find(|(p, _)| *p == phase)
                .map(|(_, c)| *c)
                .unwrap_or(0);
            PhaseReport {
                phase,
                seconds: secs.round() as i64,
                gate_failures: fails,
            }
        })
        .collect()
}

fn summarize_findings(root: &Path, run_id: &str) -> Option<FindingsSummary> {
    let parsed = parse_review_file(&run_paths(root, run_id).review);
    let review = parsed.review?;
    let mut f = FindingsSummary {
        total: review.findings.len() as i64,
        ..Default::default()
    };
    for finding in &review.findings {
        match finding.severity {
            Severity::Blocker => f.blocker += 1,
            Severity::Major => f.major += 1,
            Severity::Minor => f.minor += 1,
            Severity::Nit => f.nit += 1,
        }
        match finding.status {
            FindingStatus::Open => f.open += 1,
            FindingStatus::Resolved => f.resolved += 1,
            FindingStatus::Waived => f.waived += 1,
        }
    }
    Some(f)
}

pub fn build_report_data(root: &Path, run: &Run) -> ReportData {
    let phases = phase_reports(&run.history);
    let total_seconds: i64 = phases.iter().map(|p| p.seconds).sum();
    let gate_failures: i64 = phases.iter().map(|p| p.gate_failures).sum();
    let findings = summarize_findings(root, &run.id);
    let overrides = run
        .overrides
        .iter()
        .map(|o| OverrideOut {
            phase: o.phase,
            reason: o.reason.clone(),
            at: o.at.clone(),
            by: o.by.clone(),
        })
        .collect();
    let artifacts = run.artifacts.iter().map(|(k, _)| k.clone()).collect();

    ReportData {
        id: run.id.clone(),
        title: run.title.clone(),
        profile: run.profile.clone(),
        phase: run.phase,
        status: run.status,
        total_seconds,
        gate_failures,
        phases,
        findings,
        overrides,
        artifacts,
    }
}

fn format_duration(seconds: i64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let m = seconds / 60;
    let s = seconds % 60;
    if m < 60 {
        return if s != 0 {
            format!("{m}m{s}s")
        } else {
            format!("{m}m")
        };
    }
    let h = m / 60;
    format!("{h}h{}m", m % 60)
}

/// Render `build_report_data`'s output as the human-readable report text.
pub fn render_report_human(data: &ReportData, archived: bool) -> String {
    let mut lines = vec![
        format!(
            "Report: {}{}",
            data.id,
            if archived { " (archived)" } else { "" }
        ),
        format!("  Title:    {}", data.title),
        format!(
            "  Profile:  {}    Phase: {}    Status: {}",
            data.profile,
            data.phase,
            data.status.as_str()
        ),
        format!(
            "  Duration: {} total, {} gate failure(s)",
            format_duration(data.total_seconds),
            data.gate_failures
        ),
        String::new(),
        "  Phase durations:".to_string(),
    ];
    for p in &data.phases {
        let mut line = format!(
            "    {:<10} {:>8}",
            p.phase.as_str(),
            format_duration(p.seconds)
        );
        if p.gate_failures != 0 {
            line.push_str(&format!("   ({} failed attempt(s))", p.gate_failures));
        }
        lines.push(line);
    }
    lines.push(String::new());
    lines.push(match &data.findings {
        Some(f) => format!(
            "  Findings: {} total - {} blocker, {} major, {} minor, {} nit ({} open, {} resolved, {} waived)",
            f.total, f.blocker, f.major, f.minor, f.nit, f.open, f.resolved, f.waived
        ),
        None => "  Findings: (no review.md)".to_string(),
    });
    lines.push(if !data.overrides.is_empty() {
        let joined = data
            .overrides
            .iter()
            .map(|o| {
                let by_part =
                    o.by.as_deref()
                        .filter(|s| !s.is_empty())
                        .map(|s| format!(", by {s}"))
                        .unwrap_or_default();
                format!("{} ({}{})", o.phase, o.reason, by_part)
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("  Overrides: {joined}")
    } else {
        "  Overrides: none".to_string()
    });
    lines.push(if !data.artifacts.is_empty() {
        format!("  Artifacts: {}", data.artifacts.join(", "))
    } else {
        "  Artifacts: none".to_string()
    });
    lines.join("\n")
}

fn phase_report_to_json(p: &PhaseReport) -> Value {
    let mut v = Value::object();
    v.insert("phase", p.phase.as_str());
    v.insert("seconds", p.seconds);
    v.insert("gateFailures", p.gate_failures);
    v
}

fn override_out_to_json(o: &OverrideOut) -> Value {
    let mut v = Value::object();
    v.insert("phase", o.phase.as_str());
    v.insert("reason", o.reason.as_str());
    v.insert("at", o.at.as_str());
    v.insert("by", o.by.clone());
    v
}

fn findings_to_json(f: &FindingsSummary) -> Value {
    let mut v = Value::object();
    v.insert("total", f.total);
    v.insert("blocker", f.blocker);
    v.insert("major", f.major);
    v.insert("minor", f.minor);
    v.insert("nit", f.nit);
    v.insert("open", f.open);
    v.insert("resolved", f.resolved);
    v.insert("waived", f.waived);
    v
}

/// `ReportData` -> JSON, in the exact field order `JSON.stringify` produces
/// for the TS `ReportData` interface - both the `--json` output and the
/// `.gate/archive/<id>.json` shape `gate prune` writes.
pub fn report_data_to_json(data: &ReportData) -> Value {
    let mut v = Value::object();
    v.insert("id", data.id.as_str());
    v.insert("title", data.title.as_str());
    v.insert("profile", data.profile.as_str());
    v.insert("phase", data.phase.as_str());
    v.insert("status", data.status.as_str());
    v.insert("totalSeconds", data.total_seconds);
    v.insert("gateFailures", data.gate_failures);
    v.insert(
        "phases",
        Value::Array(data.phases.iter().map(phase_report_to_json).collect()),
    );
    v.insert(
        "findings",
        match &data.findings {
            Some(f) => findings_to_json(f),
            None => Value::Null,
        },
    );
    v.insert(
        "overrides",
        Value::Array(data.overrides.iter().map(override_out_to_json).collect()),
    );
    v.insert(
        "artifacts",
        Value::Array(
            data.artifacts
                .iter()
                .map(|a| Value::from(a.as_str()))
                .collect(),
        ),
    );
    v
}

fn as_i64(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Int(n)) => *n,
        Some(Value::Float(f)) => *f as i64,
        _ => 0,
    }
}

fn phase_report_from_json(v: &Value) -> PhaseReport {
    PhaseReport {
        phase: v
            .get("phase")
            .and_then(|x| x.as_str())
            .and_then(Phase::from_str_opt)
            .unwrap_or(Phase::Plan),
        seconds: as_i64(v.get("seconds")),
        gate_failures: as_i64(v.get("gateFailures")),
    }
}

fn override_out_from_json(v: &Value) -> OverrideOut {
    OverrideOut {
        phase: v
            .get("phase")
            .and_then(|x| x.as_str())
            .and_then(Phase::from_str_opt)
            .unwrap_or(Phase::Plan),
        reason: v
            .get("reason")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        at: v
            .get("at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        by: v.get("by").and_then(|x| x.as_str()).map(String::from),
    }
}

fn findings_from_json(v: &Value) -> FindingsSummary {
    FindingsSummary {
        total: as_i64(v.get("total")),
        blocker: as_i64(v.get("blocker")),
        major: as_i64(v.get("major")),
        minor: as_i64(v.get("minor")),
        nit: as_i64(v.get("nit")),
        open: as_i64(v.get("open")),
        resolved: as_i64(v.get("resolved")),
        waived: as_i64(v.get("waived")),
    }
}

fn report_data_from_json(v: &Value) -> ReportData {
    ReportData {
        id: v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        title: v
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        profile: v
            .get("profile")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        phase: v
            .get("phase")
            .and_then(|x| x.as_str())
            .and_then(Phase::from_str_opt)
            .unwrap_or(Phase::Plan),
        status: v
            .get("status")
            .and_then(|x| x.as_str())
            .and_then(RunStatus::from_str_opt)
            .unwrap_or(RunStatus::Active),
        total_seconds: as_i64(v.get("totalSeconds")),
        gate_failures: as_i64(v.get("gateFailures")),
        phases: v
            .get("phases")
            .and_then(|x| x.as_array())
            .map(|arr| arr.iter().map(phase_report_from_json).collect())
            .unwrap_or_default(),
        findings: match v.get("findings") {
            Some(Value::Object(_)) => v.get("findings").map(findings_from_json),
            _ => None,
        },
        overrides: v
            .get("overrides")
            .and_then(|x| x.as_array())
            .map(|arr| arr.iter().map(override_out_from_json).collect())
            .unwrap_or_default(),
        artifacts: v
            .get("artifacts")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn most_recent_run_id(root: &Path) -> Option<String> {
    let runs_dir = gate_paths(root).runs;
    if !runs_dir.exists() {
        return None;
    }
    let mut best: Option<(String, i64)> = None;
    let entries = fs::read_dir(&runs_dir).ok()?;
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        let run_json = runs_dir.join(&id).join("run.json");
        if !run_json.exists() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&run_json) else {
            continue;
        };
        let Ok(parsed) = json::parse(&text) else {
            continue;
        };
        let Some(updated_at) = parsed.get("updatedAt").and_then(|v| v.as_str()) else {
            continue;
        };
        let at = parse_iso_millis_or(updated_at, i64::MIN);
        if best.as_ref().map(|(_, b)| at > *b).unwrap_or(true) {
            best = Some((id, at));
        }
    }
    best.map(|(id, _)| id)
}

/// The most recently archived run's id, by archive-file mtime
/// (`ReportData` carries no timestamp of its own). Used only when no live
/// run exists at all.
fn most_recent_archived_run_id(root: &Path) -> Option<String> {
    let archive_dir = gate_paths(root).archive;
    if !archive_dir.exists() {
        return None;
    }
    let mut best: Option<(String, std::time::SystemTime)> = None;
    let entries = fs::read_dir(&archive_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(mtime) = meta.modified() else {
            continue;
        };
        let id = name[..name.len() - ".json".len()].to_string();
        if best.as_ref().map(|(_, b)| mtime > *b).unwrap_or(true) {
            best = Some((id, mtime));
        }
    }
    best.map(|(id, _)| id)
}

fn load_report_data(root: &Path, run_id: &str) -> Result<(ReportData, bool), UserError> {
    if run_paths(root, run_id).run_json.exists() {
        let run = read_run(root, run_id)?;
        return Ok((build_report_data(root, &run), false));
    }
    let archived = archive_path(root, run_id);
    if archived.exists() {
        let text = fs::read_to_string(&archived).map_err(|e| UserError::new(e.to_string()))?;
        let value = json::parse(&text).map_err(|e| UserError::new(e.0))?;
        return Ok((report_data_from_json(&value), true));
    }
    Err(UserError::new(format!(
        "run \"{run_id}\" not found (not live, and no archive summary at {})",
        archived.display()
    )))
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    // `main.rs` hands every command its argv with the command token itself
    // already stripped (see `dispatch`'s doc comment); `parse_args` treats
    // whatever token is first as the command name, so a real leading
    // positional (the run id) would otherwise be swallowed into
    // `args.command` and vanish from `args.positionals`. Re-prepend a
    // placeholder command token first, the same workaround `start.rs` uses
    // for its own leading positional (the title).
    let mut full = vec!["report".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = require_root()?;
    let explicit_id = args.positionals.first().cloned();
    if let Some(id) = &explicit_id {
        validate_run_id(id)?;
    }
    let resolved = resolve_branch_key(&root);

    // A detached HEAD has no branch to resolve a default run from - same
    // policy as the phase commands (check/next/review/...): refuse rather
    // than silently falling back to `most_recent_run_id`.
    if explicit_id.is_none() && matches!(resolved, BranchKeyResolution::Detached) {
        return Err(UserError::new(
            "HEAD is detached - no branch to resolve a default run from; pass a run id: gate report <id>",
        ));
    }
    let current_id = match &resolved {
        BranchKeyResolution::Key(key) => read_current_run_id(&root, key)?,
        BranchKeyResolution::Detached => None,
    };
    let run_id = explicit_id
        .or(current_id)
        .or_else(|| most_recent_run_id(&root))
        .or_else(|| most_recent_archived_run_id(&root));
    let Some(run_id) = run_id else {
        return Err(UserError::new(
            "no run to report on - pass a run id: gate report <id>",
        ));
    };

    let (data, archived) = load_report_data(&root, &run_id)?;
    let human = render_report_human(&data, archived);
    let json = report_data_to_json(&data);
    emit(&human, &json, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(phase: Phase, event: HistoryEvent, at: &str) -> HistoryEntry {
        HistoryEntry {
            phase,
            event,
            at: at.to_string(),
            detail: None,
            tree_hash: None,
        }
    }

    #[test]
    fn parse_iso_millis_round_trips_now_iso_shape() {
        let ms = parse_iso_millis("2026-07-27T14:05:30.250Z").unwrap();
        // Sanity: distinct instants order correctly.
        let later = parse_iso_millis("2026-07-27T14:05:31.000Z").unwrap();
        assert!(later > ms);
        assert_eq!(later - ms, 750);
    }

    #[test]
    fn parse_iso_millis_rejects_garbage() {
        assert_eq!(parse_iso_millis("not-a-date"), None);
    }

    #[test]
    fn phase_reports_sums_duration_across_re_entries_and_counts_failures() {
        let history = vec![
            entry(
                Phase::Plan,
                HistoryEvent::Entered,
                "2026-01-01T00:00:00.000Z",
            ),
            entry(
                Phase::Plan,
                HistoryEvent::Failed,
                "2026-01-01T00:00:05.000Z",
            ),
            entry(
                Phase::Implement,
                HistoryEvent::Entered,
                "2026-01-01T00:01:00.000Z",
            ),
            entry(
                Phase::Implement,
                HistoryEvent::Entered,
                "2026-01-01T00:02:00.000Z",
            ),
        ];
        let reports = phase_reports(&history);
        let plan = reports.iter().find(|r| r.phase == Phase::Plan).unwrap();
        assert_eq!(plan.seconds, 60);
        assert_eq!(plan.gate_failures, 1);
        let implement = reports
            .iter()
            .find(|r| r.phase == Phase::Implement)
            .unwrap();
        // Only one "entered" span for IMPLEMENT counts (from 00:01:00 to the
        // next "entered" at 00:02:00); the second entered event starts a
        // fresh zero-length span with nothing after it.
        assert_eq!(implement.seconds, 60);
    }

    #[test]
    fn phase_reports_preserves_first_appearance_order() {
        let history = vec![
            entry(
                Phase::Plan,
                HistoryEvent::Entered,
                "2026-01-01T00:00:00.000Z",
            ),
            entry(
                Phase::Implement,
                HistoryEvent::Entered,
                "2026-01-01T00:01:00.000Z",
            ),
        ];
        let reports = phase_reports(&history);
        assert_eq!(reports[0].phase, Phase::Plan);
        assert_eq!(reports[1].phase, Phase::Implement);
    }

    #[test]
    fn format_duration_formats_seconds_minutes_and_hours() {
        assert_eq!(format_duration(5), "5s");
        assert_eq!(format_duration(65), "1m5s");
        assert_eq!(format_duration(120), "2m");
        assert_eq!(format_duration(3661), "1h1m");
    }

    #[test]
    fn report_data_json_round_trips() {
        let data = ReportData {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            phase: Phase::Done,
            status: RunStatus::Done,
            total_seconds: 42,
            gate_failures: 1,
            phases: vec![PhaseReport {
                phase: Phase::Plan,
                seconds: 42,
                gate_failures: 1,
            }],
            findings: Some(FindingsSummary {
                total: 1,
                blocker: 1,
                ..Default::default()
            }),
            overrides: vec![OverrideOut {
                phase: Phase::Plan,
                reason: "trivial".to_string(),
                at: "2026-01-01T00:00:00.000Z".to_string(),
                by: Some("alice".to_string()),
            }],
            artifacts: vec!["plan.md".to_string()],
        };
        let json_val = report_data_to_json(&data);
        let round_tripped = report_data_from_json(&json_val);
        assert_eq!(round_tripped, data);
    }

    #[test]
    fn report_data_to_json_field_order_matches_ts_interface() {
        let data = ReportData {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            phase: Phase::Plan,
            status: RunStatus::Active,
            total_seconds: 0,
            gate_failures: 0,
            phases: vec![],
            findings: None,
            overrides: vec![],
            artifacts: vec![],
        };
        let text = json::stringify_pretty(&report_data_to_json(&data));
        let expected_order = [
            "id",
            "title",
            "profile",
            "phase",
            "status",
            "totalSeconds",
            "gateFailures",
            "phases",
            "findings",
            "overrides",
            "artifacts",
        ];
        let mut last = 0;
        for key in expected_order {
            let idx = text.find(&format!("\"{key}\"")).unwrap();
            assert!(idx >= last, "key {key} out of order");
            last = idx;
        }
    }

    #[test]
    fn render_report_human_shows_archived_suffix_and_placeholders() {
        let data = ReportData {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            phase: Phase::Done,
            status: RunStatus::Done,
            total_seconds: 5,
            gate_failures: 0,
            phases: vec![],
            findings: None,
            overrides: vec![],
            artifacts: vec![],
        };
        let human = render_report_human(&data, true);
        assert!(human.starts_with("Report: r1 (archived)"));
        assert!(human.contains("Findings: (no review.md)"));
        assert!(human.contains("Overrides: none"));
        assert!(human.contains("Artifacts: none"));
    }
}
