import { readCurrentRunId } from "../core/current.js";
import { loadConfig } from "../core/config.js";
import { readRun } from "../core/run.js";
import { phaseSequence } from "../core/stateMachine.js";
import { hasGate } from "../gates/index.js";
import { emit, requireRoot, type ParsedArgs } from "./shared.js";

/**
 * `gate status` — cheap metadata only. This is on the agent hot path (called
 * constantly), so it never executes build/test commands; use `gate check` to
 * actually run the gate.
 */
export function cmdStatus(args: ParsedArgs): void {
  const root = requireRoot();
  const id = readCurrentRunId(root);
  if (!id) {
    emit("No active run. Start one with: gate start \"<title>\"", { active: false }, args.flags);
    return;
  }
  const run = readRun(root, id);
  loadConfig(root); // validate config is loadable; surfaces parse errors early
  const artifacts = Object.keys(run.artifacts);
  const phases = phaseSequence(run.profile);
  const nextAction = hasGate(run.phase)
    ? `run \`gate check\` to see if the ${run.phase} gate passes`
    : `${run.phase} is terminal`;

  const human = [
    `Run:      ${run.id}`,
    `Title:    ${run.title}`,
    `Phase:    ${run.phase}  (profile ${run.profile}, status ${run.status})`,
    `Flow:     ${phases.map((p) => (p === run.phase ? `[${p}]` : p)).join(" → ")}`,
    `Base:     ${run.baseRef ?? "(no git base)"}`,
    artifacts.length ? `Artifacts: ${artifacts.join(", ")}` : "Artifacts: none",
    `Next:     ${nextAction}`,
  ].join("\n");

  emit(
    human,
    {
      active: true,
      id: run.id,
      title: run.title,
      phase: run.phase,
      profile: run.profile,
      phases,
      status: run.status,
      baseRef: run.baseRef,
      sessionId: run.sessionId,
      artifacts,
      nextAction,
    },
    args.flags,
  );
}
