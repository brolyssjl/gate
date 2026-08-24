//! `gate streak` / `gate streak reset` - not a TS port (new, a
//! failure-streak loop-enforcement cap). `gate streak` shows each phase's
//! consecutive `gate check`/`gate next` failure count against the
//! configured cap;
//! `gate streak reset [<phase>] --reason "..."` is one of the two ways past
//! a blocked phase (the other is `gate skip`) - an explicit, reasoned,
//! audited act, never automatic, the same discipline the README's threat
//! model already applies to `gate skip`/`gate trust`/`gate approve`.

use std::path::Path;

use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::{require_active_run, require_root};
use crate::cli::output::{emit, UserError};
use crate::commands::report::most_recent_run_id;
use crate::core::config::{load_config, GateConfig};
use crate::core::current::{read_current_run_id, resolve_branch_key, BranchKeyResolution};
use crate::core::json::Value;
use crate::core::paths::validate_run_id;
use crate::core::run::{
    now_iso, read_run, write_run, OverrideAction, OverrideEntry, Run, RunStatus,
};
use crate::core::state_machine::{Phase, GATED_PHASES};

/// The message `gate check`/`gate next` return when a phase is blocked -
/// distinct from a normal gate failure, naming both recovery paths.
pub fn blocked_error(phase: Phase, streak: i64, limit: i64) -> UserError {
    UserError::gate(
        format!(
            "{phase} blocked: {streak} consecutive failures (limit {limit}). \
See the run's worklog/debug log for what's failing, then either \
`gate skip {phase} --reason \"<why>\"` or \
`gate streak reset {phase} --reason \"<why>\"` once a human has reviewed \
and a retry is warranted."
        ),
        3,
    )
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    // See `check::run`'s comment: prepend a placeholder command token so
    // `parse_args` never misreads a leading flag/positional as the command
    // (the same workaround `guard.rs` uses for its own install/uninstall/run
    // subcommand).
    let mut full = vec!["streak".to_string()];
    full.extend(argv);
    let args = parse_args(&full);

    match args.positionals.first().map(String::as_str) {
        // Read-only: also resolves a run that has already reached DONE (or
        // any other non-active status), unlike `require_active_run` - a
        // finished run still has a final streak state worth reporting
        // (usually all-zero, since a phase's streak clears the moment it
        // passes) instead of just erroring because there's nothing "active"
        // for the branch anymore.
        None => {
            let root = require_root()?;
            let config = load_config(&root)?;
            let run = resolve_run_for_show(&root, &args)?;
            show(&run, &config, &args)
        }
        Some("reset") => {
            let ctx = require_active_run(Some(&args))?;
            reset(&ctx.root, ctx.run, &args)
        }
        Some(other) => Err(UserError::usage(format!(
            "unknown gate streak subcommand \"{other}\" - use `gate streak` or `gate streak reset`"
        ))),
    }
}

/// Resolve which run `gate streak` (no subcommand) reports on: `--run <id>`
/// wins outright (any status - unlike `require_active_run`'s `--run`, which
/// only accepts an active run); otherwise the branch's active run, falling
/// back to the most recently updated run overall (live or just-finished) so
/// a run that reached DONE and had its "current" pointer cleared
/// (`clear_run_everywhere`, called from `advance`) is still reachable
/// without an explicit id. Mirrors `gate report`'s own fallback chain.
fn resolve_run_for_show(root: &Path, args: &ParsedArgs) -> Result<Run, UserError> {
    if let Some(id) = args.flags.str("run") {
        validate_run_id(id)?;
        return read_run(root, id);
    }
    let resolved = resolve_branch_key(root);
    let BranchKeyResolution::Key(key) = &resolved else {
        return Err(UserError::new(
            "HEAD is detached - no branch to resolve a run from; pass a run id: gate streak --run <id>",
        ));
    };
    let id = read_current_run_id(root, key)?
        .or_else(|| most_recent_run_id(root))
        .ok_or_else(|| {
            UserError::new(
                "no run to show a streak for - start one with `gate start \"<title>\"`, or pass a run id: gate streak --run <id>",
            )
        })?;
    read_run(root, &id)
}

fn show(run: &Run, config: &GateConfig, args: &ParsedArgs) -> Result<(), UserError> {
    let limit = config.failure_streak_cap();
    let mut lines = vec![format!(
        "Run \"{}\" - current phase {}{}",
        run.id,
        run.phase,
        if run.status == RunStatus::Active {
            String::new()
        } else {
            format!(" (status: {})", run.status.as_str())
        }
    )];
    lines.push(match limit {
        Some(n) => format!("Failure-streak cap: {n} (thresholds.failure_streak_limit)"),
        None => "Failure-streak cap: disabled (thresholds.failure_streak_limit: 0)".to_string(),
    });

    let mut streaks = Value::object();
    let mut any_nonzero = false;
    for phase in GATED_PHASES {
        let n = run.failure_streak(phase);
        streaks.insert(phase.as_str(), n);
        if n > 0 {
            any_nonzero = true;
            let blocked = limit.map(|l| n >= l).unwrap_or(false);
            let marker = if blocked { " (BLOCKED)" } else { "" };
            lines.push(format!(
                "  {:<10} {n} consecutive failure(s){marker}",
                phase.as_str()
            ));
        }
    }
    if !any_nonzero {
        lines.push("  no phase has a failure streak".to_string());
    }

    let mut data = Value::object();
    data.insert("phase", run.phase.as_str());
    data.insert("status", run.status.as_str());
    data.insert("limit", limit);
    data.insert("streaks", streaks);
    emit(&lines.join("\n"), &data, &args.flags)
}

fn reset(root: &Path, mut run: Run, args: &ParsedArgs) -> Result<(), UserError> {
    let phase = match args.positionals.get(1) {
        Some(p) => {
            let up = p.to_uppercase();
            Phase::from_str_opt(&up)
                .ok_or_else(|| UserError::usage(format!("unknown phase \"{p}\"")))?
        }
        None => run.phase,
    };

    let reason = args
        .flags
        .str("reason")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if reason.is_empty() {
        return Err(UserError::usage(
            "gate streak reset requires --reason \"<why>\"",
        ));
    }

    let previous = run.failure_streak(phase);
    if previous == 0 {
        return Err(UserError::new(format!(
            "{phase} has no failure streak to reset (currently 0)"
        )));
    }

    let at = now_iso();
    let by = args
        .flags
        .str("by")
        .map(str::to_string)
        .or_else(|| std::env::var("GATE_SESSION_ID").ok());
    run.overrides.push(OverrideEntry {
        phase,
        action: OverrideAction::StreakReset,
        reason: reason.clone(),
        at,
        by,
    });
    run.clear_failure_streak(phase);
    write_run(root, &mut run).map_err(|e| UserError::new(e.to_string()))?;

    let human = format!("Reset {phase}'s failure streak ({previous} \u{2192} 0; reason: {reason})");
    let mut data = Value::object();
    data.insert("phase", phase.as_str());
    data.insert("previousStreak", previous);
    data.insert("reason", reason);
    emit(&human, &data, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::run::{new_run, NewRunParams};
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-streak-rs-{name}-{}", std::process::id()));
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
        let mut full = vec!["streak".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn reset_clears_the_streak_and_records_an_audited_override() {
        let root = tmp_dir("reset-ok");
        let mut run = setup_run(&root);
        run.record_gate_evaluation(Phase::Plan, false);
        run.record_gate_evaluation(Phase::Plan, false);
        write_run(&root, &mut run).unwrap();
        let id = run.id.clone();

        reset(&root, run, &args(&["reset", "--reason", "human reviewed"])).unwrap();

        let saved = crate::core::run::read_run(&root, &id).unwrap();
        assert_eq!(saved.failure_streak(Phase::Plan), 0);
        assert_eq!(saved.overrides.len(), 1);
        assert_eq!(saved.overrides[0].action, OverrideAction::StreakReset);
        assert_eq!(saved.overrides[0].reason, "human reviewed");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reset_requires_a_reason() {
        let root = tmp_dir("reset-no-reason");
        let mut run = setup_run(&root);
        run.record_gate_evaluation(Phase::Plan, false);
        let err = reset(&root, run, &args(&["reset"])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn reset_refuses_when_the_streak_is_already_zero() {
        let root = tmp_dir("reset-already-zero");
        let run = setup_run(&root);
        let err = reset(&root, run, &args(&["reset", "--reason", "x"])).unwrap_err();
        assert!(err.message().contains("no failure streak to reset"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reset_accepts_an_explicit_phase_argument() {
        let root = tmp_dir("reset-explicit-phase");
        let mut run = setup_run(&root);
        run.phase = Phase::Implement;
        run.record_gate_evaluation(Phase::Plan, false);
        write_run(&root, &mut run).unwrap();
        let id = run.id.clone();

        reset(&root, run, &args(&["reset", "PLAN", "--reason", "x"])).unwrap();

        let saved = crate::core::run::read_run(&root, &id).unwrap();
        assert_eq!(saved.failure_streak(Phase::Plan), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reset_records_who_authorized_it_from_the_by_flag() {
        let root = tmp_dir("reset-by");
        let mut run = setup_run(&root);
        run.record_gate_evaluation(Phase::Plan, false);
        write_run(&root, &mut run).unwrap();
        let id = run.id.clone();

        reset(
            &root,
            run,
            &args(&["reset", "--reason", "x", "--by", "alice"]),
        )
        .unwrap();

        let saved = crate::core::run::read_run(&root, &id).unwrap();
        assert_eq!(saved.overrides[0].by.as_deref(), Some("alice"));
        fs::remove_dir_all(&root).unwrap();
    }
}
