import { existsSync } from "node:fs";
import { RETRO_TEMPLATE } from "../artifacts/retro.js";
import { clearCurrentRunId } from "../core/current.js";
import { writeFileAtomic } from "../core/fsx.js";
import { runPaths } from "../core/paths.js";
import { nowIso, writeRun, type Run } from "../core/run.js";
import { isTerminal, nextPhase, type Phase } from "../core/stateMachine.js";

/**
 * Advance a run out of its current phase and persist it. Shared by `gate next`
 * (on a passing gate) and `gate skip` (on a human override) so the terminal /
 * current-run bookkeeping lives in one place.
 */
export function advance(root: string, run: Run): { from: Phase; to: Phase } {
  const from = run.phase;
  const to = nextPhase(from, run.profile);
  if (to) {
    run.phase = to;
    run.history.push({ phase: to, event: "entered", at: nowIso() });
    if (isTerminal(to)) run.status = "done";
    if (to === "RETRO") scaffoldRetroIfMissing(root, run);
  }
  writeRun(root, run);
  if (isTerminal(run.phase)) clearCurrentRunId(root);
  return { from, to: run.phase };
}

/**
 * `gate start` scaffolds retro.md up front for any run whose profile walks
 * through RETRO - but a run started before that existed (pre-Milestone-3)
 * would enter RETRO with no retro.md and no way to satisfy the gate. Repair
 * it here, on the transition itself, so no run can stall permanently.
 */
function scaffoldRetroIfMissing(root: string, run: Run): void {
  const { retro } = runPaths(root, run.id);
  if (!existsSync(retro)) writeFileAtomic(retro, RETRO_TEMPLATE.replace("%TITLE%", run.title));
}
