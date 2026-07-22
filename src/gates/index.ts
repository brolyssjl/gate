import type { Phase } from "../core/stateMachine.js";
import type { Gate, GateContext, GateResult } from "./types.js";
import { planGate } from "./plan.js";
import { implementGate } from "./implement.js";
import { testGate } from "./test.js";

const GATES: Partial<Record<Phase, Gate>> = {
  PLAN: planGate,
  IMPLEMENT: implementGate,
  TEST: testGate,
};

export function hasGate(phase: Phase): boolean {
  return phase in GATES;
}

/** Run the gate for the run's current phase. Terminal phases have no gate. */
export function runGate(ctx: GateContext): GateResult {
  const gate = GATES[ctx.run.phase];
  if (!gate) {
    return { phase: ctx.run.phase, ok: true, checks: [{ name: "phase", ok: true, detail: `${ctx.run.phase} has no gate` }] };
  }
  return gate(ctx);
}

export type { GateResult, GateContext };
