import { runPaths } from "../core/paths.js";
import { hashPlanFile } from "../artifacts/plan.js";
import { type Check, type GateContext, fail } from "./types.js";

/**
 * Approved-plan drift (Milestone 5): once a plan is approved, every later
 * gate re-checks plan.md's content hash against the hash `gate approve`
 * recorded, restoring void-on-edit for the whole run - previously this was
 * only enforced up to PLAN itself (`gates/plan.ts`'s own `approvalCheck`),
 * so a post-PLAN edit to plan.md (including scope widening) was accepted
 * with no re-approval and no mechanism to record one. `gate amend` shows the
 * diff and records intent; `gate approve --amend` re-approves the delta.
 *
 * Returns null when there's nothing to drift from - never approved (PLAN's
 * own gate owns that message), or the approved content still matches - so
 * this stays silent (no check line at all) for the overwhelming majority of
 * runs where the plan was never touched again after approval.
 */
export function planDriftCheck(ctx: GateContext, name: string): Check | null {
  const approval = ctx.run.approval;
  if (!approval) return null;
  const currentHash = hashPlanFile(runPaths(ctx.root, ctx.run.id).plan);
  if (currentHash === approval.planHash) return null;
  return fail(
    name,
    "plan.md changed since approval - run `gate amend` to review the diff, then `gate approve --amend` to re-approve",
  );
}
