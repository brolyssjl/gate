# Gate

An **agent-agnostic quality harness**. Gate turns the development flow -
`PLAN → IMPLEMENT → TEST → REVIEW → RETRO → DONE` (a bugfix run swaps
IMPLEMENT for DEBUG) - into an enforced state machine with deterministic
quality gates. Any AI agent (or human) does the thinking; Gate holds the
state, checks the evidence, and refuses to advance until the evidence is real.

> Gate is an **umpire, not a driver.** It never invokes agents or LLMs, makes no
> network calls, and has no telemetry. It holds state, verifies evidence, and
> returns exit codes. That is what keeps it agent-agnostic.

Which phases a run walks is chosen by its **profile** (`feature`, `bugfix`,
`refactor`, `docs`); the phase *catalog* and profiles are pure data, so adding a
phase or profile touches no transition logic.

## Install

The npm name is not decided yet (`gate-cli` is a placeholder), so the package
stays `private` and isn't published. For now:

```bash
git clone https://github.com/brolyssjl/gate.git
cd gate
npm install
npm run build
npm link          # puts `gate` on your PATH
```

Once published, global install will be `npm install -g gate-cli`.
`install.sh` fetches a single-file binary for Node-less machines (falls back
to npm when no matching release asset exists yet - no binaries are published
either; `npm run build:binary` is mechanical prep, see "Single-file binary"
below):

```bash
curl -fsSL https://raw.githubusercontent.com/brolyssjl/gate/main/install.sh | bash
```

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
#   → RETRO: answer broke/avoid/conventions in retro.md (at least one)
gate retro                    # sync retro.md into .agnosgram/journal/ if a store is present (no-op otherwise)
gate next                     #   → DONE
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
`.gate/runs/` and `.gate/current.json` are gitignored by default (`gate init`
writes the entries into `.gitignore`), so a CI checkout has no run to check -
commit your run folders (or re-create the run in CI) if you want `gate check`
as a pipeline step. `config.yml` and `trust.json` are tracked, so CI always
inherits the command pin.

## The gates

| Phase | Deterministic checks |
|---|---|
| **PLAN** | `plan.md` schema valid; ≥1 acceptance criterion; each criterion declares a verify method; if an SDD dir is detected, `spec:` cites a path under it; if an Agnosgram advise report exists, every contradiction is acknowledged; approved via `gate approve` (bound to the plan's content hash) |
| **DEBUG** | plan unchanged since approval (see "Approved-plan drift"); `debug-log.md` valid; reproduction attested (`reproduced: true` - an agent claim; the mechanical proof is the test); ≥1 complete protocol cycle; touched files in scope (declared in `plan.md`, or matching `scope_ignore`); triggering test named in the run's report and whole suite green (no regressions) |
| **IMPLEMENT** | plan unchanged since approval; working diff is non-empty; every touched file is declared in `plan.md` or matches `scope_ignore`; `build` exits 0; `lint` exits 0 |
| **TEST** | plan unchanged since approval; `build` and `lint` exit 0 (load-bearing for `bugfix`, which skips IMPLEMENT); `test` command exits 0; every `test:`-verified criterion maps to a named passing test; no skipped tests; diff coverage ≥ threshold |
| **REVIEW** | plan unchanged since approval; packet emitted **and matches the current code** (tree fingerprint); review.md signed by a reviewer (≠ implementer when both known); no blocker/major finding left open (or waived with a rationale); if the code changed since the last gate passed, `build`/`lint`/`test` are re-run here and must be green |
| **RETRO** | plan unchanged since approval; `retro.md` valid; at least one of `broke`/`avoid`/`conventions` answered; if an `.agnosgram/` store is present, the entry was synced to its journal (`gate retro`) - otherwise this check passes with a note |

Plan *quality*, debugging *rigor*, and review *depth* are judgment, not code -
they live in editable Markdown **playbooks** (`.gate/playbooks/*.md`) the CLI
serves to the agent, never in the gates.

## Profiles

`--profile` on `gate start` chooses which phases run:

| Profile | Phases |
|---|---|
| `feature` (default) | PLAN → IMPLEMENT → TEST → REVIEW → RETRO |
| `bugfix` | PLAN → DEBUG → TEST → REVIEW → RETRO |
| `refactor` | PLAN → IMPLEMENT → TEST → REVIEW → RETRO |
| `docs` | PLAN → IMPLEMENT |

## Concurrency (branch-keyed runs)

One active run per branch, not one globally: `.gate/current.json` maps each
branch to its own active run id, so parallel work-in-progress on separate
branches never collides. `gate start` on a branch that already has an active
run **resumes it** instead of erroring (unless the plan was approved and then
edited since - that refuses, with a pointer to re-approve or resolve the run
first); a new branch gets its own independent run. `gate status` shows the
current branch's run plus a list of any other branches with one in flight.

A detached `HEAD` has no branch to resolve "current" from - that's genuinely
ambiguous, not a missing-run case - so every phase command (`approve`,
`check`, `next`, `review`, `retro`, `skip`, `log`, `playbook`) accepts an
explicit `--run <id>` to select a run without relying on the checked-out
branch.

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

Solo, with no second agent session or human reviewer handy: `gate review
--human` walks the same rubric as terminal prompts instead of handing the
packet off. It emits the packet, prints the rubric, then asks for each
finding (id, severity, note, status, waiver if waived) and a reviewer name,
and writes `review.md` in the exact shape the REVIEW gate parses - no
special-casing, so a human-recorded review satisfies the same gate as an
agent-recorded one. No new dependencies (`node:readline` is a Node builtin).

`gate report [<run-id>]` prints a per-run summary - phase durations, failed gate
attempts, findings, and any skips (with who and why) - defaulting to the active
or most recent run. `--json` gives the machine-readable form. It falls back to
an archived summary (see "Pruning runs") once a run's live folder is gone.

## Retro and the Agnosgram journal

RETRO closes every profile except `docs`: answer at least one of `broke`,
`avoid`, `conventions` in `retro.md`. If this repo has an
[Agnosgram](https://github.com/brolyssjl/agnosgram) `.agnosgram/` store,
`gate retro` syncs the answers into its journal, source-linked to the run:

```
## 2026-07-27 14:05 · gate · milestone-3-ecosystem
- **Did:** Completed gate run "add password reset" (2026-07-27-add-password-reset, profile feature)
- **Learned:** <broke entries, joined "; ">
- **Decided:** <conventions entries>
- **Avoid:** <avoid entries>
- **Source:** .gate/runs/2026-07-27-add-password-reset
```

It spawns `agnosgram log --stdin --agent gate`; if the binary isn't installed
(ENOENT), it falls back to appending the same entry directly to
`.agnosgram/journal/<month>.md`. Re-running is idempotent - it checks the
recorded journal file already contains the run id before syncing again. With
no `.agnosgram/` store, `gate retro` is a no-op and the RETRO gate's journal
check passes with a note; nothing about RETRO depends on Agnosgram being
installed (advisory, never load-bearing).

The PLAN gate is the read side of the same integration: with an `.agnosgram/`
store present, if `agnosgram advise` has written `<plan>.advise.json`
(the pinned `agnosgram_advise` report schema), every flagged contradiction
must be listed under plan.md's `acknowledgments:` before PLAN passes. No
report on disk yet is advisory-only - Gate never runs `agnosgram advise`
itself.

## SDD spec citation

With an SDD directory detected (`openspec/`, `.specify/`, `_bmad/`/`.bmad/`)
and `integrations.sdd` not `off`, the PLAN gate requires plan.md's `spec:`
field to cite a path that exists under that directory, instead of restating
the spec. No SDD directory present: the check doesn't run at all - same
behavior as before this existed.

## Targets (multi-stack repos)

Single-stack projects use the flat `commands:`/`thresholds:` shown above.
Multi-stack repos declare named **targets** in `config.yml`:

```yaml
targets:
  api:
    match: ["apps/api/**"]
    commands: { test: "pytest", coverage: "pytest --cov --cov-report=json:coverage/coverage.json" }
    thresholds: { diff_coverage: 85 }
    coverage_format: coverage-py                          # overrides the top-level default for this target
    playbooks: { test: ".gate/playbooks/test.api.md" }   # overlay on the base playbook
  web:
    match: ["apps/web/**"]
    commands: { test: "vitest run" }
```

Composition rule: **profiles choose which phases run; targets choose how each
phase runs.** IMPLEMENT/TEST/DEBUG resolve affected targets from the real
changed files and run each target's own commands, producing bracketed check
names (`implement.build[api]`, `test.command[web]`) so a change touching both
stacks must pass both. `gate playbook` appends a `## Target overlay: <name>`
section per affected target that declares one. With no `targets:` configured,
or none affected by the change, behavior and check names are byte-identical
to a single-stack repo - targets are purely additive. `gate start --target
<a,b>` overrides resolution for the whole run.

## Scope noise (`scope_ignore`)

The IMPLEMENT/DEBUG scope check normally fails on any touched file not
declared in `plan.md`. Environment cruft - a node compile cache, a go build
dir, coverage output - gets rewritten by the very commands Gate runs, and
would otherwise hard-block the gate with false violations having nothing to
do with the plan. `scope_ignore` in `config.yml` is a glob list of paths the
scope check treats as noise instead:

```yaml
scope_ignore:
  - "node_modules/**"
  - "coverage/**"
```

`gate init` seeds it per detected stack. It's part of the trust hash
(`commandsBlockHashSource`) - the same class as a command - so widening it
needs a `gate trust` re-approval, same as editing `commands:`; an untrusted
edit is never silently applied. A match is never hidden: it surfaces as its
own info-level check line (`implement.scope.ignored` / `debug.scope.ignored`)
naming exactly which files were excluded and why.

## Agent adapters

`gate adapt [adapter...]` writes (or refreshes) a pointer block into each
agent's config file - the same ~12-line loop (status → playbook → work →
next) in every target, managed between `<!-- gate:start -->` /
`<!-- gate:end -->` markers so your own content around it is untouched.
With no arguments it writes every adapter; idempotent - running it again with
nothing changed reports `unchanged` and doesn't touch the file.

| Adapter | Target file |
|---|---|
| `claude` | `CLAUDE.md` |
| `claude-skill` | `.claude/skills/gate/SKILL.md` |
| `cursor` | `.cursor/rules/gate.mdc` |
| `cline` | `.clinerules/gate.md` |
| `windsurf` | `.windsurf/rules/gate.md` |
| `agents` | `AGENTS.md` |

These are ergonomics only - the contract is always the CLI; every adapter
just tells the agent to run it.

## Pruning runs

Run folders are ephemeral by design. `gate prune` archives non-active runs
past the retention window: a summary (the same shape `gate report` prints)
goes to `.gate/archive/<id>.json`, then the run folder is deleted.

```bash
gate prune --dry-run          # see what would be pruned, touch nothing
gate prune                    # keep the newest 10 (default), archive+remove the rest
gate prune --keep 20 --days 30   # also require a candidate to be >30 days old
```

`--keep` (default 10, or config.yml's `retention.keep`) always spares the
newest N by `updatedAt`; `--days` (or `retention.days`) is an *additional* age
requirement, not a replacement for `--keep`. The active run is never a
candidate. `retention:` in config.yml is deliberately outside the trust
hash - it's not a command.

## `gate guard` (opt-in pre-commit hook)

Off by default; only the explicit `gate guard install` writes anything.
It manages `.git/hooks/pre-commit`, backing up and chaining any hook already
there (or refusing, with instructions, if it can't compose safely):

```bash
gate guard install      # writes .git/hooks/pre-commit (backs up + chains an existing one)
gate guard uninstall    # removes it, restoring the backup if there was one
```

The hook runs only cheap, deterministic checks - never a build/test command:
an active run exists for the branch, staged files stay within the plan's
declared scope, and the run isn't still in PLAN (nothing should be committed
before IMPLEMENT starts). It is **advisory, never load-bearing** - the real
enforcement is `gate check`/`gate next`. Escape hatches always work: `git
commit --no-verify` skips it for one commit; `GATE_GUARD=0 git commit`
disables it globally without uninstalling. `init` never touches git hooks.

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
  agnosgram: auto                # advisory only - presence changes hint lines and check behavior
  sdd: auto
targets: {}                      # optional - see "Targets (multi-stack repos)"
retention: {}                    # optional - gate prune defaults, e.g. { keep: 10, days: 30 }
scope_ignore: []                 # optional - see "Scope noise (scope_ignore)"; seeded by gate init
```

Diff coverage understands five report formats, auto-detected under `coverage/`
(point your configured `coverage` command at the matching path), or pinned
explicitly with `coverage_format`:

- `istanbul` - `coverage/coverage-final.json` (jest, vitest --coverage)
- `generic` - `coverage/gate-coverage.json`:
  `{ "files": [{ "path", "covered": [], "uncovered": [] }] }`
- `coverage-py` - `coverage/coverage.json` (`coverage json -o coverage/coverage.json`)
- `go-cover` - `coverage/go-cover.out` (`go test -coverprofile=coverage/go-cover.out`);
  profile paths are de-prefixed against `go.mod`'s `module` line so they line
  up with git's repo-relative paths
- `lcov` - `coverage/lcov.info` (nyc, gcov, and many others' standard output path)

`coverage_format` can also be set per target (see "Targets"), overriding the
top-level default for that stack only - useful in a multi-language repo where
one target's tests emit lcov and another's emit coverage.py.

## Command trust (TOFU)

Gate executes the commands in `config.yml` - the same trust class as npm
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
can't prove a *human* ran `approve` - the mechanical guarantee is that approval
is a distinct, hash-bound act.

### Approved-plan drift + `gate amend`

Void-on-edit holds for the *whole run*, not just PLAN: every later gate
(DEBUG/IMPLEMENT/TEST/REVIEW/RETRO) re-checks plan.md's hash against the one
`gate approve` recorded, and fails closed on a mismatch - including a
post-approval scope widening that used to sail through with no re-approval.

```bash
gate amend             # print the diff vs the approved plan.md, record intent
gate approve --amend   # re-approve the delta (requires `gate amend` first)
```

`gate amend` requires a prior approval and an actual drift to show; it never
re-approves by itself - that split means a delta re-approval always happens
after a human has actually seen the diff `gate amend` printed, the same
discipline the initial `gate approve` already applies. `gate approve --amend`
refuses if the plan changed again after `gate amend` ran (re-run `gate amend`
on the current plan first) and works at any phase, unlike the initial
approval.

## Output formats

Human-readable by default. `--json` for CI and agents (stable schema). `--format
toon` opts into [TOON](https://github.com/toon-format) for uniform, tabular
payloads (findings lists) where it saves tokens; JSON stays the default because
TOON is worse on small objects.

## Deliberately not yet

A Go port (startup-latency escape hatch). See `ROADMAP.md` for the full plan.

## Development

```bash
npm test          # vitest: state machine, gates against fixtures, TOON, CLI e2e
npm run build     # embeds playbooks (scripts/embedPlaybooks.mjs), then tsc → dist/
npm run typecheck # tsc --noEmit - the project's lint
npm run conformance   # the CLI-only black-box subset, runnable against any $GATE_BIN
```

`npm run conformance` never imports Gate's internals - it only spawns the
`gate` binary and asserts on argv-in/exit-code+stdout+`.gate/`-state-out, so
the same suite can validate a future non-TS port (see CONTRIBUTING.md).

### Single-file binary

`npm run build:binary` (after `npm run build`) produces `dist-bin/gate`: a
single executable with the CLI, its one runtime dependency, and the
playbooks all compiled in - no `node_modules` or `playbooks/` directory
needed alongside it. Uses `bun build --compile` when `bun` is on PATH
(zero extra dependencies), falling back to Node's Single Executable
Applications support (`--experimental-sea-config` + `postject`) otherwise.
Mechanical prep for CI/Node-less distribution - not part of `npm run build`
or wired into a release yet.
