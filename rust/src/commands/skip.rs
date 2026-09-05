//! Port of `src/commands/skip.ts`: `gate skip <phase> --reason "..."` -
//! human-authorized skip of the current gated phase.

use std::path::Path;

use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_active_run;
use crate::cli::output::{emit, UserError};
use crate::commands::advance::advance;
use crate::core::config::load_config;
use crate::core::identity;
use crate::core::json::Value;
use crate::core::run::{
    now_iso, write_run, HistoryEntry, HistoryEvent, OverrideAction, OverrideEntry, Run,
};
use crate::core::state_machine::{can_skip, Phase};

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["skip".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let ctx = require_active_run(Some(&args))?;
    execute(&ctx.root, ctx.run, &args)
}

/// The whole command, minus resolving `root`/`run` via `require_active_run` -
/// split out so it can be unit-tested against explicit values without
/// touching global process state (see `trust::execute`'s doc comment for
/// why).
fn execute(root: &Path, mut run: Run, args: &ParsedArgs) -> Result<(), UserError> {
    let Some(arg) = args.positionals.first() else {
        return Err(UserError::usage(
            "gate skip needs a phase: gate skip <phase> --reason \"...\"",
        ));
    };
    let phase_str = arg.to_uppercase();
    let Some(phase) = Phase::from_str_opt(&phase_str) else {
        return Err(UserError::usage(format!("unknown phase \"{arg}\"")));
    };

    let reason = args
        .flags
        .str("reason")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if reason.is_empty() {
        return Err(UserError::usage("gate skip requires --reason \"<why>\""));
    }

    if phase != run.phase {
        return Err(UserError::new(format!(
            "can only skip the current phase ({}), not {}",
            run.phase, phase
        )));
    }
    if !can_skip(run.phase) {
        return Err(UserError::new(format!("{} cannot be skipped", run.phase)));
    }

    // A skip is a deliberate authorization past a gated phase - the same
    // class of sign-off as trust/approve/streak reset, so it gets the
    // full identity treatment from #29: fallback chain plus the opt-in
    // fail-closed knob (issue #33).
    let config = load_config(root)?;
    let by = identity::resolve(args, root);
    identity::require_if_configured(&by, &config)?;

    let at = now_iso();
    run.overrides.push(OverrideEntry {
        phase: run.phase,
        action: OverrideAction::Skip,
        reason: reason.clone(),
        at: at.clone(),
        by,
    });
    run.history.push(HistoryEntry {
        phase: run.phase,
        event: HistoryEvent::Skipped,
        at,
        detail: Some(reason.clone()),
        tree_hash: None,
    });
    // A skip is one of the two ways past a blocked phase: the human has
    // explicitly moved on, so the failure streak that was blocking it no
    // longer applies.
    run.clear_failure_streak(run.phase);
    write_run(root, &mut run).map_err(|e| UserError::new(e.to_string()))?;
    let result = advance(root, &mut run)?;

    let human = format!(
        "Skipped {} (reason: {}) \u{2192} {}",
        result.from, reason, result.to
    );
    let mut data = Value::object();
    data.insert("skipped", result.from.as_str());
    data.insert("to", result.to.as_str());
    data.insert("reason", reason);
    emit(&human, &data, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::paths::run_paths;
    use crate::core::run::{new_run, NewRunParams};
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-skip-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup_run(root: &Path) -> Run {
        fs::create_dir_all(root.join(".gate")).unwrap();
        let mut r = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        write_run(root, &mut r).unwrap();
        r
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["skip".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn skips_the_current_phase_and_advances() {
        let root = tmp_dir("skip-plan");
        let run = setup_run(&root);
        let id = run.id.clone();
        execute(&root, run, &args(&["plan", "--reason", "trust me"])).unwrap();

        let saved = crate::core::run::read_run(&root, &id).unwrap();
        assert_eq!(saved.phase, Phase::Implement);
        assert_eq!(saved.overrides.len(), 1);
        assert_eq!(saved.overrides[0].reason, "trust me");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn requires_a_reason() {
        let root = tmp_dir("skip-no-reason");
        let run = setup_run(&root);
        let err = execute(&root, run, &args(&["plan"])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_skipping_a_phase_other_than_the_current_one() {
        let root = tmp_dir("skip-wrong-phase");
        let run = setup_run(&root);
        let err = execute(&root, run, &args(&["review", "--reason", "x"])).unwrap_err();
        assert!(err.message().contains("can only skip the current phase"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_an_unknown_phase_name() {
        let root = tmp_dir("skip-unknown-phase");
        let run = setup_run(&root);
        let err = execute(&root, run, &args(&["nope", "--reason", "x"])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn records_who_authorized_the_skip_from_the_by_flag() {
        let root = tmp_dir("skip-by");
        let run = setup_run(&root);
        let id = run.id.clone();
        execute(
            &root,
            run,
            &args(&["plan", "--reason", "x", "--by", "alice"]),
        )
        .unwrap();
        let saved = crate::core::run::read_run(&root, &id).unwrap();
        assert_eq!(saved.overrides[0].by.as_deref(), Some("alice"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn uses_run_paths_for_the_scaffolded_run_directory() {
        let root = tmp_dir("paths-sanity");
        let run = setup_run(&root);
        assert!(run_paths(&root, &run.id).run_json.exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
