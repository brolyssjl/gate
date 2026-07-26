import { describe, expect, it } from "vitest";
import { diffCoverage, type CoverageMap } from "../src/gates/coverage.js";

describe("diff coverage", () => {
  const coverage: CoverageMap = new Map([
    ["src/a.ts", { covered: new Set([1, 2, 3]), uncovered: new Set([4, 5]) }],
  ]);

  it("is 100% when the covered changed lines are all covered", () => {
    const changed = new Map([["src/a.ts", new Set([1, 2])]]);
    expect(diffCoverage(changed, coverage).percent).toBe(100);
  });

  it("computes the covered fraction of changed executable lines", () => {
    const changed = new Map([["src/a.ts", new Set([1, 2, 4])]]); // 4 is uncovered
    const dc = diffCoverage(changed, coverage);
    expect(dc.changedExecutable).toBe(3);
    expect(dc.covered).toBe(2);
    expect(dc.percent).toBe(66);
    expect(dc.gaps).toEqual([{ file: "src/a.ts", uncovered: [4] }]);
  });

  it("ignores changed lines with no coverage data (non-source)", () => {
    const changed = new Map([["README.md", new Set([1, 2])]]);
    expect(diffCoverage(changed, coverage).percent).toBe(100);
  });

  it("treats an empty changed set (deletion-only diff) as nothing to measure", () => {
    // Untracked/new files arrive with every line enumerated by changedLines();
    // an empty set means no added lines and must not demand whole-file coverage.
    const changed = new Map([["src/a.ts", new Set<number>()]]);
    const dc = diffCoverage(changed, coverage);
    expect(dc.changedExecutable).toBe(0);
    expect(dc.percent).toBe(100);
    expect(dc.gaps).toEqual([]);
  });

  it("reports 100% when nothing measurable changed", () => {
    expect(diffCoverage(new Map(), coverage).percent).toBe(100);
  });
});
