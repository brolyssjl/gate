import { describe, expect, it } from "vitest";
import { parsePlan } from "../src/artifacts/plan.js";

const VALID = `---
goal: Do the thing.
approved: true
files:
  - src/**
criteria:
  - id: c1
    text: "does the thing"
    verify: "test: does the thing"
---
# Plan
body`;

describe("plan parser", () => {
  it("parses a valid plan", () => {
    const { plan, errors } = parsePlan(VALID);
    expect(errors).toEqual([]);
    expect(plan?.goal).toBe("Do the thing.");
    expect(plan?.approved).toBe(true);
    expect(plan?.criteria).toHaveLength(1);
    expect(plan?.files).toEqual(["src/**"]);
  });

  it("rejects missing frontmatter", () => {
    const { plan, errors } = parsePlan("# just markdown");
    expect(plan).toBeNull();
    expect(errors[0]).toMatch(/frontmatter/);
  });

  it("requires goal, files, and at least one criterion", () => {
    const { errors } = parsePlan(`---\napproved: true\n---\n`);
    expect(errors).toEqual(
      expect.arrayContaining([
        expect.stringMatching(/goal/),
        expect.stringMatching(/files/),
        expect.stringMatching(/criteria/),
      ]),
    );
  });

  it("requires each criterion to declare a verify method (checkability)", () => {
    const raw = `---
goal: g
approved: true
files: [a.ts]
criteria:
  - id: c1
    text: "x"
---`;
    const { errors } = parsePlan(raw);
    expect(errors).toEqual(expect.arrayContaining([expect.stringMatching(/verify is required/)]));
  });

  it("rejects duplicate criterion ids", () => {
    const raw = `---
goal: g
approved: true
files: [a.ts]
criteria:
  - id: c1
    text: x
    verify: manual
  - id: c1
    text: y
    verify: manual
---`;
    const { errors } = parsePlan(raw);
    expect(errors).toEqual(expect.arrayContaining([expect.stringMatching(/duplicated/)]));
  });

  it("treats approved as false unless explicitly true", () => {
    const { plan } = parsePlan(VALID.replace("approved: true", "approved: false"));
    expect(plan?.approved).toBe(false);
  });
});
