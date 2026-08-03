import { existsSync } from "node:fs";
import { runPaths } from "../core/paths.js";
import { changedFiles, isGateBookkeeping, treeFingerprint } from "../core/git.js";
import { runCommand } from "../core/exec.js";
import { isCommandsTrusted } from "../core/trust.js";
import { checkName, resolvePhaseTargets } from "../core/targets.js";
import { parseReviewFile, blockingFindings, unjustifiedWaivers, type Review } from "../artifacts/review.js";
import { planDriftCheck } from "./planDrift.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * REVIEW gate - deterministic checks over a fresh, self-contained review:
 *  - a review packet was emitted and matches the *current* code (tree
 *    fingerprint) - a reviewer must have seen what actually ships
 *  - review.md parses and records who reviewed; no blocker/major finding is
 *    left open, and any waived blocker/major carries a waiver rationale
 *  - the reviewer differs from the implementer, when both are known
 *  - the build/lint/test evidence is not stale: if the tree changed since the
 *    last gate passed (the review-fix loop), Gate re-runs those commands here
 *
 * Whether the review was *thorough* is judgment and lives in the REVIEW
 * playbook; the gate only checks that a distinct reviewer signed off on the
 * final code with no load-bearing findings left hanging.
 */
export function reviewGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];
  const drift = planDriftCheck(ctx, "review.plan-drift");
  if (drift) checks.push(drift);
  const { reviewPacket, review: reviewPath } = runPaths(ctx.root, ctx.run.id);
  const current = treeFingerprint(ctx.root);

  checks.push(packetCheck(ctx, reviewPacket, current));

  const { review, errors } = parseReviewFile(reviewPath);
  if (!review) {
    checks.push(fail("review.findings", `review.md invalid: ${errors.join("; ")}`));
  } else {
    checks.push(findingsCheck(review));
    checks.push(reviewerCheck(ctx, review));
  }

  const touched = changedFiles(ctx.root, ctx.run.baseRef).filter((f) => !isGateBookkeeping(ctx.root, f));
  checks.push(...evidenceChecks(ctx, current, touched));

  return result("REVIEW", checks);
}

/**
 * The packet must exist and have been generated from the code as it stands
 * now; otherwise the reviewer signed off on a different diff. Without git the
 * fingerprint is unavailable - existence is all we can check, and we say so.
 */
function packetCheck(ctx: GateContext, packetPath: string, current: string | null): Check {
  if (!ctx.run.review || !existsSync(packetPath)) {
    return fail("review.packet", "no review packet - run `gate review --fresh` to emit one");
  }
  if (current === null) {
    return pass("review.packet", "packet emitted (not a git repo - freshness unverifiable)");
  }
  if (ctx.run.review.treeHash !== current) {
    return fail(
      "review.packet",
      "packet is stale - the code changed after it was generated; re-run `gate review --fresh` " +
        "so the reviewer sees the code that ships",
    );
  }
  return pass("review.packet", "review packet matches the current code");
}

function findingsCheck(review: Review): Check {
  const blocking = blockingFindings(review);
  if (blocking.length > 0) {
    return fail(
      "review.findings",
      `unresolved blocker/major findings: ${blocking.map((f) => `${f.id} (${f.severity})`).join(", ")}`,
    );
  }
  const noWaiver = unjustifiedWaivers(review);
  if (noWaiver.length > 0) {
    return fail(
      "review.findings",
      `waived findings missing a rationale: ${noWaiver.map((f) => f.id).join(", ")} (add a \`waiver:\`)`,
    );
  }
  return pass("review.findings", `${review.findings.length} finding(s); none blocking`);
}

/**
 * The reviewer signs review.md (`reviewer:` in its frontmatter) - identity is
 * claimed at sign-off time, not when the packet was emitted. A missing name
 * fails: it is the cheapest mechanical proof that *someone* went through the
 * rubric, and without it `gate review --fresh && gate next` would pass on the
 * untouched scaffold. Equal to the implementer's session id fails (self-review);
 * an unknown implementer passes with a note - a CLI cannot prove a human.
 */
function reviewerCheck(ctx: GateContext, review: Review): Check {
  if (!review.reviewer) {
    return fail(
      "review.reviewer",
      "review.md does not say who reviewed - the reviewer must fill in `reviewer:` when signing off",
    );
  }
  const implementer = ctx.run.sessionId;
  if (implementer && review.reviewer === implementer) {
    return fail(
      "review.reviewer",
      `reviewer (${review.reviewer}) is the implementer - get a fresh pair of eyes`,
    );
  }
  return pass(
    "review.reviewer",
    implementer
      ? `reviewed by ${review.reviewer} (≠ implementer)`
      : `reviewed by ${review.reviewer} (implementer identity unknown - independence unverified)`,
  );
}

/**
 * Staleness guard for the review-fix loop: fixing a finding changes the code
 * *after* IMPLEMENT/TEST certified it, and DONE is one `gate next` away. When
 * the current tree still matches the fingerprint recorded at the last gate
 * pass, the earlier evidence stands; otherwise Gate re-runs build/lint/test
 * right here and requires them green. Fails closed - untrusted commands are
 * never spawned, and a red re-run blocks DONE.
 *
 * Targets (Milestone 3): resolved via `resolvePhaseTargets`, mirroring the
 * IMPLEMENT/TEST gates. Reading only `ctx.config.commands` (the top-level
 * set) let a per-target-only repo - no top-level build/lint/test configured
 * at all - pass this check vacuously, never re-verifying any target's actual
 * commands. Bare/legacy case (no targets configured, or none affected)
 * collapses to a single unbracketed "review.evidence" check, byte-identical
 * to before targets existed.
 */
function evidenceChecks(ctx: GateContext, current: string | null, touched: string[]): Check[] {
  const lastVerified = [...ctx.run.history]
    .reverse()
    .find((h) => h.event === "passed" && h.treeHash)?.treeHash;
  if (current !== null && lastVerified === current) {
    return [pass("review.evidence", "code unchanged since the last gate passed - evidence still valid")];
  }

  const trusted = isCommandsTrusted(ctx.root);
  return resolvePhaseTargets(ctx.config, ctx.run, touched).map((t) => {
    const name = checkName("review.evidence", t.target);
    const { build, lint, test } = t.commands;
    const toRun = Object.entries({ build, lint, test }).filter(([, cmd]) => cmd) as Array<[string, string]>;
    if (toRun.length === 0) {
      return pass(name, "no build/lint/test commands configured - nothing to re-verify");
    }
    if (!trusted) {
      return fail(name, "code changed since the last gate passed and commands are untrusted - run `gate trust`");
    }
    const red: string[] = [];
    for (const [label, cmd] of toRun) {
      const res = runCommand(cmd, ctx.root);
      if (res.code !== 0) red.push(`${label} (exit ${res.code})`);
    }
    return red.length === 0
      ? pass(name, `code changed since the last gate passed - re-verified: ${toRun.map(([l]) => l).join("/")} green`)
      : fail(name, `code changed since the last gate passed and re-verification failed: ${red.join(", ")}`);
  });
}
