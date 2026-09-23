//! Port of `src/gates/index.ts` (`hasGate`, `runGate`, dispatching to the
//! per-phase gate for `ctx.run.phase`). Ported in wave 3 (see
//! `docs/rust-port.md`).
//!
//! One module per `src/gates/*.ts` file, same names.

pub mod coverage;
pub mod debug;
pub mod implement;
pub mod plan;
pub mod plan_drift;
pub mod retro;
pub mod review;
pub mod test;
pub mod test_report;
pub mod types;

use crate::core::state_machine::Phase;
use types::{pass, GateContext, GateResult};

pub fn has_gate(phase: Phase) -> bool {
    matches!(
        phase,
        Phase::Plan | Phase::Debug | Phase::Implement | Phase::Test | Phase::Review | Phase::Retro
    )
}

/// Run the gate for the run's current phase. Terminal phases have no gate.
pub fn run_gate(ctx: &GateContext) -> GateResult {
    match ctx.run.phase {
        Phase::Plan => plan::plan_gate(ctx),
        Phase::Debug => debug::debug_gate(ctx),
        Phase::Implement => implement::implement_gate(ctx),
        Phase::Test => test::test_gate(ctx),
        Phase::Review => review::review_gate(ctx),
        Phase::Retro => retro::retro_gate(ctx),
        other => GateResult {
            phase: other,
            ok: true,
            checks: vec![pass("phase", format!("{other} has no gate"))],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::GateConfig;
    use crate::core::run::{new_run, NewRunParams};

    fn run_on(phase: Phase) -> crate::core::run::Run {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.phase = phase;
        run
    }

    #[test]
    fn has_gate_is_true_for_every_gated_phase_and_false_for_done() {
        for phase in [
            Phase::Plan,
            Phase::Debug,
            Phase::Implement,
            Phase::Test,
            Phase::Review,
            Phase::Retro,
        ] {
            assert!(has_gate(phase), "{phase} should have a gate");
        }
        assert!(!has_gate(Phase::Done));
    }

    #[test]
    fn run_gate_on_done_passes_vacuously_with_an_explanatory_note() {
        let ctx = GateContext {
            root: crate::core::testutil::unique_temp_dir("gates-mod-rs-done"),
            run: run_on(Phase::Done),
            config: GateConfig::default(),
        };
        let res = run_gate(&ctx);
        assert!(res.ok);
        assert_eq!(res.phase, Phase::Done);
        assert_eq!(res.checks.len(), 1);
        assert!(res.checks[0].detail.contains("DONE has no gate"));
        std::fs::remove_dir_all(&ctx.root).unwrap();
    }

    #[test]
    fn run_gate_dispatches_plan_on_a_run_in_plan() {
        let root = crate::core::testutil::unique_temp_dir("gates-mod-rs-dispatch");
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Phase::Plan),
            config: GateConfig::default(),
        };
        let res = run_gate(&ctx);
        // No plan.md written - dispatched to the real PLAN gate, which fails
        // schema validation rather than returning the "no gate" placeholder.
        assert!(!res.ok);
        assert_eq!(res.phase, Phase::Plan);
        assert!(res.checks.iter().any(|c| c.name == "plan.schema"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
