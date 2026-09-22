<div align="center">
  <img src="docs/assets/gate-mascot.svg" width="150" alt="Gate's mascot: a traffic-officer owl with aviators and a stop paddle">

# Gate

**An agent-agnostic quality harness - the umpire for any AI agent's dev loop.**

[![Release](https://img.shields.io/github/v/release/brolyssjl/gate)](https://github.com/brolyssjl/gate/releases)
[![CI](https://github.com/brolyssjl/gate/actions/workflows/ci.yml/badge.svg)](https://github.com/brolyssjl/gate/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/github/license/brolyssjl/gate)](LICENSE)

</div>

Gate turns the development flow -
`PLAN → IMPLEMENT → TEST → REVIEW → RETRO → DONE` (a bugfix run swaps
IMPLEMENT for DEBUG) - into an enforced state machine with deterministic
quality gates. Any AI agent (or human) does the thinking; Gate holds the
state, checks the evidence, and refuses to advance until the evidence is real.
Every override - a skip, a re-trust, a streak reset - is a distinct, reasoned,
recorded act, so the run's `run.json` is a tamper-evident audit trail of what
actually happened, not a log an agent could hand-edit into looking clean. And
because Gate is the one thing standing between an agent and "just keep
retrying", it enforces a hard **failure-streak cap** ([see below](#loop-enforcement-failure-streak-cap)):
a phase that fails the same gate three times in a row stops evaluating
until a human explicitly clears it - the mechanical answer to a mindless
retry loop.

> Gate is an **umpire, not a driver.** It never invokes agents or LLMs, makes no
> network calls, and has no telemetry. It holds state, verifies evidence, and
> returns exit codes. That is what keeps it agent-agnostic.

Which phases a run walks is chosen by its **profile** (`feature`, `bugfix`,
`refactor`, `docs`); the phase *catalog* and profiles are pure data, so adding a
phase or profile touches no transition logic.

## Contents

**Using it** - [Install](#install) · [The agent loop](#the-agent-loop-is-two-commands) · [Walkthrough](#walkthrough) · [The gates](#the-gates) · [Profiles](#profiles) · [Concurrency](#concurrency-branch-keyed-runs) · [Review & report](#review--report) · [Retro & the Agnosgram journal](#retro-and-the-agnosgram-journal)

**Fitting your repo** - [SDD composition](#sdd-composition) · [Targets](#targets-multi-stack-repos) · [Scope noise](#scope-noise-scope_ignore) · [Agent adapters](#agent-adapters) · [doctor & update](#gate-doctor-and-gate-update) · [Pruning runs](#pruning-runs) · [gate guard](#gate-guard-opt-in-pre-commit-hook) · [Configuration](#configuration) · [Output formats](#output-formats)

**Guarantees** - [Evidence integrity](#evidence-integrity) · [Loop enforcement](#loop-enforcement-failure-streak-cap) · [Identity fallback](#identity-fallback) · [Threat model](#threat-model) · [Command trust (TOFU)](#command-trust-tofu)

## Install

Prebuilt binaries ship with every release from v0.3.0 onward (`linux-x64`,
`darwin-arm64`). `install.sh` fetches the right one for your platform and
puts it on your PATH - no Node required:

```bash
curl -fsSL https://raw.githubusercontent.com/brolyssjl/gate/main/install.sh | bash
```

If the direct download fails (network hiccup, GitHub API rate limiting on an
unauthenticated request), `install.sh` falls back to `gh release download`
when the `gh` CLI is available - it reuses your existing GitHub auth. Clone
the repo first so the script is on disk to fall back within:

```bash
git clone https://github.com/brolyssjl/gate.git && ./gate/install.sh
```

`install.sh` verifies the downloaded binary against the release's
`SHA256SUMS` before installing it - a checksum mismatch aborts with nothing
installed. It installs the latest release by default; `GATE_VERSION=X.Y.Z
./install.sh` pins an exact one instead. On a platform without a prebuilt
binary it prints build-from-source steps rather than failing silently.
Building from source needs only a stable Rust toolchain - the crate has zero
external dependencies, and the same conformance suite that gates every
release defines its behavior:

```bash
git clone https://github.com/brolyssjl/gate.git
cd gate
cargo build --release --manifest-path rust/Cargo.toml
# -> rust/target/release/gate; put it on your PATH
```

gate is not published to npm, and per the Milestone 6 owner decision
recorded in ROADMAP.md, never will be. Rust is the only implementation;
see CONTRIBUTING.md for the dev loop.

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
gate init                     # scaffold .gate/, infer commands, install CLAUDE.md/AGENTS.md pointers
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
verdict** (0 pass, 1 fail; **3** past the failure-streak cap - see "Loop
enforcement"), so scripts and agents can branch on it. Note for CI:
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
serves to the agent, never in the gates. A playbook is an instruction to the
agent the same way a `commands:` entry is an instruction to the shell, so an
override or overlay only takes effect once it's covered by `gate trust` -
see "Command trust (TOFU)".

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
review and a reviewer equal to the implementer's session id, *when a session
id was recorded for the run*. Most CLI-driven flows never set one (it only
exists if the calling harness passed `gate start --session`/
`GATE_SESSION_ID`), so in practice this check is advisory: `review.reviewer`
still passes, but with a distinct warning marker and an "independence
unverified" detail rather than the plain checkmark a genuinely confirmed
pass gets.

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
agent-recorded one.

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

## SDD composition

Gate is the agnostic loop enforcer; a Spec-Driven-Development framework
(SDD) supplies the *content* of some phases. With an SDD directory detected
(`openspec/`, `.specify/`, `_bmad/`/`.bmad/`) and `integrations.sdd` not
`off`, Gate composes instead of competing: it maps its own phases against
that framework's own steps - which SDD step **fulfills** a gate phase, which
phases are **gate-only** (no SDD equivalent - TEST/REVIEW/RETRO, typically),
and which SDD step **closes the loop** after DONE. For openspec:

| Gate phase | Fulfilled by |
|---|---|
| PLAN | openspec's `propose` step; approval is always `gate approve`, not openspec's own sign-off |
| IMPLEMENT | openspec's `apply` step |
| TEST / REVIEW / RETRO | gate-only - openspec has no equivalent; the playbook says what and how |
| *(after DONE)* | `openspec archive <change>` closes openspec's own record - Gate doesn't run it |

This composed story shows up everywhere an agent looks: the adapter pointer
blocks (`gate adapt`/`init`), `gate playbook`'s per-phase hint line, and a
one-line reminder printed when a run reaches DONE. Sensible built-in
mappings exist for spec-kit and BMAD too. The PLAN gate's own check is the
read side of the same integration: it requires plan.md's `spec:` field to
cite a path that exists under the detected SDD directory instead of
restating the spec. No SDD directory present, or `integrations.sdd: off`:
none of this runs - the generic pointer body and no spec-citation
requirement, same as before this existed.

A framework Gate doesn't recognize by name (or a built-in mapping you want
to correct) can still declare its own equivalences:

```yaml
integrations:
  sdd_mapping:
    plan: draft            # SDD step that fulfills PLAN
    implement: build        # SDD step that fulfills IMPLEMENT
    closing_step: close     # SDD step that closes the loop after DONE
    closing_command: "myddl close <change>"
```

Every key is optional and independently overridable; an empty string
(`test: ""`) explicitly marks a phase gate-only rather than leaving a
built-in mapping's default in place. This is advisory content only (it
never changes what a gate checks) - deliberately outside the trust hash.

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
section per affected target that declares one - once `gate trust` covers it
(see "Command trust (TOFU)"); an untrusted overlay is refused, not appended.
With no `targets:` configured, or none affected by the change, behavior and
check names are byte-identical to a single-stack repo - targets are purely
additive. `gate start --target <a,b>` overrides resolution for the whole
run.

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

**Upgrading from before 0.4.0:** an empty (or absent) `scope_ignore:` hashes
identically to the pre-0.4.0 commands block, so an existing `trust.json`
stays valid after upgrading `gate` - you are not forced into a surprise
re-trust just because a new version understands a key you never set. The
first time you actually populate `scope_ignore:`, it joins the hash and a
normal `gate trust` is required, exactly like changing a command.

## Agent adapters

`gate adapt [adapter...]` writes (or refreshes) a pointer block into each
agent's config file - the same ~12-line loop (status → playbook → work →
next), composed with the detected SDD when one is present (see "SDD
composition" above), managed between `<!-- gate:start -->` /
`<!-- gate:end -->` markers so your own content around it is untouched.
With no arguments it writes every adapter; idempotent - running it again with
nothing changed reports `unchanged` and doesn't touch the file. `gate init`
installs `claude` + `agents` by default (see "Install" above and `gate init
--help`); `gate adapt` is how you add the rest, any time.

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

## `gate doctor` and `gate update`

An installed-but-untended gate leaves no trace an agent will find: adapter
blocks go stale as `gate` itself is upgraded, playbook copies under
`.gate/playbooks/` drift from the bundled defaults, and a repo with an SDD
now present might still be carrying the pre-composition generic pointer
body. `gate doctor` diagnoses all of it, read-only:

```bash
gate doctor          # human-readable findings, [info] vs [action] severity
gate doctor --json    # same findings, structured
```

It checks: which agent files are missing gate's managed block or carry a
stale one; whether each playbook copy under `.gate/playbooks/` is current,
pristine-but-outdated (bundled content moved on, safe to auto-refresh),
user-edited (never touched without asking), or of unknown provenance (no
manifest entry - a pre-manifest project, or a hand-deleted one); trust
status; and whether `.gate/config.yml` exists and parses. Exit code 0 when
nothing actionable was found, 1 otherwise - script against it like `gate
trust --check`.

`gate update` applies what `doctor` diagnosed - the upgrade story:

```bash
gate update                       # refresh what doctor flagged; never touches a user-edited playbook
gate update --adapt cursor,cline  # also install adapter targets not covered by default
gate update --force-playbooks     # replace user-edited/unknown-provenance playbooks too
```

Managed adapter blocks are gate-owned by contract, so they're always safe to
regenerate. A playbook a human edited is never silently replaced -
`--force-playbooks` is the explicit override, and forcing one is reported
alongside a reminder to re-run `gate trust` (playbook overrides ride in the
same trust hash as `commands:` - see "Command trust (TOFU)" below). Every
copy `gate init`/`gate update` materializes gets a version-stamped manifest
entry (`.gate/playbooks.lock`) so a later `doctor`/`update` - on this machine
or a teammate's - can tell "unchanged since materialized" apart from
"hand-edited"; like `config.yml` and `trust.json`, it stays tracked so CI and
every clone see the same picture. Both commands are idempotent: run either
twice right after itself and the second run reports nothing to do.

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

## Loop enforcement (failure-streak cap)

An agent stuck in a loop tends to do one thing: run `gate check`/`gate next`
again and hope. Gate tracks each phase's *consecutive* failed evaluations in
`run.json` and, once a phase hits the limit (default **3**), refuses to
evaluate that phase at all - a distinct, non-zero exit (`3`) and a message
naming the streak, pointing at the run's worklog/debug log, and naming both
ways forward:

```
$ gate next
gate: TEST blocked: 3 consecutive failures (limit 3). See the run's
worklog/debug log for what's failing, then either `gate skip TEST --reason
"<why>"` or `gate streak reset TEST --reason "<why>"` once a human has
reviewed and a retry is warranted.
```

- **What counts as a failure**: `gate check`/`gate next` evaluating the
  phase's gate and it failing. A gate refusing to run an untrusted command
  (`gate trust`) does *not* count - retrying gives the identical result
  until a human runs `gate trust`, so the cap would just add friction to a
  problem it can't fix. Usage errors and a streak refusal itself never
  count either.
- **What resets the streak to 0**: a passing evaluation, advancing to the
  next phase, `gate skip`, and `gate streak reset`.
- **Recovery, once blocked** - both audited in `run.json` and visible in
  `gate report`, the same as any other override:
  - `gate skip <phase> --reason "…"` - the existing human-authorized skip.
  - `gate streak reset [<phase>] --reason "…" [--by …]` - clear the streak
    without skipping the phase, so the very next `gate check`/`gate next`
    evaluates for real. `gate streak` (no subcommand) shows every phase's
    current streak against the configured limit.
- **Configuring the limit** - `thresholds.failure_streak_limit` in
  `config.yml` (default 3 when unset); `0` disables the cap entirely. This
  key is *not* part of the TOFU commands-block trust hash (see "Command
  trust (TOFU)") - it doesn't change what gets executed, only how many
  times Gate will look.
- **Every real failure is persisted, not just scrollback** - `gate check`
  and `gate next` both record a `Failed` history event (which checks
  failed, when) and bump a permanent per-phase failure count in `run.json`
  the moment an evaluation fails, whether or not it also moves the streak
  (a trust-blocked failure doesn't count toward the streak but is still
  recorded, matching every other real failure). `gate report` reads the
  permanent count for its per-phase failure totals - `run.json`'s `history`
  itself only keeps the most recent 20 `Failed` events (oldest pruned first)
  so a long, troubled run's file doesn't grow without bound, but the count
  `gate report` shows stays exact regardless. `gate streak` (no subcommand)
  also works after a run reaches DONE - it reports the finished run's final
  streak state instead of erroring just because the branch's "current run"
  pointer was cleared.

Like every override in Gate, `gate streak reset` is honest about what it
is: a CLI cannot stop an agent in the same shell from running it, the same
way it cannot stop `gate skip` or `gate trust`. What it guarantees is that
doing so is explicit and visible, not automatic - the point isn't to make
looping impossible, it's to make continuing past a real, repeated failure a
deliberate act with a paper trail, not something that happens by default.

## Identity fallback

Every command that records who acted resolves that identity through one
shared fallback chain: the three explicit human checkpoints - `gate
trust`, `gate approve`, `gate streak reset` (`trustedBy`, `approval.by`,
`override.by`) - plus `gate skip` (`override.by`), `gate amend`
(`amendment.by`), `gate review`'s packet request (`review.requestedBy`),
and `gate start` (`startedBy`). Left alone, those fields stayed `null`
unless the caller happened to pass `--by` or set `GATE_SESSION_ID` -
nothing nudged toward either, so the audit trail said *that* a checkpoint
was passed but not *who* passed it. The chain, in order:

1. `--by <name>`
2. `GATE_SESSION_ID` (env)
3. `git config user.name` (trimmed; empty or erroring is treated as absent)
4. `null`, when all three are absent

Weak identity beats none: `git config user.name` is self-reported and just
as spoofable as `--by` itself - see "Threat model" below. It just means the
common case (an agent or human running these commands from inside a normal
git checkout) no longer defaults to `null`.

One deliberate exception: `gate start`'s `sessionId` stays strictly
harness-provided (`--session` or `GATE_SESSION_ID`, no git-name
fallback). The REVIEW gate's reviewer-independence check hard-fails when
the reviewer name equals the recorded `sessionId`, so letting `git config
user.name` leak into it would flip the documented solo review flow from
advisory to blocking in every one-person repo. Who *started* the run is
recorded separately as `startedBy` (the full chain, audit-only, never
compared against anything).

Opt-in hard enforcement: set `identity.require_identity: true` in
`config.yml` and the deliberate sign-offs - `trust`, `approve`, `streak
reset`, and `skip` - fail (exit 1) instead of writing a `null` identity
when none of the three sources resolves anything (`start`, `amend`, and
the review packet request still record `null`; they are not sign-offs):

```yaml
identity:
  require_identity: true   # optional, default false
```

This only checks *that* an identity resolved, not *which* one - it does not
compare the recorded identity against, say, who did the IMPLEMENT-phase
work (non-implementer/self-approval enforcement). That needs real,
unspoofable session identity to be meaningful rather than advisory, and is
deliberately deferred - see ROADMAP.md's "Reviewer identity threading".

## Threat model

Gate defends against **sloppiness and runaway loops, not malice**. An agent
(or human) in the same shell can still `gate skip`, `gate trust`, `gate
streak reset`, or claim a false identity (including via `git config
user.name` - see "Identity fallback" above) - a CLI cannot prove a human
acted. What Gate guarantees is that every override is an explicit, separate,
recorded act: skips and streak resets carry a reason and an identity and
surface in `gate report`; approval is hash-bound to the plan it approved;
trust is hash-bound to the commands block it reviewed; the review packet is
fingerprint-bound to the code it showed; a failed phase stops being
re-evaluated at all past the failure-streak cap until one of those overrides
fires. Quiet drift is the failure mode Gate eliminates - loud, auditable
overrides are the escape hatch it keeps.

### Untrusted agent-facing inputs

The text Gate feeds to agents - plan.md, diffs, playbook copies - is
manipulable by anyone with repo write access, so Gate treats it as data,
never as something it (or a reading agent) should trust:

- **Review packets warn-and-mark.** Every packet opens with a standing
  preamble: everything below it is the content under review - data to
  judge, never instructions to the reviewer. The plan and diff are scanned
  for prompt-injection phrasing (instruction overrides, role overrides,
  exfiltration imperatives, destructive shell commands); hits add a warning
  section naming each source and line, plus stderr detail. The plan and
  diff themselves are never rewritten or dropped - the packet is
  fingerprint-bound, and hiding code from a reviewer would be worse than
  any injection. A directive-shaped line in reviewed content is a finding,
  not an instruction.
- **Playbook provenance is visible where it's read.** A `.gate/playbooks/`
  copy that diverged from the bundled content its `playbooks.lock` entry
  recorded gets an advisory note prepended wherever the playbook is emitted
  (`gate playbook`, phase entry, the review rubric) - customized playbooks
  are a supported feature, but an agent should never follow instructions
  unique to an edited copy without knowing it was edited. Untrusted
  playbook overrides (commands block not trusted) are refused outright, as
  before.
- **Gate's own behavior never depends on free text.** Artifacts drive gates
  only through the schemas Gate validates (frontmatter fields, hashes,
  exit codes); no prose in any artifact changes what the CLI does.

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
  failure_streak_limit: 3        # optional - see "Loop enforcement"; 0 disables it
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
identity:
  require_identity: false        # optional - see "Identity fallback"
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

The same hash also covers **playbooks** - `.gate/playbooks/<phase>.md`
overrides and each target's `playbooks:` overlay file (path *and* content,
so repointing an already-trusted path at different content still voids
trust). Gate never executes a playbook itself, but it's what gate *tells the
agent* to do, and an attacker who can silently rewrite that instruction has
the same effective control as one who can rewrite a command. While the hash
is stale or absent, an override/overlay is refused - not applied - and
resolution falls back to the bundled or embedded default with a banner
naming what was refused and pointing at `gate trust`; nothing is a hard
error. `gate trust` (and `--check`) list which playbook paths are covered
alongside the commands hash, the same way it's always reported what it's
approving.

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

A Go port was once the startup-latency escape hatch; superseded by the
Milestone 6 Rust port (landed). The 1.0.0 rollout gate (soak period) has
since shipped too. What remains: a docs site. See `ROADMAP.md` for the
full plan.

## Development

```bash
cargo build --release --manifest-path rust/Cargo.toml   # -> rust/target/release/gate
cargo test --manifest-path rust/Cargo.toml               # unit tests + conformance suite
cargo clippy --all-targets --manifest-path rust/Cargo.toml -- -D warnings
```

`cargo test` runs both the crate's unit tests and the black-box conformance
suite in `rust/tests/` - integration tests that never import Gate's
internals, only spawn the `gate` binary and assert on
argv-in/exit-code+stdout+`.gate/`-state-out (point them at any binary via
`$GATE_BIN`, e.g. a downloaded release asset). See CONTRIBUTING.md.

### Single-file binary

Release binaries are produced by `cargo build --release` in
`.github/workflows/release.yml`'s build-and-verify job. Playbooks are embedded
in the Rust binary at compile time via `include_str!()`. To build one
locally, use the same cargo command from the repo root:

```bash
cargo build --release --manifest-path rust/Cargo.toml
# -> rust/target/release/gate
```
