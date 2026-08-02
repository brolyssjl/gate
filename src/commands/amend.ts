import { existsSync, readFileSync } from "node:fs";
import { runPaths } from "../core/paths.js";
import { diffNoIndex } from "../core/git.js";
import { hashPlanFile, parsePlanFile } from "../artifacts/plan.js";
import { nowIso, writeRun } from "../core/run.js";
import { emit, GateError, requireActiveRun, type ParsedArgs } from "./shared.js";

/**
 * `gate amend` - show the diff between the current plan.md and the last
 * approved snapshot, and record intent to re-approve it (Milestone 5). Does
 * not itself re-approve - that's `gate approve --amend` - so a delta
 * re-approval always happens after a human has actually seen the diff, the
 * same discipline `gate approve` already applies to the initial sign-off
 * (writing the plan and approving it are two separate, deliberate steps).
 */
export function cmdAmend(args: ParsedArgs): void {
  const { root, run } = requireActiveRun(args);
  if (!run.approval) {
    throw new GateError("plan was never approved - nothing to amend; run `gate approve` first");
  }

  const { plan: planPath, planApproved: approvedPath } = runPaths(root, run.id);
  const { plan, errors } = parsePlanFile(planPath);
  if (!plan) {
    throw new GateError(`cannot amend an invalid plan: ${errors.join("; ")}`);
  }
  const currentHash = hashPlanFile(planPath);
  if (!currentHash) throw new GateError("plan.md not found");
  if (currentHash === run.approval.planHash) {
    throw new GateError("plan.md matches the approved version - nothing to amend");
  }

  const diff = existsSync(approvedPath)
    ? diffNoIndex(approvedPath, planPath)
    : `(no approved snapshot on disk - showing the full current plan)\n\n${readFileSync(planPath, "utf8")}`;

  const by =
    (typeof args.flags.by === "string" ? args.flags.by : undefined) ??
    process.env.GATE_SESSION_ID ??
    null;

  run.amendment = { planHash: currentHash, at: nowIso(), by };
  writeRun(root, run);

  const human = [
    `Amendment recorded${by ? ` by ${by}` : ""}. Diff vs the approved plan:`,
    "",
    diff.trim() || "(no textual diff produced)",
    "",
    "Run `gate approve --amend` to re-approve this delta.",
  ].join("\n");
  emit(human, { amended: true, diff, ...run.amendment }, args.flags);
}
