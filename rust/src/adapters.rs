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
/// evidence).
pub fn build_pointer_body() -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn build_pointer_body_matches_the_ts_literal_byte_for_byte() {
        let expected = "## Gate quality flow\n\nThis repo uses Gate to enforce PLAN -> IMPLEMENT -> TEST -> REVIEW -> RETRO\nas a state machine with deterministic gates. Before doing any work:\n\n1. Run `gate status`. No active run? Start one: `gate start \"<title>\"`.\n2. Run `gate playbook` for the current phase - that is your instruction set.\n3. Do the work the playbook describes.\n4. Run `gate next`. It checks the gate and advances on pass; on fail it\n   prints exactly what evidence is missing - fix that, don't argue with it.\n5. Loop 2-4 until the run reaches DONE.\n\nNever hand-edit files under `.gate/runs/` (use `gate log` to register\nartifacts). A red gate means missing evidence, not a suggestion to skip it.";
        assert_eq!(build_pointer_body(), expected);
    }
}
