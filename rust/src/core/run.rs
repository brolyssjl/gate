//! Port of `src/core/run.ts`: the `Run` record persisted at
//! `.gate/runs/<id>/run.json` (`newRun`, `readRun`, `writeRun`,
//! `makeRunId`, `nowIso`), plus the schema-1..3 -> schema-4 migration.
//!
//! No TS counterpart: `now_iso`'s UTC calendar math (`civil_from_days`), a
//! std-only replacement for `Date#toISOString()` (no `chrono`, per
//! `docs/rust-port.md`'s dependency policy). It's Howard Hinnant's
//! `civil_from_days` algorithm (public domain, widely used - e.g. libc++'s
//! `<chrono>`), verified against real `new Date().toISOString()` output in
//! this module's tests.

use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::output::UserError;
use crate::core::fsx::write_file_atomic;
use crate::core::json::{self, Value};
use crate::core::paths::run_paths;
use crate::core::state_machine::Phase;

pub const CURRENT_SCHEMA: i64 = 4;

/// Cap on how many `Failed` events `history` retains at once - the oldest
/// `Failed` entries are pruned past this so a long, failure-heavy run's
/// `run.json` doesn't grow without bound. `Entered`/`Passed`/`Skipped`
/// entries are never pruned (phase-duration reporting needs every
/// `Entered` span). The permanent per-phase count (`failure_counts`) is
/// unaffected by pruning - it's the source `gate report` reads, not a count
/// of what's left in `history`.
pub const MAX_FAILURE_HISTORY_EVENTS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Active,
    Done,
    Abandoned,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Active => "active",
            RunStatus::Done => "done",
            RunStatus::Abandoned => "abandoned",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<RunStatus> {
        match value {
            "active" => Some(RunStatus::Active),
            "done" => Some(RunStatus::Done),
            "abandoned" => Some(RunStatus::Abandoned),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryEvent {
    Entered,
    Passed,
    Failed,
    Skipped,
}

impl HistoryEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            HistoryEvent::Entered => "entered",
            HistoryEvent::Passed => "passed",
            HistoryEvent::Failed => "failed",
            HistoryEvent::Skipped => "skipped",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<HistoryEvent> {
        match value {
            "entered" => Some(HistoryEvent::Entered),
            "passed" => Some(HistoryEvent::Passed),
            "failed" => Some(HistoryEvent::Failed),
            "skipped" => Some(HistoryEvent::Skipped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    pub phase: Phase,
    pub event: HistoryEvent,
    pub at: String,
    /// For failed/passed: short summary of the gate verdict. Optional (not
    /// nullable) in TS - omitted from JSON when absent, never emitted null.
    pub detail: Option<String>,
    /// For passed: fingerprint of the working tree the gate certified.
    /// Optional (not nullable) in TS - omitted from JSON when absent.
    pub tree_hash: Option<String>,
}

/// What kind of explicit human override an `OverrideEntry` records. Both
/// are "loud, auditable overrides" in the README's threat-model sense - a
/// distinct, reasoned, recorded act, never automatic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideAction {
    /// `gate skip <phase> --reason` - move past the phase without its gate
    /// passing.
    Skip,
    /// `gate streak reset <phase> --reason` - clear a phase's
    /// consecutive-failure count so `gate check`/`gate next` can evaluate it
    /// again (the loop-enforcement cap).
    StreakReset,
}

impl OverrideAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            OverrideAction::Skip => "skip",
            OverrideAction::StreakReset => "streak_reset",
        }
    }

    /// Unrecognized/missing (every `run.json` written before `StreakReset`
    /// existed) defaults to `Skip` - the only action that ever existed, and
    /// the value `override_entry_to_json` already wrote unconditionally.
    pub fn from_str_opt(value: &str) -> OverrideAction {
        match value {
            "streak_reset" => OverrideAction::StreakReset,
            _ => OverrideAction::Skip,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverrideEntry {
    pub phase: Phase,
    pub action: OverrideAction,
    pub reason: String,
    pub at: String,
    /// Who authorized (from --by or GATE_SESSION_ID); best-effort. TS types
    /// this `string | null` and its one call site (`cmdSkip`) always sets it
    /// to a string or `null`, never omits it - always emitted.
    pub by: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactEntry {
    pub phase: Phase,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Approval {
    pub by: Option<String>,
    pub at: String,
    pub reason: Option<String>,
    pub plan_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Amendment {
    pub plan_hash: String,
    pub at: String,
    pub by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetroMethod {
    AgnosgramCli,
    Fallback,
}

impl RetroMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            RetroMethod::AgnosgramCli => "agnosgram-cli",
            RetroMethod::Fallback => "fallback",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<RetroMethod> {
        match value {
            "agnosgram-cli" => Some(RetroMethod::AgnosgramCli),
            "fallback" => Some(RetroMethod::Fallback),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetroSync {
    pub journal_file: String,
    pub synced_at: String,
    pub method: RetroMethod,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewRequest {
    pub requested_by: Option<String>,
    pub requested_at: String,
    pub tree_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// Schema version of this run.json; always `4` after `read_run`
    /// migrates it (see module docs).
    pub schema: i64,
    pub id: String,
    pub title: String,
    pub profile: String,
    pub phase: Phase,
    pub status: RunStatus,
    pub created_at: String,
    pub updated_at: String,
    /// Branch this run was started on; `None` when there was no branch to
    /// record. Always present in JSON (as `null` when absent) - not an
    /// optional TS key.
    pub branch: Option<String>,
    /// git ref (sha) captured at `gate start`. Always present in JSON.
    pub base_ref: Option<String>,
    /// Explicit `gate start --target` override. Omitted from JSON entirely
    /// when absent or empty (TS: `...(x && x.length > 0 ? {...} : {})`).
    pub target_override: Option<Vec<String>>,
    /// Session id of the implementer. Always present in JSON.
    pub session_id: Option<String>,
    pub history: Vec<HistoryEntry>,
    pub overrides: Vec<OverrideEntry>,
    /// Registered artifacts keyed by relative filename, insertion-ordered.
    pub artifacts: Vec<(String, ArtifactEntry)>,
    /// Consecutive failed `gate check`/`gate next` evaluations per phase,
    /// since the last pass, phase advance, `gate skip`, or `gate streak
    /// reset` (the loop-enforcement cap). Sparse: a phase absent here has
    /// streak 0 - see `failure_streak`/`record_gate_evaluation`.
    pub failure_streaks: Vec<(Phase, i64)>,
    /// Total failed gate evaluations per phase over the run's entire
    /// lifetime - unlike `failure_streaks`, never cleared by a pass, skip,
    /// or streak reset, and unaffected by `history`'s failure-event pruning
    /// (see `MAX_FAILURE_HISTORY_EVENTS`). Sparse: a phase absent here has
    /// never failed. `gate report`'s per-phase failure counts read this,
    /// not a count of `Failed` events left in `history`.
    pub failure_counts: Vec<(Phase, i64)>,
    pub approval: Option<Approval>,
    pub amendment: Option<Amendment>,
    pub review: Option<ReviewRequest>,
    pub retro: Option<RetroSync>,
}

/// Parameters for `new_run`, mirroring `newRun`'s TS params object.
pub struct NewRunParams {
    pub id: String,
    pub title: String,
    pub profile: String,
    pub branch: Option<String>,
    pub base_ref: Option<String>,
    pub session_id: Option<String>,
    pub target_override: Option<Vec<String>>,
}

pub fn new_run(params: NewRunParams) -> Run {
    let at = now_iso();
    Run {
        schema: CURRENT_SCHEMA,
        id: params.id,
        title: params.title,
        profile: params.profile,
        phase: Phase::Plan,
        status: RunStatus::Active,
        created_at: at.clone(),
        updated_at: at.clone(),
        branch: params.branch,
        base_ref: params.base_ref,
        target_override: params.target_override.filter(|t| !t.is_empty()),
        session_id: params.session_id,
        history: vec![HistoryEntry {
            phase: Phase::Plan,
            event: HistoryEvent::Entered,
            at,
            detail: None,
            tree_hash: None,
        }],
        overrides: Vec::new(),
        artifacts: Vec::new(),
        failure_streaks: Vec::new(),
        failure_counts: Vec::new(),
        approval: None,
        amendment: None,
        review: None,
        retro: None,
    }
}

impl Run {
    /// Consecutive failed `gate check`/`gate next` evaluations of `phase`
    /// since it was last cleared. A phase never evaluated, or already
    /// cleared, is 0.
    pub fn failure_streak(&self, phase: Phase) -> i64 {
        self.failure_streaks
            .iter()
            .find(|(p, _)| *p == phase)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }

    /// Clear `phase`'s streak to 0 - on a passing evaluation, a phase
    /// advance, `gate skip`, or `gate streak reset`.
    pub fn clear_failure_streak(&mut self, phase: Phase) {
        self.failure_streaks.retain(|(p, _)| *p != phase);
    }

    /// Record one `gate check`/`gate next` evaluation of `phase`: passing
    /// clears the streak, failing increments it. Callers decide when an
    /// evaluation doesn't count at all (a trust-blocked failure) by
    /// simply not calling this.
    pub fn record_gate_evaluation(&mut self, phase: Phase, passed: bool) {
        if passed {
            self.clear_failure_streak(phase);
            return;
        }
        let n = self.failure_streak(phase) + 1;
        match self.failure_streaks.iter_mut().find(|(p, _)| *p == phase) {
            Some(entry) => entry.1 = n,
            None => self.failure_streaks.push((phase, n)),
        }
    }

    /// Total failed gate evaluations `phase` has ever recorded (see
    /// `failure_counts`). A phase never failed reads as 0.
    pub fn failure_count(&self, phase: Phase) -> i64 {
        self.failure_counts
            .iter()
            .find(|(p, _)| *p == phase)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }

    /// Persist one failed gate evaluation of `phase` into `run.json`'s
    /// audit trail: bumps the permanent per-phase count (`failure_counts`)
    /// and appends a `Failed` history event naming which checks failed,
    /// then prunes the oldest `Failed` events past
    /// `MAX_FAILURE_HISTORY_EVENTS`. Independent of `record_gate_evaluation`
    /// (the streak/loop-enforcement bookkeeping) - callers that exclude a
    /// trust-blocked failure from the streak make the same choice here.
    pub fn record_gate_failure_event(&mut self, phase: Phase, failed_checks: &[String]) {
        match self.failure_counts.iter_mut().find(|(p, _)| *p == phase) {
            Some(entry) => entry.1 += 1,
            None => self.failure_counts.push((phase, 1)),
        }
        self.history.push(HistoryEntry {
            phase,
            event: HistoryEvent::Failed,
            at: now_iso(),
            detail: Some(failed_checks.join(", ")),
            tree_hash: None,
        });
        self.prune_failure_history();
    }

    /// Drop the oldest `Failed` history entries past
    /// `MAX_FAILURE_HISTORY_EVENTS`, leaving every other event untouched.
    fn prune_failure_history(&mut self) {
        let failed_count = self
            .history
            .iter()
            .filter(|h| h.event == HistoryEvent::Failed)
            .count();
        let Some(mut to_drop) = failed_count.checked_sub(MAX_FAILURE_HISTORY_EVENTS) else {
            return;
        };
        if to_drop == 0 {
            return;
        }
        self.history.retain(|h| {
            if h.event == HistoryEvent::Failed && to_drop > 0 {
                to_drop -= 1;
                false
            } else {
                true
            }
        });
    }
}

fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Int(n) => Some(*n),
        Value::Float(f) => Some(*f as i64),
        _ => None,
    }
}

/// `String(parsed.schema)` for the "unsupported schema" error message: a
/// missing key prints as `"undefined"`, matching JS's `String(undefined)`.
fn schema_repr(v: Option<&Value>) -> String {
    match v {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Int(n)) => n.to_string(),
        Some(Value::Float(f)) => json::format_float(*f),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => json::stringify_compact(other),
    }
}

fn history_entry_from_json(v: &Value) -> HistoryEntry {
    HistoryEntry {
        phase: v
            .get("phase")
            .and_then(|x| x.as_str())
            .and_then(Phase::from_str_opt)
            .unwrap_or(Phase::Plan),
        event: v
            .get("event")
            .and_then(|x| x.as_str())
            .and_then(HistoryEvent::from_str_opt)
            .unwrap_or(HistoryEvent::Entered),
        at: v
            .get("at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        detail: v.get("detail").and_then(|x| x.as_str()).map(String::from),
        tree_hash: v.get("treeHash").and_then(|x| x.as_str()).map(String::from),
    }
}

fn history_entry_to_json(h: &HistoryEntry) -> Value {
    let mut o = Value::object();
    o.insert("phase", h.phase.as_str());
    o.insert("event", h.event.as_str());
    o.insert("at", h.at.as_str());
    if let Some(d) = &h.detail {
        o.insert("detail", d.as_str());
    }
    if let Some(t) = &h.tree_hash {
        o.insert("treeHash", t.as_str());
    }
    o
}

fn override_entry_from_json(v: &Value) -> OverrideEntry {
    OverrideEntry {
        phase: v
            .get("phase")
            .and_then(|x| x.as_str())
            .and_then(Phase::from_str_opt)
            .unwrap_or(Phase::Plan),
        action: v
            .get("action")
            .and_then(|x| x.as_str())
            .map(OverrideAction::from_str_opt)
            .unwrap_or(OverrideAction::Skip),
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

fn override_entry_to_json(o: &OverrideEntry) -> Value {
    let mut v = Value::object();
    v.insert("phase", o.phase.as_str());
    v.insert("action", o.action.as_str());
    v.insert("reason", o.reason.as_str());
    v.insert("at", o.at.as_str());
    v.insert("by", o.by.clone());
    v
}

fn artifact_entry_from_json(v: &Value) -> ArtifactEntry {
    ArtifactEntry {
        phase: v
            .get("phase")
            .and_then(|x| x.as_str())
            .and_then(Phase::from_str_opt)
            .unwrap_or(Phase::Plan),
        at: v
            .get("at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

fn artifact_entry_to_json(a: &ArtifactEntry) -> Value {
    let mut v = Value::object();
    v.insert("phase", a.phase.as_str());
    v.insert("at", a.at.as_str());
    v
}

fn approval_from_json(v: &Value) -> Approval {
    Approval {
        by: v.get("by").and_then(|x| x.as_str()).map(String::from),
        at: v
            .get("at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        reason: v.get("reason").and_then(|x| x.as_str()).map(String::from),
        plan_hash: v
            .get("planHash")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

fn approval_to_json(a: &Approval) -> Value {
    let mut o = Value::object();
    o.insert("by", a.by.clone());
    o.insert("at", a.at.as_str());
    o.insert("reason", a.reason.clone());
    o.insert("planHash", a.plan_hash.as_str());
    o
}

fn amendment_from_json(v: &Value) -> Amendment {
    Amendment {
        plan_hash: v
            .get("planHash")
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

fn amendment_to_json(a: &Amendment) -> Value {
    let mut o = Value::object();
    o.insert("planHash", a.plan_hash.as_str());
    o.insert("at", a.at.as_str());
    o.insert("by", a.by.clone());
    o
}

fn retro_sync_from_json(v: &Value) -> RetroSync {
    RetroSync {
        journal_file: v
            .get("journalFile")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        synced_at: v
            .get("syncedAt")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        method: v
            .get("method")
            .and_then(|x| x.as_str())
            .and_then(RetroMethod::from_str_opt)
            .unwrap_or(RetroMethod::Fallback),
    }
}

fn retro_sync_to_json(r: &RetroSync) -> Value {
    let mut o = Value::object();
    o.insert("journalFile", r.journal_file.as_str());
    o.insert("syncedAt", r.synced_at.as_str());
    o.insert("method", r.method.as_str());
    o
}

/// `requestedBy: legacy.requestedBy ?? legacy.reviewer ?? null` handles both
/// the schema-1 legacy shape (`reviewer`) and the current shape
/// (`requestedBy`) uniformly - a schema-1 review's `reviewer` key is simply
/// the only one of the two present, so this same extraction is correct both
/// pre- and post- schema-1 migration without a separate migration step.
fn review_request_from_json(v: &Value) -> ReviewRequest {
    ReviewRequest {
        requested_by: v
            .get("requestedBy")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("reviewer").and_then(|x| x.as_str()))
            .map(String::from),
        requested_at: v
            .get("requestedAt")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        tree_hash: v.get("treeHash").and_then(|x| x.as_str()).map(String::from),
    }
}

fn review_request_to_json(r: &ReviewRequest) -> Value {
    let mut o = Value::object();
    o.insert("requestedBy", r.requested_by.clone());
    o.insert("requestedAt", r.requested_at.as_str());
    o.insert("treeHash", r.tree_hash.clone());
    o
}

fn phase_counts_to_json(streaks: &[(Phase, i64)]) -> Value {
    let mut obj = Value::object();
    for (phase, n) in streaks {
        obj.insert(phase.as_str(), *n);
    }
    obj
}

fn phase_counts_from_json(v: &Value) -> Vec<(Phase, i64)> {
    let Some(entries) = v.as_object() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(k, v)| {
            let phase = Phase::from_str_opt(k)?;
            let n = as_i64(v)?;
            if n > 0 {
                Some((phase, n))
            } else {
                None
            }
        })
        .collect()
}

/// `JSON.stringify(run, null, 2)`'s exact key order/shape for a `Run`.
pub fn run_to_json(run: &Run) -> Value {
    let mut o = Value::object();
    o.insert("schema", run.schema);
    o.insert("id", run.id.as_str());
    o.insert("title", run.title.as_str());
    o.insert("profile", run.profile.as_str());
    o.insert("phase", run.phase.as_str());
    o.insert("status", run.status.as_str());
    o.insert("createdAt", run.created_at.as_str());
    o.insert("updatedAt", run.updated_at.as_str());
    o.insert("branch", run.branch.clone());
    o.insert("baseRef", run.base_ref.clone());
    if let Some(t) = &run.target_override {
        if !t.is_empty() {
            o.insert("targetOverride", t.clone());
        }
    }
    o.insert("sessionId", run.session_id.clone());
    o.insert(
        "history",
        Value::Array(run.history.iter().map(history_entry_to_json).collect()),
    );
    o.insert(
        "overrides",
        Value::Array(run.overrides.iter().map(override_entry_to_json).collect()),
    );
    let mut artifacts = Value::object();
    for (name, entry) in &run.artifacts {
        artifacts.insert(name.as_str(), artifact_entry_to_json(entry));
    }
    o.insert("artifacts", artifacts);
    if !run.failure_streaks.is_empty() {
        o.insert("failureStreaks", phase_counts_to_json(&run.failure_streaks));
    }
    if !run.failure_counts.is_empty() {
        o.insert("failureCounts", phase_counts_to_json(&run.failure_counts));
    }
    if let Some(a) = &run.approval {
        o.insert("approval", approval_to_json(a));
    }
    if let Some(a) = &run.amendment {
        o.insert("amendment", amendment_to_json(a));
    }
    if let Some(r) = &run.review {
        o.insert("review", review_request_to_json(r));
    }
    if let Some(r) = &run.retro {
        o.insert("retro", retro_sync_to_json(r));
    }
    o
}

/// Migrate + parse a raw `run.json` document into a `Run`. Schema 1
/// (Milestone 1) predates profiles and review requests: it gains `profile:
/// "feature"` and any `review.reviewer` becomes `review.requestedBy`.
/// Schema 2 (Milestone 1-3) predates branch-keyed concurrency: it gains
/// `branch: null`. Schema 3 (Milestone 1-4) predates amend/drift tracking:
/// `amendment` stays absent. Unsupported/future schemas are rejected.
fn parse_run(raw: &Value, run_id: &str) -> Result<Run, UserError> {
    if raw.as_object().is_none() {
        return Err(UserError::new(format!(
            "run \"{run_id}\" is malformed: run.json is not an object"
        )));
    }

    let schema_raw = raw.get("schema");
    let mut schema = schema_raw.and_then(as_i64);

    let mut profile = raw
        .get("profile")
        .and_then(|v| v.as_str())
        .map(String::from);
    if schema == Some(1) {
        if profile.is_none() {
            profile = Some("feature".to_string());
        }
        schema = Some(2);
    }
    if schema == Some(2) {
        schema = Some(3);
    }
    if schema == Some(3) {
        schema = Some(4);
    }
    if schema != Some(CURRENT_SCHEMA) {
        return Err(UserError::new(format!(
            "Unsupported run.json schema {} in {run_id}.",
            schema_repr(schema_raw)
        )));
    }

    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or(run_id)
        .to_string();
    let title = raw
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let profile = profile.unwrap_or_else(|| "feature".to_string());
    let phase = raw
        .get("phase")
        .and_then(|v| v.as_str())
        .and_then(Phase::from_str_opt)
        .unwrap_or(Phase::Plan);
    let status = raw
        .get("status")
        .and_then(|v| v.as_str())
        .and_then(RunStatus::from_str_opt)
        .unwrap_or(RunStatus::Active);
    let created_at = raw
        .get("createdAt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let updated_at = raw
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // branch ?? null (schema 2->3): missing/null both collapse to None here.
    let branch = raw.get("branch").and_then(|v| v.as_str()).map(String::from);
    let base_ref = raw
        .get("baseRef")
        .and_then(|v| v.as_str())
        .map(String::from);
    let target_override = raw
        .get("targetOverride")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
    let session_id = raw
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(String::from);
    let history = raw
        .get("history")
        .and_then(|v| v.as_array())
        .map(|items| items.iter().map(history_entry_from_json).collect())
        .unwrap_or_default();
    let overrides = raw
        .get("overrides")
        .and_then(|v| v.as_array())
        .map(|items| items.iter().map(override_entry_from_json).collect())
        .unwrap_or_default();
    let artifacts = raw
        .get("artifacts")
        .and_then(|v| v.as_object())
        .map(|entries| {
            entries
                .iter()
                .map(|(k, v)| (k.clone(), artifact_entry_from_json(v)))
                .collect()
        })
        .unwrap_or_default();
    let failure_streaks = raw
        .get("failureStreaks")
        .map(phase_counts_from_json)
        .unwrap_or_default();
    let failure_counts = raw
        .get("failureCounts")
        .map(phase_counts_from_json)
        .unwrap_or_default();
    let approval = raw.get("approval").map(approval_from_json);
    let amendment = raw.get("amendment").map(amendment_from_json);
    let review = raw.get("review").map(review_request_from_json);
    let retro = raw.get("retro").map(retro_sync_from_json);

    Ok(Run {
        schema: CURRENT_SCHEMA,
        id,
        title,
        profile,
        phase,
        status,
        created_at,
        updated_at,
        branch,
        base_ref,
        target_override,
        session_id,
        history,
        overrides,
        artifacts,
        failure_streaks,
        failure_counts,
        approval,
        amendment,
        review,
        retro,
    })
}

pub fn read_run(root: &Path, run_id: &str) -> Result<Run, UserError> {
    let run_json = run_paths(root, run_id).run_json;
    if !run_json.exists() {
        return Err(UserError::new(format!(
            "Run \"{run_id}\" not found ({}).",
            run_json.display()
        )));
    }
    let text = fs::read_to_string(&run_json).map_err(|e| UserError::new(e.to_string()))?;
    let parsed = json::parse(&text).map_err(|e| UserError::new(e.to_string()))?;
    parse_run(&parsed, run_id)
}

/// Atomic: a crash mid-write must never corrupt the run (all state on disk).
pub fn write_run(root: &Path, run: &mut Run) -> io::Result<()> {
    run.updated_at = now_iso();
    let run_json = run_paths(root, &run.id).run_json;
    let text = json::stringify_pretty(&run_to_json(run)) + "\n";
    write_file_atomic(&run_json, &text)
}

/// Days-since-epoch -> (year, month, day), proleptic Gregorian calendar.
/// Howard Hinnant's `civil_from_days` (public domain); see module docs.
/// The crate's single copy - report/prune/agnosgram_write import it from here.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// `new Date().toISOString()`: UTC, millisecond precision.
pub fn now_iso() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = secs.div_euclid(86400);
    let secs_of_day = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let h = secs_of_day / 3600;
    let mi = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{millis:03}Z")
}

fn slugify(title: &str) -> String {
    let mut replaced = String::new();
    let mut in_run = false;
    for ch in title.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            replaced.push(ch);
            in_run = false;
        } else if !in_run {
            replaced.push('-');
            in_run = true;
        }
    }
    let trimmed = replaced.trim_matches('-');
    let sliced: String = trimmed.chars().take(48).collect();
    if sliced.is_empty() {
        "run".to_string()
    } else {
        sliced
    }
}

/// Deterministic run id: `YYYY-MM-DD-<slug>`, deduped by caller if needed.
/// TS's `makeRunId(title, date = new Date())` takes an optional date for
/// testability; `day` here plays that role (pass `&now_iso()[..10]` for the
/// production default - see `make_run_id`).
pub fn make_run_id_on(title: &str, day: &str) -> String {
    format!("{day}-{}", slugify(title))
}

pub fn make_run_id(title: &str) -> String {
    make_run_id_on(title, &now_iso()[..10])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-run-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_run_json(root: &Path, run_id: &str, content: &str) {
        let dir = root.join(".gate").join("runs").join(run_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("run.json"), content).unwrap();
    }

    #[test]
    fn now_iso_matches_a_known_utc_instant() {
        // node -e 'console.log(new Date(1755871950180).toISOString())'
        // -> "2026-08-22T13:12:30.180Z"
        let secs = 1787404350i64;
        let days = secs.div_euclid(86400);
        let (y, m, d) = civil_from_days(days);
        assert_eq!((y, m, d), (2026, 8, 22));
    }

    #[test]
    fn civil_from_days_matches_the_unix_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn writes_atomically_valid_json_on_disk_no_temp_file_left_behind() {
        let root = tmp_dir("write-atomic");
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        write_run(&root, &mut run).unwrap();
        let path = root.join(".gate/runs/r1/run.json");
        assert!(!Path::new(&format!("{}.tmp", path.display())).exists());
        let text = fs::read_to_string(&path).unwrap();
        let parsed = json::parse(&text).unwrap();
        assert_eq!(parsed.get("id").unwrap().as_str(), Some("r1"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn matches_the_real_ts_binarys_freshly_started_run_json_byte_for_byte() {
        // node dist/cli.js init && node dist/cli.js start "My First Run" in a
        // fresh temp git repo produced this exact run.json (branch "master",
        // a real baseRef sha, id "2026-08-22-my-first-run"):
        let mut run = new_run(NewRunParams {
            id: "2026-08-22-my-first-run".to_string(),
            title: "My First Run".to_string(),
            profile: "feature".to_string(),
            branch: Some("master".to_string()),
            base_ref: Some("a35fa5966607d0c58338514db43cd97123f2bc5e".to_string()),
            session_id: None,
            target_override: None,
        });
        run.created_at = "2026-08-22T13:12:30.180Z".to_string();
        run.updated_at = "2026-08-22T13:12:30.180Z".to_string();
        run.history[0].at = "2026-08-22T13:12:30.180Z".to_string();
        let text = json::stringify_pretty(&run_to_json(&run)) + "\n";
        let expected = "{\n  \"schema\": 4,\n  \"id\": \"2026-08-22-my-first-run\",\n  \"title\": \"My First Run\",\n  \"profile\": \"feature\",\n  \"phase\": \"PLAN\",\n  \"status\": \"active\",\n  \"createdAt\": \"2026-08-22T13:12:30.180Z\",\n  \"updatedAt\": \"2026-08-22T13:12:30.180Z\",\n  \"branch\": \"master\",\n  \"baseRef\": \"a35fa5966607d0c58338514db43cd97123f2bc5e\",\n  \"sessionId\": null,\n  \"history\": [\n    {\n      \"phase\": \"PLAN\",\n      \"event\": \"entered\",\n      \"at\": \"2026-08-22T13:12:30.180Z\"\n    }\n  ],\n  \"overrides\": [],\n  \"artifacts\": {}\n}\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn migrates_a_schema_1_run_all_the_way_to_schema_4() {
        let root = tmp_dir("migrate-1");
        let legacy = r#"{
            "schema": 1,
            "id": "legacy",
            "title": "old run",
            "phase": "TEST",
            "status": "active",
            "createdAt": "2026-01-01T00:00:00.000Z",
            "updatedAt": "2026-01-01T00:00:00.000Z",
            "baseRef": null,
            "sessionId": null,
            "history": [{"phase": "PLAN", "event": "entered", "at": "2026-01-01T00:00:00.000Z"}],
            "overrides": [],
            "artifacts": {},
            "review": {"reviewer": "rev-1", "requestedAt": "2026-01-02T00:00:00.000Z"}
        }"#;
        write_run_json(&root, "legacy", legacy);
        let run = read_run(&root, "legacy").unwrap();
        assert_eq!(run.schema, 4);
        assert_eq!(run.profile, "feature");
        assert_eq!(run.branch, None);
        assert_eq!(
            run.review,
            Some(ReviewRequest {
                requested_by: Some("rev-1".to_string()),
                requested_at: "2026-01-02T00:00:00.000Z".to_string(),
                tree_hash: None,
            })
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn migrates_a_schema_2_run_to_schema_4_gains_branch_null_only() {
        let root = tmp_dir("migrate-2");
        let legacy = r#"{
            "schema": 2,
            "id": "pre-m4",
            "title": "concurrency-less run",
            "profile": "bugfix",
            "phase": "REVIEW",
            "status": "active",
            "createdAt": "2026-02-01T00:00:00.000Z",
            "updatedAt": "2026-02-01T00:00:00.000Z",
            "baseRef": "abc123",
            "sessionId": "sess-1",
            "history": [{"phase": "PLAN", "event": "entered", "at": "2026-02-01T00:00:00.000Z"}],
            "overrides": [],
            "artifacts": {}
        }"#;
        write_run_json(&root, "pre-m4", legacy);
        let run = read_run(&root, "pre-m4").unwrap();
        assert_eq!(run.schema, 4);
        assert_eq!(run.branch, None);
        assert_eq!(run.profile, "bugfix");
        assert_eq!(run.base_ref.as_deref(), Some("abc123"));
        assert_eq!(run.session_id.as_deref(), Some("sess-1"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn migrates_a_schema_3_run_to_schema_4_no_amendment_everything_else_untouched() {
        let root = tmp_dir("migrate-3");
        let legacy = r#"{
            "schema": 3,
            "id": "pre-m5",
            "title": "drift-less run",
            "profile": "feature",
            "phase": "IMPLEMENT",
            "status": "active",
            "createdAt": "2026-07-01T00:00:00.000Z",
            "updatedAt": "2026-07-01T00:00:00.000Z",
            "branch": "main",
            "baseRef": "def456",
            "sessionId": "sess-2",
            "history": [{"phase": "PLAN", "event": "entered", "at": "2026-07-01T00:00:00.000Z"}],
            "overrides": [],
            "artifacts": {},
            "approval": {"by": "human", "at": "2026-07-01T00:00:00.000Z", "reason": null, "planHash": "sha256:abc"}
        }"#;
        write_run_json(&root, "pre-m5", legacy);
        let run = read_run(&root, "pre-m5").unwrap();
        assert_eq!(run.schema, 4);
        assert_eq!(run.amendment, None);
        assert_eq!(run.branch.as_deref(), Some("main"));
        assert_eq!(run.approval.as_ref().unwrap().plan_hash, "sha256:abc");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_unknown_future_schemas() {
        let root = tmp_dir("future-schema");
        write_run_json(&root, "future", r#"{"schema": 99, "id": "future"}"#);
        let err = read_run(&root, "future").unwrap_err();
        assert!(err.message().contains("schema 99"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn make_run_id_slugifies_and_prefixes_the_day() {
        assert_eq!(
            make_run_id_on("Fix Login Bug!!", "2026-08-22"),
            "2026-08-22-fix-login-bug"
        );
        assert_eq!(make_run_id_on("   ", "2026-08-22"), "2026-08-22-run");
        assert_eq!(
            make_run_id_on("日本語 café", "2026-08-22"),
            "2026-08-22-caf"
        );
        assert_eq!(
            make_run_id_on(&"A".repeat(60), "2026-08-22"),
            format!("2026-08-22-{}", "a".repeat(48))
        );
        assert_eq!(make_run_id_on("--wow--", "2026-08-22"), "2026-08-22-wow");
    }

    #[test]
    fn failure_streak_starts_at_zero_and_increments_on_failure() {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        assert_eq!(run.failure_streak(Phase::Plan), 0);
        run.record_gate_evaluation(Phase::Plan, false);
        run.record_gate_evaluation(Phase::Plan, false);
        assert_eq!(run.failure_streak(Phase::Plan), 2);
        // Unrelated phase untouched.
        assert_eq!(run.failure_streak(Phase::Implement), 0);
    }

    #[test]
    fn failure_streak_clears_on_a_passing_evaluation() {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.record_gate_evaluation(Phase::Plan, false);
        run.record_gate_evaluation(Phase::Plan, false);
        run.record_gate_evaluation(Phase::Plan, true);
        assert_eq!(run.failure_streak(Phase::Plan), 0);
    }

    #[test]
    fn clear_failure_streak_resets_a_single_phase_only() {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.record_gate_evaluation(Phase::Plan, false);
        run.record_gate_evaluation(Phase::Implement, false);
        run.clear_failure_streak(Phase::Plan);
        assert_eq!(run.failure_streak(Phase::Plan), 0);
        assert_eq!(run.failure_streak(Phase::Implement), 1);
    }

    #[test]
    fn failure_streaks_round_trip_through_json_sparse() {
        let root = tmp_dir("failure-streaks-json");
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.record_gate_evaluation(Phase::Implement, false);
        run.record_gate_evaluation(Phase::Implement, false);
        write_run(&root, &mut run).unwrap();

        let text = fs::read_to_string(root.join(".gate/runs/r1/run.json")).unwrap();
        assert!(text.contains("\"failureStreaks\""));
        assert!(text.contains("\"IMPLEMENT\": 2"));

        let reloaded = read_run(&root, "r1").unwrap();
        assert_eq!(reloaded.failure_streak(Phase::Implement), 2);
        assert_eq!(reloaded.failure_streak(Phase::Plan), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_fresh_run_omits_failure_streaks_from_json_entirely() {
        let root = tmp_dir("failure-streaks-empty");
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        write_run(&root, &mut run).unwrap();
        let text = fs::read_to_string(root.join(".gate/runs/r1/run.json")).unwrap();
        assert!(!text.contains("failureStreaks"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn record_gate_failure_event_bumps_the_permanent_count_and_appends_history() {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        assert_eq!(run.failure_count(Phase::Test), 0);
        run.record_gate_failure_event(Phase::Test, &["test.suite".to_string()]);
        run.record_gate_failure_event(Phase::Test, &["test.suite".to_string()]);
        assert_eq!(run.failure_count(Phase::Test), 2);
        let failed_entries: Vec<&HistoryEntry> = run
            .history
            .iter()
            .filter(|h| h.event == HistoryEvent::Failed)
            .collect();
        assert_eq!(failed_entries.len(), 2);
        assert_eq!(failed_entries[0].detail.as_deref(), Some("test.suite"));
    }

    #[test]
    fn failure_count_never_clears_on_a_passing_evaluation_unlike_the_streak() {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.record_gate_failure_event(Phase::Test, &["c".to_string()]);
        run.record_gate_evaluation(Phase::Test, false);
        run.record_gate_evaluation(Phase::Test, true); // clears the streak
        assert_eq!(run.failure_streak(Phase::Test), 0);
        assert_eq!(run.failure_count(Phase::Test), 1);
    }

    #[test]
    fn prunes_the_oldest_failed_history_events_past_the_cap_keeping_the_count_accurate() {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        for i in 0..(MAX_FAILURE_HISTORY_EVENTS + 5) {
            run.record_gate_failure_event(Phase::Test, &[format!("attempt-{i}")]);
        }
        let failed_entries: Vec<&HistoryEntry> = run
            .history
            .iter()
            .filter(|h| h.event == HistoryEvent::Failed)
            .collect();
        assert_eq!(failed_entries.len(), MAX_FAILURE_HISTORY_EVENTS);
        // The oldest entries were dropped; the most recent ones survive.
        assert_eq!(
            failed_entries.last().unwrap().detail.as_deref(),
            Some(format!("attempt-{}", MAX_FAILURE_HISTORY_EVENTS + 4).as_str())
        );
        // The permanent count is unaffected by pruning.
        assert_eq!(
            run.failure_count(Phase::Test),
            (MAX_FAILURE_HISTORY_EVENTS + 5) as i64
        );
    }

    #[test]
    fn failure_counts_round_trip_through_json_sparse() {
        let root = tmp_dir("failure-counts-json");
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.record_gate_failure_event(Phase::Implement, &["implement.build".to_string()]);
        run.record_gate_failure_event(Phase::Implement, &["implement.build".to_string()]);
        write_run(&root, &mut run).unwrap();

        let text = fs::read_to_string(root.join(".gate/runs/r1/run.json")).unwrap();
        assert!(text.contains("\"failureCounts\""));
        assert!(text.contains("\"IMPLEMENT\": 2"));

        let reloaded = read_run(&root, "r1").unwrap();
        assert_eq!(reloaded.failure_count(Phase::Implement), 2);
        assert_eq!(reloaded.failure_count(Phase::Plan), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn override_action_round_trips_and_defaults_unknown_to_skip() {
        assert_eq!(OverrideAction::from_str_opt("skip"), OverrideAction::Skip);
        assert_eq!(
            OverrideAction::from_str_opt("streak_reset"),
            OverrideAction::StreakReset
        );
        assert_eq!(
            OverrideAction::from_str_opt("garbage"),
            OverrideAction::Skip
        );
        assert_eq!(OverrideAction::Skip.as_str(), "skip");
        assert_eq!(OverrideAction::StreakReset.as_str(), "streak_reset");
    }

    #[test]
    fn override_entry_json_round_trips_the_action_field() {
        let entry = OverrideEntry {
            phase: Phase::Test,
            action: OverrideAction::StreakReset,
            reason: "reviewed, retry warranted".to_string(),
            at: "2026-08-23T00:00:00.000Z".to_string(),
            by: Some("human".to_string()),
        };
        let json = override_entry_to_json(&entry);
        assert_eq!(json.get("action").unwrap().as_str(), Some("streak_reset"));
        let back = override_entry_from_json(&json);
        assert_eq!(back, entry);
    }
}
