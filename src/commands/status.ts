import {
  clearCurrentRunId,
  listActiveBranches,
  NO_GIT_BRANCH_KEY,
  readCurrentRunId,
  resolveBranchKey,
} from "../core/current.js";
import { loadConfig } from "../core/config.js";
import { readRun, type Run } from "../core/run.js";
import { phaseSequence } from "../core/stateMachine.js";
import { hasGate } from "../gates/index.js";
import { emit, requireRoot, type ParsedArgs } from "./shared.js";

interface OtherRun {
  branch: string;
  id: string;
  phase: string;
}

/**
 * `gate status` - cheap metadata only. This is on the agent hot path (called
 * constantly), so it never executes build/test commands; use `gate check` to
 * actually run the gate. Shows the active run for the current branch (if
 * any), plus every other branch with a run in flight (Milestone 4
 * concurrency) so an agent working across branches can see what else is
 * mid-flow. Never throws on detached HEAD - there's no branch to resolve a
 * "current" run from, but the in-flight list is still useful.
 */
export function cmdStatus(args: ParsedArgs): void {
  const root = requireRoot();
  loadConfig(root); // validate config is loadable; surfaces parse errors early

  const resolved = resolveBranchKey(root);
  const others = otherActiveRuns(root, resolved.kind === "key" ? resolved.key : null);

  if (resolved.kind === "detached") {
    const human = [
      "HEAD is detached - no branch to resolve a current run from; pass --run <id> to a phase command.",
      others.length ? "\nOther runs in flight:\n" + others.map(formatOther).join("\n") : "No runs in flight.",
    ].join("\n");
    emit(human, { active: false, detached: true, others }, args.flags);
    return;
  }

  const id = readCurrentRunId(root, resolved.key);
  if (!id) {
    const human = [
      "No active run. Start one with: gate start \"<title>\"",
      others.length ? "\nOther runs in flight:\n" + others.map(formatOther).join("\n") : "",
    ]
      .filter(Boolean)
      .join("\n");
    emit(human, { active: false, others }, args.flags);
    return;
  }
  let run: Run;
  try {
    run = readRun(root, id);
  } catch {
    // Dangling mapping - e.g. the run folder was pruned or removed by hand
    // after something left a stale current.json entry. Degrade gracefully
    // and self-heal instead of crashing with an uncaught "run not found":
    // clear the stale pointer so this branch stops resolving a run that no
    // longer exists.
    clearCurrentRunId(root, resolved.key);
    const human = [
      `No active run (a stale pointer to "${id}" was cleared - its run folder is gone).`,
      others.length ? "\nOther runs in flight:\n" + others.map(formatOther).join("\n") : "",
    ]
      .filter(Boolean)
      .join("\n");
    emit(human, { active: false, healed: id, others }, args.flags);
    return;
  }
  const artifacts = Object.keys(run.artifacts);
  const phases = phaseSequence(run.profile);
  const nextAction = hasGate(run.phase)
    ? `run \`gate check\` to see if the ${run.phase} gate passes`
    : `${run.phase} is terminal`;

  const human = [
    `Run:      ${run.id}`,
    `Title:    ${run.title}`,
    `Branch:   ${run.branch ?? "(no branch)"}`,
    `Phase:    ${run.phase}  (profile ${run.profile}, status ${run.status})`,
    `Flow:     ${phases.map((p) => (p === run.phase ? `[${p}]` : p)).join(" → ")}`,
    `Base:     ${run.baseRef ?? "(no git base)"}`,
    artifacts.length ? `Artifacts: ${artifacts.join(", ")}` : "Artifacts: none",
    `Next:     ${nextAction}`,
    others.length ? "\nOther runs in flight:\n" + others.map(formatOther).join("\n") : "",
  ]
    .filter(Boolean)
    .join("\n");

  emit(
    human,
    {
      active: true,
      id: run.id,
      title: run.title,
      branch: run.branch,
      phase: run.phase,
      profile: run.profile,
      phases,
      status: run.status,
      baseRef: run.baseRef,
      sessionId: run.sessionId,
      artifacts,
      nextAction,
      others,
    },
    args.flags,
  );
}

/** Every branch-keyed run other than `excludeKey` (the current branch, if resolvable). */
function otherActiveRuns(root: string, excludeKey: string | null): OtherRun[] {
  const out: OtherRun[] = [];
  for (const { key, runId } of listActiveBranches(root)) {
    if (key === excludeKey) continue;
    let run: Run;
    try {
      run = readRun(root, runId);
    } catch {
      continue; // unreadable run.json - leave it out, don't guess
    }
    out.push({ branch: key === NO_GIT_BRANCH_KEY ? "(no git)" : key, id: run.id, phase: run.phase });
  }
  return out;
}

function formatOther(o: OtherRun): string {
  return `  ${o.branch}: ${o.id} (${o.phase})`;
}
