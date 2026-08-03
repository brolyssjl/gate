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

  const { diff, trustedSnapshot, warning } = renderDiff(approvedPath, planPath, run.approval.planHash);

  const by =
    (typeof args.flags.by === "string" ? args.flags.by : undefined) ??
    process.env.GATE_SESSION_ID ??
    null;

  run.amendment = { planHash: currentHash, at: nowIso(), by };
  writeRun(root, run);

  const human = [
    `Amendment recorded${by ? ` by ${by}` : ""}.`,
    warning ?? "Diff vs the approved plan:",
    "",
    diff.trim() || "(no textual diff produced)",
    "",
    "Run `gate approve --amend` to re-approve this delta.",
  ].join("\n");
  emit(human, { amended: true, diff, trustedSnapshot, warning, ...run.amendment }, args.flags);
}

/**
 * The diff `gate amend` shows, only ever computed from a snapshot proven to
 * be the plan `gate approve` actually recorded (review finding F2): trusting
 * `plan.approved.md` on its face would let a doctored snapshot produce an
 * empty or misleading diff - e.g. a snapshot rewritten to match the
 * *current* plan.md would show "no changes" for a plan that in fact drifted
 * far from what was approved, and a human re-approving on that diff approves
 * blind. The snapshot is trustworthy only when its own hash matches
 * `approval.planHash` - the same value `gate approve` bound at sign-off, and
 * the same one the drift check (`gates/planDrift.ts`) already compares
 * against. Anything else (missing, unreadable, or hash-mismatched) falls
 * back to showing the full current plan with a loud warning, never a diff
 * computed against content that can't be trusted.
 */
function renderDiff(
  approvedPath: string,
  planPath: string,
  approvedHash: string,
): { diff: string; trustedSnapshot: boolean; warning: string | null } {
  if (!existsSync(approvedPath)) {
    return {
      diff: `(no approved snapshot on disk - showing the full current plan)\n\n${readFileSync(planPath, "utf8")}`,
      trustedSnapshot: false,
      warning: null,
    };
  }
  const snapshotHash = hashPlanFile(approvedPath);
  if (snapshotHash !== approvedHash) {
    return {
      diff: readFileSync(planPath, "utf8"),
      trustedSnapshot: false,
      warning:
        "WARNING: the approved-plan snapshot on disk does not match the recorded approval hash " +
        "(edited or corrupted since `gate approve` wrote it) - showing the full current plan " +
        "instead of an untrustworthy diff. Review it in full before re-approving.",
    };
  }
  return { diff: diffNoIndex(approvedPath, planPath), trustedSnapshot: true, warning: null };
}
