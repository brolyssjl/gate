import type { GateResult } from "../gates/index.js";
import { emit, type ParsedArgs } from "./shared.js";

/** Render a gate result in the requested format and return whether it passed. */
export function renderGate(res: GateResult, extra: Record<string, unknown>, flags: ParsedArgs["flags"]): boolean {
  const mark = (ok: boolean) => (ok ? "✓" : "✗");
  const lines = [
    `${mark(res.ok)} ${res.phase} gate: ${res.ok ? "PASS" : "FAIL"}`,
    ...res.checks.map((c) => `  ${mark(c.ok)} ${c.name}${c.detail ? " — " + c.detail : ""}`),
  ];
  emit(lines.join("\n"), { phase: res.phase, ok: res.ok, checks: res.checks, ...extra }, flags);
  return res.ok;
}
