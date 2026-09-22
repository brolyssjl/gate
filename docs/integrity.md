# Integrity: evidence, loop enforcement, identity, and trust

## Evidence integrity

If an agent could hand Gate the evidence, enforcement would be fiction. So
machine evidence is produced by Gate itself:

- Gate runs the configured `build`/`lint`/`test`/`coverage` commands and reads
  their exit codes directly.
- The test report comes from the run Gate just executed: JSON on the command's
  stdout (Gate persists the normalized copy to the run folder), or a file the
  command itself wrote during the run - runners can target
  `$GATE_TEST_REPORT`, e.g. `vitest --reporter=json --outputFile=$GATE_TEST_REPORT`.
  A report staged in advance (by hand or via `gate log`) is ignored.
- `run.json` is written atomically, so a crash never corrupts run state.

## Loop enforcement (failure-streak cap)

An agent stuck in a loop tends to do one thing: run `gate check`/`gate next`
again and hope. Gate tracks each phase's *consecutive* failed evaluations in
`run.json` and, once a phase hits the limit (default **3**), refuses to
evaluate that phase at all - a distinct, non-zero exit (`3`) and a message
naming the streak, pointing at the run's worklog/debug log, and naming both
ways forward:

```
$ gate next
gate: TEST blocked: 3 consecutive failures (limit 3). See the run's
worklog/debug log for what's failing, then either `gate skip TEST --reason
"<why>"` or `gate streak reset TEST --reason "<why>"` once a human has
reviewed and a retry is warranted.
```

- **What counts as a failure**: `gate check`/`gate next` evaluating the
  phase's gate and it failing. A gate refusing to run an untrusted command
  (`gate trust`) does *not* count - retrying gives the identical result
  until a human runs `gate trust`, so the cap would just add friction to a
  problem it can't fix. Usage errors and a streak refusal itself never
  count either.
- **What resets the streak to 0**: a passing evaluation, advancing to the
  next phase, `gate skip`, and `gate streak reset`.
- **Recovery, once blocked** - both audited in `run.json` and visible in
  `gate report`, the same as any other override:
  - `gate skip <phase> --reason "…"` - the existing human-authorized skip.
  - `gate streak reset [<phase>] --reason "…" [--by …]` - clear the streak
    without skipping the phase, so the very next `gate check`/`gate next`
    evaluates for real. `gate streak` (no subcommand) shows every phase's
    current streak against the configured limit.
- **Configuring the limit** - `thresholds.failure_streak_limit` in
  `config.yml` (default 3 when unset); `0` disables the cap entirely. This
  key is *not* part of the TOFU commands-block trust hash (see "Command
  trust (TOFU)" below) - it doesn't change what gets executed, only how many
  times Gate will look.
- **Every real failure is persisted, not just scrollback** - `gate check`
  and `gate next` both record a `Failed` history event (which checks
  failed, when) and bump a permanent per-phase failure count in `run.json`
  the moment an evaluation fails, whether or not it also moves the streak
  (a trust-blocked failure doesn't count toward the streak but is still
  recorded, matching every other real failure). `gate report` reads the
  permanent count for its per-phase failure totals - `run.json`'s `history`
  itself only keeps the most recent 20 `Failed` events (oldest pruned first)
  so a long, troubled run's file doesn't grow without bound, but the count
  `gate report` shows stays exact regardless. `gate streak` (no subcommand)
  also works after a run reaches DONE - it reports the finished run's final
  streak state instead of erroring just because the branch's "current run"
  pointer was cleared.

Like every override in Gate, `gate streak reset` is honest about what it
is: a CLI cannot stop an agent in the same shell from running it, the same
way it cannot stop `gate skip` or `gate trust`. What it guarantees is that
doing so is explicit and visible, not automatic - the point isn't to make
looping impossible, it's to make continuing past a real, repeated failure a
deliberate act with a paper trail, not something that happens by default.

## Identity fallback

Every command that records who acted resolves that identity through one
shared fallback chain: the three explicit human checkpoints - `gate
trust`, `gate approve`, `gate streak reset` (`trustedBy`, `approval.by`,
`override.by`) - plus `gate skip` (`override.by`), `gate amend`
(`amendment.by`), `gate review`'s packet request (`review.requestedBy`),
and `gate start` (`startedBy`). Left alone, those fields stayed `null`
unless the caller happened to pass `--by` or set `GATE_SESSION_ID` -
nothing nudged toward either, so the audit trail said *that* a checkpoint
was passed but not *who* passed it. The chain, in order:

1. `--by <name>`
2. `GATE_SESSION_ID` (env)
3. `git config user.name` (trimmed; empty or erroring is treated as absent)
4. `null`, when all three are absent

Weak identity beats none: `git config user.name` is self-reported and just
as spoofable as `--by` itself - see [Threat model](threat-model.md). It just
means the common case (an agent or human running these commands from inside a
normal git checkout) no longer defaults to `null`.

One deliberate exception: `gate start`'s `sessionId` stays strictly
harness-provided (`--session` or `GATE_SESSION_ID`, no git-name
fallback). The REVIEW gate's reviewer-independence check hard-fails when
the reviewer name equals the recorded `sessionId`, so letting `git config
user.name` leak into it would flip the documented solo review flow from
advisory to blocking in every one-person repo. Who *started* the run is
recorded separately as `startedBy` (the full chain, audit-only, never
compared against anything).

Opt-in hard enforcement: set `identity.require_identity: true` in
`config.yml` and the deliberate sign-offs - `trust`, `approve`, `streak
reset`, and `skip` - fail (exit 1) instead of writing a `null` identity
when none of the three sources resolves anything (`start`, `amend`, and
the review packet request still record `null`; they are not sign-offs):

```yaml
identity:
  require_identity: true   # optional, default false
```

This only checks *that* an identity resolved, not *which* one - it does not
compare the recorded identity against, say, who did the IMPLEMENT-phase
work (non-implementer/self-approval enforcement). That needs real,
unspoofable session identity to be meaningful rather than advisory, and is
deliberately deferred - see ROADMAP.md's "Reviewer identity threading".

## Command trust (TOFU)

Gate executes the commands in `config.yml` - the same trust class as npm
scripts. Before any gate will run them, a human must review the config and run
`gate trust`, which hashes the `commands:` block and writes the record to a
machine-local store, keyed by a hash of the checkout's canonicalized project
root - `$GATE_CONFIG_DIR/trust/<hash>.json`, falling back to
`$XDG_CONFIG_HOME/gate/trust/` and then `$HOME/.config/gate/trust/`. The
IMPLEMENT and TEST gates **refuse to spawn a process** until the current hash
matches; changing a command invalidates trust until you re-run `gate trust`.
`gate trust --check` reports status (exit 0/1) without writing.

Trust is a property of *this machine and this checkout*, never of the repo.
Earlier versions stored the record at `.gate/trust.json`, tracked in the repo
- that meant a cloned repo could ship its own trust decision: an attacker ran
`gate trust` in their own copy, committed the resulting `trust.json` next to a
hostile `commands:` block, and the victim's very first `gate check` on a fresh
clone ran it, no local `gate trust` ever required (security audit 2026-09-22,
finding 1). A `.gate/trust.json` left over in a repo now has no bearing on the
decision at all - `gate doctor` flags one as a stale file to delete, and CI
runners need their own `gate trust` (or a pre-seeded `GATE_CONFIG_DIR`) the
same as any other machine; there is no repo-level pin to inherit any more.

The same hash also covers **playbooks** - `.gate/playbooks/<phase>.md`
overrides and each target's `playbooks:` overlay file (path *and* content,
so repointing an already-trusted path at different content still voids
trust). Gate never executes a playbook itself, but it's what gate *tells the
agent* to do, and an attacker who can silently rewrite that instruction has
the same effective control as one who can rewrite a command. While the hash
is stale or absent, an override/overlay is refused - not applied - and
resolution falls back to the bundled or embedded default with a banner
naming what was refused and pointing at `gate trust`; nothing is a hard
error. `gate trust` (and `--check`) list which playbook paths are covered
alongside the commands hash, the same way it's always reported what it's
approving. A target's `playbooks:` path is additionally confined to the
project root - see [Runs and concurrency: targets](runs-and-concurrency.md#targets-multi-stack-repos)
- independent of trust coverage: an escaping path is refused at config-load
time, before trust is even consulted.

Trust covering an override is not the same as gate having *installed* it.
`gate playbook` also reports each `.gate/playbooks/<phase>.md` copy's
provenance against `.gate/playbooks.lock` (the record `gate init`/`gate
update` write when they materialize a copy):

- no entry at all - the copy was never materialized by gate, so it could be
  entirely hand-authored - prints a loud **UNVERIFIED PLAYBOOK** note saying
  so, even though a trusted override still takes effect.
- an entry whose hash no longer matches the copy's content - a tracked copy
  a human edited after materializing it - prints a quieter advisory note;
  customized playbooks are a supported feature.
- an entry whose hash matches - pristine - no note.

Both notes point at `gate doctor` to compare the copy against the bundled
default before trusting instructions unique to it.

Approval is deliberately separate from writing the plan: `gate approve` records
who signed off and the plan's content hash in `run.json`, so the author can't
self-approve by flipping a flag, and editing the plan afterward voids it. A CLI
can't prove a *human* ran `approve` - the mechanical guarantee is that approval
is a distinct, hash-bound act. See
[Review and report: approved-plan drift and `gate amend`](review-and-report.md#approved-plan-drift-and-gate-amend)
for what happens when the plan changes after approval.
