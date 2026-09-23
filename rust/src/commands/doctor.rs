//! `gate doctor` (new command, read-only): diagnose a project's gate
//! installation - adapter blocks, playbook copies under `.gate/playbooks/`,
//! trust, and config sanity - and print findings with severity. `diagnose`
//! below is the single source of truth for what's wrong; `gate update`
//! (`commands::update`) reuses it to decide what to fix, so the two commands
//! can never disagree about what's broken.
//!
//! Exit-code contract (same shape as `gate trust --check`): 0 when nothing
//! actionable was found, 1 when at least one `action`-severity finding
//! exists. `info`-severity findings (a user-edited playbook, a repo with no
//! SDD detected) never affect the exit code - they're context, not a
//! problem.

use std::fs;
use std::path::Path;

use crate::adapters::{adapter_keys, get_adapter, resolve_pointer_body};
use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::core::config::load_config;
use crate::core::json::Value;
use crate::core::markers::{has_managed_block, managed_block_matches};
use crate::core::paths::gate_paths;
use crate::core::playbook_manifest::{content_hash, read_manifest};
use crate::core::playbooks::current_bundled_playbooks;
use crate::core::state_machine::Phase;
use crate::core::trust::{
    current_commands_hash, diagnose_mismatch, is_commands_trusted, read_trust, MismatchReason,
};
use crate::integrations::{detect, sdd_integration_enabled};

/// Every phase that has a playbook (all of `state_machine::PHASES` except
/// the terminal DONE, which has none).
pub const PLAYBOOK_PHASES: [Phase; 6] = [
    Phase::Plan,
    Phase::Debug,
    Phase::Implement,
    Phase::Test,
    Phase::Review,
    Phase::Retro,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Action,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Action => "action",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterStatus {
    /// A default adapter (claude/agents) whose target file doesn't exist at all.
    Missing,
    /// The target file exists but carries no gate managed block.
    BlockMissing,
    /// A managed block exists but doesn't match the current pointer body.
    Outdated,
    Current,
}

impl AdapterStatus {
    pub fn severity(&self) -> Severity {
        match self {
            AdapterStatus::Current => Severity::Info,
            _ => Severity::Action,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AdapterStatus::Missing => "missing",
            AdapterStatus::BlockMissing => "block-missing",
            AdapterStatus::Outdated => "outdated",
            AdapterStatus::Current => "current",
        }
    }

    fn message(&self) -> &'static str {
        match self {
            AdapterStatus::Missing => "not installed - `gate update` will install it",
            AdapterStatus::BlockMissing => {
                "file exists but has no gate managed block - `gate update` will add one"
            }
            AdapterStatus::Outdated => "managed block is stale - `gate update` will refresh it",
            AdapterStatus::Current => "current",
        }
    }
}

pub struct AdapterFinding {
    pub adapter_key: &'static str,
    pub path: &'static str,
    pub status: AdapterStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybookStatus {
    /// On-disk content matches the current bundled default exactly.
    Current,
    /// Unchanged since materialized, but the bundled default has since
    /// moved on - safe to refresh automatically.
    PristineOutdated,
    /// Content diverges from the manifest's recorded source hash - a human
    /// edited it after materialization. Never auto-replaced without force.
    UserEdited,
    Missing,
    /// No manifest entry for this file (pre-manifest legacy, or a hand-
    /// deleted manifest) and content doesn't match the current bundled
    /// default either - provenance genuinely unknown.
    NoManifest,
}

impl PlaybookStatus {
    pub fn severity(&self) -> Severity {
        match self {
            PlaybookStatus::Current => Severity::Info,
            PlaybookStatus::PristineOutdated => Severity::Action,
            PlaybookStatus::Missing => Severity::Action,
            PlaybookStatus::UserEdited => Severity::Info,
            PlaybookStatus::NoManifest => Severity::Info,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PlaybookStatus::Current => "current",
            PlaybookStatus::PristineOutdated => "pristine-outdated",
            PlaybookStatus::UserEdited => "user-edited",
            PlaybookStatus::Missing => "missing",
            PlaybookStatus::NoManifest => "no-manifest",
        }
    }

    fn message(&self) -> &'static str {
        match self {
            PlaybookStatus::Current => "current",
            PlaybookStatus::PristineOutdated => {
                "pristine but outdated (bundled content changed since this copy was made) - `gate update` will refresh it"
            }
            PlaybookStatus::UserEdited => {
                "user-edited - left alone; pass --force-playbooks to `gate update` to replace it"
            }
            PlaybookStatus::Missing => "missing - `gate update` will materialize it",
            PlaybookStatus::NoManifest => {
                "unknown provenance (predates gate's playbook manifest) - left alone unless forced"
            }
        }
    }
}

pub struct PlaybookFinding {
    pub file: String,
    pub status: PlaybookStatus,
}

pub struct TrustFinding {
    pub current_hash: String,
    pub stored_hash: Option<String>,
    pub reason: Option<MismatchReason>,
}

/// A `.gate/trust.json` still present in the repo (security audit
/// 2026-09-22, finding 1: trust moved to a machine-local store; a
/// repo-tracked copy is stale at best, and misleading - it looks like it
/// still matters). `gate init` no longer creates this file; this finding is
/// the migration path for a repo that predates that change.
pub struct LegacyTrustFileFinding {
    pub path: std::path::PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigFinding {
    Missing,
    Invalid(String),
}

pub struct DoctorReport {
    /// `(framework name, composition currently active)` - `None` when no SDD
    /// is detected at all.
    pub sdd: Option<(String, bool)>,
    pub adapters: Vec<AdapterFinding>,
    pub playbooks: Vec<PlaybookFinding>,
    pub trust: Option<TrustFinding>,
    pub legacy_trust_file: Option<LegacyTrustFileFinding>,
    pub config: Vec<ConfigFinding>,
}

impl DoctorReport {
    pub fn actionable_count(&self) -> usize {
        self.adapters
            .iter()
            .filter(|f| f.status.severity() == Severity::Action)
            .count()
            + self
                .playbooks
                .iter()
                .filter(|f| f.status.severity() == Severity::Action)
                .count()
            + self.trust.is_some() as usize
            + self.legacy_trust_file.is_some() as usize
            + self.config.len()
    }
}

/// Diagnose `root`'s gate installation. Read-only - never writes anything.
pub fn diagnose(root: &Path) -> DoctorReport {
    let body = resolve_pointer_body(root);
    let det = detect(root);
    let config = load_config(root).unwrap_or_default();
    let sdd = det
        .sdd
        .map(|s| (s.as_str().to_string(), sdd_integration_enabled(&config)));
    DoctorReport {
        sdd,
        adapters: diagnose_adapters(root, &body),
        playbooks: diagnose_playbooks(root),
        trust: diagnose_trust(root),
        legacy_trust_file: diagnose_legacy_trust_file(root),
        config: diagnose_config(root),
    }
}

/// Every default adapter (claude/agents, always checked) plus every other
/// adapter whose target file already exists on disk ("installed" - an
/// adapter never requested is never reported as missing).
fn diagnose_adapters(root: &Path, body: &str) -> Vec<AdapterFinding> {
    let mut out = Vec::new();
    for key in adapter_keys() {
        let adapter = get_adapter(key).expect("adapter_keys() entries always resolve");
        let is_default = key == "claude" || key == "agents";
        let target = root.join(adapter.target_path);
        let status = if !target.exists() {
            if !is_default {
                continue;
            }
            AdapterStatus::Missing
        } else {
            let existing = fs::read_to_string(&target).unwrap_or_default();
            if !has_managed_block(&existing) {
                AdapterStatus::BlockMissing
            } else if !managed_block_matches(&existing, body) {
                AdapterStatus::Outdated
            } else {
                AdapterStatus::Current
            }
        };
        out.push(AdapterFinding {
            adapter_key: adapter.key,
            path: adapter.target_path,
            status,
        });
    }
    out
}

fn diagnose_playbooks(root: &Path) -> Vec<PlaybookFinding> {
    let manifest = read_manifest(root);
    let bundled = current_bundled_playbooks();
    let dir = gate_paths(root).playbooks;
    let mut out = Vec::with_capacity(PLAYBOOK_PHASES.len());
    for phase in PLAYBOOK_PHASES {
        let name = format!("{}.md", phase.as_str().to_lowercase());
        let path = dir.join(&name);
        let current_hash = bundled
            .iter()
            .find(|(n, _)| n == &name)
            .map(|(_, c)| content_hash(c));
        let status = if !path.exists() {
            PlaybookStatus::Missing
        } else {
            let on_disk = fs::read_to_string(&path).unwrap_or_default();
            let on_disk_hash = content_hash(&on_disk);
            if Some(&on_disk_hash) == current_hash.as_ref() {
                PlaybookStatus::Current
            } else {
                match manifest.get(&name) {
                    Some(entry) if entry.source_hash == on_disk_hash => {
                        PlaybookStatus::PristineOutdated
                    }
                    Some(_) => PlaybookStatus::UserEdited,
                    None => PlaybookStatus::NoManifest,
                }
            }
        };
        out.push(PlaybookFinding { file: name, status });
    }
    out
}

fn diagnose_trust(root: &Path) -> Option<TrustFinding> {
    if is_commands_trusted(root) {
        return None;
    }
    let record = read_trust(root);
    let reason = record.as_ref().map(|r| diagnose_mismatch(root, r));
    Some(TrustFinding {
        current_hash: current_commands_hash(root),
        stored_hash: record.map(|r| r.commands_hash),
        reason,
    })
}

/// A `.gate/trust.json` in the repo is inert (finding 1) but still worth
/// flagging: it's easy to mistake for the thing that matters. Read-only -
/// existence check, never parsed or compared against anything.
fn diagnose_legacy_trust_file(root: &Path) -> Option<LegacyTrustFileFinding> {
    let path = gate_paths(root).gate.join("trust.json");
    if path.exists() {
        Some(LegacyTrustFileFinding { path })
    } else {
        None
    }
}

fn diagnose_config(root: &Path) -> Vec<ConfigFinding> {
    let mut out = Vec::new();
    if !gate_paths(root).config.exists() {
        out.push(ConfigFinding::Missing);
    } else if let Err(e) = load_config(root) {
        out.push(ConfigFinding::Invalid(e.message().to_string()));
    }
    out
}

fn trust_message(t: &TrustFinding) -> String {
    match t.reason {
        Some(MismatchReason::CoverageExpanded) => {
            "gate's trust coverage expanded in this version - review the newly covered playbooks and run `gate trust`".to_string()
        }
        Some(MismatchReason::Changed) | None => {
            "commands block is not trusted (or has drifted) - run `gate trust`".to_string()
        }
    }
}

/// `trust.legacy-file`: named so it can be grepped for/scripted against
/// like the other finding kinds, even though this crate doesn't (yet) carry
/// stable string ids on every finding.
fn legacy_trust_file_message(f: &LegacyTrustFileFinding) -> String {
    match crate::core::trust::env_config_dir() {
        Some(dir) => format!(
            "`{}` is tracked in the repo but no longer used for anything - trust is now stored per machine under `{}`; delete this file",
            f.path.display(),
            dir.display()
        ),
        None => format!(
            "`{}` is tracked in the repo but no longer used for anything - trust is now stored per machine (set GATE_CONFIG_DIR, XDG_CONFIG_HOME, or HOME to see where); delete this file",
            f.path.display()
        ),
    }
}

fn config_message(c: &ConfigFinding) -> String {
    match c {
        ConfigFinding::Missing => {
            "`.gate/config.yml` is missing - run `gate init` to scaffold it".to_string()
        }
        ConfigFinding::Invalid(msg) => format!("`.gate/config.yml` is invalid: {msg}"),
    }
}

fn render_human(report: &DoctorReport, actionable: usize) -> String {
    let mut lines = Vec::new();
    lines.push(match &report.sdd {
        Some((name, true)) => format!("SDD: {name} detected, composed pointer body active"),
        Some((name, false)) => {
            format!("SDD: {name} detected, but integrations.sdd is off - generic pointer body")
        }
        None => "SDD: none detected".to_string(),
    });
    lines.push(String::new());
    lines.push("Adapters:".to_string());
    for f in &report.adapters {
        lines.push(format!(
            "  [{}] {}: {}",
            f.status.severity().as_str(),
            f.path,
            f.status.message()
        ));
    }
    lines.push(String::new());
    lines.push("Playbooks (.gate/playbooks/):".to_string());
    for f in &report.playbooks {
        lines.push(format!(
            "  [{}] {}: {}",
            f.status.severity().as_str(),
            f.file,
            f.status.message()
        ));
    }
    lines.push(String::new());
    lines.push("Trust:".to_string());
    match &report.trust {
        None => lines
            .push("  [info] commands block trusted on this machine, for this checkout".to_string()),
        Some(t) => lines.push(format!("  [action] {}", trust_message(t))),
    }
    if let Some(f) = &report.legacy_trust_file {
        lines.push(format!("  [action] {}", legacy_trust_file_message(f)));
    }
    if !report.config.is_empty() {
        lines.push(String::new());
        lines.push("Config:".to_string());
        for c in &report.config {
            lines.push(format!("  [action] {}", config_message(c)));
        }
    }
    lines.push(String::new());
    lines.push(if actionable == 0 {
        "gate doctor: nothing actionable found.".to_string()
    } else {
        format!(
            "gate doctor: {actionable} actionable finding(s) - run `gate update` to apply the auto-fixable ones."
        )
    });
    lines.join("\n")
}

fn report_to_json(report: &DoctorReport, actionable: usize) -> Value {
    let mut data = Value::object();
    data.insert("actionable", actionable as i64);

    data.insert(
        "sdd",
        match &report.sdd {
            Some((name, composed)) => {
                let mut o = Value::object();
                o.insert("framework", name.as_str());
                o.insert("composed", *composed);
                o
            }
            None => Value::Null,
        },
    );

    let mut adapters = Value::array();
    for f in &report.adapters {
        let mut o = Value::object();
        o.insert("adapter", f.adapter_key);
        o.insert("path", f.path);
        o.insert("status", f.status.as_str());
        o.insert("severity", f.status.severity().as_str());
        adapters.push(o);
    }
    data.insert("adapters", adapters);

    let mut playbooks = Value::array();
    for f in &report.playbooks {
        let mut o = Value::object();
        o.insert("file", f.file.as_str());
        o.insert("status", f.status.as_str());
        o.insert("severity", f.status.severity().as_str());
        playbooks.push(o);
    }
    data.insert("playbooks", playbooks);

    let mut trust = Value::object();
    match &report.trust {
        Some(t) => {
            trust.insert("trusted", false);
            trust.insert("currentHash", t.current_hash.clone());
            trust.insert("storedHash", t.stored_hash.clone());
            trust.insert(
                "mismatchReason",
                t.reason.map(|r| match r {
                    MismatchReason::CoverageExpanded => "coverageExpanded",
                    MismatchReason::Changed => "changed",
                }),
            );
        }
        None => {
            trust.insert("trusted", true);
        }
    }
    data.insert("trust", trust);

    data.insert(
        "legacyTrustFile",
        match &report.legacy_trust_file {
            Some(f) => {
                let mut o = Value::object();
                o.insert("id", "trust.legacy-file");
                o.insert("path", f.path.display().to_string());
                o.insert("message", legacy_trust_file_message(f));
                o
            }
            None => Value::Null,
        },
    );

    let mut config = Value::array();
    for c in &report.config {
        let mut o = Value::object();
        match c {
            ConfigFinding::Missing => {
                o.insert("kind", "missing");
            }
            ConfigFinding::Invalid(msg) => {
                o.insert("kind", "invalid");
                o.insert("message", msg.as_str());
            }
        }
        config.push(o);
    }
    data.insert("config", config);

    data
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["doctor".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = require_root()?;
    execute(&root, &args)
}

fn execute(root: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    let report = diagnose(root);
    let actionable = report.actionable_count();
    let human = render_human(&report, actionable);
    let data = report_to_json(&report, actionable);
    emit(&human, &data, &args.flags)?;
    // Mirrors `gate trust --check`: verdict-as-exit-code, not the `gate:
    // <message>` error path - the report is already on stdout either way.
    std::process::exit(if actionable == 0 { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-doctor-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".gate")).unwrap();
        dir
    }

    #[test]
    fn a_freshly_initialized_project_has_only_missing_default_adapters_and_playbooks() {
        let root = tmp_dir("fresh");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        let report = diagnose(&root);

        let claude = report
            .adapters
            .iter()
            .find(|f| f.adapter_key == "claude")
            .unwrap();
        assert_eq!(claude.status, AdapterStatus::Missing);
        let agents = report
            .adapters
            .iter()
            .find(|f| f.adapter_key == "agents")
            .unwrap();
        assert_eq!(agents.status, AdapterStatus::Missing);
        // Non-default, never-installed adapters are not reported at all.
        assert!(!report.adapters.iter().any(|f| f.adapter_key == "cursor"));

        assert!(report
            .playbooks
            .iter()
            .all(|f| f.status == PlaybookStatus::Missing));
        assert!(report.trust.is_none()); // trivially trusted - no commands configured
        assert!(report.config.is_empty());
        assert!(report.actionable_count() > 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_current_adapter_block_reports_as_current() {
        let root = tmp_dir("current-adapter");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        let body = resolve_pointer_body(&root);
        let adapter = get_adapter("claude").unwrap();
        crate::commands::adapt::apply_adapter(&root, adapter, &body).unwrap();

        let report = diagnose(&root);
        let claude = report
            .adapters
            .iter()
            .find(|f| f.adapter_key == "claude")
            .unwrap();
        assert_eq!(claude.status, AdapterStatus::Current);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_hand_edited_block_that_no_longer_matches_reports_outdated() {
        let root = tmp_dir("outdated-adapter");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        fs::write(
            root.join("CLAUDE.md"),
            "<!-- gate:start -->\n<!-- Managed by gate. Edits inside this block are overwritten on `gate adapt`. -->\nold body\n<!-- gate:end -->\n",
        )
        .unwrap();

        let report = diagnose(&root);
        let claude = report
            .adapters
            .iter()
            .find(|f| f.adapter_key == "claude")
            .unwrap();
        assert_eq!(claude.status, AdapterStatus::Outdated);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_with_no_managed_block_at_all_reports_block_missing() {
        let root = tmp_dir("block-missing");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        fs::write(root.join("AGENTS.md"), "# just some notes\n").unwrap();

        let report = diagnose(&root);
        let agents = report
            .adapters
            .iter()
            .find(|f| f.adapter_key == "agents")
            .unwrap();
        assert_eq!(agents.status, AdapterStatus::BlockMissing);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn playbook_drift_matrix_current_pristine_outdated_user_edited_no_manifest() {
        let root = tmp_dir("drift-matrix");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        let dir = root.join(".gate/playbooks");
        fs::create_dir_all(&dir).unwrap();
        let bundled = current_bundled_playbooks();
        let plan_current = bundled
            .iter()
            .find(|(n, _)| n == "plan.md")
            .unwrap()
            .1
            .clone();

        // current: content matches the bundled default exactly.
        fs::write(dir.join("plan.md"), &plan_current).unwrap();

        // pristine-outdated: manifest says it came from an older source that
        // no longer matches current bundled content.
        fs::write(dir.join("test.md"), "# old test playbook\n").unwrap();
        crate::core::playbook_manifest::record_entry(&root, "test.md", "# old test playbook\n")
            .unwrap();

        // user-edited: manifest recorded one source, on-disk content is
        // neither that nor the current bundled default.
        fs::write(dir.join("implement.md"), "# original\n").unwrap();
        crate::core::playbook_manifest::record_entry(&root, "implement.md", "# original\n")
            .unwrap();
        fs::write(dir.join("implement.md"), "# hand-edited by a human\n").unwrap();

        // no-manifest: content present, no manifest entry, doesn't match
        // current bundled content.
        fs::write(dir.join("review.md"), "# pre-manifest legacy copy\n").unwrap();

        // missing: retro.md/debug.md never materialized.

        let report = diagnose(&root);
        let status_of = |file: &str| {
            report
                .playbooks
                .iter()
                .find(|f| f.file == file)
                .unwrap()
                .status
        };
        assert_eq!(status_of("plan.md"), PlaybookStatus::Current);
        assert_eq!(status_of("test.md"), PlaybookStatus::PristineOutdated);
        assert_eq!(status_of("implement.md"), PlaybookStatus::UserEdited);
        assert_eq!(status_of("review.md"), PlaybookStatus::NoManifest);
        assert_eq!(status_of("retro.md"), PlaybookStatus::Missing);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn trust_finding_is_none_when_trivially_trusted_and_present_when_not() {
        let root = tmp_dir("trust-diagnosis");
        fs::write(
            root.join(".gate/config.yml"),
            "commands:\n  test: echo hi\n",
        )
        .unwrap();
        assert!(diagnose(&root).trust.is_some());

        crate::core::trust::write_trust(&root, None).unwrap();
        assert!(diagnose(&root).trust.is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn legacy_trust_json_in_the_repo_is_flagged_and_absent_by_default() {
        // Finding 1's migration path: `gate init` no longer creates
        // `.gate/trust.json`, so a fresh project has no such finding at
        // all; a repo that still carries one (pre-fix, or hand-restored)
        // gets an actionable `trust.legacy-file` warning naming it.
        let root = tmp_dir("legacy-trust-file");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        let fresh = diagnose(&root);
        assert!(fresh.legacy_trust_file.is_none());

        fs::write(
            root.join(".gate/trust.json"),
            "{\n  \"commandsHash\": \"sha256:whatever\"\n}\n",
        )
        .unwrap();
        let report = diagnose(&root);
        assert!(report.legacy_trust_file.is_some());
        assert!(report.actionable_count() > fresh.actionable_count());

        let human = render_human(&report, report.actionable_count());
        assert!(human.contains("trust.json"));
        assert!(human.contains("per machine"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn config_missing_and_invalid_are_reported() {
        let root_missing = tmp_dir("config-missing");
        assert_eq!(diagnose(&root_missing).config, vec![ConfigFinding::Missing]);
        fs::remove_dir_all(&root_missing).unwrap();

        let root_invalid = tmp_dir("config-invalid");
        fs::write(
            root_invalid.join(".gate/config.yml"),
            "targets:\n  api: nope\n",
        )
        .unwrap();
        match &diagnose(&root_invalid).config[..] {
            [ConfigFinding::Invalid(_)] => {}
            other => panic!("expected one Invalid finding, got {other:?}"),
        }
        fs::remove_dir_all(&root_invalid).unwrap();
    }

    #[test]
    fn sdd_present_and_disabled_report_composed_false() {
        let root = tmp_dir("sdd-disabled");
        fs::create_dir_all(root.join("openspec")).unwrap();
        fs::write(root.join(".gate/config.yml"), "integrations:\n  sdd: off\n").unwrap();
        let report = diagnose(&root);
        assert_eq!(report.sdd, Some(("openspec".to_string(), false)));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_freshly_initialized_project_has_actionable_findings() {
        let root = tmp_dir("dirty");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        assert!(diagnose(&root).actionable_count() > 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_fully_healed_project_has_zero_actionable_findings() {
        let root = tmp_dir("healed");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();

        let body = resolve_pointer_body(&root);
        for key in ["claude", "agents"] {
            crate::commands::adapt::apply_adapter(&root, get_adapter(key).unwrap(), &body).unwrap();
        }
        let dir = root.join(".gate/playbooks");
        fs::create_dir_all(&dir).unwrap();
        for (name, content) in current_bundled_playbooks() {
            fs::write(dir.join(&name), &content).unwrap();
            crate::core::playbook_manifest::record_entry(&root, &name, &content).unwrap();
        }
        // Playbook overrides ride in the trust hash too (`core::config`'s
        // `commands_block_hash_source`), so a repo with copies materialized
        // under `.gate/playbooks/` is never *trivially* trusted even with an
        // empty `commands:` block - "fully healed" still needs a real
        // `gate trust`, same as any other install.
        crate::core::trust::write_trust(&root, None).unwrap();

        assert_eq!(diagnose(&root).actionable_count(), 0);
        fs::remove_dir_all(&root).unwrap();
    }
}
