# Stack

_Languages, tooling, and the exact commands to build / test / lint. Pin versions
where they matter._

## Commands
- **Build:** `cargo build --release --manifest-path rust/Cargo.toml`
- **Test:** `./scripts/test-with-report.sh` (runs the full cargo suite and,
  when gate provides `$GATE_TEST_REPORT`, emits the generic JSON report the
  TEST gate parses; plain `cargo test --manifest-path rust/Cargo.toml` works
  for humans)
- **Lint:** `cargo clippy --all-targets --manifest-path rust/Cargo.toml -- -D warnings`
- **Format:** `cargo fmt --check --manifest-path rust/Cargo.toml` (rustup's
  brew shims may be absent; use
  `$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin` on PATH)

## Versions
- Rust stable, edition 2021, crate `gate` under `rust/`, bin target `gate`
- **Zero external crates** - std only; JSON, YAML, and SHA-256 are
  hand-rolled. Never add a `[dependencies]` entry without an owner decision
  (deliberate reviewability property, see CONTRIBUTING.md)
- Releases: semver tag + GitHub release with cargo-built binaries and a
  SHA256SUMS file; `install.sh` verifies checksums. npm is permanently out
