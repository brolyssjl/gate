# Architecture

_System shape, module map, key invariants. Keep it to what an agent must know
before touching the code - not an exhaustive tour._

## Module map
- `rust/src/main.rs` - argv dispatch + per-command help strings (help must
  track flag changes; drift here was review finding f2 on #33)
- `rust/src/cli/` - args (never validates unknown flags, TS-parser parity),
  output/emit (human + `--json`), context (root/run resolution)
- `rust/src/commands/` - one file per subcommand; each splits an
  `execute(root, ...)` out of `run(argv)` so unit tests avoid global cwd
- `rust/src/core/` - run.json read/write + schema migrations (`run.rs`),
  state machine/profiles, config + trust hashing, identity resolution
  (`identity.rs`), hand-rolled json/yaml/sha256, git plumbing
- `rust/src/gates/` - deterministic phase checks only; judgment lives in
  `playbooks/` (bundled copies embedded via `core/embedded_playbooks.rs`)
- `rust/src/artifacts/` - plan.md / review.md / retro.md parsing
- `rust/src/integrations/` - agnosgram journal write, SDD detection
- `rust/tests/` - black-box conformance suite; spawns the binary, asserts
  argv in -> exit code + stdout/stderr + `.gate/` state out

## Invariants
- **The CLI surface is frozen** (docs/decisions/0001-surface-freeze.md);
  the conformance suite is the normative definition. Any surface change
  lands with its test update in the same change
- Deterministic checks in code, judgment in playbooks - never blur it
- run.json: additive keys are omit-when-absent (`targetOverride`,
  `startedBy` precedent), schema bumps only for real migrations
- `sessionId` is strictly harness-provided; the identity fallback chain
  (`--by` > `GATE_SESSION_ID` > git user.name) feeds every other recorded
  identity but must never leak into fields a gate compares (see #33)
- Tests must stay hermetic: never depend on binaries on PATH
  (`$GATE_AGNOSGRAM_BIN` override) or the machine's git identity
  (isolate `$HOME`/`GIT_CONFIG_*`)
