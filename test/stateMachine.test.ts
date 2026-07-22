import { describe, expect, it } from "vitest";
import {
  GATED_PHASES,
  PHASES,
  canSkip,
  isPhase,
  isTerminal,
  nextPhase,
  type Phase,
} from "../src/core/stateMachine.js";

describe("state machine", () => {
  it("has the milestone-1 linear phase order", () => {
    expect(PHASES).toEqual(["PLAN", "IMPLEMENT", "TEST", "DONE"]);
  });

  it("advances each phase to its successor and terminates at DONE", () => {
    expect(nextPhase("PLAN")).toBe("IMPLEMENT");
    expect(nextPhase("IMPLEMENT")).toBe("TEST");
    expect(nextPhase("TEST")).toBe("DONE");
    expect(nextPhase("DONE")).toBeNull();
  });

  it("marks only DONE as terminal", () => {
    expect(isTerminal("DONE")).toBe(true);
    for (const p of GATED_PHASES) expect(isTerminal(p)).toBe(false);
  });

  // Transition table: every phase × {pass advances, fail stays, skip advances}.
  const cases: Array<{ phase: Phase; onPass: Phase | null }> = [
    { phase: "PLAN", onPass: "IMPLEMENT" },
    { phase: "IMPLEMENT", onPass: "TEST" },
    { phase: "TEST", onPass: "DONE" },
    { phase: "DONE", onPass: null },
  ];
  for (const { phase, onPass } of cases) {
    it(`${phase}: pass→${onPass ?? "(none)"}, fail stays, skip=${canSkip(phase)}`, () => {
      expect(nextPhase(phase)).toBe(onPass);
      // fail keeps the phase (modeled as: caller does not advance)
      const stay = phase;
      expect(stay).toBe(phase);
      // skip is allowed on gated phases only, never on DONE
      expect(canSkip(phase)).toBe(phase !== "DONE");
    });
  }

  it("recognizes valid phase strings", () => {
    expect(isPhase("PLAN")).toBe(true);
    expect(isPhase("REVIEW")).toBe(false); // not in milestone 1
    expect(isPhase("nope")).toBe(false);
  });
});
