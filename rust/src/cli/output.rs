//! Port of `src/cli/output.ts`: output-format resolution, structured
//! emission, and the two error types every command signals failure with.
//!
//! TS models `UsageError`/`GateError` as two `Error` subclasses that
//! `cli.ts`'s `reportError` distinguishes with `instanceof`. Rust commands
//! return `Result<(), UserError>` (see `docs/rust-port.md`'s wave-1 stub
//! contract), so this unifies both into one enum `main.rs` matches on for
//! the exact same exit-code mapping: `Usage` -> exit 2, `Gate` -> exit
//! `code` (default 1, matching `GateError`'s `exitCode = 1` default). Both
//! variants print identically (`gate: <message>`) - only the exit code
//! differs, exactly as `reportError` does it.

use crate::cli::args::Flags;
use crate::core::json::Value;
use crate::serialize::{self, Format};

/// Port of `UsageError`/`GateError`. Every fallible command returns
/// `Result<(), UserError>`; wave-1 stubs use `UserError::new("not yet
/// ported")` (a `Gate` error with the default exit code 1 - the same
/// bucket TS's `reportError` falls back to for a plain, non-`GateError`
/// exception).
#[derive(Debug, Clone, PartialEq)]
pub enum UserError {
    /// Port of `UsageError`: bad CLI usage, exit 2.
    Usage(String),
    /// Port of `GateError`: an expected failure (no run, gate red), exit
    /// `code` (default 1).
    Gate(String, i32),
}

impl UserError {
    /// The common case: a `GateError` with the default exit code (1).
    pub fn new(message: impl Into<String>) -> Self {
        UserError::Gate(message.into(), 1)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        UserError::Usage(message.into())
    }

    pub fn gate(message: impl Into<String>, exit_code: i32) -> Self {
        UserError::Gate(message.into(), exit_code)
    }

    pub fn message(&self) -> &str {
        match self {
            UserError::Usage(m) => m,
            UserError::Gate(m, _) => m,
        }
    }

    /// The exact exit-code mapping from `cli.ts`'s `reportError`.
    pub fn exit_code(&self) -> i32 {
        match self {
            UserError::Usage(_) => 2,
            UserError::Gate(_, code) => *code,
        }
    }
}

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for UserError {}

/// `resolveFormat`'s TS return type is `Format | "human"`; this is that
/// union.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Structured(Format),
}

/// Resolve the output format from `--format <fmt>` / `--json` flags.
pub fn resolve_format(flags: &Flags) -> Result<OutputFormat, UserError> {
    if let Some(fmt) = flags.str("format") {
        return serialize::parse_format(fmt)
            .map(OutputFormat::Structured)
            .ok_or_else(|| {
                UserError::usage(format!("unknown --format \"{fmt}\" (use json or toon)"))
            });
    }
    if flags.is_true("json") {
        return Ok(OutputFormat::Structured(Format::Json));
    }
    Ok(OutputFormat::Human)
}

/// Emit a command result. When a structured format is requested, print the
/// data; otherwise print the human string. This keeps `--json` output
/// schema-stable for CI and agents while humans get readable text by
/// default.
pub fn emit(human: &str, data: &Value, flags: &Flags) -> Result<(), UserError> {
    match resolve_format(flags)? {
        OutputFormat::Human => {
            if human.ends_with('\n') {
                print!("{human}");
            } else {
                println!("{human}");
            }
        }
        OutputFormat::Structured(fmt) => {
            println!("{}", serialize::serialize(data, fmt));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::parse_args;

    fn flags_from(args: &[&str]) -> Flags {
        parse_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).flags
    }

    #[test]
    fn resolves_json_via_the_json_flag() {
        assert_eq!(
            resolve_format(&flags_from(&["cmd", "--json"])).unwrap(),
            OutputFormat::Structured(Format::Json)
        );
    }

    #[test]
    fn resolves_format_flag_over_json_flag() {
        assert_eq!(
            resolve_format(&flags_from(&["cmd", "--format", "toon"])).unwrap(),
            OutputFormat::Structured(Format::Toon)
        );
    }

    #[test]
    fn defaults_to_human_with_no_flags() {
        assert_eq!(
            resolve_format(&flags_from(&["cmd"])).unwrap(),
            OutputFormat::Human
        );
    }

    #[test]
    fn rejects_an_unknown_format_as_a_usage_error() {
        let err = resolve_format(&flags_from(&["cmd", "--format", "xml"])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.message().contains("xml"));
    }

    #[test]
    fn user_error_exit_codes_match_reportError() {
        assert_eq!(UserError::usage("bad usage").exit_code(), 2);
        assert_eq!(UserError::new("plain failure").exit_code(), 1);
        assert_eq!(UserError::gate("coded failure", 1).exit_code(), 1);
    }
}
