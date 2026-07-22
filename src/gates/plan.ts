import { runPaths } from "../core/paths.js";
import { parsePlanFile } from "../artifacts/plan.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * PLAN gate — all deterministic:
 *  - plan.md exists and parses against the schema
 *  - ≥1 acceptance criterion, each with a declared verification method
 *  - approval flag set (`approved: true`)
 *
 * Plan *quality* (are these the right criteria?) is judgment and lives in the
 * playbook, not here.
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
  checks.push(
    plan.approved
      ? pass("plan.approved", "plan approved")
      : fail(
          "plan.approved",
          "plan not approved — set `approved: true` after human/agent-of-record sign-off",
        ),
  );
  return result("PLAN", checks);
}
