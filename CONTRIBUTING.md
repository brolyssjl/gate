# Contributing

## Branching & pull requests

`main` is the release branch. **From Milestone 2 onward, all changes land through
pull requests** — no direct pushes to `main`.

1. Branch from `main`: `git switch -c <type>/<short-topic>` (e.g.
   `feat/debug-phase`, `fix/coverage-parser`, `chore/ci`).
2. Keep commits focused; the working tree must pass `npm run build`,
   `npm run lint`, and `npm test` before you open the PR.
3. Open the PR against `main` (`gh pr create`). The description states what
   changed and how it was verified. Do not append AI/co-author signatures.
4. Merge only after review and green checks.

Dogfood where practical: run the change through Gate itself
(`gate start … → approve → next …`) so process discipline is exercised, not just
described.

## Releases

Releases are cut from `main` with a semver tag and a GitHub release.

1. Bump the version in `package.json` and `rust/Cargo.toml` (the release.yml
   workflow enforces they stay in lockstep; it hard-fails if the tag, package.json,
   and Cargo.toml versions disagree).
2. Ensure `main` is green: `npm run build && npm run lint && npm test`, and the
   Rust CI job passes (see "Rust implementation" below).
3. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
4. `gh release create vX.Y.Z --title "vX.Y.Z - <name>" --notes "…"`.

npm distribution is permanently out per the Milestone 6 owner decision
(see ROADMAP.md). Releases are tags + GitHub releases with cargo-built
binaries; no npm publishing.

## Conformance testing

1.0.0 is planned as a from-scratch port against a frozen CLI surface. To
make that port checkable against this TS implementation without a
parallel test suite, the CLI-driving test files (`test/cli.e2e.test.ts`,
`test/concurrency.test.ts`, `test/humanReview.test.ts`, `test/prune.test.ts`,
`test/guard.test.ts`, `test/amend.test.ts`) never import Gate's internals - they only spawn the
`gate` binary and assert on its stdout/stderr/exit code and the state it
writes under `.gate/`. That's the black-box contract: argv in, exit code +
stdout/stderr + `.gate/` state out.

They resolve which binary to spawn through a single helper
(`test/helpers.ts`'s `gate()`): `$GATE_BIN` when set, otherwise `node
dist/cli.js` (the local build). Point `GATE_BIN` at any binary that
implements the same contract - including the Rust build (rust/, the canonical
binary) - and the same suite exercises it unchanged:

```bash
npm run build                    # only needed for the default (unset GATE_BIN) case
GATE_BIN=/path/to/other/gate npm run conformance
```

`npm run conformance` runs exactly that binary-agnostic subset (not the full
`npm test`, which also runs unit-level suites that import `src/` directly and
therefore only make sense against this TS implementation).

### Test hermeticity

The suite must be green regardless of what happens to be installed on the
machine running it - a real `agnosgram` binary on PATH must not change what
the RETRO tests exercise. `writeJournalEntry` (`src/integrations/agnosgramWrite.ts`)
resolves the binary from `$GATE_AGNOSGRAM_BIN`, defaulting to `agnosgram` on
PATH; the fallback (ENOENT) tests point it at a path that can't possibly
resolve, and the CLI-success-path test points it at a throwaway stub script,
so both exercise their intended path deterministically either way. This is a
test-only override, not a user-facing config knob.

## Rust implementation

Milestone 6 is a from-scratch Rust port living in `rust/` (crate `gate`,
`rust/Cargo.toml`, edition 2021, bin target `gate`). See `docs/rust-port.md`
for the full plan and module layout; the short version:

- **Zero dependencies, std only.** No external crates, mirroring the
  TypeScript implementation's zero-runtime-dependency policy. JSON, YAML, and
  SHA-256 are all hand-rolled ports of the `core/*.ts` equivalents. Do not add
  a `[dependencies]` entry without discussing it first - it breaks a
  deliberate reviewability property.
- **The CLI surface is frozen** (`docs/decisions/0001-surface-freeze.md`):
  the TypeScript implementation is the reference and the conformance suite
  (see "Conformance testing" above) is the normative definition of correct
  behavior. The Rust port must match it byte-for-byte - stdout/stderr, exit
  codes, on-disk effects - not just "behave similarly."

Dev loop, from repo root:

```bash
# rustup's brew shims may be absent; the toolchain itself is stable:
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
cargo fmt --check --manifest-path rust/Cargo.toml
cargo clippy --all-targets --manifest-path rust/Cargo.toml -- -D warnings
cargo test --manifest-path rust/Cargo.toml
cargo build --release --manifest-path rust/Cargo.toml
GATE_BIN="$PWD/rust/target/release/gate" npm run conformance
```

All five must pass before opening a PR that touches `rust/`. Any change to
the CLI surface (a new command, flag, output string, or on-disk effect) must
land with a corresponding update to the conformance suite in the **same**
change, whichever implementation you touched first - the suite is what keeps
the two implementations from silently drifting apart.

## Conventions

- Commit author must match the local git config; never add `Co-authored-by`
  trailers or AI signatures.
- Deterministic checks live in code (`src/gates/`), judgment lives in playbooks
  (`playbooks/`). Never blur that line.
- Every new gate check needs a passing- and a failing-fixture test.
