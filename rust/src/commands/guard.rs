//! Wave-1 stub for `src/commands/guard.ts`. Replaced in a later wave
//! (see `docs/rust-port.md`'s wave plan) without touching this file's
//! signature or `main.rs`'s dispatch.

use crate::cli::output::UserError;

pub fn run(_argv: Vec<String>) -> Result<(), UserError> {
    Err(UserError::new("not yet ported"))
}
