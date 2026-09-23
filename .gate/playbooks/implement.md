# IMPLEMENT playbook

You are in the **IMPLEMENT** phase. Turn the plan into code. Gate checks that you
changed something, stayed in scope, and left the build and lint green.

## Do

1. Work **one acceptance criterion at a time**. Small, coherent commits.
2. **Stay in scope.** Only touch files covered by a `plan.files` glob. If you
   genuinely need another file, go back and amend `plan.md` (that is real
   re-planning, not a workaround) - the gate compares your diff against it.
3. Keep a running `worklog.md` in the run folder: what you did per criterion and
   any decisions worth remembering.
4. Keep the **build and lint passing** as you go; don't leave them for the end.

## Gate checks

- The working diff is non-empty (measured from the run's base commit).
- Every touched file (outside `.gate/`) is declared in `plan.md`.
- `build` command exits 0 (if configured).
- `lint` command exits 0 (if configured).

## Advance

`gate next`. Most profiles enter TEST next - do **not** write tests to game
criteria here, that is TEST's own gate, not this one. A `docs` run has no
TEST phase at all and goes straight to DONE; `gate status` shows the actual
sequence if you're unsure.
