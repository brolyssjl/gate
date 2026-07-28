/**
 * The Gate state machine.
 *
 * The phase *catalog* is data-driven and the *sequence* a run walks is chosen by
 * its profile (feature/bugfix/refactor/docs). Profiles decide which phases run;
 * the transition logic itself is profile-agnostic. DONE is always terminal.
 *
 * Gate is an umpire: it never advances a phase on its own. Advancement is the
 * result of a gate passing (`gate next`) or an explicit human skip; nothing
 * here calls an agent or an LLM.
 */

/** Every phase Gate knows about. Not every run walks all of them (see profiles). */
export const PHASES = ["PLAN", "DEBUG", "IMPLEMENT", "TEST", "REVIEW", "RETRO", "DONE"] as const;
export type Phase = (typeof PHASES)[number];

/**
 * Run profiles select which phases run, in order. DONE is appended implicitly and
 * is never listed here. Adding a profile is a pure data edit - no transition code
 * changes. `feature` is the default (back-compatible with the Milestone 1 flow,
 * now with a REVIEW gate before DONE). RETRO closes every profile except `docs`
 * (kickoff interpretation: a docs-only change has nothing worth a retro).
 */
export const PROFILES = {
  feature: ["PLAN", "IMPLEMENT", "TEST", "REVIEW", "RETRO"],
  bugfix: ["PLAN", "DEBUG", "TEST", "REVIEW", "RETRO"],
  refactor: ["PLAN", "IMPLEMENT", "TEST", "REVIEW", "RETRO"],
  docs: ["PLAN", "IMPLEMENT"],
} as const satisfies Record<string, readonly Phase[]>;

export type Profile = keyof typeof PROFILES;
export const DEFAULT_PROFILE: Profile = "feature";

/** Phases that have a gate the agent must clear. DONE is terminal, no gate. */
export const GATED_PHASES: readonly Phase[] = ["PLAN", "DEBUG", "IMPLEMENT", "TEST", "REVIEW", "RETRO"];

export function isPhase(value: string): value is Phase {
  return (PHASES as readonly string[]).includes(value);
}

export function isProfile(value: string): value is Profile {
  return value in PROFILES;
}

export function isTerminal(phase: Phase): boolean {
  return phase === "DONE";
}

/** The ordered phases a run of `profile` walks, ending at DONE. */
export function phaseSequence(profile: string): Phase[] {
  const base = (isProfile(profile) ? PROFILES[profile] : PROFILES[DEFAULT_PROFILE]) as readonly Phase[];
  return [...base, "DONE"];
}

/**
 * The phase that follows `phase` within `profile`'s sequence, or null if `phase`
 * is terminal or not part of this profile. Defaults to the standard profile so
 * callers without a run in hand (tests, tooling) still get a sensible answer.
 */
export function nextPhase(phase: Phase, profile: string = DEFAULT_PROFILE): Phase | null {
  const seq = phaseSequence(profile);
  const idx = seq.indexOf(phase);
  if (idx < 0 || idx >= seq.length - 1) return null;
  return seq[idx + 1]!;
}

/**
 * Whether `phase` may be skipped by a human override. DONE cannot be skipped;
 * every other gated phase can. Skips are always recorded in run.json.
 */
export function canSkip(phase: Phase): boolean {
  return GATED_PHASES.includes(phase);
}
