//! Port of `src/commands/playbook.ts`: `gate playbook [phase] [--run <id>]` -
//! print the active playbook (agents call this at each phase entry).

use std::path::Path;

use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::{require_active_run, require_root};
use crate::cli::output::{emit, UserError};
use crate::core::config::{load_config, GateConfig};
use crate::core::current::current_run_id_or_null;
use crate::core::json::Value;
use crate::core::playbooks::resolve_playbook_with_overlays;
use crate::core::run::{read_run, Run};
use crate::core::state_machine::Phase;
use crate::core::targets::resolve_display_targets;
use crate::integrations::{detect, plan_hints};

fn safe_read_run(root: &Path, id: &str) -> Option<Run> {
    read_run(root, id).ok()
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["playbook".to_string()];
    full.extend(argv);
    let args = parse_args(&full);

    if let Some(arg) = args.positionals.first().cloned() {
        let root = require_root()?;
        execute_explicit_phase(&root, &arg, &args)
    } else {
        let ctx = require_active_run(Some(&args))?;
        let target_names = resolve_display_targets(
            &ctx.root,
            &ctx.run.id,
            ctx.run.base_ref.as_deref(),
            ctx.run.target_override.as_deref(),
            &ctx.config,
        );
        finish(&ctx.root, ctx.run.phase, &ctx.config, &target_names, &args)
    }
}

/// The `gate playbook <phase>` branch, minus resolving `root` from the
/// process's current directory - split out so it can be unit-tested against
/// an explicit root without touching global process state (see
/// `trust::execute`'s doc comment for why).
fn execute_explicit_phase(root: &Path, arg: &str, args: &ParsedArgs) -> Result<(), UserError> {
    let upper = arg.to_uppercase();
    let Some(phase) = Phase::from_str_opt(&upper) else {
        return Err(UserError::usage(format!("unknown phase \"{arg}\"")));
    };
    let config = load_config(root)?;
    // No active-run context to reuse here - an explicit phase argument works
    // even with no run started, so targets (if any) come from whatever run
    // is named. `--run` picks a specific run's targets (honored here too);
    // otherwise whatever run is current for this branch, if any. An
    // unreadable/missing run degrades to no targets rather than failing.
    let explicit_run_id = args.flags.str("run").map(str::to_string);
    let run_id = match explicit_run_id {
        Some(id) => Some(id),
        None => current_run_id_or_null(root)?,
    };
    let run = run_id.and_then(|id| safe_read_run(root, &id));
    let target_names = match &run {
        Some(r) => resolve_display_targets(
            root,
            &r.id,
            r.base_ref.as_deref(),
            r.target_override.as_deref(),
            &config,
        ),
        None => Vec::new(),
    };
    finish(root, phase, &config, &target_names, args)
}

/// The shared tail: resolve the playbook + target overlays, append PLAN
/// integration hints, and emit. Takes everything as plain values so it can
/// be unit-tested without any process-state dependency at all.
fn finish(
    root: &Path,
    phase: Phase,
    config: &GateConfig,
    target_names: &[String],
    args: &ParsedArgs,
) -> Result<(), UserError> {
    let Some(playbook) = resolve_playbook_with_overlays(root, phase, config, target_names) else {
        return Err(UserError::new(format!("no playbook for phase {phase}")));
    };

    let hints: Vec<String> = if phase == Phase::Plan {
        plan_hints(detect(root))
    } else {
        Vec::new()
    };
    let human = if hints.is_empty() {
        playbook.clone()
    } else {
        format!(
            "{playbook}\n\n## Detected integrations\n{}",
            hints
                .iter()
                .map(|h| format!("- {h}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let mut data = Value::object();
    data.insert("phase", phase.as_str());
    data.insert("playbook", playbook);
    data.insert("hints", hints);
    emit(&human, &data, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-playbook-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".gate")).unwrap();
        dir
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["playbook".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn prints_the_base_playbook_for_an_explicit_phase_with_no_run() {
        let root = tmp_dir("explicit-phase");
        execute_explicit_phase(&root, "plan", &args(&["plan"])).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_an_unknown_phase_argument() {
        let root = tmp_dir("unknown-phase");
        let err = execute_explicit_phase(&root, "nope", &args(&["nope"])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.message().contains("nope"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn appends_a_detected_integrations_section_for_the_plan_phase_only() {
        let root = tmp_dir("plan-hints");
        fs::create_dir_all(root.join(".agnosgram")).unwrap();
        execute_explicit_phase(&root, "plan", &args(&["plan", "--json"])).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finish_errors_when_the_phase_has_no_playbook() {
        let root = tmp_dir("no-playbook-phase");
        let err = finish(&root, Phase::Done, &GateConfig::default(), &[], &args(&[])).unwrap_err();
        assert!(err.message().contains("no playbook for phase DONE"));
        fs::remove_dir_all(&root).unwrap();
    }
}
