import { existsSync } from "node:fs";
import { join } from "node:path";
import { runPaths } from "../core/paths.js";
import { hashPlanFile, parsePlanFile, type Plan } from "../artifacts/plan.js";
import { sddDir } from "../integrations/index.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * PLAN gate — all deterministic:
 *  - plan.md exists and parses against the schema
 *  - ≥1 acceptance criterion, each with a declared verification method
 *  - approved via `gate approve`, bound to the current plan's content hash
 *  - when an SDD directory is detected and not disabled, the plan cites a spec
 *    path under it (advisory otherwise — see `specCheck`)
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
  const spec = specCheck(ctx, plan);
  if (spec) checks.push(spec);
  checks.push(approvalCheck(ctx, planPath));
  return result("PLAN", checks);
}

/**
 * SDD citation (Milestone 3, additive): fires only when an SDD directory is
 * detected on disk and `integrations.sdd` is not explicitly "off" — presence
 * plus opt-out, never a hard dependency on the framework being installed. Not
 * added to the checks at all otherwise, so the JSON schema for a repo with no
 * SDD framework stays byte-identical to before this feature existed.
 */
function specCheck(ctx: GateContext, plan: Plan): Check | null {
  if (ctx.config.integrations.sdd === "off") return null;
  const dir = sddDir(ctx.root);
  if (!dir) return null;

  if (!plan.spec) {
    return fail(
      "plan.spec",
      `SDD detected (${dir}) — cite the spec path in plan.md's \`spec:\` field instead of restating it`,
    );
  }
  const normalized = plan.spec.replace(/^\.\//, "");
  if (normalized !== dir && !normalized.startsWith(dir + "/")) {
    return fail("plan.spec", `spec path "${plan.spec}" is not under the detected SDD dir "${dir}"`);
  }
  if (!existsSync(join(ctx.root, normalized))) {
    return fail("plan.spec", `spec path "${plan.spec}" does not exist`);
  }
  return pass("plan.spec", `cites spec ${plan.spec}`);
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
