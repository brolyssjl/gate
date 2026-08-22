//! Port of `src/commands/check.ts`.
//!
//! `gate check` - run the current phase's gate; print pass/fail with
//! reasons. Exit code IS the verdict (0 pass, 1 fail) so CI and agents can
//! branch on it. Gate never advances here; use `gate next` to advance on
//! pass.

use crate::cli::args::parse_args;
use crate::cli::context::require_active_run;
use crate::cli::output::UserError;
use crate::commands::gate_run::render_gate;
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
    let ctx = require_active_run(Some(&args))?;
    let gate_ctx = GateContext {
        root: ctx.root,
        run: ctx.run,
        config: ctx.config,
    };
    let res = run_gate(&gate_ctx);
    let ok = render_gate(&res, Vec::new(), &args.flags)?;
    std::process::exit(if ok { 0 } else { 1 });
}
