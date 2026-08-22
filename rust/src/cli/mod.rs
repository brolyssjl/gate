//! `src/cli/` - argv parsing, output formatting, and (wave 4) run context
//! resolution. `main.rs` is the port of `src/cli.ts` itself (dispatch,
//! help, version, exit codes), kept at the crate root like agnosgram's.

pub mod args;
pub mod context;
pub mod output;
