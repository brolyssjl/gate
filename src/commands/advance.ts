import { clearCurrentRunId } from "../core/current.js";
import { nowIso, writeRun, type Run } from "../core/run.js";
import { isTerminal, nextPhase, type Phase } from "../core/stateMachine.js";

/**
 * Advance a run out of its current phase and persist it. Shared by `gate next`
 * (on a passing gate) and `gate skip` (on a human override) so the terminal /
 * current-run bookkeeping lives in one place.
 */
export function advance(root: string, run: Run): { from: Phase; to: Phase } {
  const from = run.phase;
  const to = nextPhase(from);
  if (to) {
    run.phase = to;
    run.history.push({ phase: to, event: "entered", at: nowIso() });
    if (isTerminal(to)) run.status = "done";
  }
  writeRun(root, run);
  if (isTerminal(run.phase)) clearCurrentRunId(root);
  return { from, to: run.phase };
}
