# PLAN playbook

You are in the **PLAN** phase. Produce `plan.md` in the run folder. Gate will not
let you into IMPLEMENT until the plan is schema-valid and approved. Plan *quality*
is your job; Gate only checks structure.

## Do

1. **Restate the goal** in one sentence - what does "done" mean for this work?
2. If project memory is present (`.agnosgram/`, an SDD spec under `openspec/`,
   `.specify/`, `_bmad/`), read it first and **cite** the spec path rather than
   restating it. When Gate detects an SDD directory (and `integrations.sdd` is
   not `off`), put that path in the `spec:` frontmatter field - the gate checks
   it points somewhere real under the SDD dir. No SDD present: leave it blank.
3. **List every file** you expect to touch (globs allowed). IMPLEMENT will reject
   changes to files not covered here, so be honest and complete.
4. **Write acceptance criteria.** Each must be *checkable*: give it a `verify`
   method. Prefer `test: <substring of the test title>` so the TEST gate can map
   it to a real passing test. Use `manual` only when a machine genuinely cannot
   check it.
5. List **risks** and **out-of-scope** items to hold the line against scope creep.
6. If this repo has an `.agnosgram/` project-memory store, an `agnosgram
   advise` report over this plan may already exist. Gate reads it (advisory -
   it never runs the check itself); if it flags contradictions, resolve each
   one and list its record id under `acknowledgments:` before approving.
7. Get sign-off, then run `gate approve`. Approval is a separate, deliberate
   step recorded outside plan.md - you cannot self-approve by editing the plan,
   and any later edit to plan.md voids the approval. While still in PLAN,
   re-run `gate approve`. Once you've moved past PLAN, every later gate fails
   on the drift instead: run `gate amend` to see the diff against the approved
   plan and record intent, then `gate approve --amend` to re-approve the delta.

## plan.md schema

```markdown
---
goal: One sentence describing done.
spec: openspec/changes/my-change/spec.md   # only when an SDD dir is detected
files:
  - src/**                 # every path/glob you intend to touch
out_of_scope:
  - unrelated refactors
criteria:
  - id: c1
    text: "User can reset their password from the login screen"
    verify: "test: resets password"   # or: manual
risks:
  - "Token expiry edge cases"
acknowledgments: []           # record ids from an agnosgram advise report you've resolved
---

# Plan: <title>

Free-form detail. Cite the SDD spec path here if one exists.
```

## Advance

`gate check` to see what's missing → `gate approve` (after sign-off) →
`gate next` to enter IMPLEMENT.
