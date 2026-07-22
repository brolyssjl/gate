import { resolvePlaybook } from "../core/playbooks.js";
import { isPhase, type Phase } from "../core/stateMachine.js";
import { detect, planHints } from "../integrations/index.js";
import { emit, GateError, requireActiveRun, requireRoot, UsageError, type ParsedArgs } from "./shared.js";

/**
 * `gate playbook [phase]` — print the active playbook (agents call this at each
 * phase entry). With no argument it prints the current run's phase playbook.
 */
export function cmdPlaybook(args: ParsedArgs): void {
  const arg = args.positionals[0];
  let root: string;
  let phase: Phase;

  if (arg) {
    const upper = arg.toUpperCase();
    if (!isPhase(upper)) throw new UsageError(`unknown phase "${arg}"`);
    phase = upper;
    root = requireRoot();
  } else {
    const ctx = requireActiveRun();
    root = ctx.root;
    phase = ctx.run.phase;
  }

  const playbook = resolvePlaybook(root, phase);
  if (!playbook) throw new GateError(`no playbook for phase ${phase}`);

  const hints = phase === "PLAN" ? planHints(detect(root)) : [];
  const human = hints.length
    ? playbook + "\n\n## Detected integrations\n" + hints.map((h) => "- " + h).join("\n")
    : playbook;

  emit(human, { phase, playbook, hints }, args.flags);
}
