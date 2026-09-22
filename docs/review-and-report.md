# Review, report, and retro

## Review & report

`gate review` writes `review-packet.md` (plan + rubric + the full diff,
**including untracked files** - nothing in the flow requires committing) so a
reviewer needs no prior context, and scaffolds `review.md` for their findings.
The packet records a fingerprint of the tree it was generated from; the REVIEW
gate refuses to pass while the code differs from it, so a reviewer always signs
off on the code that ships. `--fresh` regenerates an existing packet - required
after any review fix; without it an existing packet is never silently
re-baselined.

The reviewer signs off by filling in `reviewer:` in `review.md` - identity is
claimed at sign-off, not at packet time, and the gate rejects an anonymous
review and a reviewer equal to the implementer's session id, *when a session
id was recorded for the run*. Most CLI-driven flows never set one (it only
exists if the calling harness passed `gate start --session`/
`GATE_SESSION_ID`), so in practice this check is advisory: `review.reviewer`
still passes, but with a distinct warning marker and an "independence
unverified" detail rather than the plain checkmark a genuinely confirmed
pass gets.

Fixes made during review change the code *after* IMPLEMENT/TEST certified it,
so when the tree no longer matches the fingerprint recorded at the last gate
pass, the REVIEW gate re-runs `build`/`lint`/`test` itself and requires them
green before DONE.

Solo, with no second agent session or human reviewer handy: `gate review
--human` walks the same rubric as terminal prompts instead of handing the
packet off. It emits the packet, prints the rubric, then asks for each
finding (id, severity, note, status, waiver if waived) and a reviewer name,
and writes `review.md` in the exact shape the REVIEW gate parses - no
special-casing, so a human-recorded review satisfies the same gate as an
agent-recorded one.

`gate report [<run-id>]` prints a per-run summary - phase durations, failed gate
attempts, findings, and any skips (with who and why) - defaulting to the active
or most recent run. `--json` gives the machine-readable form. It falls back to
an archived summary (see "Pruning runs" in
[Runs and concurrency](runs-and-concurrency.md)) once a run's live folder is
gone.

## Retro and the Agnosgram journal

RETRO closes every profile except `docs`: answer at least one of `broke`,
`avoid`, `conventions` in `retro.md`. If this repo has an
[Agnosgram](https://github.com/brolyssjl/agnosgram) `.agnosgram/` store,
`gate retro` syncs the answers into its journal, source-linked to the run:

```
## 2026-07-27 14:05 · gate · milestone-3-ecosystem
- **Did:** Completed gate run "add password reset" (2026-07-27-add-password-reset, profile feature)
- **Learned:** <broke entries, joined "; ">
- **Decided:** <conventions entries>
- **Avoid:** <avoid entries>
- **Source:** .gate/runs/2026-07-27-add-password-reset
```

It spawns `agnosgram log --stdin --agent gate`; if the binary isn't installed
(ENOENT), it falls back to appending the same entry directly to
`.agnosgram/journal/<month>.md`. Re-running is idempotent - it checks the
recorded journal file already contains the run id before syncing again. With
no `.agnosgram/` store, `gate retro` is a no-op and the RETRO gate's journal
check passes with a note; nothing about RETRO depends on Agnosgram being
installed (advisory, never load-bearing).

The PLAN gate is the read side of the same integration: with an `.agnosgram/`
store present, if `agnosgram advise` has written `<plan>.advise.json`
(the pinned `agnosgram_advise` report schema), every flagged contradiction
must be listed under plan.md's `acknowledgments:` before PLAN passes. No
report on disk yet is advisory-only - Gate never runs `agnosgram advise`
itself.

## Approved-plan drift and `gate amend`

Void-on-edit holds for the *whole run*, not just PLAN: every later gate
(DEBUG/IMPLEMENT/TEST/REVIEW/RETRO) re-checks plan.md's hash against the one
`gate approve` recorded, and fails closed on a mismatch - including a
post-approval scope widening that used to sail through with no re-approval.

```bash
gate amend             # print the diff vs the approved plan.md, record intent
gate approve --amend   # re-approve the delta (requires `gate amend` first)
```

`gate amend` requires a prior approval and an actual drift to show; it never
re-approves by itself - that split means a delta re-approval always happens
after a human has actually seen the diff `gate amend` printed, the same
discipline the initial `gate approve` already applies. `gate approve --amend`
refuses if the plan changed again after `gate amend` ran (re-run `gate amend`
on the current plan first) and works at any phase, unlike the initial
approval.
