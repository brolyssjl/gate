# Proposal 2 — Gate: an agent-agnostic quality harness for the software flow

**Status:** Draft proposal · July 2026
**Working name:** Gate (as in quality gate). Alternatives: `cadence`, `flowright`, `railcar`.
**One-liner:** A CLI that turns the development flow — plan → implement → test → debug → review → retro — into an enforced state machine with deterministic quality gates, where *any* AI agent does the thinking and the CLI verifies the results.

---

## 1. Problem

AI agents are good at each individual activity but terrible at **flow discipline**. Left alone they: start coding before the plan is agreed, mark work "done" with failing tests, skip review entirely, declare victory on partial implementations, and never feed lessons back. SDD frameworks (Spec Kit, OpenSpec, BMAD) improve the *planning* half but rely on the agent's goodwill for the *quality* half — a `/plan` command doesn't stop an agent from skipping tests.

The missing piece is not more prompts. It's an **umpire**: something outside the agent that holds the state, checks the evidence, and refuses to advance the phase until the evidence is real.

## 2. Form: skill pack vs CLI — comparison and recommendation

| Criterion | Pure skill/prompt pack | Pure CLI app | **Hybrid: CLI core + skill wrappers** |
|---|---|---|---|
| Agent agnosticism | Medium — needs a port per agent format | High — anything with a shell | **High** — CLI is universal; wrappers add comfort per agent |
| Enforcement | None — agent can ignore any instruction | Strong — exit codes don't negotiate | **Strong** — gates are code, not prose |
| State across sessions/agents | None (or ad-hoc files) | Explicit state file | **Explicit** — resume in a different agent mid-task |
| Judgment tasks (is this plan good?) | Native | Impossible without embedding an LLM | **Delegated** — CLI emits the rubric, agent judges, CLI records |
| Maintenance | Low (markdown) | Medium (code) | Medium |
| Install friction | Copy files | `npx` one-liner | `npx` + optional per-agent adapter |

**Recommendation: hybrid.** Deterministic checks (tests ran, coverage didn't drop, artifacts exist, schema valid) live in the CLI where the agent can't rationalize past them. Judgment-heavy steps (plan quality, review depth) live in playbook prompts the CLI serves to whichever agent is present. Thin skill/command wrappers (`/gate:next` in Claude Code, a `.cursor/rules` block, etc.) exist only for ergonomics — the contract is always the CLI.

This mirrors the Agnosgram trick (Proposal 1): **the CLI never calls an LLM.** No API keys, no model config, no vendor coupling. The agent in the room does the thinking; Gate verifies and gatekeeps.

## 3. The flow model

A unit of work is a **run** (feature, fix, refactor). Each run walks a state machine:

```
PLAN ──► IMPLEMENT ──► TEST ──► REVIEW ──► RETRO ──► DONE
             ▲            │
             └── DEBUG ◄──┘        (test failures loop through DEBUG)
```

Each phase has: a **playbook** (prompt/checklist served to the agent), **required artifacts** (files the phase must produce), and a **gate** (deterministic checks that must pass to advance). Phases can be skipped only by explicit human override (`gate skip test --reason "docs-only change"`), which is recorded.

### Phase contract summary

| Phase | Playbook tells the agent to… | Artifacts | Gate checks (deterministic) |
|---|---|---|---|
| **Plan** | Restate the goal, list affected files, define testable acceptance criteria, identify risks; consult project memory if present | `plan.md` (schema: goal, criteria[], files[], risks[], out-of-scope) | Schema valid; ≥1 acceptance criterion; each criterion is checkable; **human or agent-of-record approval flag set** |
| **Implement** | Work criterion by criterion; small commits; no scope creep beyond `plan.md` files ± declared additions | code + `worklog.md` | Diff is non-empty; touched files ⊆ declared (or additions logged with reason); build passes; lint passes |
| **Test** | Write/extend tests per acceptance criterion; run full suite | test files + `test-report.json` | Test command exits 0; every acceptance criterion maps to ≥1 named test; diff coverage ≥ configured threshold; no skipped tests without reason |
| **Debug** | Reproduce first, hypothesize, instrument, fix root cause (playbook = disciplined debugging protocol, not "try stuff") | `debug-log.md` (hypothesis → evidence → outcome per iteration) | Failing test that triggered entry now passes; no other test regressed; debug-log has ≥1 completed hypothesis cycle |
| **Review** | Fresh-context review against a rubric: correctness, security, simplicity, tests, plan-conformance. Ideally a *different* agent/session than the implementer | `review.md` (findings[] with severity, verdict) | Schema valid; all `blocker`/`major` findings resolved or explicitly waived by human; reviewer identity ≠ implementer identity (session id) when available |
| **Retro** | 3 questions: what broke, what to avoid next time, what convention emerged | `retro.md` | Non-empty; if Agnosgram (Proposal 1) detected → entries appended to its journal, ids recorded |

### Run profiles: the flow adapts to the kind of work

Not every task is a feature. `gate start --profile feature|bugfix|refactor|docs` (default `feature`) selects a profile defined in `config.yml`: which phases run, which playbooks apply, and which thresholds hold. The `bugfix` profile *leads* with debug discipline — PLAN is minimal (reproduction target + fix criteria), then a REPRODUCE-first debug loop (write the failing test before touching code), then fix → TEST → REVIEW. `docs` skips test/coverage gates; `refactor` swaps the test gate's "new criteria need new tests" for "behavior unchanged: full suite green, no coverage drop." Profiles are user-definable — a profile is just a phase list + playbook set + thresholds, so teams can add `spike`, `hotfix`, etc. Skips and overrides remain recorded per run regardless of profile.

### Why DEBUG is a phase, not a vibe

Agents debug by thrashing. The debug playbook enforces the loop *reproduce → hypothesize → predict → test the prediction → conclude*, and the gate requires the log to show it. This alone measurably reduces "changed 9 files, fixed nothing" sessions.

## 4. On-disk layout

```
.gate/
├── config.yml            # commands (build/test/lint/coverage), thresholds, phases on/off
├── playbooks/            # per-phase prompts+rubrics — user-editable, versioned defaults
│   ├── plan.md  implement.md  test.md  debug.md  review.md  retro.md
└── runs/
    └── 2026-07-16-payment-retry/
        ├── run.json       # state machine: phase, history, overrides, session ids
        ├── plan.md  worklog.md  test-report.json
        ├── debug-log.md  review.md  retro.md
```

`config.yml` example (single-stack projects can use the flat `commands:` form; multi-stack repos define **targets**):

```yaml
commands:                 # default target (single-stack shorthand)
  build: "npm run build"
  test:  "npm test -- --reporter json"
  lint:  "npm run lint"
  coverage: "npm run coverage -- --json"
thresholds:
  diff_coverage: 80
targets:                  # optional: per-stack overrides, matched by path
  backend:
    match: ["apps/api/**"]
    commands: { test: "pytest", coverage: "pytest --cov --cov-report=json" }
    thresholds: { diff_coverage: 85 }
    playbooks: { test: "playbooks/test.backend.md" }   # overlay on the base playbook
  frontend:
    match: ["apps/web/**"]
    commands: { test: "vitest run", e2e: "playwright test" }
    thresholds: { diff_coverage: 70 }
    playbooks: { test: "playbooks/test.frontend.md" }
phases:
  review: required        # required | optional | off
  retro:  required
integrations:
  agnosgram: auto         # feed retro into .agnosgram/journal
  sdd: auto               # detect openspec/.specify/_bmad and link plan.md to spec
```

### 4.1 Customization model: three layers per phase

What happens "inside" a phase is customizable at three independent layers, and stack differences map onto them cleanly:

1. **Commands** (`config.yml`) — the deterministic actions: what "run tests" executes. Per target for multi-stack repos.
2. **Playbooks** (`.gate/playbooks/*.md`) — the agent's instructions for the phase. Base playbook + optional per-target overlay: shared discipline (never weaken assertions, criteria→tests mapping) lives in the base; stack-specifics (a11y + visual regression for frontend; testcontainers + contract tests for backend) live in the overlay.
3. **Gate checks + thresholds** — which deterministic verifications run and at what bar, per target and per profile.

A run resolves its targets from the files declared in `plan.md` (or explicitly: `gate start --target backend`); each phase runs the commands, playbook overlays, and gates of **affected targets only** — a change touching both API and web must pass both test suites. Composition rule: **profiles choose which phases run; targets choose how each phase runs.**

## 5. CLI surface

| Command | Purpose |
|---|---|
| `gate init` | Scaffold `.gate/`, detect stack (infer build/test/lint commands), detect SDD + Agnosgram, install adapters on request |
| `gate start "<title>"` | Create a run, enter PLAN, print the plan playbook |
| `gate status [--json]` | Current run, phase, what the gate is waiting for |
| `gate check` | Run the current phase's gate; print pass/fail with reasons; **exit code = verdict** |
| `gate next` | `check` + advance on pass; on fail, print exactly what's missing (agent-readable) |
| `gate playbook [phase]` | Print the active playbook (agents call this at each phase entry) |
| `gate skip <phase> --reason "…"` | Human-authorized skip, recorded in run.json |
| `gate review --fresh` | Emit a self-contained review packet (diff + plan + rubric) for a fresh agent session |
| `gate log <file>` | Register an artifact against the current phase |
| `gate report` | Summarize a finished run (durations per phase, gate failures, review findings) |
| `gate prune` | Archive finished runs past the retention window (summaries kept, evidence dropped) |

**The agent loop is two commands:** `gate playbook` (what should I do?) → work → `gate next` (am I done?). Every agent that can run shell commands can follow it; the wrappers below just automate the calling.

All output has `--json` for programmatic consumption; human-readable is the default.

**Token efficiency:** same layered policy as Agnosgram (Proposal 1 §8). Storage (`run.json`, artifacts) is never compressed. Agent-facing outputs of uniform data — `status`, `report`, findings lists — get an opt-in `--format toon` (~30–40% savings on tabular payloads; JSON stays default for CI). Playbooks remain plain structured Markdown — tight checklists and tables, no filler prose — rather than SudoLang-style compressed languages: agent-agnostic means assuming the least capable reader, and playbooks must stay user-editable. A SudoLang playbook pack is a possible later A/B experiment, not the default.

## 6. Agent integration

Same adapter pattern as Agnosgram — a managed block in each agent's config file:

> When working on a task in this repo, run `npx gate status --json` first. If a run is active, follow `gate playbook` for the current phase and use `gate next` to advance. Never edit files under `.gate/runs/` except through `gate log`. Do not claim completion while `gate status` shows an unfinished run.

Plus optional deep hooks where supported: Claude Code gets a skill (`/gate` commands) and a `Stop` hook that warns if a run is mid-phase; Cursor/Cline/Windsurf get rules files; everything else gets the `AGENTS.md` block.

## 7. Relationship to SDD frameworks and Proposal 1

Gate is **orthogonal to SDD**: Spec Kit/OpenSpec/BMAD produce better *inputs* (specs, plans); Gate enforces *throughput quality* regardless of where the plan came from. With `sdd: auto`, PLAN phase artifacts may simply link to the spec (`plan.md` cites `openspec/changes/...`), and the plan gate accepts the framework's artifact as satisfying the schema.

All integrations (SDD, Agnosgram, graphify-style code graphs) follow the **advisory, never load-bearing** rule (Proposal 1 §6.1): presence only changes hint lines in playbooks/pointer blocks, `gate init --refresh` re-detects and idempotently rewrites them, and no gate check ever depends on an external tool being installed — so adding or removing one mid-project cannot corrupt run state.

### 7.1 Composition scenarios

No tool runs another; the **agent is the only executor**. Authority is split: **SDD owns intent** (what to build), **Agnosgram owns knowledge** (what we've learned), **Gate owns process** (what counts as done). Gate's state machine makes it the *scheduler of interactions* — its phases decide when the others are consulted.

| Composition | How it works | What's missing |
|---|---|---|
| Agnosgram + SDD | SDD keeps its native flow; Agnosgram wraps the edges: lessons read before `/specify`-`/plan`, journal on session end and on spec archive, decisions link to spec paths | No enforcement — discipline is still agent goodwill |
| Gate + SDD | Agent drives SDD skills *inside* Gate's PLAN phase; `sdd: auto` accepts the framework's artifact as plan evidence. Gate referees the full cycle, including everything SDD doesn't cover (test evidence, debug discipline, fresh review) | No learning loop — `retro.md` dies in the run folder |
| Agnosgram + Gate | Gate's PLAN phase acts as lightweight SDD (`plan.md` + criteria); Agnosgram feeds PLAN and receives RETRO. Complete closed loop, least ceremony — best for solo projects | Heavyweight spec artifacts (fine for most work) |
| All three | Full run: read memory → `gate start` → PLAN (read pitfalls, produce spec via SDD, cite it) → gated IMPLEMENT/TEST/DEBUG → fresh REVIEW (rubric checks spec conformance *and* Agnosgram conventions) → RETRO (`agnosgram log` with `source:` run id; SDD archive) → later `distill` + `gate prune`. Data flow is a triangle of pointers: intent SDD→Gate, experience Gate→Agnosgram, knowledge Agnosgram→Gate's next PLAN | — |

Precedence rule for conflicts (full version in Proposal 1 §6): *normative* conflicts (style, principles) — spec wins, Agnosgram records exceptions. *Empirical* conflicts (spec proposes Z, a lesson documents Z failed) — **the contradiction wins**: with `agnosgram: auto`, the PLAN gate requires an `agnosgram advise` report over the spec/plan and explicit human acknowledgment of every flagged contradiction before PLAN passes; each acknowledgment resolves into exactly one store (plan changed, or an override decision superseding the lesson).

**Data ownership & retention (anti-duplication):** every fact has one home — SDD specs own *intent*, `.gate/runs/` owns *per-run evidence*, Agnosgram owns *durable knowledge*; integrations exchange **pointers, never copies** (`plan.md` cites the spec path; retro entries carry a `source:` link to the run). Run folders are ephemeral by design: after DONE, only the retro→journal path carries forward (and Agnosgram's distill *compresses* it); `gate prune` archives runs past a retention window (default: keep last N, summarize the rest via `gate report`). No past run ever enters a future agent's context. This is the context-health guarantee: no divergent copies of the same fact (the main hallucination trigger), and total context per session stays bounded by Agnosgram's budgets + the single active run.

With Agnosgram present, the loop closes: RETRO writes lessons into `.agnosgram/journal/`, and the PLAN playbook instructs the agent to read `lessons/pitfalls.md` before planning. **Gate is how memory gets produced; Agnosgram is where it lives.** The two ship independently but are designed as a pair.

## 8. Prior art check

- [Spec Kit](https://vibecoding.app/blog/spec-kit-review) / [OpenSpec / BMAD comparisons](https://www.nosam.com/spec-driven-development-openspec-vs-spec-kit-vs-bmad-which-ones-actually-worth-your-time/) — all prompt-driven; none has deterministic gates or cross-agent state. BMAD's agent-team roleplay approximates review but can't *enforce* it.
- CI systems (GitHub Actions etc.) — enforce quality *after push*; Gate enforces it *inside the working session*, where the agent can still fix things cheaply. They're complementary; `gate check` logic should be runnable in CI too (`gate check --phase test` as a CI step).
- Git hooks / husky — single-event checks, no phase state, no playbooks.
- Agent-harness lists ([awesome-agent-harness](https://github.com/Picrew/awesome-agent-harness)) show orchestrators that *drive* agents; Gate deliberately doesn't drive anything — it referees. That's what keeps it agnostic.

## 9. Tech-stack agnosticism & security posture

**Stack:** the state machine, playbooks, and artifacts are language-agnostic; all stack-specifics are quarantined in `config.yml` commands (build/test/lint/coverage), which `gate init` infers for common ecosystems (package.json, pyproject, go.mod, Cargo.toml, …) and users hand-edit for anything else. The one stack-sensitive feature is diff-coverage parsing: ship parsers for the major tools (jest/vitest, pytest-cov, go cover) plus a generic JSON contract for the rest.

**Security by architecture:** no LLM calls, no API keys, no telemetry, no network after install; all state is reviewable text/JSON in the repo.

**The real risk — command execution:** Gate runs shell commands from `config.yml`, the same trust class as npm scripts or Makefiles: a malicious repo could plant a hostile command. Mitigation: trust-on-first-use — Gate hashes the `commands` block, requires explicit `gate trust` on first run and whenever the hash changes (direnv-style), and CI environments can pin the approved hash. Secondary: playbooks and artifacts are agent input, so PR review covers them like code; `gate doctor` secret-scans artifacts before they're committed.

## 10. Implementation plan

**Stack:** TypeScript + Node ≥ 20, published as `gate-cli`, single package, no daemon. State = JSON files; gates = pluggable check functions; playbooks = markdown with a tiny template header. ~3–4k LOC for MVP.

**Why TypeScript / distribution:** same reasoning and same three-tier scheme as Agnosgram (see Proposal 1 §4): Node is near-guaranteed on agent machines since most agent CLIs are Node apps; language is treated as replaceable behind the stable CLI + file contract. Distribution: (1) npm global install as primary (avoid `npx` cold-start on hot paths — agents call `gate status`/`gate next` constantly); (2) `install.sh` fallback for Node-less machines that fetches a self-contained single-file executable (`bun build --compile` / Node SEA per platform), or bootstraps Node if preferred; (3) the same static binary serves CI, where `gate check --phase test` runs as a pipeline step. A Go port remains the v2 escape hatch if startup latency proves annoying in practice.

- **Milestone 1 (week 1–2):** state machine, `init/start/status/check/next`, PLAN + IMPLEMENT + TEST gates with configurable commands, default playbooks; output rendering behind a pluggable serializer (JSON + TOON from day one). Dogfood.
- **Milestone 2 (week 3):** DEBUG + REVIEW (including `review --fresh` packet), run profiles (`feature`/`bugfix`/`refactor`/`docs`), `gate trust`, skip/override with audit trail, `--json` everywhere.
- **Milestone 3 (week 4):** RETRO + Agnosgram integration, SDD detection, adapters (Claude skill, Cursor/Cline/Windsurf rules, AGENTS.md), `gate report`, docs, publish.

**Success test:** give an agent a deliberately vague task in a Gate-enabled repo and instruct it only to "follow the repo conventions." If it is forced to produce a plan with acceptance criteria, cannot pass TEST with a red suite, and a fresh-session review catches a planted flaw — Gate earns its name.

## 11. Open questions

1. Diff coverage needs per-language tooling — start with the config-supplied coverage command + a generic JSON parser, or ship built-in support for jest/vitest/pytest first?
2. Multi-run concurrency (several worktrees/branches at once): one active run per branch keyed by branch name?
3. Should REVIEW support a human mode (checklist TUI) for solo devs who want to review the agent themselves? (Probably yes, cheap to add.)
4. How hard should enforcement be? An agent *can* still edit code without Gate. Optional pre-commit hook (`gate guard`) that blocks commits while a run's gate is red — default off, team opt-in.
