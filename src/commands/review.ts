import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { runPaths } from "../core/paths.js";
import { diffText } from "../core/git.js";
import { resolvePlaybook } from "../core/playbooks.js";
import { nowIso, writeRun } from "../core/run.js";
import { emit, GateError, requireActiveRun, type ParsedArgs } from "./shared.js";

const REVIEW_TEMPLATE = `---
reviewer:
findings: []
# Each finding: { id, severity: blocker|major|minor|nit, status: open|resolved|waived,
#   note: "what's wrong", waiver: "why it's acceptable" (required when waived) }
---

# Review

Record findings in the frontmatter above. The gate blocks on any open
blocker/major finding; minor/nit are advisory. Resolve by fixing the code, or
waive with a human rationale.
`;

/**
 * `gate review --fresh` — emit a self-contained review packet (diff + plan +
 * rubric) so a *fresh* reviewer needs no prior context, and scaffold review.md
 * for their findings. Records the reviewer's identity so the gate can check it
 * differs from the implementer's. `--fresh` regenerates the packet from the
 * current diff; it is the canonical invocation.
 */
export function cmdReview(args: ParsedArgs): void {
  const { root, run } = requireActiveRun();
  if (run.phase !== "REVIEW") {
    throw new GateError(`nothing to review — run is in ${run.phase}, not REVIEW`);
  }

  const paths = runPaths(root, run.id);
  const plan = existsSync(paths.plan) ? readFileSync(paths.plan, "utf8").trim() : "(no plan.md)";
  const diff = diffText(root, run.baseRef).trim() || "(no diff)";
  const rubric = resolvePlaybook(root, "REVIEW") ?? "(no REVIEW playbook found)";

  const packet = [
    `# Review packet — ${run.id}`,
    `\nTitle: ${run.title}`,
    `Base: ${run.baseRef ?? "(no git base)"}`,
    `Generated: ${nowIso()}`,
    `\n## Plan\n\n${plan}`,
    `\n## Rubric\n\n${rubric.trim()}`,
    `\n## Diff\n\n\`\`\`diff\n${diff}\n\`\`\``,
    `\nRecord findings in: ${paths.review}`,
    "",
  ].join("\n");
  writeFileSync(paths.reviewPacket, packet);

  if (!existsSync(paths.review)) writeFileSync(paths.review, REVIEW_TEMPLATE);

  const reviewer =
    (typeof args.flags.by === "string" ? args.flags.by : undefined) ??
    process.env.GATE_SESSION_ID ??
    null;
  run.review = { reviewer, requestedAt: nowIso() };
  run.artifacts["review-packet.md"] = { phase: run.phase, at: nowIso() };
  writeRun(root, run);

  const human = [
    `Review packet written to: ${paths.reviewPacket}`,
    `Record findings in:        ${paths.review}`,
    reviewer ? `Reviewer:                  ${reviewer}` : "Reviewer:                  (not provided)",
    "",
    "Resolve or waive every blocker/major finding, then run `gate next`.",
    "",
    packet,
  ].join("\n");

  emit(
    human,
    {
      phase: run.phase,
      packet: paths.reviewPacket,
      findingsFile: paths.review,
      reviewer,
      plan,
      diff,
      rubric,
    },
    args.flags,
  );
}
