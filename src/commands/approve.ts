import { runPaths } from "../core/paths.js";
import { hashPlanFile, parsePlanFile } from "../artifacts/plan.js";
import { nowIso, writeRun } from "../core/run.js";
import { emit, GateError, requireActiveRun, type ParsedArgs } from "./shared.js";

/**
 * `gate approve` - record PLAN sign-off in run.json, bound to the plan's content
 * hash. Separate from writing plan.md so the author can't self-approve in one
 * step; a later plan edit voids the approval (the PLAN gate re-checks the hash).
 */
export function cmdApprove(args: ParsedArgs): void {
  const { root, run } = requireActiveRun(args);
  if (run.phase !== "PLAN") {
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

  run.approval = { by, at: nowIso(), reason, planHash };
  writeRun(root, run);

  emit(
    `Plan approved${by ? ` by ${by}` : ""}. Run \`gate next\` to enter IMPLEMENT.`,
    { approved: true, ...run.approval },
    args.flags,
  );
}
