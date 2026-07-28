import { describe, expect, it } from "vitest";
import {
  DEFAULT_PROFILE,
  GATED_PHASES,
  PHASES,
  PROFILES,
  canSkip,
  isPhase,
  isProfile,
  isTerminal,
  nextPhase,
  phaseSequence,
} from "../src/core/stateMachine.js";

describe("state machine", () => {
  it("catalogs every phase Gate knows about, DONE last", () => {
    expect(PHASES).toEqual(["PLAN", "DEBUG", "IMPLEMENT", "TEST", "REVIEW", "RETRO", "DONE"]);
    expect(PHASES[PHASES.length - 1]).toBe("DONE");
  });

  it("defaults to the feature profile", () => {
    expect(DEFAULT_PROFILE).toBe("feature");
  });

  it("builds each profile's sequence, always ending at DONE", () => {
    expect(phaseSequence("feature")).toEqual(["PLAN", "IMPLEMENT", "TEST", "REVIEW", "RETRO", "DONE"]);
    expect(phaseSequence("bugfix")).toEqual(["PLAN", "DEBUG", "TEST", "REVIEW", "RETRO", "DONE"]);
    expect(phaseSequence("refactor")).toEqual(["PLAN", "IMPLEMENT", "TEST", "REVIEW", "RETRO", "DONE"]);
    // docs skips REVIEW and RETRO - a docs-only change has no code review or retro.
    expect(phaseSequence("docs")).toEqual(["PLAN", "IMPLEMENT", "DONE"]);
  });

  it("falls back to the default profile for an unknown name", () => {
    expect(phaseSequence("nonsense")).toEqual(phaseSequence(DEFAULT_PROFILE));
    expect(isProfile("bugfix")).toBe(true);
    expect(isProfile("nonsense")).toBe(false);
  });

  it("advances within a profile and terminates at DONE", () => {
    expect(nextPhase("PLAN", "feature")).toBe("IMPLEMENT");
    expect(nextPhase("TEST", "feature")).toBe("REVIEW");
    expect(nextPhase("REVIEW", "feature")).toBe("RETRO");
    expect(nextPhase("RETRO", "feature")).toBe("DONE");
    expect(nextPhase("DONE", "feature")).toBeNull();
    // bugfix routes through DEBUG and skips IMPLEMENT.
    expect(nextPhase("PLAN", "bugfix")).toBe("DEBUG");
    expect(nextPhase("DEBUG", "bugfix")).toBe("TEST");
    // docs stops after IMPLEMENT.
    expect(nextPhase("IMPLEMENT", "docs")).toBe("DONE");
  });

  it("returns null for a phase not in the profile's sequence", () => {
    expect(nextPhase("DEBUG", "feature")).toBeNull(); // feature has no DEBUG
    expect(nextPhase("REVIEW", "docs")).toBeNull();
  });

  it("marks only DONE as terminal", () => {
    expect(isTerminal("DONE")).toBe(true);
    for (const p of GATED_PHASES) expect(isTerminal(p)).toBe(false);
  });

  it("allows skipping any gated phase but never DONE", () => {
    for (const p of PHASES) expect(canSkip(p)).toBe(p !== "DONE");
  });

  it("recognizes valid phase strings", () => {
    expect(isPhase("PLAN")).toBe(true);
    expect(isPhase("REVIEW")).toBe(true);
    expect(isPhase("DEBUG")).toBe(true);
    expect(isPhase("nope")).toBe(false);
  });

  it("only lists non-terminal phases in profile sequences", () => {
    for (const seq of Object.values(PROFILES)) {
      expect(seq).not.toContain("DONE");
    }
  });
});
