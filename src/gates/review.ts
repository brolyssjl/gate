import { existsSync } from "node:fs";
import { runPaths } from "../core/paths.js";
import { parseReviewFile, blockingFindings, unjustifiedWaivers } from "../artifacts/review.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * REVIEW gate — deterministic checks over a fresh, self-contained review:
 *  - a review packet was emitted (`gate review --fresh`)
 *  - review.md parses; no blocker/major finding is left open, and any waived
 *    blocker/major carries a human waiver rationale
 *  - the reviewer session id differs from the implementer's, when both are known
 *
 * Whether the review was *thorough* is judgment and lives in the REVIEW
 * playbook; the gate only checks that a distinct reviewer signed off with no
 * load-bearing findings left hanging.
 */
export function reviewGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];
  const { reviewPacket, review: reviewPath } = runPaths(ctx.root, ctx.run.id);

  checks.push(
    ctx.run.review && existsSync(reviewPacket)
      ? pass("review.packet", "review packet emitted via `gate review --fresh`")
      : fail("review.packet", "no review packet — run `gate review --fresh` to emit one"),
  );

  const { review, errors } = parseReviewFile(reviewPath);
  if (!review) {
    checks.push(fail("review.findings", `review.md invalid: ${errors.join("; ")}`));
  } else {
    const blocking = blockingFindings(review);
    const noWaiver = unjustifiedWaivers(review);
    if (blocking.length > 0) {
      checks.push(
        fail(
          "review.findings",
          `unresolved blocker/major findings: ${blocking.map((f) => `${f.id} (${f.severity})`).join(", ")}`,
        ),
      );
    } else if (noWaiver.length > 0) {
      checks.push(
        fail(
          "review.findings",
          `waived findings missing a rationale: ${noWaiver.map((f) => f.id).join(", ")} (add a \`waiver:\`)`,
        ),
      );
    } else {
      checks.push(pass("review.findings", `${review.findings.length} finding(s); none blocking`));
    }
  }

  checks.push(independenceCheck(ctx));

  return result("REVIEW", checks);
}

/**
 * A reviewer should be a fresh pair of eyes. When both identities are known and
 * equal, the review is self-review and fails; when the reviewer is unknown we
 * cannot enforce it (a CLI can't prove a human), so it passes with a note.
 */
function independenceCheck(ctx: GateContext): Check {
  const reviewer = ctx.run.review?.reviewer ?? null;
  const implementer = ctx.run.sessionId;
  if (!reviewer) {
    return pass("review.independence", "reviewer identity not provided — independence unverified");
  }
  if (implementer && reviewer === implementer) {
    return fail("review.independence", `reviewer (${reviewer}) is the implementer — get a fresh reviewer`);
  }
  return pass("review.independence", `reviewed by ${reviewer} (≠ implementer)`);
}
