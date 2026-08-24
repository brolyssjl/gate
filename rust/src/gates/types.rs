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
    /// True for a failing check caused by Gate refusing to spawn an
    /// untrusted command (`gate trust`), rather than the phase's actual
    /// criteria failing. `GateResult::only_trust_blocked` uses this so
    /// `gate check`/`gate next` can exclude it from the failure-streak
    /// count - retrying gives the identical result until a human
    /// runs `gate trust`, so counting it toward the loop-enforcement cap
    /// would block on a config problem the cap itself can't fix.
    pub trust_blocked: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateResult {
    pub phase: Phase,
    pub ok: bool,
    pub checks: Vec<Check>,
}

impl GateResult {
    /// True when the gate failed and every failing check is a trust
    /// refusal - Gate found nothing wrong with the work itself, only that
    /// `gate trust` hasn't been run (see `Check::trust_blocked`).
    pub fn only_trust_blocked(&self) -> bool {
        !self.ok
            && self
                .checks
                .iter()
                .filter(|c| !c.ok)
                .all(|c| c.trust_blocked)
    }
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
        trust_blocked: false,
    }
}

pub fn fail(name: impl Into<String>, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        ok: false,
        detail: detail.into(),
        trust_blocked: false,
    }
}

/// Like `fail`, for the specific case of Gate refusing to spawn an
/// untrusted command - see `Check::trust_blocked`.
pub fn fail_untrusted(name: impl Into<String>, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        ok: false,
        detail: detail.into(),
        trust_blocked: true,
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

    #[test]
    fn only_trust_blocked_is_true_when_every_failure_is_a_trust_refusal() {
        let res = result(
            Phase::Implement,
            vec![pass("a", ""), fail_untrusted("b", "not trusted")],
        );
        assert!(res.only_trust_blocked());
    }

    #[test]
    fn only_trust_blocked_is_false_when_a_real_failure_is_also_present() {
        let res = result(
            Phase::Implement,
            vec![
                fail("a", "real problem"),
                fail_untrusted("b", "not trusted"),
            ],
        );
        assert!(!res.only_trust_blocked());
    }

    #[test]
    fn only_trust_blocked_is_false_on_a_passing_result() {
        let res = result(Phase::Implement, vec![pass("a", "")]);
        assert!(!res.only_trust_blocked());
    }
}
