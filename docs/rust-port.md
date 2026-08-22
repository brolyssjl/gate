# Rust port plan (Milestone 6)

Working plan for porting the frozen CLI surface (docs/decisions/0001) to Rust.
Modeled on agnosgram's port (its `docs/rust-port.md`, executed 2026-08-20),
which validated the approach end to end: 1:1 file mapping, std-only crate,
conformance suite as the acceptance gate, waves with disjoint file ownership.
Contributor document; delete or archive once `1.0.0` ships.

## Goal and definition of done

One Rust crate at `rust/`, producing a single static `gate` binary whose CLI
surface is identical to the TypeScript implementation at `0.4.0`. Done means:

```bash
cargo build --release --manifest-path rust/Cargo.toml
GATE_BIN="$PWD/rust/target/release/gate" npm run conformance
```

passes unmodified (68 tests across 6 vitest files), on darwin-arm64 and
linux-x64. The conformance suite is the normative surface definition
(docs/decisions/0001); the TS source is the spec for everything it exercises.

## Layout

1:1 module mapping to the TS tree, same names, one Rust module per TS file:

```
rust/src/
├── main.rs             # src/cli.ts: dispatch, help/version, exit codes
├── cli/                # args.rs, context.rs, output.rs
├── core/               # one module per src/core/*.ts, plus:
│   ├── json.rs         # NEW: hand-rolled JSON (see agnosgram's core/json.rs)
│   ├── yaml.rs         # NEW: YAML subset parser/stringifier (see below)
│   └── sha256.rs       # NEW: hand-rolled SHA-256 (FIPS 180-4)
├── gates/              # one module per src/gates/*.ts
├── commands/           # one module per src/commands/*.ts
├── artifacts/          # one module per src/artifacts/*.ts
├── integrations/       # advise.rs, agnosgram_write.rs, index.rs
├── serialize/          # index.rs, toon.rs
└── adapters.rs         # src/adapters/index.ts
```

## Dependency policy: std only

Zero external crates, same as agnosgram's port. The TS implementation has one
runtime dependency (the `yaml` npm package) and uses `node:crypto`; the Rust
port replaces both with hand-rolled, unit-tested modules:

- **`core/yaml.rs`**: gate uses YAML in exactly three places - `parseYaml` on
  `config.yml` (core/config.ts), `parseYaml` on artifact frontmatter
  (artifacts/frontmatter.ts), and `stringify` in the review packet
  (artifacts/review.ts). Port a subset parser covering what those call sites
  and the shipped/documented file shapes need: block maps and sequences,
  flow sequences and maps, plain/single/double-quoted scalars, `#` comments,
  null/bool/number scalar typing as the `yaml` package produces them, and
  nested indentation. The stringify side must byte-match the `yaml` package's
  output for the review packet's actual payload shape - pin it with unit
  tests generated from the TS implementation's real output. Divergence
  policy: if a user hand-writes YAML outside the subset, the error message
  may differ from the `yaml` package's; the conformance suite defines what
  must match. Precedent: agnosgram DEC-0001 chose a hand-rolled subset for
  reviewability; gate inherits that reasoning.
- **`core/sha256.rs`**: FIPS 180-4 SHA-256, needed by trust hashing
  (core/trust.ts), tree fingerprints (core/git.ts), plan content hashes
  (artifacts/plan.ts), and gitignore state (core/gitignoreState.ts). All
  call sites format as `"sha256:" + hex`. Pin against known test vectors
  (empty string, "abc", a >64-byte input, a >1-block input) plus real
  outputs from the TS binary.
- **`core/json.rs`**: same contract as agnosgram's - emit byte-matching
  `JSON.stringify(value, null, 2)` (run.json is written pretty-printed;
  verify the exact TS write style in core/run.ts and match it), ordered
  keys, JS escaping rules; parse what `JSON.parse` accepts (test reports,
  coverage JSON, advise reports). Start from agnosgram's implementation
  (`../agnosgram/rust/src/core/json.rs`) - copying it wholesale is expected
  and fine (shared-code duplication across the two tools is accepted by the
  roadmap).

Process spawning (core/exec.ts, core/git.ts, integrations/agnosgramWrite.ts)
uses `std::process::Command`. Mirror the TS spawn semantics exactly: shell vs
no-shell (check each call site - trusted commands run through a shell in TS),
cwd, env, stdout/stderr capture, exit-code mapping, and the `$GATE_TEST_REPORT`
env contract in gates/test.ts.

## Exactness contract

Same rules as agnosgram's port, all verified there the hard way:

1. **Exit codes**: mirror src/cli.ts's `reportError`: 0 pass, UserError exit
   codes as coded (1 fail / 2 usage - read the actual mapping), unknown
   command 2. Help text and version output byte-for-byte. Version: compile
   from `Cargo.toml` via `env!("CARGO_PKG_VERSION")` (agnosgram learned to
   avoid a second hardcoded copy); the port ships as the version cli.ts's
   `readVersion()` reports from package.json - keep them in lockstep.
2. **Arg parsing**: port src/cli/args.ts faithfully, including every error
   string. If it routes through Node's parseArgs, reproduce the error
   FIRST LINES verbatim (verified list in agnosgram's plan; re-verify any
   doubt with `node -e`).
3. **Output**: every stdout/stderr literal byte-for-byte; `--json` via
   core/json.rs; `--format toon` via a faithful serialize/toon.rs port.
4. **Filesystem effects**: run.json (schema 4 + the 1->2->3->4 migration chain), current.json,
   trust.json, playbook copies (core/embeddedPlaybooks.ts - port the
   embedded literals byte-for-byte; note `npm run build` embeds playbooks/
   into TS, check scripts/embedPlaybooks.mjs for how), artifact scaffolds
   (plan.md, review.md, debug-log.md, retro.md), atomic write patterns
   (crash-safe state was a Milestone 2 hardening - mirror the
   write-to-temp-then-rename behavior in core/fsx.ts).
5. **Timestamps/durations**: check every `new Date()`/`Date.now()` call site
   and port semantics (ISO strings, duration math in report.ts). Local vs
   UTC matters; agnosgram's `localtime_r` FFI pattern is available if local
   time is needed.
6. **Byte-length semantics**: JS `.length`/`.slice` are UTF-16 code units -
   port with `encode_utf16` where truncation/lengths surface in output
   (agnosgram review fix; don't repeat it).

The conformance suite wins over this document; the TS source wins over
intuition.

## Build/test loop

```bash
# rustup's brew shims may be absent; the toolchain itself is stable:
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cargo fmt --check --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml -- -D warnings   # check real exit code
cargo test --manifest-path rust/Cargo.toml
cargo build --release --manifest-path rust/Cargo.toml
npm run build && npm test                                      # TS untouched
GATE_BIN="$PWD/rust/target/release/gate" npm run conformance
```

Twin-dir byte-diffs against the TS binary (`node dist/cli.js` vs the Rust
binary in twin temp dirs, `diff -r` the trees and diff stdout/stderr) are
mandatory per wave - the conformance suite mostly asserts substrings, and
agnosgram's two byte-level divergences were only caught by diffing.

## Waves (disjoint file ownership; coordinator commits after review)

1. **Scaffold + foundations**: crate, main.rs + cli/ (dispatch, help,
   version, exit codes byte-verified), core/json.rs + yaml.rs + sha256.rs +
   the pure-data core modules (paths, fsx, glob, markers, stateMachine,
   version, config, targets), all commands stubbed. Unit tests ported per
   module.
2. **State + process core**: exec, git, run, current, trust,
   gitignoreState, playbooks + embeddedPlaybooks, serialize/, adapters.
3. **Gates + artifacts**: gates/* and artifacts/* (the deterministic
   checks: plan, implement, test + testReport + coverage parsers, debug,
   review, retro, planDrift), integrations/*.
4. **Commands**: the 17 commands + cli/context + commands/shared, in two
   parallel groups split by conformance file ownership:
   (a) init, adapt, trust, start, approve, status, playbook, log, skip
   (cli.e2e + concurrency surface); (b) check, next, amend, review, retro,
   report, prune, guard (+ inferStack/gateRun/advance internals as needed
   by either group - assign them to whichever group's commands import them,
   and freeze the split before launching).
5. **Closure + pipeline**: full conformance green, then ci.yml rust job
   (ubuntu + macos, fmt/clippy/test/build + conformance via GATE_BIN) and
   release.yml switched to cargo-built binaries with the same asset names
   (`gate-linux-x64`, `gate-darwin-arm64`), version guard
   (tag = package.json = Cargo.toml), and a hard conformance job - mirror
   agnosgram's release.yml, which dry-ran all of this.

Wave boundaries are file-ownership boundaries: later waves never edit files
an earlier wave froze; parallel groups never share files. Agents do not run
git; the coordinator reviews, fixes, and commits.
