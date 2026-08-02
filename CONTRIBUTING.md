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

1. Bump the version in `package.json` (the CLI reports it via
   `src/core/version.ts`, which reads `package.json` — keep them from drifting by
   never hard-coding the version elsewhere).
2. Ensure `main` is green (`npm run build && npm run lint && npm test`).
3. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
4. `gh release create vX.Y.Z --title "vX.Y.Z — <name>" --notes "…"`.

The package is `private` until an npm name is chosen (see the roadmap), so
releases are tags + GitHub releases only — **no `npm publish` yet**.

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
implements the same contract - including a future Rust build - and the same
suite exercises it unchanged:

```bash
npm run build                    # only needed for the default (unset GATE_BIN) case
GATE_BIN=/path/to/other/gate npm run conformance
```

`npm run conformance` runs exactly that binary-agnostic subset (not the full
`npm test`, which also runs unit-level suites that import `src/` directly and
therefore only make sense against this TS implementation).

## Conventions

- Commit author must match the local git config; never add `Co-authored-by`
  trailers or AI signatures.
- Deterministic checks live in code (`src/gates/`), judgment lives in playbooks
  (`playbooks/`). Never blur that line.
- Every new gate check needs a passing- and a failing-fixture test.
