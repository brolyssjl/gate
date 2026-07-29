import { loadConfig } from "../core/config.js";
import { currentRunIdOrNull } from "../core/current.js";
import { resolvePlaybookWithOverlays } from "../core/playbooks.js";
import { readRun } from "../core/run.js";
import { isPhase, type Phase } from "../core/stateMachine.js";
import { resolveDisplayTargets } from "../core/targets.js";
import { detect, planHints } from "../integrations/index.js";
import { emit, GateError, requireActiveRun, requireRoot, UsageError, type ParsedArgs } from "./shared.js";

/**
 * `gate playbook [phase] [--run <id>]` - print the active playbook (agents
 * call this at each phase entry). With no argument it prints the current
 * run's phase playbook. When the run has affected targets (Milestone 3),
 * each target's overlay for this phase (if any) is appended after the base
 * playbook.
 */
export function cmdPlaybook(args: ParsedArgs): void {
  const arg = args.positionals[0];
  let root: string;
  let phase: Phase;
  let config: ReturnType<typeof loadConfig>;
  let targetNames: string[];

  if (arg) {
    const upper = arg.toUpperCase();
    if (!isPhase(upper)) throw new UsageError(`unknown phase "${arg}"`);
    phase = upper;
    root = requireRoot();
    config = loadConfig(root);
    // No active-run context to reuse here - an explicit phase argument works
    // even with no run started, so targets (if any) come from whatever run
    // is named. --run picks a specific run's targets (honored here too, not
    // just in the no-arg branch below - an explicit phase argument must not
    // silently fall back to the current branch's run instead); otherwise
    // whatever run is current for this branch, if any. An unreadable/missing
    // run degrades to no targets rather than failing - printing a phase's
    // base playbook must keep working with no run in the picture at all.
    const explicitRunId = typeof args.flags.run === "string" ? args.flags.run : undefined;
    const runId = explicitRunId ?? currentRunIdOrNull(root);
    const run = runId ? safeReadRun(root, runId) : null;
    targetNames = run ? resolveDisplayTargets(root, run, config) : [];
  } else {
    const ctx = requireActiveRun(args);
    root = ctx.root;
    phase = ctx.run.phase;
    config = ctx.config;
    targetNames = resolveDisplayTargets(root, ctx.run, config);
  }

  const playbook = resolvePlaybookWithOverlays(root, phase, config, targetNames);
  if (!playbook) throw new GateError(`no playbook for phase ${phase}`);

  const hints = phase === "PLAN" ? planHints(detect(root)) : [];
  const human = hints.length
    ? playbook + "\n\n## Detected integrations\n" + hints.map((h) => "- " + h).join("\n")
    : playbook;

  emit(human, { phase, playbook, hints }, args.flags);
}

function safeReadRun(root: string, id: string) {
  try {
    return readRun(root, id);
  } catch {
    return null;
  }
}
