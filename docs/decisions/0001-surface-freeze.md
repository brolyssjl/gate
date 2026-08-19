# 0001. Surface freeze for the Milestone 6 Rust port

- **Status:** accepted
- **Date:** 2026-08-19

## Context

`ROADMAP.md`'s Milestone 6 records the owner decision of 2026-08-02: `1.0.0`
ships as a Rust implementation, ported against a frozen CLI surface, with the
existing test suite doing double duty as a cross-implementation conformance
suite. Milestone 5 landed the conformance mechanics this depends on: the
`npm run conformance` script, `$GATE_BIN` resolution in `test/helpers.ts`, and
the conformance contract documented in `CONTRIBUTING.md`. What Milestone 6's
first checkbox still needs is the freeze itself - a dated statement of what
"the surface" is, so the Rust port has a fixed target rather than a moving one
tracked informally through PR review.

Before writing this record, `npm run build` followed by
`GATE_BIN=<repo>/dist/cli.js npm run conformance` was run against the built TS
binary: 6 test files, 68 tests, all passing. The full suite
(`npm test`) is also green: 26 test files, 288 tests. The freeze below is
declared against that green baseline.

## Decision

As of this date, the following are frozen as the port contract the Rust
implementation must match:

- **Commands** (`src/cli.ts`): `init`, `adapt`, `trust`, `start`, `approve`,
  `amend`, `status`, `check`, `next`, `review`, `retro`, `report`, `prune`,
  `skip`, `log`, `playbook`, `guard`, plus `help`/`-h`/`--help` and
  `version`/`-v`/`--version`.
- **Flags**, per command, exactly as listed in `gate --help` and README.md's
  command reference - including the global `--json`, `--format json|toon`,
  and `--run <id>`.
- **Outputs and exit codes**: human-readable stdout/stderr text, `--json`
  output shapes, and exit codes (0 pass / 1 fail / 2 usage, per
  `src/cli.ts`'s `reportError`) for every command.
- **The `--json` schema for each command's output.** There is no separate
  snapshot file to freeze against - the conformance suite's assertions on
  parsed JSON output *are* the schema, which is the point of the next
  paragraph.

The conformance suite is the normative, executable definition of this
surface: `npm run conformance` (`test/cli.e2e.test.ts`,
`test/concurrency.test.ts`, `test/humanReview.test.ts`, `test/prune.test.ts`,
`test/guard.test.ts`, `test/amend.test.ts`) run with `$GATE_BIN` pointed at a
candidate binary. Where this record and the suite ever disagree, the suite
wins - it is checkable, this document is prose. Extending the frozen surface
means extending the conformance suite first; the Rust port is done for a
command when conformance passes against it unchanged.

All run-state and schema contracts outside the CLI surface itself - `run.json`
migrations, trust hashing (`.gate/trust.json`), and playbook formats - carry
over unchanged, per the Milestone 6 owner decision recorded in `ROADMAP.md`.
They are not renegotiated by this freeze; they were never in question.

## Consequences

- Any change to a command's flags, output shape, or exit code from this point
  is a surface change, not routine maintenance - it needs a conformance test
  update in the same change, and should be weighed against the cost of
  invalidating Rust port work already underway.
- The Rust crate (Milestone 6's second checkbox) targets exactly this
  surface; "done" is defined as conformance green on linux-x64 and
  darwin-arm64, not by manual comparison against the TS implementation.
- This record does not itself change any behavior - it is the freeze
  declaration the roadmap's first Milestone 6 checkbox refers to.
