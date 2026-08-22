//! Port of `src/commands/next.ts`.
//!
//! `gate next` - check the current gate and advance on pass. On failure it
//! prints exactly what's missing and exits non-zero without changing
//! state.

use crate::cli::args::parse_args;
use crate::cli::context::{require_active_run, ActiveContext};
use crate::cli::output::UserError;
use crate::commands::advance::advance;
use crate::commands::gate_run::render_gate;
use crate::core::git::tree_fingerprint;
use crate::core::json::Value;
use crate::core::playbooks::resolve_playbook_with_overlays;
use crate::core::run::{now_iso, write_run, HistoryEntry, HistoryEvent};
use crate::core::state_machine::is_terminal;
use crate::core::targets::resolve_display_targets;
use crate::gates::run_gate;
use crate::gates::types::GateContext;

/// `args.flags.json !== true && args.flags.format === undefined`: whether
/// the extra human-only informational line (DONE banner / entered-phase
/// playbook) should print. Suppressed under any structured output request.
fn wants_human_extra(args: &crate::cli::args::ParsedArgs) -> bool {
    !args.flags.is_true("json") && !args.flags.is_present("format")
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    // See `check::run`'s comment: prepend a placeholder command token so
    // `parse_args` never misreads a leading flag/positional as the command.
    let mut full = vec!["next".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let ctx = require_active_run(Some(&args))?;
    let ActiveContext { root, run, config } = ctx;

    let gate_ctx = GateContext { root, run, config };
    let res = run_gate(&gate_ctx);
    let GateContext {
        root,
        mut run,
        config,
    } = gate_ctx;

    if !res.ok {
        // Record the failed advancement attempt so `gate report` can show
        // it. `gate check` stays side-effect free; `next` is the
        // deliberate attempt to advance.
        let failed: Vec<String> = res
            .checks
            .iter()
            .filter(|c| !c.ok)
            .map(|c| c.name.clone())
            .collect();
        run.history.push(HistoryEntry {
            phase: run.phase,
            event: HistoryEvent::Failed,
            at: now_iso(),
            detail: Some(failed.join(", ")),
            tree_hash: None,
        });
        write_run(&root, &mut run).map_err(|e| UserError::new(e.to_string()))?;
        render_gate(
            &res,
            vec![("advanced".to_string(), Value::from(false))],
            &args.flags,
        )?;
        std::process::exit(1);
    }

    let from = run.phase;
    // The fingerprint pins which tree this gate certified; the REVIEW gate
    // re-verifies build/lint/test when the tree drifts afterward
    // (staleness).
    run.history.push(HistoryEntry {
        phase: from,
        event: HistoryEvent::Passed,
        at: now_iso(),
        detail: Some(format!("{from} gate passed")),
        tree_hash: tree_fingerprint(&root),
    });
    advance(&root, &mut run)?;

    if is_terminal(run.phase) {
        render_gate(
            &res,
            vec![
                ("advanced".to_string(), Value::from(true)),
                ("from".to_string(), Value::from(from.as_str())),
                ("to".to_string(), Value::from(run.phase.as_str())),
                ("done".to_string(), Value::from(true)),
            ],
            &args.flags,
        )?;
        if wants_human_extra(&args) {
            print!("\nRun \"{}\" reached DONE. \u{1F389}\n", run.id);
        }
        return Ok(());
    }

    let target_names = resolve_display_targets(
        &root,
        &run.id,
        run.base_ref.as_deref(),
        run.target_override.as_deref(),
        &config,
    );
    let playbook = resolve_playbook_with_overlays(&root, run.phase, &config, &target_names)
        .unwrap_or_default();
    render_gate(
        &res,
        vec![
            ("advanced".to_string(), Value::from(true)),
            ("from".to_string(), Value::from(from.as_str())),
            ("to".to_string(), Value::from(run.phase.as_str())),
        ],
        &args.flags,
    )?;
    if wants_human_extra(&args) && !playbook.is_empty() {
        print!("\nEntered {}.\n\n{playbook}\n", run.phase);
    }
    Ok(())
}
