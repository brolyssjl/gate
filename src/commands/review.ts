import { existsSync, readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { runPaths } from "../core/paths.js";
import { writeFileAtomic } from "../core/fsx.js";
import { diffText, treeFingerprint } from "../core/git.js";
import { resolvePlaybookWithOverlays } from "../core/playbooks.js";
import { nowIso, writeRun, type Run } from "../core/run.js";
import type { GateConfig } from "../core/config.js";
import { resolveDisplayTargets } from "../core/targets.js";
import { parseReviewFile, serializeReview, SEVERITIES, STATUSES, type Finding } from "../artifacts/review.js";
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

interface PacketResult {
  regenerated: boolean;
  packet: string;
  findingsFile: string;
  rubric: string;
  requestedBy?: string | null;
  treeHash?: string | null;
  plan?: string;
  diff?: string;
}

/**
 * Emit the self-contained review packet (diff + plan + rubric) and scaffold
 * review.md, unless one already exists and `--fresh` wasn't passed - shared
 * by the default (agent-facing) path and `--human`, so both see exactly the
 * same packet-freshness rules.
 */
function ensurePacket(root: string, run: Run, config: GateConfig, args: ParsedArgs): PacketResult {
  const paths = runPaths(root, run.id);
  const fresh = args.flags.fresh === true;

  if (!existsSync(paths.review)) writeFileAtomic(paths.review, REVIEW_TEMPLATE);

  const targetNames = resolveDisplayTargets(root, run, config);
  const rubric = resolvePlaybookWithOverlays(root, "REVIEW", config, targetNames) ?? "(no REVIEW playbook found)";

  if (existsSync(paths.reviewPacket) && !fresh) {
    return { regenerated: false, packet: paths.reviewPacket, findingsFile: paths.review, rubric };
  }

  const plan = existsSync(paths.plan) ? readFileSync(paths.plan, "utf8").trim() : "(no plan.md)";
  const diff = diffText(root, run.baseRef).trim() || "(no diff)";
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
    (typeof args.flags.by === "string" ? args.flags.by : undefined) ?? process.env.GATE_SESSION_ID ?? null;
  run.review = { requestedBy, requestedAt: nowIso(), treeHash };
  run.artifacts["review-packet.md"] = { phase: run.phase, at: nowIso() };
  writeRun(root, run);

  return { regenerated: true, packet: paths.reviewPacket, findingsFile: paths.review, rubric, requestedBy, treeHash, plan, diff };
}

/**
 * `gate review [--fresh] [--by]` - emit a self-contained review packet (diff +
 * plan + rubric) so a *fresh* reviewer needs no prior context, and scaffold
 * review.md for their findings. The packet records the tree fingerprint it
 * was generated from; the REVIEW gate refuses to pass while the code differs
 * from it. `--fresh` regenerates an existing packet from the current code -
 * required after any review fix. Without it an existing packet is left
 * untouched so an accidental re-run cannot silently re-baseline what the
 * reviewer saw. `--human` (Milestone 4) walks the rubric as terminal prompts
 * instead, for solo devs with no second agent session to hand the packet to.
 */
export function cmdReview(args: ParsedArgs): void | Promise<void> {
  const { root, run, config } = requireActiveRun(args);
  if (run.phase !== "REVIEW") {
    throw new GateError(`nothing to review - run is in ${run.phase}, not REVIEW`);
  }

  if (args.flags.human === true) {
    return cmdReviewHuman(root, run, config, args);
  }

  const res = ensurePacket(root, run, config, args);

  if (!res.regenerated) {
    const human = [
      `A review packet already exists: ${res.packet}`,
      "Pass --fresh to regenerate it from the current code (required after any fix).",
    ].join("\n");
    emit(human, { phase: run.phase, packet: res.packet, findingsFile: res.findingsFile, regenerated: false }, args.flags);
    return;
  }

  const packetText = readFileSync(res.packet, "utf8");
  const human = [
    `Review packet written to: ${res.packet}`,
    `Record findings in:        ${res.findingsFile}`,
    "",
    "The reviewer signs off by filling in `reviewer:` in review.md. Resolve or",
    "waive every blocker/major finding, then run `gate next`.",
    "",
    packetText,
  ].join("\n");

  emit(
    human,
    {
      phase: run.phase,
      packet: res.packet,
      findingsFile: res.findingsFile,
      regenerated: true,
      requestedBy: res.requestedBy,
      treeHash: res.treeHash,
      plan: res.plan,
      diff: res.diff,
      rubric: res.rubric,
    },
    args.flags,
  );
}

/**
 * `gate review --human` (Milestone 4): a minimal terminal rubric walk for
 * solo devs with no second agent/human session to hand the packet to. Zero
 * new dependencies - `node:readline/promises` is a Node builtin. Emits the
 * same packet the agent-facing path does, then prompts for findings and a
 * reviewer identity, and writes review.md in the exact shape the REVIEW gate
 * parses - a human-recorded review satisfies the same gate as an agent one,
 * no special-casing.
 */
async function cmdReviewHuman(root: string, run: Run, config: GateConfig, args: ParsedArgs): Promise<void> {
  const res = ensurePacket(root, run, config, args);

  process.stdout.write(`\nReview packet: ${res.packet}\n`);
  process.stdout.write(res.regenerated ? "(freshly generated)\n" : "(reusing an existing packet - pass --fresh to regenerate)\n");
  process.stdout.write(`\n${res.rubric.trim()}\n\n`);
  process.stdout.write(
    "Read the diff in the packet above, then walk the rubric. Record each finding when prompted;\n" +
      "minor/nit are advisory, blocker/major must be resolved or waived with a rationale.\n\n",
  );

  const existing = parseReviewFile(res.findingsFile).review;
  const findings: Finding[] = existing ? [...existing.findings] : [];

  const prompter = createPrompter();
  try {
    for (;;) {
      const again = (await prompter.ask(`Add a finding? [y/N] (${findings.length} recorded so far) `)).toLowerCase();
      if (again !== "y" && again !== "yes") break;
      findings.push(await askFinding(prompter, findings.length + 1));
    }

    let reviewer = (typeof args.flags.by === "string" ? args.flags.by : "").trim();
    while (!reviewer) {
      reviewer = await prompter.ask("Reviewer name (required to sign off): ");
    }

    writeFileAtomic(res.findingsFile, serializeReview(reviewer, findings));

    const blocking = findings.filter((f) => (f.severity === "blocker" || f.severity === "major") && f.status === "open");
    process.stdout.write(
      `\nRecorded review.md: reviewer=${reviewer}, ${findings.length} finding(s)` +
        (blocking.length ? `, ${blocking.length} still blocking\n` : ", none blocking\n"),
    );
    process.stdout.write(blocking.length ? "Resolve or waive the blocking finding(s), then `gate next`.\n" : "Run `gate next` to advance.\n");

    emit("", { phase: run.phase, packet: res.packet, findingsFile: res.findingsFile, reviewer, findings }, args.flags);
  } finally {
    prompter.close();
  }
}

/**
 * A minimal line-prompter over stdin. `readline/promises`'s `rl.question()`
 * drops input when stdin is a fully-buffered pipe rather than a live TTY:
 * every call after the first can hang forever, because it registers a fresh
 * one-shot `'line'` listener per call, while readline itself emits queued
 * lines as soon as data arrives - not lazily on demand - so lines that arrive
 * before the next `question()` call is registered are lost. The async
 * iterator protocol (`for await...of`, which `Interface` also implements)
 * pulls one line at a time instead and never drops a buffered line; real
 * interactive typing at a TTY behaves identically either way, so this is what
 * backs every prompt here.
 */
function createPrompter(): { ask(prompt: string): Promise<string>; close(): void } {
  const rl = createInterface({ input: process.stdin });
  const lines = rl[Symbol.asyncIterator]();
  return {
    async ask(prompt: string): Promise<string> {
      process.stdout.write(prompt);
      const { value, done } = await lines.next();
      if (done) throw new GateError("gate review --human: input ended before the review was recorded");
      return value.trim();
    },
    close(): void {
      rl.close();
    },
  };
}
type Prompter = ReturnType<typeof createPrompter>;

async function askFinding(prompter: Prompter, index: number): Promise<Finding> {
  const id = (await prompter.ask(`  id [f${index}]: `)) || `f${index}`;
  const severity = await askChoice(prompter, "  severity", SEVERITIES);
  const note = await prompter.ask("  note (what's wrong): ");
  const status = await askChoice(prompter, "  status", STATUSES);
  let waiver = "";
  if (status === "waived") {
    while (!waiver) waiver = await prompter.ask("  waiver rationale (required): ");
  }
  return { id, severity, note, status, waiver };
}

async function askChoice<T extends string>(prompter: Prompter, label: string, choices: readonly T[]): Promise<T> {
  for (;;) {
    const ans = (await prompter.ask(`${label} (${choices.join("/")}): `)).toLowerCase();
    if ((choices as readonly string[]).includes(ans)) return ans as T;
    process.stdout.write(`    invalid - choose one of ${choices.join(", ")}\n`);
  }
}
