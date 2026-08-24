//! Port of `src/commands/check.ts`.
//!
//! `gate check` - run the current phase's gate; print pass/fail with
//! reasons. Exit code IS the verdict (0 pass, 1 fail) so CI and agents can
//! branch on it. Gate never advances here; use `gate next` to advance on
//! pass.

use crate::cli::args::parse_args;
use crate::cli::context::{require_active_run, ActiveContext};
use crate::cli::output::UserError;
use crate::commands::gate_run::render_gate;
use crate::commands::streak::blocked_error;
use crate::core::run::write_run;
use crate::gates::run_gate;
use crate::gates::types::GateContext;

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    // Re-prepend a placeholder command token before `parse_args`: it treats
    // a leading non-flag token as the command name, and every other
    // already-ported command module follows this same convention so a
    // command's own leading flags/positionals are never misread as one
    // (see `report::run`/`guard::run`, which need it for a real reason -
    // their leading token can be a genuine positional).
    let mut full = vec!["check".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let ActiveContext { root, run, config } = require_active_run(Some(&args))?;

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
    let GateContext { root, mut run, .. } = gate_ctx;

    // `gate check` stays otherwise side-effect-free: only touch run.json
    // when the streak itself actually changes (a failure, or a pass that
    // clears a prior streak), never on a repeated no-op pass. A trust-
    // blocked failure doesn't count at all - see
    // `GateResult::only_trust_blocked`.
    let before = run.failure_streak(phase);
    if res.ok || !res.only_trust_blocked() {
        run.record_gate_evaluation(phase, res.ok);
    }
    // A real failure is always persisted into the run's audit trail (a
    // `Failed` history event plus the permanent per-phase count), even the
    // trust-blocked case - matching `gate next`'s existing failure
    // recording (`commands/next.rs`) so `gate check` and `gate next`
    // failures show up identically in `gate report`, not just in
    // scrollback.
    if !res.ok {
        let failed: Vec<String> = res
            .checks
            .iter()
            .filter(|c| !c.ok)
            .map(|c| c.name.clone())
            .collect();
        run.record_gate_failure_event(phase, &failed);
    }
    if !res.ok || run.failure_streak(phase) != before {
        write_run(&root, &mut run).map_err(|e| UserError::new(e.to_string()))?;
    }

    let ok = render_gate(&res, Vec::new(), &args.flags)?;
    std::process::exit(if ok { 0 } else { 1 });
}
