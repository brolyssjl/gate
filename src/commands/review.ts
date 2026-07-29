import { existsSync, readFileSync } from "node:fs";
import { runPaths } from "../core/paths.js";
import { writeFileAtomic } from "../core/fsx.js";
import { diffText, treeFingerprint } from "../core/git.js";
import { resolvePlaybookWithOverlays } from "../core/playbooks.js";
import { nowIso, writeRun } from "../core/run.js";
import { resolveDisplayTargets } from "../core/targets.js";
import { emit, GateError, requireActiveRun, type ParsedArgs } from "./shared.js";

const REVIEW_TEMPLATE = `---
reviewer:
findings: []
# The reviewer fills in \`reviewer:\` when signing off - the gate refuses an
# anonymous review. Each finding: { id, severity: blocker|major|minor|nit,
#   status: open|resolved|waived, note: "what's wrong",
#   waiver: "why it's acceptable" (required when waived) }
---

# Review

Record findings in the frontmatter above. The gate blocks on any open
blocker/major finding; minor/nit are advisory. Resolve by fixing the code
(Gate re-verifies build/lint/test and needs a fresh packet after any fix), or
waive with a human rationale.
`;

/**
 * `gate review` - emit a self-contained review packet (diff + plan + rubric)
 * so a *fresh* reviewer needs no prior context, and scaffold review.md for
 * their findings. The packet records the tree fingerprint it was generated
 * from; the REVIEW gate refuses to pass while the code differs from it.
 * `--fresh` regenerates an existing packet from the current code - required
 * after any review fix. Without it an existing packet is left untouched so an
 * accidental re-run cannot silently re-baseline what the reviewer saw.
 */
export function cmdReview(args: ParsedArgs): void {
  const { root, run, config } = requireActiveRun(args);
  if (run.phase !== "REVIEW") {
    throw new GateError(`nothing to review - run is in ${run.phase}, not REVIEW`);
  }

  const paths = runPaths(root, run.id);
  const fresh = args.flags.fresh === true;

  if (!existsSync(paths.review)) writeFileAtomic(paths.review, REVIEW_TEMPLATE);

  if (existsSync(paths.reviewPacket) && !fresh) {
    const human = [
      `A review packet already exists: ${paths.reviewPacket}`,
      "Pass --fresh to regenerate it from the current code (required after any fix).",
    ].join("\n");
    emit(
      human,
      { phase: run.phase, packet: paths.reviewPacket, findingsFile: paths.review, regenerated: false },
      args.flags,
    );
    return;
  }

  const plan = existsSync(paths.plan) ? readFileSync(paths.plan, "utf8").trim() : "(no plan.md)";
  const diff = diffText(root, run.baseRef).trim() || "(no diff)";
  const targetNames = resolveDisplayTargets(root, run, config);
  const rubric = resolvePlaybookWithOverlays(root, "REVIEW", config, targetNames) ?? "(no REVIEW playbook found)";
  const treeHash = treeFingerprint(root);

  const packet = [
    `# Review packet - ${run.id}`,
    `\nTitle: ${run.title}`,
    `Base: ${run.baseRef ?? "(no git base)"}`,
    `Tree: ${treeHash ?? "(no git tree)"}`,
    `Generated: ${nowIso()}`,
    `\n## Plan\n\n${plan}`,
    `\n## Rubric\n\n${rubric.trim()}`,
    `\n## Diff\n\n\`\`\`diff\n${diff}\n\`\`\``,
    `\nRecord findings in: ${paths.review}`,
    "",
  ].join("\n");
  writeFileAtomic(paths.reviewPacket, packet);

  const requestedBy =
    (typeof args.flags.by === "string" ? args.flags.by : undefined) ??
    process.env.GATE_SESSION_ID ??
    null;
  run.review = { requestedBy, requestedAt: nowIso(), treeHash };
  run.artifacts["review-packet.md"] = { phase: run.phase, at: nowIso() };
  writeRun(root, run);

  const human = [
    `Review packet written to: ${paths.reviewPacket}`,
    `Record findings in:        ${paths.review}`,
    "",
    "The reviewer signs off by filling in `reviewer:` in review.md. Resolve or",
    "waive every blocker/major finding, then run `gate next`.",
    "",
    packet,
  ].join("\n");

  emit(
    human,
    {
      phase: run.phase,
      packet: paths.reviewPacket,
      findingsFile: paths.review,
      regenerated: true,
      requestedBy,
      treeHash,
      plan,
      diff,
      rubric,
    },
    args.flags,
  );
}
