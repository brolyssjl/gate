# Gate

An **agent-agnostic quality harness**. Gate turns the development flow —
`PLAN → DEBUG → IMPLEMENT → TEST → REVIEW → DONE` — into an enforced state machine
with deterministic quality gates. Any AI agent (or human) does the thinking; Gate
holds the state, checks the evidence, and refuses to advance until the evidence
is real.

> Gate is an **umpire, not a driver.** It never invokes agents or LLMs, makes no
> network calls, and has no telemetry. It holds state, verifies evidence, and
> returns exit codes. That is what keeps it agent-agnostic.

Which phases a run walks is chosen by its **profile** (`feature`, `bugfix`,
`refactor`, `docs`); the phase *catalog* and profiles are pure data, so adding a
phase or profile touches no transition logic.

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
gate start "add password reset"    # optional: --profile feature|bugfix|refactor|docs
#   → enters PLAN, scaffolds .gate/runs/<id>/plan.md, prints the plan playbook
# …fill in plan.md: goal, files, acceptance criteria (each with a verify)…
gate approve                  # record sign-off (separate from writing the plan)
gate next                     # PLAN gate: schema valid? ≥1 checkable criterion? approved?
# …write code touching only declared files…
gate next                     # IMPLEMENT gate: non-empty diff, in scope, build+lint green
# …write tests; each `test:`-verified criterion needs a named passing test…
gate next                     # TEST gate: suite exits 0, criteria mapped, no skips, diff coverage
gate review --fresh           # emit a self-contained review packet for a fresh reviewer
# …reviewer records findings in review.md; resolve or waive every blocker/major…
gate next                     # REVIEW gate: packet emitted, no blocking findings, distinct reviewer
#   → DONE
gate report                   # per-run summary: phase durations, gate failures, findings
```

A **bugfix** run routes through DEBUG instead of IMPLEMENT: fill in `debug-log.md`
with the enforced protocol (reproduce → hypothesize → predict → test → conclude)
before the gate will pass.

`gate skip <phase> --reason "…"` records a human-authorized skip of the current
phase; `gate log <file>` registers an artifact against the current phase.

`gate check` runs the current gate without advancing; **its exit code is the
verdict** (0 pass, 1 fail), so it also drops straight into CI.

## The gates

| Phase | Deterministic checks |
|---|---|
| **PLAN** | `plan.md` schema valid; ≥1 acceptance criterion; each criterion declares a verify method; approved via `gate approve` (bound to the plan's content hash) |
| **DEBUG** | `debug-log.md` valid; bug reproduced; ≥1 complete protocol cycle; touched files in scope; triggering test green and whole suite green (no regressions) |
| **IMPLEMENT** | working diff is non-empty; every touched file is declared in `plan.md`; `build` exits 0; `lint` exits 0 |
| **TEST** | `test` command exits 0; every `test:`-verified criterion maps to a named passing test; no skipped tests; diff coverage ≥ threshold |
| **REVIEW** | a fresh packet was emitted (`gate review --fresh`); no blocker/major finding left open (or waived with a rationale); reviewer session id ≠ implementer when both are known |

Plan *quality*, debugging *rigor*, and review *depth* are judgment, not code —
they live in editable Markdown **playbooks** (`.gate/playbooks/*.md`) the CLI
serves to the agent, never in the gates.

## Profiles

`--profile` on `gate start` chooses which phases run:

| Profile | Phases |
|---|---|
| `feature` (default) | PLAN → IMPLEMENT → TEST → REVIEW |
| `bugfix` | PLAN → DEBUG → TEST → REVIEW |
| `refactor` | PLAN → IMPLEMENT → TEST → REVIEW |
| `docs` | PLAN → IMPLEMENT |

## Review & report

`gate review --fresh` writes `review-packet.md` (plan + rubric + full diff) so a
reviewer needs no prior context, and scaffolds `review.md` for their findings. It
records the reviewer's identity (`--by` or `GATE_SESSION_ID`) so the gate can
check it differs from the implementer's.

`gate report [<run-id>]` prints a per-run summary — phase durations, failed gate
attempts, and a findings breakdown — defaulting to the active or most recent run.
`--json` gives the machine-readable form.

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

## Deliberately not yet

RETRO phase, targets for multi-stack repos, `gate prune`, Agnosgram/SDD write
integration, and agent adapters (Claude skill, Cursor rules, `AGENTS.md`). See
`ROADMAP.md` for the full plan.

## Development

```bash
npm test          # vitest: state machine, gates against fixtures, TOON, CLI e2e
npm run build     # tsc → dist/
```
