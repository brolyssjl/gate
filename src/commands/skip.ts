import { nowIso, writeRun } from "../core/run.js";
import { canSkip, isPhase } from "../core/stateMachine.js";
import { advance } from "./advance.js";
import { emit, GateError, requireActiveRun, UsageError, type ParsedArgs } from "./shared.js";

/**
 * `gate skip <phase> --reason "..."` — human-authorized skip of the current
 * gated phase, recorded in run.json overrides for the audit trail. Only the
 * current phase can be skipped, and never DONE.
 */
export function cmdSkip(args: ParsedArgs): void {
  const { root, run } = requireActiveRun();

  const arg = args.positionals[0];
  if (!arg) throw new UsageError('gate skip needs a phase: gate skip <phase> --reason "..."');
  const phase = arg.toUpperCase();
  if (!isPhase(phase)) throw new UsageError(`unknown phase "${arg}"`);

  const reason = typeof args.flags.reason === "string" ? args.flags.reason.trim() : "";
  if (!reason) throw new UsageError("gate skip requires --reason \"<why>\"");

  if (phase !== run.phase) {
    throw new GateError(`can only skip the current phase (${run.phase}), not ${phase}`);
  }
  if (!canSkip(run.phase)) throw new GateError(`${run.phase} cannot be skipped`);

  const at = nowIso();
  const by =
    (typeof args.flags.by === "string" ? args.flags.by : undefined) ??
    process.env.GATE_SESSION_ID ??
    null;
  run.overrides.push({ phase: run.phase, action: "skip", reason, at, by });
  run.history.push({ phase: run.phase, event: "skipped", at, detail: reason });
  writeRun(root, run);
  const { from, to } = advance(root, run);

  emit(
    `Skipped ${from} (reason: ${reason}) → ${to}`,
    { skipped: from, to, reason },
    args.flags,
  );
}
