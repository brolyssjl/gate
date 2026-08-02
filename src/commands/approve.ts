import { readFileSync, writeFileSync } from "node:fs";
import { runPaths } from "../core/paths.js";
import { hashPlanFile, parsePlanFile } from "../artifacts/plan.js";
import { nowIso, writeRun } from "../core/run.js";
import { emit, GateError, requireActiveRun, type ParsedArgs } from "./shared.js";

/**
 * `gate approve` - record PLAN sign-off in run.json, bound to the plan's content
 * hash. Separate from writing plan.md so the author can't self-approve in one
 * step; a later plan edit voids the approval (checked at PLAN by `gates/plan.ts`,
 * and by every later gate via `gates/planDrift.ts` - Milestone 5).
 *
 * `--amend` re-approves a plan that drifted after the initial approval: it
 * requires `gate amend` to have already recorded intent on the exact current
 * plan.md (so a human has seen the diff `gate amend` printed), and works at
 * any phase - unlike the initial approval, which only makes sense in PLAN.
 */
export function cmdApprove(args: ParsedArgs): void {
  const { root, run } = requireActiveRun(args);
  const amend = args.flags.amend === true;
  if (!amend && run.phase !== "PLAN") {
    throw new GateError(`nothing to approve - run is in ${run.phase}, not PLAN`);
  }

  const planPath = runPaths(root, run.id).plan;
  const { plan, errors } = parsePlanFile(planPath);
  if (!plan) {
    throw new GateError(`cannot approve an invalid plan: ${errors.join("; ")}`);
  }
  const planHash = hashPlanFile(planPath);
  if (!planHash) throw new GateError("plan.md not found");

  const by =
    (typeof args.flags.by === "string" ? args.flags.by : undefined) ??
    process.env.GATE_SESSION_ID ??
    null;
  const reason = typeof args.flags.reason === "string" ? args.flags.reason : null;

  if (amend) {
    if (!run.approval) {
      throw new GateError("nothing to amend - plan was never approved; run `gate approve` first");
    }
    if (!run.amendment) {
      throw new GateError("no amendment recorded - run `gate amend` first to review the diff and record intent");
    }
    if (run.amendment.planHash !== planHash) {
      throw new GateError("plan.md changed again since `gate amend` - re-run `gate amend` on the current plan");
    }
    run.approval = { by, at: nowIso(), reason, planHash };
    delete run.amendment;
    snapshotApprovedPlan(root, run.id, planPath);
    writeRun(root, run);
    emit(
      `Amendment approved${by ? ` by ${by}` : ""}. Re-run the current gate (\`gate check\`/\`gate next\`) to continue.`,
      { approved: true, amended: true, ...run.approval },
      args.flags,
    );
    return;
  }

  run.approval = { by, at: nowIso(), reason, planHash };
  snapshotApprovedPlan(root, run.id, planPath);
  writeRun(root, run);

  emit(
    `Plan approved${by ? ` by ${by}` : ""}. Run \`gate next\` to enter IMPLEMENT.`,
    { approved: true, ...run.approval },
    args.flags,
  );
}

/** Snapshot plan.md as of this approval/amendment - `gate amend` diffs the current plan against it. */
function snapshotApprovedPlan(root: string, runId: string, planPath: string): void {
  writeFileSync(runPaths(root, runId).planApproved, readFileSync(planPath, "utf8"));
}
