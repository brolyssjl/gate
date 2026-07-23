# Gate roadmap

Gate turns the dev flow into an enforced state machine with deterministic gates.
This file tracks scope and progress. Checked boxes ship in `main`; see releases
for tagged milestones.

Principles that constrain every item below (never traded away): Gate is an
umpire, not a driver; deterministic checks in code, judgment in playbooks; all
state on disk; the agent contract is the CLI; integrations advisory, never
load-bearing; pointers, never copies.

## Milestone 1 — Core MVP ✅ (v0.1.0)

- [x] State machine `PLAN → IMPLEMENT → TEST → DONE` (data-driven phase list)
- [x] Commands: `init`, `start`, `status`, `check`, `next`, `playbook`
- [x] PLAN gate — schema valid, ≥1 criterion, each criterion checkable
- [x] IMPLEMENT gate — non-empty diff, touched ⊆ declared, build, lint
- [x] TEST gate — suite exits 0, criteria→test mapping, no skips, diff coverage
- [x] Pluggable serializer (JSON default, TOON opt-in for uniform arrays)
- [x] Default Markdown playbooks (bundled + user-editable copies)
- [x] Advisory Agnosgram/SDD detection stubs (presence-only)
- [x] Test suite: state-machine table, gates vs fixtures, TOON, coverage, CLI e2e

### Milestone 1 hardening ✅ (v0.1.0)

- [x] `gate trust` — TOFU hashing of the commands block; gates refuse to execute
      untrusted commands
- [x] Real approval handshake — `gate approve`, recorded in run.json and bound to
      the plan's content hash (no self-approval, void-on-edit)
- [x] `gate skip <phase> --reason` — human-authorized skip with audit trail
- [x] `gate log <file>` — register an artifact against the current phase

## Milestone 2 — Full quality flow ⏳

- [x] DEBUG phase — enforced protocol (reproduce → hypothesize → predict → test →
      conclude) logged in `debug-log.md`; gate requires ≥1 completed cycle and the
      triggering test green with no regressions
- [x] REVIEW phase — `gate review --fresh` emits a self-contained packet
      (diff + plan + rubric); blocker/major findings resolved or human-waived;
      reviewer session id ≠ implementer when available
- [x] Run profiles — `--profile feature|bugfix|refactor|docs` (which phases run)
- [x] `gate report` — per-run summary (durations, gate failures, findings)
- [x] `--json` everywhere audit + schema snapshots

## Milestone 3 — Ecosystem & publish ⏳

- [ ] RETRO phase + Agnosgram write integration (retro → journal, `source:` ids)
- [ ] SDD detection wired into PLAN (`plan.md` cites the spec path)
- [ ] Targets — per-stack blocks for multi-stack repos (match/commands/thresholds/
      playbook overlays); profiles choose which phases, targets choose how
- [ ] Agent adapters — Claude Code skill, Cursor/Cline/Windsurf rules, `AGENTS.md`
- [ ] `gate prune` — archive finished runs past the retention window
- [ ] Decide the npm name, then publish + distribution (global install,
      `install.sh`, single-file binary for CI/Node-less machines)

## Deferred / v2

- [ ] Go port (startup-latency escape hatch)
- [ ] `gate guard` pre-commit hook (default-off, opt-in)
- [ ] Human-review TUI mode for solo devs
- [ ] Broader diff-coverage parsers beyond jest/vitest + generic JSON

## Open questions

- Multi-run concurrency: one active run per branch, keyed by branch name?
- Diff coverage: how many built-in parsers before falling back to the generic
  JSON contract?
- Approval identity: how far can a CLI go toward proving a *human* approved?
