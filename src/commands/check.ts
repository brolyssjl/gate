import { runGate } from "../gates/index.js";
import { renderGate } from "./gateRun.js";
import { requireActiveRun, type ParsedArgs } from "./shared.js";

/**
 * `gate check` — run the current phase's gate; print pass/fail with reasons.
 * Exit code IS the verdict (0 pass, 1 fail) so CI and agents can branch on it.
 * Gate never advances here; use `gate next` to advance on pass.
 */
export function cmdCheck(args: ParsedArgs): void {
  const { root, run, config } = requireActiveRun(args);
  const res = runGate({ root, run, config });
  const ok = renderGate(res, {}, args.flags);
  process.exitCode = ok ? 0 : 1;
}
