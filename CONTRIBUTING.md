# Contributing

## Branching & pull requests

`main` is the release branch. **From Milestone 2 onward, all changes land through
pull requests** — no direct pushes to `main`.

1. Branch from `main`: `git switch -c <type>/<short-topic>` (e.g.
   `feat/debug-phase`, `fix/coverage-parser`, `chore/ci`).
2. Keep commits focused; the working tree must pass `cargo build --release
   --manifest-path rust/Cargo.toml`, `cargo clippy --all-targets
   --manifest-path rust/Cargo.toml -- -D warnings`, and `cargo test
   --manifest-path rust/Cargo.toml` before you open the PR.
3. Open the PR against `main` (`gh pr create`). The description states what
   changed and how it was verified. Do not append AI/co-author signatures.
4. Merge only after review and green checks.

Dogfood where practical: run the change through Gate itself
(`gate start … → approve → next …`) so process discipline is exercised, not just
described.

## Releases

Releases are cut from `main` with a semver tag and a GitHub release.

1. Bump the version in `rust/Cargo.toml` (the release.yml workflow enforces
   it hard-fails if the tag and Cargo.toml version disagree).
2. Ensure `main` is green: `cargo build --release --manifest-path
   rust/Cargo.toml && cargo clippy --all-targets --manifest-path
   rust/Cargo.toml -- -D warnings && cargo test --manifest-path
   rust/Cargo.toml`, and the CI job passes (see "The gate crate" below).
3. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
4. `gh release create vX.Y.Z --title "vX.Y.Z - <name>" --notes "…"`.

npm distribution is permanently out per the Milestone 6 owner decision
(see ROADMAP.md). Releases are tags + GitHub releases with cargo-built
binaries; no npm publishing.

### Release checksums (SEC-02/03)

`release.yml`'s `release` job hashes every built asset (`sha256sum`) into a
`SHA256SUMS` file and publishes it alongside the binaries. `install.sh`
downloads it the same way it downloads the binary (direct, with a `gh`
fallback for network hiccups or unauthenticated rate limiting) and verifies
the binary against it *before* `chmod +x`/`mv` - a checksum mismatch, or a
missing `SHA256SUMS` entry, aborts with nothing installed. `GATE_VERSION=X.Y.Z`
pins an exact release instead of the latest one; either way, the script
prints the version actually installed (`gate --version`, read back from the
binary it just placed, not just echoed from an env var).

`install.sh` guards its `main "$@"` call behind a
`[ "${BASH_SOURCE[0]}" = "${0}" ]` check, so it can be `source`d for testing
without triggering a real install - useful for exercising `verify_checksum`
in isolation against a local fixture pair (a dummy file + a `SHA256SUMS`
generated for it) instead of a real download:

```bash
source install.sh
verify_checksum /path/to/downloaded-file gate-linux-x64 /path/to/SHA256SUMS
```

`bash -n install.sh` is the syntax-only check; the CI/release paths
themselves (actually hitting GitHub's release API) can't be run locally.

## Conformance testing

Rust is the only implementation, but the CLI surface is still frozen
(`docs/decisions/0001-surface-freeze.md`), and `rust/tests/` - a black-box
conformance suite - is the normative definition of correct behavior for that
frozen surface. The suite (`rust/tests/cli_e2e.rs`, `concurrency.rs`,
`human_review.rs`, `prune.rs`, `guard.rs`, `amend.rs`, `streak.rs`,
`doctor_update.rs`, plus the shared harness in `rust/tests/common/`) never
imports Gate's internals - it only
spawns the `gate` binary and asserts on its stdout/stderr/exit code and the
state it writes under `.gate/`. That's the black-box contract: argv in, exit
code + stdout/stderr + `.gate/` state out. `rust/tests/CONFORMANCE_MAP.md`
records where each case came from (it was ported wholesale from a
since-retired TypeScript reference implementation's equivalent test suite;
see ROADMAP.md for that history).

The harness (`rust/tests/common/mod.rs`) resolves which binary to spawn via
`$GATE_BIN` when set, otherwise the binary `cargo test` just built for this
crate (`env!("CARGO_BIN_EXE_gate")`). Point `GATE_BIN` at any binary that
implements the same contract - a downloaded release asset, for instance - and
the same suite exercises it unchanged:

```bash
cargo build --release --manifest-path rust/Cargo.toml
GATE_BIN="$PWD/rust/target/release/gate" cargo test --manifest-path rust/Cargo.toml
```

`cargo test` with no `$GATE_BIN` set runs both the crate's ~530 unit tests
and this conformance suite against a freshly built debug binary in one pass.

### Test hermeticity

The suite must be green regardless of what happens to be installed on the
machine running it - a real `agnosgram` binary on PATH must not change what
the RETRO tests exercise. `write_journal_entry`
(`rust/src/integrations/agnosgram_write.rs`) resolves the binary from
`$GATE_AGNOSGRAM_BIN`, defaulting to `agnosgram` on PATH; the fallback
(ENOENT) tests point it at a path that can't possibly resolve, and the
CLI-success-path test points it at a throwaway stub script, so both exercise
their intended path deterministically either way. This is a test-only
override, not a user-facing config knob.

## The gate crate

`rust/` (crate `gate`, `rust/Cargo.toml`, edition 2021, bin target `gate`) is
the whole implementation. See `docs/rust-port.md` for module layout; the
short version:

- **Zero dependencies, std only.** No external crates. JSON, YAML, and
  SHA-256 are all hand-rolled. Do not add a `[dependencies]` entry without
  discussing it first - it breaks a deliberate reviewability property.
- **The CLI surface is frozen** (`docs/decisions/0001-surface-freeze.md`):
  the conformance suite (see "Conformance testing" above) is the normative
  definition of correct behavior - stdout/stderr, exit codes, on-disk
  effects, not just "behaves similarly."

Dev loop, from repo root:

```bash
# rustup's brew shims may be absent; the toolchain itself is stable:
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cargo fmt --check --manifest-path rust/Cargo.toml
cargo clippy --all-targets --manifest-path rust/Cargo.toml -- -D warnings
cargo test --manifest-path rust/Cargo.toml
cargo build --release --manifest-path rust/Cargo.toml
```

All four must pass before opening a PR. Any change to the CLI surface (a new
command, flag, output string, or on-disk effect) must land with a
corresponding update to `rust/tests/` in the **same** change - the suite is
what keeps the frozen surface from silently drifting.

## Conventions

- Commit author must match the local git config; never add `Co-authored-by`
  trailers or AI signatures.
- Deterministic checks live in code (`rust/src/gates/`), judgment lives in
  playbooks (`playbooks/`). Never blur that line.
- Every new gate check needs a passing- and a failing-fixture test.
