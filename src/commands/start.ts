import { existsSync, writeFileSync } from "node:fs";
import { RETRO_TEMPLATE } from "../artifacts/retro.js";
import { loadConfig } from "../core/config.js";
import { clearCurrentRunId, readCurrentRunId, setCurrentRunId } from "../core/current.js";
import { headSha } from "../core/git.js";
import { runPaths } from "../core/paths.js";
import { resolvePlaybookWithOverlays } from "../core/playbooks.js";
import { makeRunId, newRun, readRun, writeRun } from "../core/run.js";
import { DEFAULT_PROFILE, isProfile, phaseSequence, PROFILES } from "../core/stateMachine.js";
import { resolveDisplayTargets } from "../core/targets.js";
import { detect, planHints } from "../integrations/index.js";
import { emit, GateError, requireRoot, UsageError, type ParsedArgs } from "./shared.js";

const PLAN_TEMPLATE = `---
goal:
spec:
files:
  -
out_of_scope: []
criteria:
  - id: c1
    text:
    verify: "test: "
risks: []
acknowledgments: []
---

# Plan: %TITLE%

`;

const DEBUG_TEMPLATE = `---
triggering_test:
reproduced: false
cycles:
  - hypothesis:
    prediction:
    experiment:
    observation:
    conclusion:
    status: in-progress
---

# Debug log: %TITLE%

`;

/** `gate start "<title>"` - create a run, enter PLAN, print the plan playbook. */
export function cmdStart(args: ParsedArgs): void {
  const root = requireRoot();
  const title = args.positionals.join(" ").trim();
  if (!title) throw new UsageError('gate start needs a title: gate start "<title>"');

  const existingId = readCurrentRunId(root);
  if (existingId) {
    const existing = safeRead(root, existingId);
    if (existing && existing.status === "active") {
      throw new GateError(
        `run "${existingId}" is still active (phase ${existing.phase}); finish or abandon it first`,
      );
    }
    clearCurrentRunId(root);
  }

  const profile = typeof args.flags.profile === "string" ? args.flags.profile : DEFAULT_PROFILE;
  if (!isProfile(profile)) {
    throw new UsageError(
      `unknown profile "${profile}" (choose one of ${Object.keys(PROFILES).join(", ")})`,
    );
  }
  const sessionId =
    (typeof args.flags.session === "string" ? args.flags.session : undefined) ??
    process.env.GATE_SESSION_ID ??
    null;
  const targetOverride =
    typeof args.flags.target === "string"
      ? args.flags.target.split(",").map((t) => t.trim()).filter(Boolean)
      : undefined;

  const config = loadConfig(root);
  if (targetOverride && targetOverride.length > 0) {
    const known = Object.keys(config.targets);
    const unknown = targetOverride.filter((t) => !known.includes(t));
    if (unknown.length > 0) {
      throw new UsageError(
        `unknown --target name(s): ${unknown.join(", ")} ` +
          (known.length > 0 ? `(valid targets: ${known.join(", ")})` : "(no targets configured in .gate/config.yml)"),
      );
    }
  }

  const id = uniqueRunId(root, title);
  const run = newRun({ id, title, profile, baseRef: headSha(root), sessionId, targetOverride });
  writeRun(root, run);
  setCurrentRunId(root, id);

  // Scaffold a plan.md for the agent to fill in, plus a debug-log.md / retro.md
  // when the profile walks through DEBUG / RETRO so the templates are waiting.
  const paths = runPaths(root, id);
  if (!existsSync(paths.plan)) writeFileSync(paths.plan, PLAN_TEMPLATE.replace("%TITLE%", title));
  const sequence = phaseSequence(profile);
  if (sequence.includes("DEBUG") && !existsSync(paths.debugLog)) {
    writeFileSync(paths.debugLog, DEBUG_TEMPLATE.replace("%TITLE%", title));
  }
  if (sequence.includes("RETRO") && !existsSync(paths.retro)) {
    writeFileSync(paths.retro, RETRO_TEMPLATE.replace("%TITLE%", title));
  }

  const hints = planHints(detect(root));
  const targetNames = resolveDisplayTargets(root, run, config);
  const playbook = resolvePlaybookWithOverlays(root, "PLAN", config, targetNames) ?? "(no PLAN playbook found)";
  const human = [
    `Started run "${id}" (profile: ${profile}, phases: ${sequence.join(" → ")}) → phase PLAN`,
    `Edit the plan at: ${paths.plan}`,
    "When it's ready and signed off, run `gate approve`, then `gate next`.",
    hints.length ? "\nHints:\n" + hints.map((h) => "  - " + h).join("\n") : "",
    "\n" + playbook,
  ]
    .filter(Boolean)
    .join("\n");

  emit(human, { id, phase: run.phase, profile, phases: sequence, plan: paths.plan, hints }, args.flags);
}

function safeRead(root: string, id: string) {
  try {
    return readRun(root, id);
  } catch {
    return null;
  }
}

function uniqueRunId(root: string, title: string): string {
  const base = makeRunId(title);
  let id = base;
  let n = 2;
  while (existsSync(runPaths(root, id).runJson)) id = `${base}-${n++}`;
  return id;
}
