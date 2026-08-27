//! Port of `src/adapters/index.ts`. An adapter targets one agent's config
//! file. Every adapter injects the *same* pointer body (kickoff-gate.md's
//! adapter principle: skill/rules wrappers are ergonomics only, the CLI is
//! the real contract - one shared source, no per-agent drift). Shared files
//! (CLAUDE.md, AGENTS.md) get the managed block merged into user content; a
//! dedicated file (Cursor `.mdc`, a Claude Code skill, Cline/Windsurf rules)
//! is fully Gate's, with an optional preamble the tool requires.
//!
//! Ported fully in wave 1 (rather than left a bare skeleton like the other
//! wave-2+ modules) because `main.rs`'s `HELP` text embeds
//! `ADAPTER_KEYS.join(", ")` - computing it here instead of hardcoding the
//! resolved string a second time keeps the two in lockstep by construction.
//! `apply_adapter`/`cmdAdapt`-equivalent behavior (actually writing files)
//! is still wave 4's `commands/adapt.rs`.

use std::path::Path;

use crate::core::config::load_config;
use crate::integrations::sdd_mapping::{resolve as resolve_sdd_mapping, ResolvedSddMapping};
use crate::integrations::{detect, sdd_integration_enabled, Sdd};

pub struct Adapter {
    pub key: &'static str,
    pub name: &'static str,
    /// Target path relative to the project root.
    pub target_path: &'static str,
    /// True when the whole file belongs to Gate (dedicated rule/skill file).
    pub dedicated_file: bool,
    /// Content written above the managed block when creating a dedicated
    /// file.
    pub preamble: Option<&'static str>,
}

/// In TS declaration/iteration order (`Object.keys` on an object literal
/// preserves insertion order for string keys) - `ADAPTER_KEYS` below is
/// derived from this, so the two can never drift apart.
pub const ADAPTERS: &[Adapter] = &[
    Adapter { key: "claude", name: "Claude Code", target_path: "CLAUDE.md", dedicated_file: false, preamble: None },
    Adapter {
        key: "claude-skill",
        name: "Claude Code skill",
        target_path: ".claude/skills/gate/SKILL.md",
        dedicated_file: true,
        preamble: Some(
            "---\nname: gate\ndescription: Follow Gate's enforced dev flow (status, playbook, work, next) for any task in this repo.\n---\n\n",
        ),
    },
    Adapter {
        key: "cursor",
        name: "Cursor",
        target_path: ".cursor/rules/gate.mdc",
        dedicated_file: true,
        preamble: Some("---\ndescription: Gate quality-flow protocol\nalwaysApply: true\n---\n"),
    },
    Adapter { key: "cline", name: "Cline", target_path: ".clinerules/gate.md", dedicated_file: true, preamble: None },
    Adapter {
        key: "windsurf",
        name: "Windsurf",
        target_path: ".windsurf/rules/gate.md",
        dedicated_file: true,
        preamble: Some("---\ntrigger: always_on\n---\n\n"),
    },
    Adapter { key: "agents", name: "AGENTS.md", target_path: "AGENTS.md", dedicated_file: false, preamble: None },
];

/// `ADAPTER_KEYS = Object.keys(ADAPTERS)`.
pub fn adapter_keys() -> Vec<&'static str> {
    ADAPTERS.iter().map(|a| a.key).collect()
}

pub fn get_adapter(key: &str) -> Option<&'static Adapter> {
    ADAPTERS.iter().find(|a| a.key == key)
}

/// The shared pointer body injected into every adapter target: status ->
/// playbook -> work -> next, the two invariants that keep an agent from
/// faking progress (never hand-edit run.json, a red gate means missing
/// evidence). Context-aware: with an SDD detected (and not disabled) it
/// composes that SDD's own phase-equivalences into the loop instead of the
/// generic body - see `integrations::sdd_mapping`. `resolve_pointer_body`
/// below is what every entry point (`adapt`, `init`, `doctor`, `update`)
/// actually calls, so all four resolve the SDD context identically.
pub fn build_pointer_body(sdd: Option<(Sdd, &ResolvedSddMapping)>) -> String {
    match sdd {
        None => generic_pointer_body(),
        Some((sdd, mapping)) => composed_pointer_body(sdd, mapping),
    }
}

fn generic_pointer_body() -> String {
    [
        "## Gate quality flow",
        "",
        "This repo uses Gate to enforce PLAN -> IMPLEMENT -> TEST -> REVIEW -> RETRO",
        "as a state machine with deterministic gates. Before doing any work:",
        "",
        "1. Run `gate status`. No active run? Start one: `gate start \"<title>\"`.",
        "2. Run `gate playbook` for the current phase - that is your instruction set.",
        "3. Do the work the playbook describes.",
        "4. Run `gate next`. It checks the gate and advances on pass; on fail it",
        "   prints exactly what evidence is missing - fix that, don't argue with it.",
        "5. Loop 2-4 until the run reaches DONE.",
        "",
        "Never hand-edit files under `.gate/runs/` (use `gate log` to register",
        "artifacts). A red gate means missing evidence, not a suggestion to skip it.",
    ]
    .join("\n")
}

/// A single workflow narrating the SDD's phase-equivalences instead of
/// competing with them: which gate phases an SDD step fulfills, which are
/// gate-only, and what closes the loop after DONE.
fn composed_pointer_body(sdd: Sdd, mapping: &ResolvedSddMapping) -> String {
    let name = sdd.as_str();
    let mut lines: Vec<String> = vec![
        format!("## Gate quality flow (composed with {name})"),
        String::new(),
        format!("This repo pairs Gate's enforced state machine with {name} for planning:"),
        "Gate runs PLAN -> IMPLEMENT -> TEST -> REVIEW -> RETRO as gates; each phase".to_string(),
        "below names whether it is fulfilled by one of that SDD's own steps or is".to_string(),
        "gate-only. Before doing any work:".to_string(),
        String::new(),
        "1. Run `gate status`. No active run? Start one: `gate start \"<title>\"`.".to_string(),
        "2. Run `gate playbook` for the current phase - it repeats the matching line".to_string(),
        "   below:".to_string(),
    ];
    for (label, fulfilled) in [
        ("PLAN", mapping.plan.as_deref()),
        ("IMPLEMENT", mapping.implement.as_deref()),
        ("TEST", mapping.test.as_deref()),
        ("REVIEW", mapping.review.as_deref()),
        ("RETRO", mapping.retro.as_deref()),
    ] {
        lines.push(match fulfilled {
            Some(step) => format!("   - {label}: fulfilled by {name}'s `{step}` step."),
            None => format!(
                "   - {label}: gate-only - {name} has no equivalent step; the playbook says what and how."
            ),
        });
    }
    lines.push(format!(
        "   PLAN's approval is {} - not {name}'s own sign-off, if it has one - that",
        mapping.plan_approval
    ));
    lines.push("   is the hash-bound act Gate checks.".to_string());
    lines.push("3. Do the work the current phase's step describes.".to_string());
    lines.push(
        "4. Run `gate next`. It checks the gate and advances on pass; on fail it".to_string(),
    );
    lines.push(
        "   prints exactly what evidence is missing - fix that, don't argue with it.".to_string(),
    );
    match (&mapping.closing_step, &mapping.closing_command) {
        (Some(step), Some(cmd)) => {
            lines.push(format!(
                "5. Loop 2-4 until the run reaches DONE, then run `{cmd}` ({step}) to close"
            ));
            lines.push(format!(
                "   the loop - only {name} closes it, Gate doesn't."
            ));
        }
        (Some(step), None) => {
            lines.push(format!(
                "5. Loop 2-4 until the run reaches DONE, then run {name}'s `{step}` step to"
            ));
            lines.push("   close the loop - only it closes it, Gate doesn't.".to_string());
        }
        (None, _) => lines.push("5. Loop 2-4 until the run reaches DONE.".to_string()),
    }
    lines.push(String::new());
    lines.push("Never hand-edit files under `.gate/runs/` (use `gate log` to register".to_string());
    lines.push(
        "artifacts). A red gate means missing evidence, not a suggestion to skip it.".to_string(),
    );
    lines.join("\n")
}

/// The pointer body every adapter target should currently carry: composed
/// with the detected SDD when one is present and not disabled, generic
/// otherwise. `adapt`/`init`/`doctor`/`update` all call this - one source of
/// truth so the four entry points produce byte-identical bodies (falls back
/// to the generic body on an unreadable/invalid `config.yml` rather than
/// failing the whole command over advisory content).
pub fn resolve_pointer_body(root: &Path) -> String {
    let config = load_config(root).unwrap_or_default();
    let det = detect(root);
    match det.sdd {
        Some(sdd) if sdd_integration_enabled(&config) => {
            let mapping = resolve_sdd_mapping(sdd, &config.sdd_mapping_override);
            build_pointer_body(Some((sdd, &mapping)))
        }
        _ => build_pointer_body(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-adapters-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn adapter_keys_are_in_declaration_order() {
        assert_eq!(
            adapter_keys(),
            vec![
                "claude",
                "claude-skill",
                "cursor",
                "cline",
                "windsurf",
                "agents"
            ]
        );
    }

    #[test]
    fn get_adapter_finds_known_keys_and_rejects_unknown_ones() {
        assert_eq!(get_adapter("cursor").unwrap().name, "Cursor");
        assert!(get_adapter("nope").is_none());
    }

    #[test]
    fn shared_and_dedicated_files_are_classified_correctly() {
        assert!(!get_adapter("claude").unwrap().dedicated_file);
        assert!(!get_adapter("agents").unwrap().dedicated_file);
        assert!(get_adapter("claude-skill").unwrap().dedicated_file);
        assert!(get_adapter("cursor").unwrap().dedicated_file);
        assert!(get_adapter("cline").unwrap().dedicated_file);
        assert!(get_adapter("windsurf").unwrap().dedicated_file);
    }

    #[test]
    fn build_pointer_body_matches_the_ts_literal_byte_for_byte_with_no_sdd_detected() {
        let expected = "## Gate quality flow\n\nThis repo uses Gate to enforce PLAN -> IMPLEMENT -> TEST -> REVIEW -> RETRO\nas a state machine with deterministic gates. Before doing any work:\n\n1. Run `gate status`. No active run? Start one: `gate start \"<title>\"`.\n2. Run `gate playbook` for the current phase - that is your instruction set.\n3. Do the work the playbook describes.\n4. Run `gate next`. It checks the gate and advances on pass; on fail it\n   prints exactly what evidence is missing - fix that, don't argue with it.\n5. Loop 2-4 until the run reaches DONE.\n\nNever hand-edit files under `.gate/runs/` (use `gate log` to register\nartifacts). A red gate means missing evidence, not a suggestion to skip it.";
        assert_eq!(build_pointer_body(None), expected);
    }

    #[test]
    fn composed_pointer_body_names_fulfilling_steps_gate_only_phases_and_the_closing_step() {
        let mapping = crate::integrations::sdd_mapping::resolve(
            crate::integrations::Sdd::OpenSpec,
            &crate::core::config::SddMappingOverride::default(),
        );
        let body = build_pointer_body(Some((crate::integrations::Sdd::OpenSpec, &mapping)));
        assert!(body.starts_with("## Gate quality flow (composed with openspec)"));
        assert!(body.contains("PLAN: fulfilled by openspec's `propose` step."));
        assert!(body.contains("IMPLEMENT: fulfilled by openspec's `apply` step."));
        assert!(body.contains("TEST: gate-only"));
        assert!(body.contains("REVIEW: gate-only"));
        assert!(body.contains("RETRO: gate-only"));
        assert!(body.contains("PLAN's approval is `gate approve`"));
        assert!(body.contains("openspec archive <change>"));
        assert!(body.contains("Never hand-edit files under `.gate/runs/`"));
    }

    #[test]
    fn composed_pointer_body_omits_the_closing_line_when_the_mapping_has_none() {
        let mapping = crate::integrations::sdd_mapping::resolve(
            crate::integrations::Sdd::SpecKit,
            &crate::core::config::SddMappingOverride::default(),
        );
        let body = build_pointer_body(Some((crate::integrations::Sdd::SpecKit, &mapping)));
        assert!(body.contains("5. Loop 2-4 until the run reaches DONE."));
        assert!(!body.contains("close the loop"));
    }

    #[test]
    fn resolve_pointer_body_is_generic_with_no_sdd_and_composed_once_one_is_detected() {
        let root = tmp_dir("resolve-pointer-body-toggle");
        fs::create_dir_all(root.join(".gate")).unwrap();
        assert_eq!(resolve_pointer_body(&root), build_pointer_body(None));

        fs::create_dir_all(root.join("openspec")).unwrap();
        let body = resolve_pointer_body(&root);
        assert!(body.contains("composed with openspec"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_pointer_body_falls_back_to_generic_when_integrations_sdd_is_off() {
        let root = tmp_dir("resolve-pointer-body-disabled");
        fs::create_dir_all(root.join(".gate")).unwrap();
        fs::create_dir_all(root.join("openspec")).unwrap();
        fs::write(root.join(".gate/config.yml"), "integrations:\n  sdd: off\n").unwrap();
        assert_eq!(resolve_pointer_body(&root), build_pointer_body(None));
        fs::remove_dir_all(&root).unwrap();
    }
}
