# Kickoff prompt — Build Gate (MVP)

> Use: start a fresh session, attach `proposal-2-gate-devflow-quality.md` (or read `claude/proposal-gate-devflow-quality.md` from the Brainstorm project), then paste this prompt. This prompt carries decisions and rationale from the design session that are not fully spelled out in the proposal. Ideally build after (or alongside) Agnosgram — see the sibling kickoff file.

---

Build the MVP of **Gate**: an agent-agnostic quality harness that turns the dev flow (PLAN → IMPLEMENT → TEST → [DEBUG] → REVIEW → RETRO → DONE) into an enforced state machine, per the attached proposal. TypeScript, Node ≥ 20, published as `gate-cli` on npm (check name availability first and grab it — this space fills fast). Follow the proposal's Milestone 1 scope: state machine, `init/start/status/check/next`, PLAN + IMPLEMENT + TEST gates with configurable commands, default playbooks, pluggable JSON/TOON serializer. Do not expand beyond Milestone 1 without asking.

## Non-negotiable principles (violating any of these breaks the product thesis)

1. **Gate is an umpire, not a driver.** It never invokes agents, SDD skills, or LLMs; the agent is the only executor. Gate holds state, checks evidence, refuses to advance. Exit codes don't negotiate. This is what no SDD framework has (they're all prompt-driven, goodwill-based) and what keeps Gate agent-agnostic.
2. **Deterministic checks in code; judgment in playbooks.** Gates verify only what's mechanically checkable (artifacts exist, schemas valid, tests exit 0, diff coverage ≥ threshold, touched files ⊆ declared). Plan quality / review depth live in editable Markdown playbooks the CLI serves to the agent (`gate playbook`). Never blur this line.
3. **All state on disk, none in conversation.** `run.json` + artifacts survive context compaction, crashes, and agent switches mid-task — this is a headline feature, not an accident. An agent re-anchors with two commands: `gate playbook` (what do I do?) → `gate next` (am I done?).
4. **The agent contract is the CLI.** Skill/rules wrappers (Claude Code skill, .cursor/rules, AGENTS.md block) are ergonomics only. Everything must work for any agent that can run shell commands and read `--json` output.
5. **Integrations advisory, never load-bearing** (same rule as Agnosgram): SDD/Agnosgram/graphify presence only changes hint lines and gate configuration; `init --refresh` re-detects idempotently; no gate check depends on an external tool being installed.
6. **Pointers, never copies:** `plan.md` cites the SDD spec path rather than restating it; retro entries carry `source:` run id. Run folders are ephemeral — `gate prune` archives them; no past run ever enters a future session's context.

## Decisions already made (don't relitigate)

- **Hybrid form** (CLI core + thin skill wrappers) won over pure-skill-pack and pure-CLI after explicit comparison: enforcement needs code, judgment needs prompts, ergonomics need wrappers.
- **DEBUG is a first-class phase with an enforced protocol:** reproduce → hypothesize → predict → test prediction → conclude, logged in `debug-log.md`; the gate requires ≥1 completed hypothesis cycle and the triggering test now green with no regressions. Rationale: agents debug by thrashing ("changed 9 files, fixed nothing").
- **REVIEW requires a fresh context:** `gate review --fresh` emits a self-contained packet (diff + plan + rubric); reviewer session id ≠ implementer session id when available. Blocker/major findings must be resolved or human-waived.
- **Run profiles** (`--profile feature|bugfix|refactor|docs`, user-definable): profiles choose *which* phases run. `bugfix` leads with reproduce-first (failing test before any fix); `refactor` swaps criteria-need-tests for behavior-unchanged; `docs` skips test/coverage. Milestone 2.
- **Targets** for multi-stack repos: named per-stack blocks in `config.yml` (match globs, commands, thresholds, playbook overlays); a run resolves targets from `plan.md`'s declared files; each phase runs affected targets only. Composition rule: **profiles choose which phases; targets choose how each phase.** Three customization layers: commands (machine actions) / playbooks (agent instructions, base + target overlay) / gate checks + thresholds.
- **Security:** `gate trust` — TOFU hash of the `commands:` block, confirmation on first run and on change (direnv-style); CI pins the approved hash. Rationale: config-driven command execution is the same trust class as npm scripts. No LLM calls, no network, no telemetry.
- **Precedence with Agnosgram** (empirical vs normative): if a plan/spec proposes something a lesson documents as failed, the PLAN gate (with `agnosgram: auto`) requires an `agnosgram advise` report and explicit acknowledgment of every flagged contradiction; each acknowledgment resolves into exactly one store.
- **TOON serializer from day 1, opt-in, uniform arrays only** (measured: −26% vs compact JSON on findings/lessons lists; *worse* on small status objects — keep JSON default).

## Deliberately deferred (do not build now)

Go port, `gate guard` pre-commit hook (default-off, opt-in later), human-review TUI mode, broad diff-coverage parsers (ship jest/vitest + pytest + generic JSON contract first), Milestone 2–3 features until Milestone 1 is dogfooded.

## Build guidance

- Dogfood: once `start/check/next` work, use Gate to build Gate.
- Tests: state-machine transition table tests (every phase × pass/fail/skip/override); gate checks against fixture repos (one passing, one failing per check); `--json` output schema snapshots; TOFU trust flow.
- If Agnosgram isn't built yet, stub the integration behind a small interface (`integrations/agnosgram.ts`) keyed off `.agnosgram/` presence — advisory rule makes this trivial.
- Success test for the MVP: give an agent a deliberately vague task in a Gate-enabled fixture repo with only "follow the repo conventions" — it must be forced to produce a plan with checkable acceptance criteria and must be unable to pass TEST with a red suite.

## The four compositions (context for playbook wording)

Agnosgram+SDD = memory without enforcement; Gate+SDD = enforcement without learning (retro dies in the run folder); Agnosgram+Gate = complete closed loop, least ceremony (PLAN acts as lightweight SDD); all three = SDD owns intent, Agnosgram owns knowledge, Gate owns process and schedules their interactions. Authority is split; nothing executes anything else.
