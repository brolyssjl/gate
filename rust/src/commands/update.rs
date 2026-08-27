//! `gate update` (new command, the healer): apply what `gate doctor`
//! diagnoses - (re)write stale/missing default adapter blocks, top up or
//! refresh playbook copies under `.gate/playbooks/`, and (with `--adapt`)
//! add new adapter targets. Managed adapter blocks are gate-owned by
//! contract, so they're always safe to regenerate; a playbook a human
//! edited is never silently replaced - `--force-playbooks` is the explicit
//! override, and forcing one is reported alongside a reminder that its
//! content now needs a fresh `gate trust` (playbook overrides ride in the
//! same trust hash as `commands:` - see `core::config`). Idempotent: run it
//! twice right after each other and the second run has nothing to do,
//! because `doctor::diagnose` (which this reuses) then reports nothing
//! actionable.

use std::fs;
use std::path::Path;

use crate::adapters::{adapter_keys, get_adapter, resolve_pointer_body, Adapter};
use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::commands::adapt::{apply_adapter, AdaptAction, AdaptResult};
use crate::commands::doctor::{diagnose, PlaybookStatus, Severity};
use crate::core::json::Value;
use crate::core::paths::gate_paths;
use crate::core::playbook_manifest::{read_manifest, record_entry};
use crate::core::playbooks::current_bundled_playbooks;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybookAction {
    Materialized,
    Refreshed,
    ForceReplaced,
    LeftUserEdited,
    LeftUnknownProvenance,
    /// Content already matched the current bundled default; only the
    /// manifest entry was missing, so it was backfilled quietly.
    ManifestBackfilled,
    Unchanged,
}

impl PlaybookAction {
    fn as_str(&self) -> &'static str {
        match self {
            PlaybookAction::Materialized => "materialized",
            PlaybookAction::Refreshed => "refreshed",
            PlaybookAction::ForceReplaced => "force-replaced",
            PlaybookAction::LeftUserEdited => "left (user-edited)",
            PlaybookAction::LeftUnknownProvenance => "left (unknown provenance)",
            PlaybookAction::ManifestBackfilled => "unchanged (manifest backfilled)",
            PlaybookAction::Unchanged => "unchanged",
        }
    }

    /// Counts toward the "N playbook(s) changed" summary - a manifest-only
    /// backfill touches disk but isn't a content change worth counting the
    /// same way.
    fn is_content_change(&self) -> bool {
        matches!(
            self,
            PlaybookAction::Materialized
                | PlaybookAction::Refreshed
                | PlaybookAction::ForceReplaced
        )
    }
}

pub struct PlaybookResult {
    pub file: String,
    pub action: PlaybookAction,
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["update".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = require_root()?;
    execute(&root, &args)
}

/// Parse `--adapt a,b` into the adapters it names, case-insensitively - the
/// same ergonomics as `gate adapt`'s positionals, just comma-separated
/// since `--adapt` is a flag here rather than positionals (`gate update`
/// takes no positionals of its own).
fn parse_adapt_flag(raw: Option<&str>) -> Result<Vec<&'static Adapter>, UserError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for token in raw.split(',') {
        let key = token.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        let Some(adapter) = get_adapter(&key) else {
            return Err(UserError::usage(format!(
                "unknown adapter \"{key}\" in --adapt (known: {})",
                adapter_keys().join(", ")
            )));
        };
        out.push(adapter);
    }
    Ok(out)
}

fn execute(root: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    let force_playbooks = args.flags.is_true("force-playbooks");
    let requested_adapt = parse_adapt_flag(args.flags.str("adapt"))?;

    let report = diagnose(root);
    let body = resolve_pointer_body(root);

    let mut adapter_results: Vec<AdaptResult> = Vec::new();
    let mut handled_keys: Vec<&'static str> = Vec::new();
    for finding in &report.adapters {
        if finding.status.severity() != Severity::Action {
            continue;
        }
        let adapter = get_adapter(finding.adapter_key).expect("known adapter key");
        adapter_results.push(apply_adapter(root, adapter, &body)?);
        handled_keys.push(adapter.key);
    }
    for adapter in requested_adapt {
        if handled_keys.contains(&adapter.key) {
            continue;
        }
        adapter_results.push(apply_adapter(root, adapter, &body)?);
        handled_keys.push(adapter.key);
    }

    let playbook_results = apply_playbooks(root, &report.playbooks, force_playbooks)?;

    let human = render_human(&adapter_results, &playbook_results, force_playbooks);
    let data = results_to_json(&adapter_results, &playbook_results);
    emit(&human, &data, &args.flags)
}

fn apply_playbooks(
    root: &Path,
    findings: &[crate::commands::doctor::PlaybookFinding],
    force: bool,
) -> Result<Vec<PlaybookResult>, UserError> {
    let bundled = current_bundled_playbooks();
    let manifest = read_manifest(root);
    let dir = gate_paths(root).playbooks;
    fs::create_dir_all(&dir).map_err(|e| UserError::new(e.to_string()))?;

    let mut out = Vec::with_capacity(findings.len());
    for finding in findings {
        let content = bundled
            .iter()
            .find(|(n, _)| *n == finding.file)
            .map(|(_, c)| c.as_str());
        let action = match finding.status {
            PlaybookStatus::Current => {
                if manifest.contains_key(&finding.file) {
                    PlaybookAction::Unchanged
                } else if let Some(content) = content {
                    record_entry(root, &finding.file, content)
                        .map_err(|e| UserError::new(e.to_string()))?;
                    PlaybookAction::ManifestBackfilled
                } else {
                    PlaybookAction::Unchanged
                }
            }
            PlaybookStatus::Missing => match content {
                Some(content) => {
                    write_and_record(root, &dir, &finding.file, content)?;
                    PlaybookAction::Materialized
                }
                None => PlaybookAction::Unchanged,
            },
            PlaybookStatus::PristineOutdated => match content {
                Some(content) => {
                    write_and_record(root, &dir, &finding.file, content)?;
                    PlaybookAction::Refreshed
                }
                None => PlaybookAction::Unchanged,
            },
            PlaybookStatus::UserEdited => {
                if force {
                    match content {
                        Some(content) => {
                            write_and_record(root, &dir, &finding.file, content)?;
                            PlaybookAction::ForceReplaced
                        }
                        None => PlaybookAction::LeftUserEdited,
                    }
                } else {
                    PlaybookAction::LeftUserEdited
                }
            }
            PlaybookStatus::NoManifest => {
                if force {
                    match content {
                        Some(content) => {
                            write_and_record(root, &dir, &finding.file, content)?;
                            PlaybookAction::ForceReplaced
                        }
                        None => PlaybookAction::LeftUnknownProvenance,
                    }
                } else {
                    PlaybookAction::LeftUnknownProvenance
                }
            }
        };
        out.push(PlaybookResult {
            file: finding.file.clone(),
            action,
        });
    }
    Ok(out)
}

fn write_and_record(root: &Path, dir: &Path, file: &str, content: &str) -> Result<(), UserError> {
    fs::write(dir.join(file), content).map_err(|e| UserError::new(e.to_string()))?;
    record_entry(root, file, content).map_err(|e| UserError::new(e.to_string()))
}

fn render_human(
    adapter_results: &[AdaptResult],
    playbook_results: &[PlaybookResult],
    force_playbooks: bool,
) -> String {
    let mut lines = Vec::new();
    let adapters_changed = adapter_results
        .iter()
        .filter(|r| r.action != AdaptAction::Unchanged)
        .count();
    let playbooks_changed = playbook_results
        .iter()
        .filter(|r| r.action.is_content_change())
        .count();

    lines.push("Adapters:".to_string());
    if adapter_results.is_empty() {
        lines.push("  (none needed repair, and none requested via --adapt)".to_string());
    } else {
        for r in adapter_results {
            lines.push(format!("  {:<9} {}", r.action.as_str(), r.path));
        }
    }

    lines.push(String::new());
    lines.push("Playbooks (.gate/playbooks/):".to_string());
    for r in playbook_results {
        lines.push(format!("  {:<28} {}", r.action.as_str(), r.file));
    }

    let left = playbook_results
        .iter()
        .filter(|r| {
            matches!(
                r.action,
                PlaybookAction::LeftUserEdited | PlaybookAction::LeftUnknownProvenance
            )
        })
        .count();
    if left > 0 && !force_playbooks {
        lines.push(String::new());
        lines.push(format!(
            "{left} playbook(s) left alone (user-edited or unknown provenance) - pass --force-playbooks to replace them."
        ));
    }
    let forced = playbook_results
        .iter()
        .filter(|r| r.action == PlaybookAction::ForceReplaced)
        .count();
    if forced > 0 {
        lines.push(String::new());
        lines.push(format!(
            "{forced} playbook(s) force-replaced - their content now falls under a different trust hash; run `gate trust` again."
        ));
    }

    lines.push(String::new());
    lines.push(if adapters_changed == 0 && playbooks_changed == 0 {
        "gate update: nothing to do.".to_string()
    } else {
        format!(
            "gate update: {adapters_changed} adapter(s), {playbooks_changed} playbook(s) changed."
        )
    });
    lines.join("\n")
}

fn results_to_json(adapter_results: &[AdaptResult], playbook_results: &[PlaybookResult]) -> Value {
    let mut data = Value::object();

    let mut adapters = Value::array();
    for r in adapter_results {
        let mut o = Value::object();
        o.insert("adapter", r.adapter);
        o.insert("path", r.path);
        o.insert("action", r.action.as_str());
        adapters.push(o);
    }
    data.insert("adapters", adapters);

    let mut playbooks = Value::array();
    for r in playbook_results {
        let mut o = Value::object();
        o.insert("file", r.file.as_str());
        o.insert("action", r.action.as_str());
        playbooks.push(o);
    }
    data.insert("playbooks", playbooks);

    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-update-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".gate")).unwrap();
        dir
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["update".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn installs_missing_default_adapters_and_materializes_missing_playbooks() {
        let root = tmp_dir("fresh-heal");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();

        execute(&root, &args(&[])).unwrap();

        assert!(root.join("CLAUDE.md").exists());
        assert!(root.join("AGENTS.md").exists());
        for phase in ["plan", "debug", "implement", "test", "review", "retro"] {
            assert!(root.join(format!(".gate/playbooks/{phase}.md")).exists());
        }
        assert!(root.join(".gate/playbooks.lock").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_second_run_right_after_reports_nothing_to_do() {
        let root = tmp_dir("idempotent");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        execute(&root, &args(&[])).unwrap();
        // The playbook copies `update` just materialized ride in the trust
        // hash (`core::config::commands_block_hash_source`), so they still
        // need a real `gate trust` - `update` never runs it itself (TOFU
        // approval is always a separate, deliberate human act).
        crate::core::trust::write_trust(&root, None).unwrap();

        let report = diagnose(&root);
        assert_eq!(report.actionable_count(), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn never_replaces_a_user_edited_playbook_without_force() {
        let root = tmp_dir("user-edited-protected");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        let dir = root.join(".gate/playbooks");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plan.md"), "original\n").unwrap();
        crate::core::playbook_manifest::record_entry(&root, "plan.md", "original\n").unwrap();
        fs::write(dir.join("plan.md"), "hand-edited\n").unwrap();

        execute(&root, &args(&[])).unwrap();

        assert_eq!(
            fs::read_to_string(dir.join("plan.md")).unwrap(),
            "hand-edited\n"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn force_playbooks_replaces_a_user_edited_playbook() {
        let root = tmp_dir("user-edited-forced");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        let dir = root.join(".gate/playbooks");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plan.md"), "original\n").unwrap();
        crate::core::playbook_manifest::record_entry(&root, "plan.md", "original\n").unwrap();
        fs::write(dir.join("plan.md"), "hand-edited\n").unwrap();

        execute(&root, &args(&["--force-playbooks"])).unwrap();

        let bundled_plan = current_bundled_playbooks()
            .into_iter()
            .find(|(n, _)| n == "plan.md")
            .unwrap()
            .1;
        assert_eq!(
            fs::read_to_string(dir.join("plan.md")).unwrap(),
            bundled_plan
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adapt_flag_adds_a_new_adapter_target_not_installed_by_default() {
        let root = tmp_dir("adapt-flag-adds");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();

        execute(&root, &args(&["--adapt", "cursor"])).unwrap();

        assert!(root.join(".cursor/rules/gate.mdc").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adapt_flag_rejects_an_unknown_adapter() {
        let root = tmp_dir("adapt-flag-unknown");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        let err = execute(&root, &args(&["--adapt", "nope"])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refreshes_a_pristine_outdated_playbook_and_updates_the_manifest() {
        let root = tmp_dir("pristine-outdated");
        fs::write(root.join(".gate/config.yml"), "commands: {}\n").unwrap();
        let dir = root.join(".gate/playbooks");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plan.md"), "an older bundled plan\n").unwrap();
        crate::core::playbook_manifest::record_entry(&root, "plan.md", "an older bundled plan\n")
            .unwrap();

        execute(&root, &args(&[])).unwrap();

        let bundled_plan = current_bundled_playbooks()
            .into_iter()
            .find(|(n, _)| n == "plan.md")
            .unwrap()
            .1;
        assert_eq!(
            fs::read_to_string(dir.join("plan.md")).unwrap(),
            bundled_plan
        );
        let manifest = read_manifest(&root);
        assert_eq!(
            manifest.get("plan.md").unwrap().source_hash,
            crate::core::playbook_manifest::content_hash(&bundled_plan)
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
