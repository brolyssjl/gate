import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { diffCoverage, loadCoverage, type CoverageMap } from "../src/gates/coverage.js";

/** A temp root with `coverage/<name>` seeded, for loadCoverage fixture tests. */
function withCoverageFile(name: string, content: string, extra?: { path: string; content: string }): string {
  const root = mkdtempSync(join(tmpdir(), "gate-coverage-"));
  mkdirSync(join(root, "coverage"), { recursive: true });
  writeFileSync(join(root, "coverage", name), content);
  if (extra) writeFileSync(join(root, extra.path), extra.content);
  return root;
}

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

describe("loadCoverage: coverage.py JSON", () => {
  const REPORT = JSON.stringify({
    meta: { format: 2 },
    files: {
      "pkg/mod.py": { executed_lines: [1, 2, 3], missing_lines: [4, 5] },
    },
    totals: {},
  });

  it("parses executed_lines/missing_lines per file", () => {
    const root = withCoverageFile("coverage.json", REPORT);
    const map = loadCoverage(root, "coverage-py");
    expect(map?.get("pkg/mod.py")).toEqual({ covered: new Set([1, 2, 3]), uncovered: new Set([4, 5]) });
  });

  it("auto-detects the format", () => {
    const root = withCoverageFile("coverage.json", REPORT);
    expect(loadCoverage(root, "auto")?.get("pkg/mod.py")?.covered).toEqual(new Set([1, 2, 3]));
  });

  it("fails closed on unparseable JSON", () => {
    const root = withCoverageFile("coverage.json", "not json at all");
    expect(loadCoverage(root, "coverage-py")).toBeNull();
  });

  it("fails closed when the report has no `files` key", () => {
    const root = withCoverageFile("coverage.json", JSON.stringify({ totals: {} }));
    expect(loadCoverage(root, "coverage-py")).toBeNull();
  });
});

describe("loadCoverage: Go cover profile", () => {
  const PROFILE = [
    "mode: set",
    "example.com/mod/pkg/file.go:1.1,3.2 2 1",
    "example.com/mod/pkg/file.go:4.1,4.10 1 0",
    "",
  ].join("\n");

  it("expands each block's line range, stripping the go.mod module prefix", () => {
    const root = withCoverageFile("go-cover.out", PROFILE, {
      path: "go.mod",
      content: "module example.com/mod\n\ngo 1.22\n",
    });
    const map = loadCoverage(root, "go-cover");
    expect(map?.get("pkg/file.go")).toEqual({ covered: new Set([1, 2, 3]), uncovered: new Set([4]) });
  });

  it("keeps the raw profile path when there's no go.mod to strip against", () => {
    const root = withCoverageFile("go-cover.out", PROFILE);
    const map = loadCoverage(root, "go-cover");
    expect(map?.has("example.com/mod/pkg/file.go")).toBe(true);
  });

  it("fails closed on a file with no `mode:` header", () => {
    const root = withCoverageFile("go-cover.out", "not a go cover profile\nrandom text\n");
    expect(loadCoverage(root, "go-cover")).toBeNull();
  });

  it("fails closed on an empty file", () => {
    const root = withCoverageFile("go-cover.out", "");
    expect(loadCoverage(root, "go-cover")).toBeNull();
  });

  it("a later block's covered hit wins when overlapping blocks disagree on the same line (merged multi-package profiles)", () => {
    const overlapping = [
      "mode: set",
      "example.com/mod/pkg/file.go:1.1,2.2 1 0",
      "example.com/mod/pkg/file.go:2.1,3.2 1 1",
      "",
    ].join("\n");
    const root = withCoverageFile("go-cover.out", overlapping);
    const map = loadCoverage(root, "go-cover");
    // Line 2 is uncovered per the first block, covered per the second - covered wins.
    expect(map?.get("example.com/mod/pkg/file.go")).toEqual({ covered: new Set([2, 3]), uncovered: new Set([1]) });
  });
});

describe("loadCoverage: lcov", () => {
  const LCOV = ["SF:src/file.ts", "DA:1,1", "DA:2,0", "DA:3,1", "end_of_record", ""].join("\n");

  it("parses DA lines into covered/uncovered per SF block", () => {
    const root = withCoverageFile("lcov.info", LCOV);
    const map = loadCoverage(root, "lcov");
    expect(map?.get("src/file.ts")).toEqual({ covered: new Set([1, 3]), uncovered: new Set([2]) });
  });

  it("relativizes an absolute SF: path (gcov, genhtml, cargo-llvm-cov style) instead of failing open", () => {
    const root = mkdtempSync(join(tmpdir(), "gate-coverage-"));
    mkdirSync(join(root, "coverage"), { recursive: true });
    const absolute = join(root, "src", "file.ts");
    writeFileSync(
      join(root, "coverage", "lcov.info"),
      ["SF:" + absolute, "DA:1,1", "DA:2,0", "end_of_record", ""].join("\n"),
    );
    const map = loadCoverage(root, "lcov");
    // Un-relativized, this would key under the absolute path and never match
    // a repo-relative changed-file entry - diffCoverage would then see no
    // coverage data at all for src/file.ts and silently report 100%.
    expect(map?.get("src/file.ts")).toEqual({ covered: new Set([1]), uncovered: new Set([2]) });
    expect(map?.has(absolute)).toBe(false);
  });

  it("relativizes a `./`-prefixed SF: path instead of failing open", () => {
    const root = withCoverageFile(
      "lcov.info",
      ["SF:./src/file.ts", "DA:1,1", "DA:2,0", "end_of_record", ""].join("\n"),
    );
    const map = loadCoverage(root, "lcov");
    expect(map?.get("src/file.ts")).toEqual({ covered: new Set([1]), uncovered: new Set([2]) });
    expect(map?.has("./src/file.ts")).toBe(false);
  });

  it("merges multiple record blocks for the same file", () => {
    const twoBlocks = [
      "SF:src/file.ts",
      "DA:1,0",
      "end_of_record",
      "SF:src/file.ts",
      "DA:1,1",
      "DA:2,1",
      "end_of_record",
      "",
    ].join("\n");
    const root = withCoverageFile("lcov.info", twoBlocks);
    const map = loadCoverage(root, "lcov");
    // Line 1 is covered by the second block; a covered hit anywhere wins.
    expect(map?.get("src/file.ts")).toEqual({ covered: new Set([1, 2]), uncovered: new Set([]) });
  });

  it("fails closed when there is no SF: record at all", () => {
    const root = withCoverageFile("lcov.info", "not an lcov file\njust text\n");
    expect(loadCoverage(root, "lcov")).toBeNull();
  });
});
