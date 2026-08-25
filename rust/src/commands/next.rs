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
use crate::commands::streak::blocked_error;
use crate::core::git::tree_fingerprint;
use crate::core::json::Value;
use crate::core::playbooks::resolve_playbook_with_overlays;
use crate::core::run::{now_iso, write_run, HistoryEntry, HistoryEvent};
use crate::core::state_machine::is_terminal;
use crate::core::targets::resolve_display_targets;
use crate::gates::run_gate;
use crate::gates::types::GateContext;
use crate::integrations::sdd_mapping::{closing_hint, resolve as resolve_sdd_mapping};
use crate::integrations::{detect, sdd_integration_enabled};

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

    // Loop-enforcement cap: refuse to evaluate at all once a phase has
    // failed `limit` times in a row - a streak refusal never itself counts
    // as a failure (see `blocked_error`'s callers).
    let phase = run.phase;
    if let Some(limit) = config.failure_streak_cap() {
        let streak = run.failure_streak(phase);
        if streak >= limit {
            return Err(blocked_error(phase, streak, limit));
        }
    }

    let gate_ctx = GateContext { root, run, config };
    let res = run_gate(&gate_ctx);
    let GateContext {
        root,
        mut run,
        config,
    } = gate_ctx;

    // A trust-blocked failure doesn't count toward the streak - see
    // `GateResult::only_trust_blocked`.
    if res.ok || !res.only_trust_blocked() {
        run.record_gate_evaluation(phase, res.ok);
    }

    if !res.ok {
        // Record the failed advancement attempt so `gate report` can show
        // it (both the bounded history event and the permanent per-phase
        // count - see `Run::record_gate_failure_event`).
        let failed: Vec<String> = res
            .checks
            .iter()
            .filter(|c| !c.ok)
            .map(|c| c.name.clone())
            .collect();
        run.record_gate_failure_event(run.phase, &failed);
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
            let det = detect(&root);
            if let Some(sdd) = det.sdd {
                if sdd_integration_enabled(&config) {
                    let mapping = resolve_sdd_mapping(sdd, &config.sdd_mapping_override);
                    if let Some(hint) = closing_hint(sdd, &mapping) {
                        println!("{hint}");
                    }
                }
            }
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
