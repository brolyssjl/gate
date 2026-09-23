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
4. **Wait for release.yml** - the tag push triggers it, and its release job
   creates the GitHub release itself (`gh release create --generate-notes
   --verify-tag`) with the built binaries and `SHA256SUMS` attached. Do NOT
   run `gh release create` by hand: a release existing before the workflow
   gets there makes that job fail with "a release with the same tag name
   already exists", leaving a release with no assets (this happened on
   v1.5.0; the recovery is `gh release delete vX.Y.Z -y` followed by
   rerunning the failed job).
5. Once the workflow is green, replace the auto-generated notes with curated
   ones: `gh release edit vX.Y.Z --notes "…"`. Release titles stay the bare tag
   (`vX.Y.Z`) - every release to date is titled that way; the summary
   belongs in the notes, not the title.

npm distribution is permanently out per the Milestone 6 owner decision
(see ROADMAP.md). Releases are tags + GitHub releases with cargo-built
binaries; no npm publishing. See [SECURITY.md](SECURITY.md) for how a
downloaded release is verified end to end.

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

### Release provenance

`release.yml` also attests build provenance for every release asset with
`actions/attest-build-provenance` (needs the `id-token: write` and
`attestations: write` workflow permissions - a signed SLSA statement tying
the binary to the exact workflow run that built it, on top of the checksum
above). `install.sh` checks it with `gh attestation verify` after the
checksum passes, when `gh` is on PATH; missing `gh`, or a release that
predates attestations, is a note, not a failure - only a verification
mismatch aborts the install. Source the script the same way to exercise
`verify_provenance` against a fake `gh` on `PATH` (a shell function or a
stub script that echoes canned output and exits 0/1) instead of a real
release:

```bash
source install.sh
verify_provenance /path/to/downloaded-file gate-linux-x64
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

`cargo test` with no `$GATE_BIN` set runs both the crate's ~600 unit tests
and this conformance suite against a freshly built debug binary in one pass.

### Test hermeticity

The suite must be green regardless of what happens to be installed on the
machine running it - a real `agnosgram` binary on PATH must not change what
the RETRO tests exercise. `write_journal_entry`
(`rust/src/integrations/agnosgram_write.rs`) resolves the binary from
`$GATE_AGNOSGRAM_BIN`, defaulting to `agnosgram` on PATH, and hands it to
`write_journal_entry_with(bin, ...)`, which the unit tests call directly: the
fallback (ENOENT) tests pass a path that can't possibly resolve, and the
CLI-success-path test passes a throwaway stub script, so both exercise their
intended path deterministically either way, without touching the process
environment. `$GATE_AGNOSGRAM_BIN` is a hermeticity override for running the
suite or a build under a pinned binary, not a user-facing config knob.

Two hygiene rules keep the unit suite race-free and safe on a shared `/tmp`,
and `rust/tests/test_hygiene.rs` lints `rust/src` for both (2026-09-22
security audit finding 11):

- never mutate process-global state in a unit test (`env::set_var`,
  `env::remove_var`, `set_current_dir`) - `cargo test` runs a crate's tests
  as threads in one process. Pass values through parameters, or set env on
  a spawned `Command`;
- create every unit-test temp dir through `core::testutil::unique_temp_dir`
  (`rust/src/core/testutil.rs`), which gives it an unguessable name and
  refuses to adopt a pre-existing entry, and remove it when the test is
  done. Integration tests under `rust/tests/` use `common::make_temp_dir`.

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
# If cargo isn't on PATH but rustup is installed, add your toolchain's bin
# dir (adjust the triple for your platform), e.g.:
export PATH="$HOME/.rustup/toolchains/stable-<your-target-triple>/bin:$PATH"
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
