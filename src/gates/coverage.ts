import { existsSync, readFileSync } from "node:fs";
import { join, relative, isAbsolute } from "node:path";

export interface FileCoverage {
  covered: Set<number>;
  uncovered: Set<number>;
}

/** relative POSIX path → line coverage. */
export type CoverageMap = Map<string, FileCoverage>;

/**
 * Load a coverage report. Milestone 1 ships two parsers (kickoff deferred list:
 * "jest/vitest + generic JSON contract first"):
 *   - istanbul `coverage/coverage-final.json` (jest, vitest --coverage)
 *   - generic contract: { files: [{ path, covered:[], uncovered:[] }] }
 * Returns null if no report is found or it can't be parsed.
 */
export function loadCoverage(root: string, format: "istanbul" | "generic" | "auto" = "auto"): CoverageMap | null {
  const istanbulPath = join(root, "coverage", "coverage-final.json");
  const genericPath = join(root, "coverage", "gate-coverage.json");

  if ((format === "generic" || format === "auto") && existsSync(genericPath)) {
    const g = parseGeneric(readFileSync(genericPath, "utf8"), root);
    if (g) return g;
  }
  if ((format === "istanbul" || format === "auto") && existsSync(istanbulPath)) {
    const i = parseIstanbul(readFileSync(istanbulPath, "utf8"), root);
    if (i) return i;
  }
  return null;
}

function toRel(root: string, p: string): string {
  const rel = isAbsolute(p) ? relative(root, p) : p;
  return rel.split("\\").join("/");
}

interface IstanbulStatement {
  start: { line: number };
  end: { line: number };
}
interface IstanbulFile {
  path?: string;
  statementMap: Record<string, IstanbulStatement>;
  s: Record<string, number>;
}

function parseIstanbul(raw: string, root: string): CoverageMap | null {
  let data: Record<string, IstanbulFile>;
  try {
    data = JSON.parse(raw) as Record<string, IstanbulFile>;
  } catch {
    return null;
  }
  const map: CoverageMap = new Map();
  for (const [key, file] of Object.entries(data)) {
    if (!file || !file.statementMap || !file.s) continue;
    const rel = toRel(root, file.path ?? key);
    const covered = new Set<number>();
    const uncovered = new Set<number>();
    for (const [id, stmt] of Object.entries(file.statementMap)) {
      const hits = file.s[id] ?? 0;
      for (let line = stmt.start.line; line <= stmt.end.line; line++) {
        if (hits > 0) covered.add(line);
        else if (!covered.has(line)) uncovered.add(line);
      }
    }
    // A line covered by any statement wins over an uncovered sibling statement.
    for (const line of covered) uncovered.delete(line);
    map.set(rel, { covered, uncovered });
  }
  return map;
}

interface GenericFile {
  path: string;
  covered?: number[];
  uncovered?: number[];
}

function parseGeneric(raw: string, root: string): CoverageMap | null {
  let data: { files?: GenericFile[] };
  try {
    data = JSON.parse(raw) as { files?: GenericFile[] };
  } catch {
    return null;
  }
  if (!Array.isArray(data.files)) return null;
  const map: CoverageMap = new Map();
  for (const f of data.files) {
    if (!f || typeof f.path !== "string") continue;
    map.set(toRel(root, f.path), {
      covered: new Set(f.covered ?? []),
      uncovered: new Set(f.uncovered ?? []),
    });
  }
  return map;
}

export interface DiffCoverage {
  changedExecutable: number;
  covered: number;
  /** 0–100; 100 when there are no measurable changed lines. */
  percent: number;
  /** Files with uncovered changed lines, for the failure message. */
  gaps: Array<{ file: string; uncovered: number[] }>;
}

/**
 * Diff coverage: of the executable lines changed in this run, how many are
 * covered by the tests. `changed` maps file → changed new-file line numbers;
 * an empty set means a wholly-new file (all executable lines count as changed).
 */
export function diffCoverage(changed: Map<string, Set<number>>, coverage: CoverageMap): DiffCoverage {
  let changedExecutable = 0;
  let covered = 0;
  const gaps: Array<{ file: string; uncovered: number[] }> = [];

  for (const [file, changedSet] of changed) {
    const cov = coverage.get(file);
    if (!cov) continue; // no coverage data for this file (e.g. non-source) — skip
    const executable = new Set<number>([...cov.covered, ...cov.uncovered]);
    const target =
      changedSet.size === 0
        ? executable // new file: all executable lines are "changed"
        : new Set([...changedSet].filter((l) => executable.has(l)));
    const missed: number[] = [];
    for (const line of target) {
      changedExecutable++;
      if (cov.covered.has(line)) covered++;
      else missed.push(line);
    }
    if (missed.length > 0) gaps.push({ file, uncovered: missed.sort((a, b) => a - b) });
  }

  const percent = changedExecutable === 0 ? 100 : Math.floor((covered / changedExecutable) * 100);
  return { changedExecutable, covered, percent, gaps };
}
