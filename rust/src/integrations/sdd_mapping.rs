//! Per-framework phase-equivalence mapping between Gate's phases and an
//! SDD's own workflow steps (Feature 1 of the self-healing install work).
//! Gate is the agnostic loop enforcer; the SDD supplies the content of
//! whichever phases it has an equivalent step for - PLAN and IMPLEMENT,
//! typically. TEST/REVIEW/RETRO are Gate's own contribution (no SDD framework
//! Gate detects defines an equivalent), and PLAN's approval is always
//! satisfied by `gate approve` itself, not a separate SDD sign-off. A
//! trailing "closing" step - openspec's `archive`, for instance - lives
//! outside Gate's phase sequence entirely: it runs after DONE and is what
//! actually closes the SDD's own record of the change.
//!
//! Built-in mappings are data, not special-cased per framework in the
//! callers that use them (`adapters::build_pointer_body`,
//! `commands::playbook`, `commands::next`) - a config author with a
//! framework Gate doesn't recognize by name can still declare one via
//! `integrations.sdd_mapping:` (`core::config::SddMappingOverride`).

use crate::core::config::SddMappingOverride;
use crate::core::state_machine::Phase;
use crate::integrations::Sdd;

/// One framework's built-in phase mapping. `None` for a phase means Gate
/// contributes it entirely - no equivalent step in that SDD's own workflow.
struct SddPhaseMapping {
    plan: Option<&'static str>,
    debug: Option<&'static str>,
    implement: Option<&'static str>,
    test: Option<&'static str>,
    review: Option<&'static str>,
    retro: Option<&'static str>,
    /// What accomplishes PLAN's approval - almost always Gate's own
    /// hash-bound `gate approve`, named here (rather than assumed by every
    /// caller) so a framework whose PLAN-equivalent step has its own
    /// separate sign-off concept still reads correctly.
    plan_approval: &'static str,
    /// The SDD step, if any, that closes the loop after DONE.
    closing_step: Option<&'static str>,
    /// A concrete command for the closing step, when Gate knows one.
    closing_command: Option<&'static str>,
}

const GATE_APPROVE: &str = "`gate approve`";

fn openspec_mapping() -> SddPhaseMapping {
    // propose -> PLAN, its approval -> `gate approve`, apply -> IMPLEMENT,
    // TEST/REVIEW/RETRO gate-only, archive = the closing step.
    SddPhaseMapping {
        plan: Some("propose"),
        debug: None,
        implement: Some("apply"),
        test: None,
        review: None,
        retro: None,
        plan_approval: GATE_APPROVE,
        closing_step: Some("archive"),
        closing_command: Some("openspec archive <change>"),
    }
}

fn spec_kit_mapping() -> SddPhaseMapping {
    // spec-kit's /specify + /plan produce the artifact PLAN cites; /tasks
    // is what IMPLEMENT then executes. No documented closing/archive step.
    SddPhaseMapping {
        plan: Some("specify/plan"),
        debug: None,
        implement: Some("tasks"),
        test: None,
        review: None,
        retro: None,
        plan_approval: GATE_APPROVE,
        closing_step: None,
        closing_command: None,
    }
}

fn bmad_mapping() -> SddPhaseMapping {
    // BMAD's story drafting is the PLAN-equivalent artifact; dev
    // implementation against that story is IMPLEMENT's equivalent. No
    // single documented closing step across BMAD workflows.
    SddPhaseMapping {
        plan: Some("story drafting"),
        debug: None,
        implement: Some("dev implementation"),
        test: None,
        review: None,
        retro: None,
        plan_approval: GATE_APPROVE,
        closing_step: None,
        closing_command: None,
    }
}

fn built_in_mapping(sdd: Sdd) -> SddPhaseMapping {
    match sdd {
        Sdd::OpenSpec => openspec_mapping(),
        Sdd::SpecKit => spec_kit_mapping(),
        Sdd::Bmad => bmad_mapping(),
    }
}

/// The mapping actually in effect for a detected `sdd`: the built-in table
/// with `override_` layered on top, field by field. Owned (`String`, not
/// `&'static str`) since a config override supplies runtime text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSddMapping {
    pub plan: Option<String>,
    pub debug: Option<String>,
    pub implement: Option<String>,
    pub test: Option<String>,
    pub review: Option<String>,
    pub retro: Option<String>,
    pub plan_approval: String,
    pub closing_step: Option<String>,
    pub closing_command: Option<String>,
}

/// `None` (key absent from config) keeps the built-in value; `Some("")` (an
/// explicit empty override) clears it; any other `Some(s)` replaces it.
fn layer(built_in: Option<&'static str>, override_: &Option<String>) -> Option<String> {
    match override_ {
        Some(s) if s.is_empty() => None,
        Some(s) => Some(s.clone()),
        None => built_in.map(String::from),
    }
}

pub fn resolve(sdd: Sdd, override_: &SddMappingOverride) -> ResolvedSddMapping {
    let built_in = built_in_mapping(sdd);
    ResolvedSddMapping {
        plan: layer(built_in.plan, &override_.plan),
        debug: layer(built_in.debug, &override_.debug),
        implement: layer(built_in.implement, &override_.implement),
        test: layer(built_in.test, &override_.test),
        review: layer(built_in.review, &override_.review),
        retro: layer(built_in.retro, &override_.retro),
        plan_approval: override_
            .plan_approval
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| built_in.plan_approval.to_string()),
        closing_step: layer(built_in.closing_step, &override_.closing_step),
        closing_command: layer(built_in.closing_command, &override_.closing_command),
    }
}

impl ResolvedSddMapping {
    fn fulfillment(&self, phase: Phase) -> Option<&str> {
        match phase {
            Phase::Plan => self.plan.as_deref(),
            Phase::Debug => self.debug.as_deref(),
            Phase::Implement => self.implement.as_deref(),
            Phase::Test => self.test.as_deref(),
            Phase::Review => self.review.as_deref(),
            Phase::Retro => self.retro.as_deref(),
            Phase::Done => None,
        }
    }
}

/// One-line SDD-composition hint for `phase`'s playbook entry. `None` for
/// PLAN (its existing spec-citation hint already carries the composition
/// story - see `integrations::plan_hints`) and for DONE (no playbook to
/// attach a hint to; see `closing_hint` for DONE's own banner instead).
pub fn phase_composition_hint(
    phase: Phase,
    sdd: Sdd,
    mapping: &ResolvedSddMapping,
) -> Option<String> {
    if matches!(phase, Phase::Plan | Phase::Done) {
        return None;
    }
    Some(match mapping.fulfillment(phase) {
        Some(step) => format!(
            "SDD ({}): fulfilled by its `{step}` step - run `gate next` once it's done.",
            sdd.as_str()
        ),
        None => format!(
            "SDD ({}): gate-only phase - {} has no equivalent step here, follow this playbook.",
            sdd.as_str(),
            sdd.as_str()
        ),
    })
}

/// The hint shown once a run reaches DONE: what (if anything) closes the
/// SDD's own record of the change. `None` when the mapping has no closing
/// step - DONE is then simply the end, same as without an SDD detected.
pub fn closing_hint(sdd: Sdd, mapping: &ResolvedSddMapping) -> Option<String> {
    let step = mapping.closing_step.as_deref()?;
    Some(match &mapping.closing_command {
        Some(cmd) => format!(
            "SDD ({}): run `{cmd}` to close the loop ({step}).",
            sdd.as_str()
        ),
        None => format!(
            "SDD ({}): run its `{step}` step to close the loop.",
            sdd.as_str()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_openspec_mapping_matches_the_documented_equivalences() {
        let mapping = resolve(Sdd::OpenSpec, &SddMappingOverride::default());
        assert_eq!(mapping.plan.as_deref(), Some("propose"));
        assert_eq!(mapping.implement.as_deref(), Some("apply"));
        assert_eq!(mapping.test, None);
        assert_eq!(mapping.review, None);
        assert_eq!(mapping.retro, None);
        assert_eq!(mapping.plan_approval, GATE_APPROVE);
        assert_eq!(mapping.closing_step.as_deref(), Some("archive"));
        assert_eq!(
            mapping.closing_command.as_deref(),
            Some("openspec archive <change>")
        );
    }

    #[test]
    fn spec_kit_and_bmad_have_sensible_built_ins_with_no_closing_step() {
        for sdd in [Sdd::SpecKit, Sdd::Bmad] {
            let mapping = resolve(sdd, &SddMappingOverride::default());
            assert!(mapping.plan.is_some());
            assert!(mapping.implement.is_some());
            assert_eq!(mapping.test, None);
            assert_eq!(mapping.closing_step, None);
        }
    }

    #[test]
    fn config_override_replaces_only_the_declared_fields() {
        let override_ = SddMappingOverride {
            plan: Some("draft".to_string()),
            test: Some("".to_string()), // explicit "still gate-only", same as unset
            closing_command: Some("myddl close <change>".to_string()),
            ..Default::default()
        };
        let mapping = resolve(Sdd::OpenSpec, &override_);
        assert_eq!(mapping.plan.as_deref(), Some("draft"));
        assert_eq!(mapping.implement.as_deref(), Some("apply")); // untouched
        assert_eq!(mapping.test, None);
        assert_eq!(mapping.closing_step.as_deref(), Some("archive")); // untouched
        assert_eq!(
            mapping.closing_command.as_deref(),
            Some("myddl close <change>")
        );
    }

    #[test]
    fn an_empty_string_override_clears_a_built_in_closing_step() {
        let override_ = SddMappingOverride {
            closing_step: Some("".to_string()),
            ..Default::default()
        };
        let mapping = resolve(Sdd::OpenSpec, &override_);
        assert_eq!(mapping.closing_step, None);
    }

    #[test]
    fn phase_composition_hint_is_none_for_plan_and_done() {
        let mapping = resolve(Sdd::OpenSpec, &SddMappingOverride::default());
        assert_eq!(
            phase_composition_hint(Phase::Plan, Sdd::OpenSpec, &mapping),
            None
        );
        assert_eq!(
            phase_composition_hint(Phase::Done, Sdd::OpenSpec, &mapping),
            None
        );
    }

    #[test]
    fn phase_composition_hint_names_the_fulfilling_step_when_one_exists() {
        let mapping = resolve(Sdd::OpenSpec, &SddMappingOverride::default());
        let hint = phase_composition_hint(Phase::Implement, Sdd::OpenSpec, &mapping).unwrap();
        assert!(hint.contains("openspec"));
        assert!(hint.contains("apply"));
    }

    #[test]
    fn phase_composition_hint_names_gate_only_when_no_step_fulfills_it() {
        let mapping = resolve(Sdd::OpenSpec, &SddMappingOverride::default());
        let hint = phase_composition_hint(Phase::Test, Sdd::OpenSpec, &mapping).unwrap();
        assert!(hint.contains("gate-only"));
    }

    #[test]
    fn closing_hint_names_the_command_when_known() {
        let mapping = resolve(Sdd::OpenSpec, &SddMappingOverride::default());
        let hint = closing_hint(Sdd::OpenSpec, &mapping).unwrap();
        assert!(hint.contains("openspec archive <change>"));
    }

    #[test]
    fn closing_hint_is_none_when_the_mapping_has_no_closing_step() {
        let mapping = resolve(Sdd::SpecKit, &SddMappingOverride::default());
        assert_eq!(closing_hint(Sdd::SpecKit, &mapping), None);
    }
}
