# Gate

An **agent-agnostic quality harness**. Gate turns the development flow —
`PLAN → IMPLEMENT → TEST → REVIEW → DONE` (a bugfix run swaps IMPLEMENT for
DEBUG) — into an enforced state machine with deterministic quality gates. Any AI
agent (or human) does the thinking; Gate holds the state, checks the evidence,
and refuses to advance until the evidence is real.

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
# …reviewer signs review.md (reviewer:) and records findings; resolve or waive every blocker/major…
# …fixed something during review? re-run `gate review --fresh` so the reviewer sees the final code…
gate next                     # REVIEW gate: packet matches current code, signed review, no blocking
                              # findings; if code changed since TEST, build/lint/test re-verified here
#   → DONE
gate report                   # per-run summary: phase durations, gate failures, findings
```

A **bugfix** run routes through DEBUG instead of IMPLEMENT: fill in `debug-log.md`
with the enforced protocol (reproduce → hypothesize → predict → test → conclude)
before the gate will pass.

`gate skip <phase> --reason "…" [--by …]` records an explicit skip of the
current phase - who and why, surfaced by `gate report` (see the threat model:
Gate audits overrides, it cannot prove a human made them). `gate log <file>`
registers an artifact against the current phase (context for reviewers, never
gate evidence).

`gate check` runs the current gate without advancing; **its exit code is the
verdict** (0 pass, 1 fail), so scripts and agents can branch on it. Note for CI:
`.gate/runs/` and `.gate/current` are gitignored by default, so a CI checkout has
no run to check - commit your run folders (or re-create the run in CI) if you
want `gate check` as a pipeline step. `config.yml` and `trust.json` are tracked,
so CI always inherits the command pin.

## The gates

| Phase | Deterministic checks |
|---|---|
| **PLAN** | `plan.md` schema valid; ≥1 acceptance criterion; each criterion declares a verify method; approved via `gate approve` (bound to the plan's content hash) |
| **DEBUG** | `debug-log.md` valid; reproduction attested (`reproduced: true` - an agent claim; the mechanical proof is the test); ≥1 complete protocol cycle; touched files in scope; triggering test named in the run's report and whole suite green (no regressions) |
| **IMPLEMENT** | working diff is non-empty; every touched file is declared in `plan.md`; `build` exits 0; `lint` exits 0 |
| **TEST** | `build` and `lint` exit 0 (load-bearing for `bugfix`, which skips IMPLEMENT); `test` command exits 0; every `test:`-verified criterion maps to a named passing test; no skipped tests; diff coverage ≥ threshold |
| **REVIEW** | packet emitted **and matches the current code** (tree fingerprint); review.md signed by a reviewer (≠ implementer when both known); no blocker/major finding left open (or waived with a rationale); if the code changed since the last gate passed, `build`/`lint`/`test` are re-run here and must be green |

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

`gate review` writes `review-packet.md` (plan + rubric + the full diff,
**including untracked files** - nothing in the flow requires committing) so a
reviewer needs no prior context, and scaffolds `review.md` for their findings.
The packet records a fingerprint of the tree it was generated from; the REVIEW
gate refuses to pass while the code differs from it, so a reviewer always signs
off on the code that ships. `--fresh` regenerates an existing packet - required
after any review fix; without it an existing packet is never silently
re-baselined.

The reviewer signs off by filling in `reviewer:` in `review.md` - identity is
claimed at sign-off, not at packet time, and the gate rejects an anonymous
review and a reviewer equal to the implementer's session id.

Fixes made during review change the code *after* IMPLEMENT/TEST certified it,
so when the tree no longer matches the fingerprint recorded at the last gate
pass, the REVIEW gate re-runs `build`/`lint`/`test` itself and requires them
green before DONE.

`gate report [<run-id>]` prints a per-run summary — phase durations, failed gate
attempts, findings, and any skips (with who and why) — defaulting to the active
or most recent run. `--json` gives the machine-readable form.

## Evidence integrity

If an agent could hand Gate the evidence, enforcement would be fiction. So
machine evidence is produced by Gate itself:

- Gate runs the configured `build`/`lint`/`test`/`coverage` commands and reads
  their exit codes directly.
- The test report comes from the run Gate just executed: JSON on the command's
  stdout (Gate persists the normalized copy to the run folder), or a file the
  command itself wrote during the run - runners can target
  `$GATE_TEST_REPORT`, e.g. `vitest --reporter=json --outputFile=$GATE_TEST_REPORT`.
  A report staged in advance (by hand or via `gate log`) is ignored.
- `run.json` is written atomically, so a crash never corrupts run state.

## Threat model

Gate defends against **sloppiness, not malice**. An agent (or human) in the
same shell can still `gate skip`, `gate trust`, or claim a false identity -
a CLI cannot prove a human acted. What Gate guarantees is that every override
is an explicit, separate, recorded act: skips carry a reason and an identity
and surface in `gate report`; approval is hash-bound to the plan it approved;
trust is hash-bound to the commands block it reviewed; the review packet is
fingerprint-bound to the code it showed. Quiet drift is the failure mode Gate
eliminates - loud, auditable overrides are the escape hatch it keeps.

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
