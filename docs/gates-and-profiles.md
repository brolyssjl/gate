# Gates and profiles

Which phases a run walks is chosen by its **profile** (`feature`, `bugfix`,
`refactor`, `docs`); the phase *catalog* and profiles are pure data, so adding a
phase or profile touches no transition logic.

## Profiles

`--profile` on `gate start` chooses which phases run:

| Profile | Phases |
|---|---|
| `feature` (default) | PLAN → IMPLEMENT → TEST → REVIEW → RETRO |
| `bugfix` | PLAN → DEBUG → TEST → REVIEW → RETRO |
| `refactor` | PLAN → IMPLEMENT → TEST → REVIEW → RETRO |
| `docs` | PLAN → IMPLEMENT |

A **bugfix** run routes through DEBUG instead of IMPLEMENT: fill in `debug-log.md`
with the enforced protocol (reproduce → hypothesize → predict → test → conclude)
before the gate will pass.

## The gates

| Phase | Deterministic checks |
|---|---|
| **PLAN** | `plan.md` schema valid; ≥1 acceptance criterion; each criterion declares a verify method; if an SDD dir is detected, `spec:` cites a path under it; if an Agnosgram advise report exists, every contradiction is acknowledged; approved via `gate approve` (bound to the plan's content hash) |
| **DEBUG** | plan unchanged since approval (see "Approved-plan drift" in [Review and report](review-and-report.md)); `debug-log.md` valid; reproduction attested (`reproduced: true` - an agent claim; the mechanical proof is the test); ≥1 complete protocol cycle; touched files in scope (declared in `plan.md`, or matching `scope_ignore`); triggering test named in the run's report and whole suite green (no regressions) |
| **IMPLEMENT** | plan unchanged since approval; working diff is non-empty; every touched file is declared in `plan.md` or matches `scope_ignore`; `build` exits 0; `lint` exits 0 |
| **TEST** | plan unchanged since approval; `build` and `lint` exit 0 (load-bearing for `bugfix`, which skips IMPLEMENT); `test` command exits 0; every `test:`-verified criterion maps to a named passing test; no skipped tests; diff coverage ≥ threshold |
| **REVIEW** | plan unchanged since approval; packet emitted **and matches the current code** (tree fingerprint); review.md signed by a reviewer (≠ implementer when both known); no blocker/major finding left open (or waived with a rationale); if the code changed since the last gate passed, `build`/`lint`/`test` are re-run here and must be green |
| **RETRO** | plan unchanged since approval; `retro.md` valid; at least one of `broke`/`avoid`/`conventions` answered; if an `.agnosgram/` store is present, the entry was synced to its journal (`gate retro`) - otherwise this check passes with a note |

Plan *quality*, debugging *rigor*, and review *depth* are judgment, not code -
they live in editable Markdown **playbooks** (`.gate/playbooks/*.md`) the CLI
serves to the agent, never in the gates. A playbook is an instruction to the
agent the same way a `commands:` entry is an instruction to the shell, so an
override or overlay only takes effect once it's covered by `gate trust` - see
[Integrity: command trust (TOFU)](integrity.md#command-trust-tofu).
