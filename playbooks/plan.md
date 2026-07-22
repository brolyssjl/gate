# PLAN playbook

You are in the **PLAN** phase. Produce `plan.md` in the run folder. Gate will not
let you into IMPLEMENT until the plan is schema-valid and approved. Plan *quality*
is your job; Gate only checks structure.

## Do

1. **Restate the goal** in one sentence — what does "done" mean for this work?
2. If project memory is present (`.agnosgram/`, an SDD spec under `openspec/`,
   `.specify/`, `_bmad/`), read it first and **cite** the spec path in the body
   rather than restating it.
3. **List every file** you expect to touch (globs allowed). IMPLEMENT will reject
   changes to files not covered here, so be honest and complete.
4. **Write acceptance criteria.** Each must be *checkable*: give it a `verify`
   method. Prefer `test: <substring of the test title>` so the TEST gate can map
   it to a real passing test. Use `manual` only when a machine genuinely cannot
   check it.
5. List **risks** and **out-of-scope** items to hold the line against scope creep.
6. Get sign-off, then run `gate approve`. Approval is a separate, deliberate
   step recorded outside plan.md — you cannot self-approve by editing the plan,
   and any later edit to plan.md voids the approval (re-run `gate approve`).

## plan.md schema

```markdown
---
goal: One sentence describing done.
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
---

# Plan: <title>

Free-form detail. Cite the SDD spec path here if one exists.
```

## Advance

`gate check` to see what's missing → `gate approve` (after sign-off) →
`gate next` to enter IMPLEMENT.
