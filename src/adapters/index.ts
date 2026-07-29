/**
 * An adapter targets one agent's config file. Every adapter injects the *same*
 * pointer body (kickoff-gate.md's adapter principle: skill/rules wrappers are
 * ergonomics only, the CLI is the real contract - one shared source, no per-agent
 * drift). Shared files (CLAUDE.md, AGENTS.md) get the managed block merged into
 * user content; a dedicated file (Cursor `.mdc`, a Claude Code skill, Cline/
 * Windsurf rules) is fully Gate's, with an optional preamble the tool requires.
 */
export interface Adapter {
  key: string;
  name: string;
  /** Target path relative to the project root. */
  targetPath: string;
  /** True when the whole file belongs to Gate (dedicated rule/skill file). */
  dedicatedFile: boolean;
  /** Content written above the managed block when creating a dedicated file. */
  preamble?: string;
}

export const ADAPTERS: Record<string, Adapter> = {
  claude: {
    key: "claude",
    name: "Claude Code",
    targetPath: "CLAUDE.md",
    dedicatedFile: false,
  },
  "claude-skill": {
    key: "claude-skill",
    name: "Claude Code skill",
    targetPath: ".claude/skills/gate/SKILL.md",
    dedicatedFile: true,
    preamble:
      "---\nname: gate\ndescription: Follow Gate's enforced dev flow (status, playbook, work, next) for any task in this repo.\n---\n\n",
  },
  cursor: {
    key: "cursor",
    name: "Cursor",
    targetPath: ".cursor/rules/gate.mdc",
    dedicatedFile: true,
    preamble: "---\ndescription: Gate quality-flow protocol\nalwaysApply: true\n---\n",
  },
  cline: {
    key: "cline",
    name: "Cline",
    targetPath: ".clinerules/gate.md",
    dedicatedFile: true,
  },
  windsurf: {
    key: "windsurf",
    name: "Windsurf",
    targetPath: ".windsurf/rules/gate.md",
    dedicatedFile: true,
    preamble: "---\ntrigger: always_on\n---\n\n",
  },
  agents: {
    key: "agents",
    name: "AGENTS.md",
    targetPath: "AGENTS.md",
    dedicatedFile: false,
  },
};

export const ADAPTER_KEYS = Object.keys(ADAPTERS);

/**
 * The shared pointer body injected into every adapter target: status ->
 * playbook -> work -> next, the two invariants that keep an agent from faking
 * progress (never hand-edit run.json, a red gate means missing evidence).
 */
export function buildPointerBody(): string {
  return [
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
  ].join("\n");
}
