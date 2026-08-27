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

## Milestone 3 - Ecosystem & publish ✅

- [x] RETRO phase + Agnosgram write integration (retro → journal, `source:` ids)
- [x] SDD detection wired into PLAN (`plan.md` cites the spec path)
- [x] Targets - per-stack blocks for multi-stack repos (match/commands/thresholds/
      playbook overlays); profiles choose which phases, targets choose how
- [x] Agent adapters - Claude Code skill, Cursor/Cline/Windsurf rules, `AGENTS.md`
- [x] `gate prune` - archive finished runs past the retention window
- [ ] ~~Decide the npm name, then publish + distribution (global install,
      `install.sh`, single-file binary for CI/Node-less machines)~~ superseded
      by Milestone 6 (Rust port + GitHub Releases distribution) - mechanical
      prep landed (placeholder name, `install.sh`, `build:binary`, CI/release
      workflows) but is now moot; npm leaves the user-facing install path

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

- [x] Surface freeze after Milestone 5 lands: CLI commands/flags/outputs +
      `--json` schema snapshots declared the port contract; conformance mode
      (`$GATE_BIN` + `npm run conformance`) green against the TS binary.
      Declared in `docs/decisions/0001-surface-freeze.md`
- [x] Rust crate in-repo: identical surface, conformance suite green on
      linux-x64 + darwin-arm64; release pipeline builds Rust binaries on tag;
      `1.0.0-rc` tags from here
_Owner decision (2026-08-22): `1.0.0` ships directly from the merged port -
no rc cycle, matching agnosgram's precedent. The conformance suite (68/68 on
linux-x64 + darwin-arm64) and the release pipeline's hard conformance gate
stand in for the rc soak; the Rust binary is canonical from `1.0.0`. The
remaining items below stay wanted, as post-`1.0.0` hardening rather than
release gates._

- [x] `1.0.0`: Rust binary is the canonical distribution; TS implementation
      demoted to reference, then fully retired (see below)

_Owner decision (2026-08-22): the TypeScript implementation is fully retired.
Rust is the only implementation - zero npm anywhere (no package.json, no
vitest, no npm in CI or in docs' development/contribution instructions). The
e2e conformance layer was ported to `rust/tests/` (std-only integration
tests, see `rust/tests/CONFORMANCE_MAP.md` for the case-by-case mapping)
before `src/` and `test/` were deleted, so the black-box behavioral contract
carries over unchanged._

- [x] TypeScript retirement: `src/`, `test/`, and the npm toolchain
      (`package.json`, `tsconfig.json`, `vitest.config.ts`) removed;
      conformance suite ported 1:1 to `rust/tests/`; CI and release workflows
      are cargo-only
- [x] Upgrade story: shipped as `gate doctor` (read-only diagnosis) +
      `gate update` (the healer) rather than a `--refresh`-with-diff flag on
      `init` - a repo needs the same repair after any gate upgrade, not just
      right after init, so it earned its own pair of commands. Landed
      alongside the SDD-aware adapter composition work below (real data from
      two monitored agent-session rounds: agnosgram was followed 9/9 because
      its managed block lives in an agent-facing file; gate was followed
      0/9 because `gate init` left no trace anywhere an agent looks, and the
      projects' own SDD workflow competed with gate's generic pointer body
      rather than composing with it) - both problems needed the same fix:
      gate has to install, diagnose, and repair its own project presence.
    - SDD-aware pointer body: `build_pointer_body` composes one workflow
      narrating which SDD step fulfills which gate phase (data-driven per
      framework, `integrations.sdd_mapping:` overrides for the rest) instead
      of emitting the generic body next to a competing SDD flow. Surfaces at
      `gate adapt`/`init`/`update`, one-line hints at IMPLEMENT/TEST/DONE
      (`gate playbook`/`gate next`), same as PLAN's existing spec-citation
      hint always did
    - `gate doctor`: adapter blocks missing/stale, playbook drift matrix
      (current / pristine-outdated / user-edited / no-manifest, via a new
      `.gate/playbooks.lock` provenance manifest), trust, config sanity;
      `--json`, exit 0/1
    - `gate update`: applies what doctor diagnoses; never replaces a
      user-edited playbook without `--force-playbooks`; idempotent
    - `gate init` installs `claude` + `agents` by default now (`--no-adapt`/
      `--adapt <keys>` to change that), writes the playbook manifest for
      what it materializes, and all four entry points
      (`adapt`/`init`/`doctor`/`update`) compute the pointer body identically
- [ ] Real-project soak on the Rust binaries, extended to include one
      gate→agnosgram RETRO-sync exercise run with both Rust binaries
      together - the one integration point neither repo's own conformance
      suite can catch (audit RM-06)
- [ ] Decide going public / recruit at least one outside pilot user - owner
      decision: the docs site below can't be useful while its audience
      can't reach a private repo (audit RM-03)
- [ ] Docs site + case study (with agnosgram, the 2026-07/08 constructflow
      soak)

## Fixes & improvements

_Open items from the 2026-08-22 audit + remediation (PRs gate#11/#12,
agnosgram#14, all merged) that don't fit the milestones above._

- [x] Cut `1.0.1` so checksum-verified installs become real: the `1.0.0`
      release predates `SHA256SUMS`; the first post-#11 tag publishes it.
      Small, do soon. (audit SEC-02 tail)
- [x] Failure-streak cap - owner decision: implement, default limit 3.
      `gate check`/`gate next` track consecutive failed evaluations per
      phase in `run.json` and refuse to evaluate past the limit (distinct
      exit code 3) until `gate skip` or the new `gate streak reset`
      explicitly clears it - makes the "loop enforcement" reading of gate's
      purpose true in code. Configurable via `thresholds.failure_streak_limit`
      (`0` disables it). See README's "Loop enforcement" section. (audit
      PUR-01)
- [x] Pitch wording - owner decision: narrowed the README's pitch to
      "tamper-evident audit trail" phrasing, matching the threat model, and
      added an honest "loop enforcement" claim now that the failure-streak
      cap above backs it with code. (audit PUR-01, PUR-03)
- [x] Pin GitHub Actions to commit SHAs - owner decision: pinned every
      `uses:` in `ci.yml`/`release.yml` to the commit SHA its tag/branch
      currently resolves to (with a `# vX.Y.Z` comment), and added
      `.github/dependabot.yml` (`github-actions` ecosystem, weekly) so pins
      get bumped automatically instead of going stale. (audit SEC-08)

_Open items from the 2026-08-24 soak-friction round (real-project soak of
the Rust binaries, feeding Milestone 6's "real-project soak" item above)._

- [ ] Reviewer identity threading - `review.reviewer`'s independence check
      can only compare against `run.session_id`, which is `None` in every
      CLI-driven flow unless the calling harness explicitly passes `gate
      start --session`/`GATE_SESSION_ID`; today that leaves the check
      advisory-only (a distinct, non-blocking warning marker, not real
      enforcement - see the soak-friction fix below). Needs a stable,
      unspoofable session/agent identity threaded from the calling harness
      into every gate-touching command, not a one-line fix.
- [x] 2026-08-24 soak-friction fixes - owner decision: shipped directly
      (this branch IS the fix, no separate remediation PR). Trust-hash
      mismatches now distinguish a coverage-version expansion from a real
      config edit; `review.reviewer` renders unverifiable independence as
      advisory rather than a plain pass; `gate check`/`gate next` failures
      persist into `run.json` (bounded history + a permanent per-phase
      count `gate report` reads) and `gate streak` now reports a finished
      run's final state; phase-transition text (playbooks and `gate
      approve`'s confirmation) derives from the run's actual profile
      instead of assuming feature; PLAN/TEST playbooks state their real
      failure conditions before an agent hits them; plus `gate <cmd>
      --help`, `gate amend`'s diff headers, the RETRO->journal separator,
      and an `install.sh` legacy-install-dir note. Also fixed a real, unrelated
      test flake found along the way (a racy shared-env-var mutation in
      `integrations::agnosgram_write`'s test suite).

## Deferred / v2

- [ ] ~~Go port (startup-latency escape hatch)~~ superseded by Milestone 6
      (Rust port)

## Open questions

- Approval identity: how far can a CLI go toward proving a *human* approved?
