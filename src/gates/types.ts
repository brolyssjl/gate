import type { GateConfig } from "../core/config.js";
import type { Run } from "../core/run.js";
import type { Phase } from "../core/stateMachine.js";

export interface Check {
  name: string;
  ok: boolean;
  detail: string;
}

export interface GateResult {
  phase: Phase;
  ok: boolean;
  checks: Check[];
}

export interface GateContext {
  root: string;
  run: Run;
  config: GateConfig;
}

export type Gate = (ctx: GateContext) => GateResult;

export function pass(name: string, detail = ""): Check {
  return { name, ok: true, detail };
}

export function fail(name: string, detail: string): Check {
  return { name, ok: false, detail };
}

export function result(phase: Phase, checks: Check[]): GateResult {
  return { phase, ok: checks.every((c) => c.ok), checks };
}
