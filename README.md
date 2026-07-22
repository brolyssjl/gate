# Gate

An **agent-agnostic quality harness**. Gate turns the development flow —
`PLAN → IMPLEMENT → TEST → DONE` — into an enforced state machine with
deterministic quality gates. Any AI agent (or human) does the thinking; Gate
holds the state, checks the evidence, and refuses to advance until the evidence
is real.

> Gate is an **umpire, not a driver.** It never invokes agents or LLMs, makes no
> network calls, and has no telemetry. It holds state, verifies evidence, and
> returns exit codes. That is what keeps it agent-agnostic.

This is the **Milestone 1** MVP: the state machine, `init / start / status /
check / next / playbook`, the PLAN + IMPLEMENT + TEST gates with configurable
commands, default playbooks, and a pluggable JSON/TOON serializer.

## Install (local dev)

```bash
npm install
npm run build
npm link          # puts `gate` on your PATH
```

The published package name is not decided yet, so the package is `private` for
now. The CLI command is `gate`.

## The agent loop is two commands

```
gate playbook   # what should I do in this phase?
# …do the work…
gate next       # am I done? (runs the gate; advances on pass)
```

Everything an agent needs works over the shell and `--json` output. All state
lives on disk under `.gate/`, so it survives context compaction, crashes, and
switching agents mid-task.

## Walkthrough

```bash
gate init                     # scaffold .gate/, infer build/test/lint commands
gate trust                    # review .gate/config.yml, then approve its commands (TOFU)
gate start "add password reset"
#   → enters PLAN, scaffolds .gate/runs/<id>/plan.md, prints the plan playbook
# …fill in plan.md: goal, files, acceptance criteria (each with a verify)…
gate approve                  # record sign-off (separate from writing the plan)
gate next                     # PLAN gate: schema valid? ≥1 checkable criterion? approved?
# …write code touching only declared files…
gate next                     # IMPLEMENT gate: non-empty diff, in scope, build+lint green
# …write tests; each `test:`-verified criterion needs a named passing test…
gate next                     # TEST gate: suite exits 0, criteria mapped, no skips, diff coverage
#   → DONE
```

`gate skip <phase> --reason "…"` records a human-authorized skip of the current
phase; `gate log <file>` registers an artifact against the current phase.

`gate check` runs the current gate without advancing; **its exit code is the
verdict** (0 pass, 1 fail), so it also drops straight into CI.

## The gates (Milestone 1)

| Phase | Deterministic checks |
|---|---|
| **PLAN** | `plan.md` schema valid; ≥1 acceptance criterion; each criterion declares a verify method; approved via `gate approve` (bound to the plan's content hash) |
| **IMPLEMENT** | working diff is non-empty; every touched file is declared in `plan.md`; `build` exits 0; `lint` exits 0 |
| **TEST** | `test` command exits 0; every `test:`-verified criterion maps to a named passing test; no skipped tests; diff coverage ≥ threshold |

Plan *quality* and review *depth* are judgment, not code — they live in editable
Markdown **playbooks** (`.gate/playbooks/*.md`) the CLI serves to the agent,
never in the gates.

## Configuration

`.gate/config.yml` (inferred by `gate init`, then hand-editable):

```yaml
commands:
  build: "npm run build"
  test:  "npm test"
  lint:  "npm run lint"
  coverage: "npm run coverage"   # optional
thresholds:
  diff_coverage: 80              # only enforced when a coverage command is set
phases:
  plan: required
  implement: required
  test: required
integrations:
  agnosgram: auto                # advisory only — presence changes hint lines
  sdd: auto
```

Diff coverage understands istanbul `coverage/coverage-final.json` (jest, vitest)
and a generic `coverage/gate-coverage.json` contract:
`{ "files": [{ "path", "covered": [], "uncovered": [] }] }`.

## Command trust (TOFU)

Gate executes the commands in `config.yml` — the same trust class as npm
scripts. Before any gate will run them, a human must review the config and run
`gate trust`, which hashes the `commands:` block into `.gate/trust.json`. The
IMPLEMENT and TEST gates **refuse to spawn a process** until the current hash
matches; changing a command invalidates trust until you re-run `gate trust`.
`.gate/trust.json` is committed, so CI inherits the pin and a command change +
re-trust land in the same diff for review. `gate trust --check` reports status
(exit 0/1) without writing.

Approval is deliberately separate from writing the plan: `gate approve` records
who signed off and the plan's content hash in `run.json`, so the author can't
self-approve by flipping a flag, and editing the plan afterward voids it. A CLI
can't prove a *human* ran `approve` — the mechanical guarantee is that approval
is a distinct, hash-bound act.

## Output formats

Human-readable by default. `--json` for CI and agents (stable schema). `--format
toon` opts into [TOON](https://github.com/toon-format) for uniform, tabular
payloads (findings lists) where it saves tokens; JSON stays the default because
TOON is worse on small objects.

## Deliberately not in this milestone

DEBUG / REVIEW / RETRO phases, run profiles (`bugfix`/`refactor`/`docs`),
targets for multi-stack repos, `gate report` / `gate prune`, Agnosgram/SDD write
integration, and agent adapters (Claude skill, Cursor rules, `AGENTS.md`).

## Development

```bash
npm test          # vitest: state machine, gates against fixtures, TOON, CLI e2e
npm run build     # tsc → dist/
```
