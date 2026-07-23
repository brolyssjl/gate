import { resolvePlaybook } from "../core/playbooks.js";
import { nowIso, writeRun } from "../core/run.js";
import { isTerminal } from "../core/stateMachine.js";
import { runGate } from "../gates/index.js";
import { advance } from "./advance.js";
import { renderGate } from "./gateRun.js";
import { requireActiveRun, type ParsedArgs } from "./shared.js";

/**
 * `gate next` — check the current gate and advance on pass. On failure it prints
 * exactly what's missing and exits non-zero without changing state.
 */
export function cmdNext(args: ParsedArgs): void {
  const { root, run, config } = requireActiveRun();
  const res = runGate({ root, run, config });

  if (!res.ok) {
    // Record the failed advancement attempt so `gate report` can show it. `gate
    // check` stays side-effect free; `next` is the deliberate attempt to advance.
    const failed = res.checks.filter((c) => !c.ok).map((c) => c.name).join(", ");
    run.history.push({ phase: run.phase, event: "failed", at: nowIso(), detail: failed });
    writeRun(root, run);
    renderGate(res, { advanced: false }, args.flags);
    process.exitCode = 1;
    return;
  }

  const from = run.phase;
  run.history.push({ phase: from, event: "passed", at: nowIso(), detail: `${from} gate passed` });
  advance(root, run);

  if (isTerminal(run.phase)) {
    renderGate(
      res,
      { advanced: true, from, to: run.phase, done: true },
      args.flags,
    );
    if (args.flags.json !== true && args.flags.format === undefined) {
      process.stdout.write(`\nRun "${run.id}" reached DONE. 🎉\n`);
    }
    return;
  }

  const playbook = resolvePlaybook(root, run.phase) ?? "";
  renderGate(res, { advanced: true, from, to: run.phase }, args.flags);
  if (args.flags.json !== true && args.flags.format === undefined && playbook) {
    process.stdout.write(`\nEntered ${run.phase}.\n\n${playbook}\n`);
  }
}
