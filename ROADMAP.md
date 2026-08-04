# Gate roadmap

Gate turns the dev flow into an enforced state machine with deterministic gates.
This file tracks scope and progress. Checked boxes ship in `main`; see releases
for tagged milestones.

Principles that constrain every item below (never traded away): Gate is an
umpire, not a driver; deterministic checks in code, judgment in playbooks; all
state on disk; the agent contract is the CLI; integrations advisory, never
load-bearing; pointers, never copies.

## Milestone 1 - Core MVP ✅ (v0.1.0)

- [x] State machine `PLAN → IMPLEMENT → TEST → DONE` (data-driven phase list)
- [x] Commands: `init`, `start`, `status`, `check`, `next`, `playbook`
- [x] PLAN gate - schema valid, ≥1 criterion, each criterion checkable
- [x] IMPLEMENT gate - non-empty diff, touched ⊆ declared, build, lint
- [x] TEST gate - suite exits 0, criteria→test mapping, no skips, diff coverage
- [x] Pluggable serializer (JSON default, TOON opt-in for uniform arrays)
- [x] Default Markdown playbooks (bundled + user-editable copies)
- [x] Advisory Agnosgram/SDD detection stubs (presence-only)
- [x] Test suite: state-machine table, gates vs fixtures, TOON, coverage, CLI e2e

### Milestone 1 hardening ✅ (v0.1.0)

- [x] `gate trust` - TOFU hashing of the commands block; gates refuse to execute
      untrusted commands
- [x] Real approval handshake - `gate approve`, recorded in run.json and bound to
      the plan's content hash (no self-approval, void-on-edit)
- [x] `gate skip <phase> --reason` - human-authorized skip with audit trail
- [x] `gate log <file>` - register an artifact against the current phase

## Milestone 2 - Full quality flow ✅ (v0.2.0)

- [x] DEBUG phase - enforced protocol (reproduce → hypothesize → predict → test →
      conclude) logged in `debug-log.md`; gate requires ≥1 completed cycle and the
      triggering test green with no regressions
- [x] REVIEW phase - `gate review --fresh` emits a self-contained packet
      (diff + plan + rubric); blocker/major findings resolved or human-waived;
      reviewer session id ≠ implementer when available
- [x] Run profiles - `--profile feature|bugfix|refactor|docs` (which phases run)
- [x] `gate report` - per-run summary (durations, gate failures, findings)
- [x] `--json` everywhere audit + schema snapshots

### Milestone 2 hardening ✅ (v0.2.0)

- [x] Staleness guard - tree fingerprint recorded at every gate pass and on the
      review packet; REVIEW fails on a stale packet and re-runs build/lint/test
      when the code changed since the last gate passed (review fixes cannot ship
      unverified)
- [x] Evidence integrity - test reports come only from the run Gate executes
      (stdout, or a file the command wrote during the run via $GATE_TEST_REPORT);
      pre-staged reports are ignored; Gate persists the normalized report
- [x] Review packet includes untracked files in its diff (commit-less agent
      flows no longer hide new files from the reviewer)
- [x] Reviewer sign-off - review.md must name its reviewer; the untouched
      scaffold cannot pass REVIEW
- [x] TEST gate runs build+lint (bugfix profile skips IMPLEMENT and would
      otherwise never build or lint)
- [x] DEBUG triggering-test check fails closed without a parseable report
- [x] Atomic run.json/trust/current writes (crash-safe state)
- [x] run.json schema 2 with explicit migration from Milestone 1 runs
- [x] Honest docs - threat model section (defends against sloppiness, not
      malice), CI caveat for gitignored run state, report durations fixed

## Milestone 3 - Ecosystem & publish ⏳

- [x] RETRO phase + Agnosgram write integration (retro → journal, `source:` ids)
- [x] SDD detection wired into PLAN (`plan.md` cites the spec path)
- [x] Targets - per-stack blocks for multi-stack repos (match/commands/thresholds/
      playbook overlays); profiles choose which phases, targets choose how
- [x] Agent adapters - Claude Code skill, Cursor/Cline/Windsurf rules, `AGENTS.md`
- [x] `gate prune` - archive finished runs past the retention window
- [ ] Decide the npm name, then publish + distribution (global install,
      `install.sh`, single-file binary for CI/Node-less machines) - mechanical
      prep landed (placeholder name, `install.sh`, `build:binary`, CI/release
      workflows); the owner still decides the final name and publish timing

## Milestone 4 - Concurrency & guardrails ✅ (v0.3.0)

- [x] Multi-run concurrency - one active run per branch, keyed by branch name;
      `gate start` resumes a branch's existing run or starts an independent
      one; detached HEAD falls back to explicit `--run` selection
- [x] `gate guard` pre-commit hook (default-off, opt-in) - cheap deterministic
      checks only (active run, scope, phase); never load-bearing, always
      escapable (`--no-verify`, `GATE_GUARD=0`)
- [x] Broader diff-coverage parsers - coverage.py JSON, Go cover profiles, LCOV
      - alongside jest/vitest and the generic JSON contract
- [x] `gate review --human` - minimal terminal rubric walk for solo devs (no
      TUI framework, no dependencies); satisfies the same REVIEW gate as agent
      review

## Milestone 5 - Soak feedback: a tool, not a blocker ✅ (v0.4.0)

_Scoped from the first real-world soak (2026-07-30): gate v0.3.0 run through
full PLAN..DONE flows on a Nuxt repo and a Go repo, with OpenSpec SDD
integration. Every item reproduced during a real gated task; details in the
soak friction logs. The soak also validated the core machinery: the staleness
guard, evidence-integrity refusal, SDD detection, reviewer/implementer
separation, and RETRO-to-Agnosgram sync all worked first try - and the
independent REVIEW round caught a real regression that self-review and a
green 1435-test suite missed._

- [x] `scope_ignore:` glob list for the scope checks - stored inside the
      trusted commands block (TOFU-hashed, re-trust to change), seeded by
      `gate init` per detected stack (node caches, go build dirs); scope
      report shows "matched scope_ignore" as info, never silently. Motivation:
      untracked environment cruft (a node compile-cache dir rewritten by every
      command, including gate's own runs) produced hundreds of false scope
      violations that hard-blocked a DEBUG gate.
- [x] Approved-plan drift detection + `gate amend` - record the approved
      plan's content hash in run.json; every later gate fails on drift
      (restores void-on-edit for the whole run, today it ends at PLAN: post-
      PLAN edits to plan.md, including scope widening, are accepted with no
      re-approval and no mechanism to record one). `gate amend` shows the
      diff vs the approved plan and `gate approve --amend` records a cheap
      delta re-approval under the same no-self-approval rules.
- [x] Fix the TEST playbook's report section - it still claims a
      hand-registered `test-report.json` (via `gate log`) is valid evidence;
      the CLI correctly refuses exactly that since the M2 hardening. Document
      the real contract (trusted command stdout or `$GATE_TEST_REPORT`) plus
      runner-wiring notes (vitest/jest `--reporter=json`, `go test -json`),
      and make `gate init` actually write the `.gate/runs/` gitignore entries
      the docs already claim ("gitignored by default" is prose today, not
      behavior).

_Deferred from the same soak (logged, not scheduled): separate `typecheck`
command slot; `coverage_format` docs/example mismatch; `plan.spec` doc example
vs change-directory paths; bugfix-vs-feature profile guidance for known-cause
bugs._

## Milestone 6 - Rust port & `1.0.0`

_Owner decision (2026-08-02): `1.0.0` ships as a Rust implementation, ported
against a frozen CLI surface with the existing test suite as a
cross-implementation conformance suite (one crate per tool; shared-code
duplication with agnosgram accepted). Distribution moves to prebuilt static
binaries on GitHub Releases - npm leaves the user-facing install path. All
run-state/schema contracts (run.json migrations, trust hashing, playbook
formats) carry over unchanged._

- [ ] Surface freeze after Milestone 5 lands: CLI commands/flags/outputs +
      `--json` schema snapshots declared the port contract; conformance mode
      (`$GATE_BIN` + `npm run conformance`) green against the TS binary
- [ ] Rust crate in-repo: identical surface, conformance suite green on
      linux-x64 + darwin-arm64; release pipeline builds Rust binaries on tag;
      `1.0.0-rc` tags from here
- [ ] `1.0.0` gate (all required): upgrade story (version-stamped playbook
      copies + drift warning + `--refresh` with diff), real-project soak on
      the Rust binaries through at least one rc cycle, docs site + case study
      (with agnosgram, the 2026-07/08 constructflow soak); Rust binary becomes
      canonical, TS retired or demoted to reference

## Deferred / v2

- [ ] ~~Go port (startup-latency escape hatch)~~ superseded by Milestone 6
      (Rust port)

## Open questions

- Approval identity: how far can a CLI go toward proving a *human* approved?
