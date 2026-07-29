import { existsSync, writeFileSync } from "node:fs";
import { RETRO_TEMPLATE } from "../artifacts/retro.js";
import { hashPlanFile } from "../artifacts/plan.js";
import { loadConfig } from "../core/config.js";
import { NO_GIT_BRANCH_KEY, resolveBranchKey, withCurrentLock } from "../core/current.js";
import { headSha } from "../core/git.js";
import { runPaths } from "../core/paths.js";
import { resolvePlaybookWithOverlays } from "../core/playbooks.js";
import { makeRunId, newRun, readRun, writeRun, type Run } from "../core/run.js";
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

  const resolved = resolveBranchKey(root);
  if (resolved.kind === "detached") {
    throw new GateError(
      "HEAD is detached — runs are keyed by branch; checkout a branch before `gate start`",
    );
  }
  const branchKey = resolved.key;
  const branch = branchKey === NO_GIT_BRANCH_KEY ? null : branchKey;

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

  // The whole "read this branch's existing mapping -> decide resume/create ->
  // create the new run -> point the branch at it" sequence runs under one
  // lock, so a concurrent `gate start` on the same branch can't interleave
  // between the decision and the write (the milestone's own use case is
  // several agents in flight at once).
  type Outcome = { kind: "resumed"; run: Run; titleMismatch: boolean } | { kind: "created"; run: Run };
  const outcome = withCurrentLock(root, (branches): Outcome => {
    const existingId = branches[branchKey];
    if (existingId) {
      const existing = safeRead(root, existingId);
      if (existing && existing.status === "active") {
        const planPath = runPaths(root, existingId).plan;
        if (existing.approval && hashPlanFile(planPath) !== existing.approval.planHash) {
          throw new GateError(
            `run "${existingId}" on this branch is approved but plan.md has changed since — ` +
              "re-run `gate approve` or resolve the run before starting fresh",
          );
        }
        // Resuming silently on a real profile/target conflict would let an
        // agent believe it started (say) a bugfix-profile run when it's
        // actually driving an old docs-profile one - a hard error only when
        // the flag was *explicitly* requested (an unset flag defaulting to
        // "feature" must not manufacture a false conflict against whatever
        // profile the resumed run happens to be).
        if (typeof args.flags.profile === "string" && profile !== existing.profile) {
          throw new GateError(
            `run "${existingId}" on this branch is profile "${existing.profile}", but --profile ${profile} ` +
              "was requested for a resumed run — drop --profile to resume it as-is, or finish/abandon it first",
          );
        }
        const existingTargets = existing.targetOverride ?? [];
        const requestedTargets = targetOverride ?? [];
        const targetsDiffer =
          requestedTargets.length !== existingTargets.length ||
          requestedTargets.some((t, i) => t !== existingTargets[i]);
        if (typeof args.flags.target === "string" && targetsDiffer) {
          throw new GateError(
            `run "${existingId}" on this branch has --target ${existingTargets.join(",") || "(none)"}, but ` +
              `${requestedTargets.join(",") || "(none)"} was requested for a resumed run — ` +
              "drop --target to resume it as-is, or finish/abandon it first",
          );
        }
        // A title mismatch is cosmetic (it doesn't change gate behavior the
        // way profile/target do) - surfaced as a loud warning, not a hard
        // error, so the resumed run's own title is always what's kept.
        return { kind: "resumed", run: existing, titleMismatch: existing.title !== title };
      }
      delete branches[branchKey];
    }

    const id = uniqueRunId(root, title);
    const run = newRun({ id, title, profile, branch, baseRef: headSha(root), sessionId, targetOverride });
    writeRun(root, run);
    branches[branchKey] = id;
    return { kind: "created", run };
  });

  if (outcome.kind === "resumed") {
    // Resuming, not starting fresh: this branch already has work in flight.
    const existing = outcome.run;
    const sequence = phaseSequence(existing.profile);
    const human = [
      `Branch already has an active run: "${existing.id}" (phase ${existing.phase}) — resuming it.`,
      outcome.titleMismatch
        ? `WARNING: requested title "${title}" differs from the resumed run's title "${existing.title}" - ` +
          "the resumed run's own title was kept; pass --profile/--target to detect a real conflict instead of guessing from the title."
        : "",
      `Flow: ${sequence.map((p) => (p === existing.phase ? `[${p}]` : p)).join(" → ")}`,
      `Next: run \`gate status\` or \`gate playbook\` to continue.`,
    ]
      .filter(Boolean)
      .join("\n");
    emit(
      human,
      {
        id: existing.id,
        phase: existing.phase,
        profile: existing.profile,
        phases: sequence,
        resumed: true,
        requestedTitle: title,
        titleMismatch: outcome.titleMismatch,
      },
      args.flags,
    );
    return;
  }

  const run = outcome.run;
  const id = run.id;

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
    `Started run "${id}" (profile: ${profile}, phases: ${sequence.join(" → ")}) → phase PLAN` +
      (branch ? ` on branch "${branch}"` : ""),
    `Edit the plan at: ${paths.plan}`,
    "When it's ready and signed off, run `gate approve`, then `gate next`.",
    hints.length ? "\nHints:\n" + hints.map((h) => "  - " + h).join("\n") : "",
    "\n" + playbook,
  ]
    .filter(Boolean)
    .join("\n");

  emit(human, { id, phase: run.phase, profile, branch, phases: sequence, plan: paths.plan, hints }, args.flags);
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
