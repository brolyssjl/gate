# Runs, concurrency, targets, and scope

## Concurrency (branch-keyed runs)

One active run per branch, not one globally: `.gate/current.json` maps each
branch to its own active run id, so parallel work-in-progress on separate
branches never collides. `gate start` on a branch that already has an active
run **resumes it** instead of erroring (unless the plan was approved and then
edited since - that refuses, with a pointer to re-approve or resolve the run
first); a new branch gets its own independent run. `gate status` shows the
current branch's run plus a list of any other branches with one in flight.

A detached `HEAD` has no branch to resolve "current" from - that's genuinely
ambiguous, not a missing-run case - so every phase command (`approve`,
`check`, `next`, `review`, `retro`, `skip`, `log`, `playbook`) accepts an
explicit `--run <id>` to select a run without relying on the checked-out
branch.

## Targets (multi-stack repos)

Single-stack projects use a flat `commands:`/`thresholds:` block. Multi-stack
repos declare named **targets** in `config.yml`:

```yaml
targets:
  api:
    match: ["apps/api/**"]
    commands: { test: "pytest", coverage: "pytest --cov --cov-report=json:coverage/coverage.json" }
    thresholds: { diff_coverage: 85 }
    coverage_format: coverage-py                          # overrides the top-level default for this target
    playbooks: { test: ".gate/playbooks/test.api.md" }   # overlay on the base playbook
  web:
    match: ["apps/web/**"]
    commands: { test: "vitest run" }
```

Composition rule: **profiles choose which phases run; targets choose how each
phase runs.** IMPLEMENT/TEST/DEBUG resolve affected targets from the real
changed files and run each target's own commands, producing bracketed check
names (`implement.build[api]`, `test.command[web]`) so a change touching both
stacks must pass both. `gate playbook` appends a `## Target overlay: <name>`
section per affected target that declares one - once `gate trust` covers it
(see [Integrity: command trust (TOFU)](integrity.md#command-trust-tofu)); an
untrusted overlay is refused, not appended. With no `targets:` configured, or
none affected by the change, behavior and check names are byte-identical to a
single-stack repo - targets are purely additive. `gate start --target <a,b>`
overrides resolution for the whole run.

## Scope noise (`scope_ignore`)

The IMPLEMENT/DEBUG scope check normally fails on any touched file not
declared in `plan.md`. Environment cruft - a node compile cache, a go build
dir, coverage output - gets rewritten by the very commands Gate runs, and
would otherwise hard-block the gate with false violations having nothing to
do with the plan. `scope_ignore` in `config.yml` is a glob list of paths the
scope check treats as noise instead:

```yaml
scope_ignore:
  - "node_modules/**"
  - "coverage/**"
```

`gate init` seeds it per detected stack. It's part of the trust hash
(`commandsBlockHashSource`) - the same class as a command - so widening it
needs a `gate trust` re-approval, same as editing `commands:`; an untrusted
edit is never silently applied. A match is never hidden: it surfaces as its
own info-level check line (`implement.scope.ignored` / `debug.scope.ignored`)
naming exactly which files were excluded and why.

**Upgrading from before 0.4.0:** an empty (or absent) `scope_ignore:` hashes
identically to the pre-0.4.0 commands block, so an existing `trust.json`
stays valid after upgrading `gate` - you are not forced into a surprise
re-trust just because a new version understands a key you never set. The
first time you actually populate `scope_ignore:`, it joins the hash and a
normal `gate trust` is required, exactly like changing a command.

## Pruning runs

Run folders are ephemeral by design. `gate prune` archives non-active runs
past the retention window: a summary (the same shape `gate report` prints)
goes to `.gate/archive/<id>.json`, then the run folder is deleted.

```bash
gate prune --dry-run          # see what would be pruned, touch nothing
gate prune                    # keep the newest 10 (default), archive+remove the rest
gate prune --keep 20 --days 30   # also require a candidate to be >30 days old
```

`--keep` (default 10, or config.yml's `retention.keep`) always spares the
newest N by `updatedAt`; `--days` (or `retention.days`) is an *additional* age
requirement, not a replacement for `--keep`. The active run is never a
candidate. `retention:` in config.yml is deliberately outside the trust
hash - it's not a command.
