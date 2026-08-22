//! Port of `src/core/stateMachine.ts`: the Gate state machine.
//!
//! The phase *catalog* is data-driven and the *sequence* a run walks is
//! chosen by its profile (feature/bugfix/refactor/docs). Profiles decide
//! which phases run; the transition logic itself is profile-agnostic. DONE
//! is always terminal.
//!
//! Gate is an umpire: it never advances a phase on its own. Advancement is
//! the result of a gate passing (`gate next`) or an explicit human skip;
//! nothing here calls an agent or an LLM.

/// Every phase Gate knows about. Not every run walks all of them (see
/// profiles).
pub const PHASES: [Phase; 7] = [
    Phase::Plan,
    Phase::Debug,
    Phase::Implement,
    Phase::Test,
    Phase::Review,
    Phase::Retro,
    Phase::Done,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    Plan,
    Debug,
    Implement,
    Test,
    Review,
    Retro,
    Done,
}

impl Phase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Plan => "PLAN",
            Phase::Debug => "DEBUG",
            Phase::Implement => "IMPLEMENT",
            Phase::Test => "TEST",
            Phase::Review => "REVIEW",
            Phase::Retro => "RETRO",
            Phase::Done => "DONE",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<Phase> {
        match value {
            "PLAN" => Some(Phase::Plan),
            "DEBUG" => Some(Phase::Debug),
            "IMPLEMENT" => Some(Phase::Implement),
            "TEST" => Some(Phase::Test),
            "REVIEW" => Some(Phase::Review),
            "RETRO" => Some(Phase::Retro),
            "DONE" => Some(Phase::Done),
            _ => None,
        }
    }
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Feature,
    Bugfix,
    Refactor,
    Docs,
}

pub const DEFAULT_PROFILE: Profile = Profile::Feature;

impl Profile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Profile::Feature => "feature",
            Profile::Bugfix => "bugfix",
            Profile::Refactor => "refactor",
            Profile::Docs => "docs",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<Profile> {
        match value {
            "feature" => Some(Profile::Feature),
            "bugfix" => Some(Profile::Bugfix),
            "refactor" => Some(Profile::Refactor),
            "docs" => Some(Profile::Docs),
            _ => None,
        }
    }

    /// The profile's phases, in order, *not* including the implicit
    /// trailing `DONE` (see `phase_sequence`).
    fn base_phases(&self) -> &'static [Phase] {
        match self {
            Profile::Feature => &[
                Phase::Plan,
                Phase::Implement,
                Phase::Test,
                Phase::Review,
                Phase::Retro,
            ],
            Profile::Bugfix => &[
                Phase::Plan,
                Phase::Debug,
                Phase::Test,
                Phase::Review,
                Phase::Retro,
            ],
            Profile::Refactor => &[
                Phase::Plan,
                Phase::Implement,
                Phase::Test,
                Phase::Review,
                Phase::Retro,
            ],
            // docs skips REVIEW and RETRO - a docs-only change has no code review or retro.
            Profile::Docs => &[Phase::Plan, Phase::Implement],
        }
    }
}

pub fn is_phase(value: &str) -> bool {
    Phase::from_str_opt(value).is_some()
}

pub fn is_profile(value: &str) -> bool {
    Profile::from_str_opt(value).is_some()
}

pub fn is_terminal(phase: Phase) -> bool {
    phase == Phase::Done
}

/// Phases that have a gate the agent must clear. DONE is terminal, no gate.
pub const GATED_PHASES: [Phase; 6] = [
    Phase::Plan,
    Phase::Debug,
    Phase::Implement,
    Phase::Test,
    Phase::Review,
    Phase::Retro,
];

/// The ordered phases a run of `profile` walks, ending at DONE. Falls back
/// to the default profile for an unknown name, same as the TS
/// implementation.
pub fn phase_sequence(profile: &str) -> Vec<Phase> {
    let base = Profile::from_str_opt(profile)
        .unwrap_or(DEFAULT_PROFILE)
        .base_phases();
    let mut seq: Vec<Phase> = base.to_vec();
    seq.push(Phase::Done);
    seq
}

/// The phase that follows `phase` within `profile`'s sequence, or `None` if
/// `phase` is terminal or not part of this profile. Defaults to the
/// standard profile so callers without a run in hand (tests, tooling) still
/// get a sensible answer.
pub fn next_phase(phase: Phase, profile: &str) -> Option<Phase> {
    let seq = phase_sequence(profile);
    let idx = seq.iter().position(|p| *p == phase)?;
    if idx + 1 >= seq.len() {
        return None;
    }
    Some(seq[idx + 1])
}

/// Whether `phase` may be skipped by a human override. DONE cannot be
/// skipped; every other gated phase can. Skips are always recorded in
/// run.json.
pub fn can_skip(phase: Phase) -> bool {
    GATED_PHASES.contains(&phase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_every_phase_gate_knows_about_done_last() {
        assert_eq!(
            PHASES.map(|p| p.as_str()),
            [
                "PLAN",
                "DEBUG",
                "IMPLEMENT",
                "TEST",
                "REVIEW",
                "RETRO",
                "DONE"
            ]
        );
        assert_eq!(PHASES[PHASES.len() - 1], Phase::Done);
    }

    #[test]
    fn defaults_to_the_feature_profile() {
        assert_eq!(DEFAULT_PROFILE, Profile::Feature);
    }

    #[test]
    fn builds_each_profiles_sequence_always_ending_at_done() {
        assert_eq!(
            phase_sequence("feature"),
            vec![
                Phase::Plan,
                Phase::Implement,
                Phase::Test,
                Phase::Review,
                Phase::Retro,
                Phase::Done
            ]
        );
        assert_eq!(
            phase_sequence("bugfix"),
            vec![
                Phase::Plan,
                Phase::Debug,
                Phase::Test,
                Phase::Review,
                Phase::Retro,
                Phase::Done
            ]
        );
        assert_eq!(
            phase_sequence("refactor"),
            vec![
                Phase::Plan,
                Phase::Implement,
                Phase::Test,
                Phase::Review,
                Phase::Retro,
                Phase::Done
            ]
        );
        // docs skips REVIEW and RETRO - a docs-only change has no code review or retro.
        assert_eq!(
            phase_sequence("docs"),
            vec![Phase::Plan, Phase::Implement, Phase::Done]
        );
    }

    #[test]
    fn falls_back_to_the_default_profile_for_an_unknown_name() {
        assert_eq!(
            phase_sequence("nonsense"),
            phase_sequence(DEFAULT_PROFILE.as_str())
        );
        assert!(is_profile("bugfix"));
        assert!(!is_profile("nonsense"));
    }

    #[test]
    fn advances_within_a_profile_and_terminates_at_done() {
        assert_eq!(next_phase(Phase::Plan, "feature"), Some(Phase::Implement));
        assert_eq!(next_phase(Phase::Test, "feature"), Some(Phase::Review));
        assert_eq!(next_phase(Phase::Review, "feature"), Some(Phase::Retro));
        assert_eq!(next_phase(Phase::Retro, "feature"), Some(Phase::Done));
        assert_eq!(next_phase(Phase::Done, "feature"), None);
        // bugfix routes through DEBUG and skips IMPLEMENT.
        assert_eq!(next_phase(Phase::Plan, "bugfix"), Some(Phase::Debug));
        assert_eq!(next_phase(Phase::Debug, "bugfix"), Some(Phase::Test));
        // docs stops after IMPLEMENT.
        assert_eq!(next_phase(Phase::Implement, "docs"), Some(Phase::Done));
    }

    #[test]
    fn returns_none_for_a_phase_not_in_the_profiles_sequence() {
        assert_eq!(next_phase(Phase::Debug, "feature"), None); // feature has no DEBUG
        assert_eq!(next_phase(Phase::Review, "docs"), None);
    }

    #[test]
    fn marks_only_done_as_terminal() {
        assert!(is_terminal(Phase::Done));
        for p in GATED_PHASES {
            assert!(!is_terminal(p));
        }
    }

    #[test]
    fn allows_skipping_any_gated_phase_but_never_done() {
        for p in PHASES {
            assert_eq!(can_skip(p), p != Phase::Done);
        }
    }

    #[test]
    fn recognizes_valid_phase_strings() {
        assert!(is_phase("PLAN"));
        assert!(is_phase("REVIEW"));
        assert!(is_phase("DEBUG"));
        assert!(!is_phase("nope"));
    }

    #[test]
    fn only_lists_non_terminal_phases_in_profile_sequences() {
        for profile in [
            Profile::Feature,
            Profile::Bugfix,
            Profile::Refactor,
            Profile::Docs,
        ] {
            assert!(!profile.base_phases().contains(&Phase::Done));
        }
    }
}
