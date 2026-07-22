/**
 * The Gate state machine.
 *
 * Milestone 1 implements the linear slice PLAN → IMPLEMENT → TEST → DONE.
 * The phase list is data-driven so later milestones can splice in DEBUG,
 * REVIEW and RETRO without touching transition logic. DONE is terminal.
 *
 * Gate is an umpire: it never advances a phase on its own. Advancement is the
 * result of a gate passing (`gate next`) or an explicit human skip; nothing
 * here calls an agent or an LLM.
 */

export const PHASES = ["PLAN", "IMPLEMENT", "TEST", "DONE"] as const;
export type Phase = (typeof PHASES)[number];

/** Phases that have a gate the agent must clear. DONE is terminal, no gate. */
export const GATED_PHASES: readonly Phase[] = ["PLAN", "IMPLEMENT", "TEST"];

export function isPhase(value: string): value is Phase {
  return (PHASES as readonly string[]).includes(value);
}

export function isTerminal(phase: Phase): boolean {
  return phase === "DONE";
}

/** The phase that follows `phase`, or null if `phase` is terminal. */
export function nextPhase(phase: Phase): Phase | null {
  const idx = PHASES.indexOf(phase);
  if (idx < 0 || idx >= PHASES.length - 1) return null;
  return PHASES[idx + 1]!;
}

/**
 * Whether `phase` may be skipped by a human override. DONE cannot be skipped;
 * every other gated phase can. Skips are always recorded in run.json.
 */
export function canSkip(phase: Phase): boolean {
  return GATED_PHASES.includes(phase);
}
