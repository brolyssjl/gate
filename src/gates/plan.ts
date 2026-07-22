import { runPaths } from "../core/paths.js";
import { hashPlanFile, parsePlanFile } from "../artifacts/plan.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * PLAN gate — all deterministic:
 *  - plan.md exists and parses against the schema
 *  - ≥1 acceptance criterion, each with a declared verification method
 *  - approved via `gate approve`, bound to the current plan's content hash
 *
 * Approval lives in run.json (written by `gate approve`), not in the
 * agent-editable plan.md, so the author can't self-approve; editing the plan
 * after approval voids it. Plan *quality* (are these the right criteria?) is
 * judgment and lives in the playbook, not here.
 */
export function planGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];
  const { plan: planPath } = runPaths(ctx.root, ctx.run.id);
  const { plan, errors } = parsePlanFile(planPath);

  if (!plan) {
    checks.push(fail("plan.schema", `plan.md invalid: ${errors.join("; ")}`));
    return result("PLAN", checks);
  }

  checks.push(pass("plan.schema", "plan.md matches the required schema"));
  checks.push(
    plan.criteria.length >= 1
      ? pass("plan.criteria", `${plan.criteria.length} acceptance criteria`)
      : fail("plan.criteria", "at least one acceptance criterion is required"),
  );
  // Every criterion carries a verify method (enforced by the parser), so
  // "checkable" is already satisfied when the schema is valid; surface it.
  checks.push(
    pass("plan.checkable", "every criterion declares a verification method"),
  );
  checks.push(approvalCheck(ctx, planPath));
  return result("PLAN", checks);
}

function approvalCheck(ctx: GateContext, planPath: string): Check {
  const approval = ctx.run.approval;
  if (!approval) {
    return fail("plan.approved", "plan not approved — get sign-off, then run `gate approve`");
  }
  const currentHash = hashPlanFile(planPath);
  if (currentHash !== approval.planHash) {
    return fail(
      "plan.approved",
      "plan changed since approval — re-run `gate approve` on the current plan",
    );
  }
  return pass("plan.approved", `plan approved${approval.by ? ` by ${approval.by}` : ""}`);
}
