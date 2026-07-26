# REVIEW playbook

You are in the **REVIEW** phase. A *fresh* reviewer — ideally a different agent
session or a human, not the implementer — reads the change cold and judges it
against this rubric. Gate emits a self-contained packet so the reviewer needs no
prior context, and refuses to advance while any blocker/major finding is open.

## For the implementer

Run `gate review --fresh`. It writes `review-packet.md` (plan + rubric + the
full diff, untracked files included) and scaffolds `review.md` for findings.
The packet is fingerprint-bound to the code it was generated from: if you
change anything afterwards - fixing a finding counts - the gate blocks until
you re-run `gate review --fresh` so the reviewer sees the final code, and Gate
re-verifies build/lint/test on its own. Hand the packet to a reviewer who is
not you; the gate rejects a reviewer equal to your session id when both are
known.

## For the reviewer — the rubric

Read the packet end to end, then judge:

1. **Correctness** — does it do what the plan says? Edge cases, error paths,
   concurrency, off-by-ones. Is every acceptance criterion actually met?
2. **Scope** — does the diff match the plan, with nothing snuck in or left out?
3. **Tests** — do the tests prove the behavior, or just exercise it? Would they
   fail if the code were wrong?
4. **Clarity & maintainability** — names, structure, comments where the *why* is
   non-obvious. Would the next person understand this?
5. **Safety** — secrets, injection, unsafe defaults, data loss.

Record each issue as a finding. Be specific and actionable. Sign your review by
filling in `reviewer:` in `review.md` - the gate refuses an anonymous review.

## review.md schema

```markdown
---
reviewer: "session-or-name"
findings:
  - id: f1
    severity: blocker   # blocker | major | minor | nit
    status: open        # open | resolved | waived
    note: "resetToken is compared with == not a constant-time check"
  - id: f2
    severity: minor
    status: open
    note: "Extract the retry loop; it's duplicated"
---

# Review
```

Severity: **blocker** = must not ship; **major** = must fix before done;
**minor**/**nit** = advisory. To clear a blocker/major, fix the code and set
`status: resolved`, or set `status: waived` with a `waiver:` explaining why it's
acceptable (a human decision, recorded for the audit trail).

## Gate checks

- A review packet was emitted and **matches the current code** (tree
  fingerprint) - any edit after the packet requires `gate review --fresh`.
- `review.md` parses and names its `reviewer:`; no blocker/major finding is
  left `open`; any waived blocker/major carries a `waiver:` rationale.
- The reviewer differs from the implementer's session id, when both are known.
- If the code changed since the last gate passed (review fixes), Gate re-runs
  `build`/`lint`/`test` and they must be green.

## Advance

`gate next` once every blocker/major is resolved or waived → DONE. After any
fix: re-run `gate review --fresh` first, and expect the gate to re-verify the
commands.
