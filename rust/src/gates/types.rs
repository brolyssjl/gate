//! Port of `src/gates/types.ts` (`Check`, `GateResult`, `GateContext`, the
//! `Gate` fn type, `pass`/`fail`/`result` helpers). Ported in wave 3 (see
//! `docs/rust-port.md`), once `core::run::Run` (wave 2) exists for
//! `GateContext` to hold.

use std::path::PathBuf;

use crate::core::config::GateConfig;
use crate::core::run::Run;
use crate::core::state_machine::Phase;

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateResult {
    pub phase: Phase,
    pub ok: bool,
    pub checks: Vec<Check>,
}

/// TS's `GateContext` takes `root: string`; the Rust port uses `PathBuf`
/// since every function it's threaded through (`core::git`, `core::exec`,
/// `core::trust`, ...) takes a `&Path`.
pub struct GateContext {
    pub root: PathBuf,
    pub run: Run,
    pub config: GateConfig,
}

pub type Gate = fn(&GateContext) -> GateResult;

pub fn pass(name: impl Into<String>, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        ok: true,
        detail: detail.into(),
    }
}

pub fn fail(name: impl Into<String>, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        ok: false,
        detail: detail.into(),
    }
}

pub fn result(phase: Phase, checks: Vec<Check>) -> GateResult {
    let ok = checks.iter().all(|c| c.ok);
    GateResult { phase, ok, checks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_and_fail_set_ok_accordingly() {
        assert!(pass("n", "").ok);
        assert!(!fail("n", "d").ok);
    }

    #[test]
    fn result_is_ok_only_when_every_check_passed() {
        let all_ok = result(Phase::Plan, vec![pass("a", ""), pass("b", "")]);
        assert!(all_ok.ok);
        let one_failed = result(Phase::Plan, vec![pass("a", ""), fail("b", "bad")]);
        assert!(!one_failed.ok);
    }

    #[test]
    fn result_with_no_checks_is_ok_vacuously() {
        assert!(result(Phase::Done, vec![]).ok);
    }
}
