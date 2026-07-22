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

## Conventions

- Commit author must match the local git config; never add `Co-authored-by`
  trailers or AI signatures.
- Deterministic checks live in code (`src/gates/`), judgment lives in playbooks
  (`playbooks/`). Never blur that line.
- Every new gate check needs a passing- and a failing-fixture test.
