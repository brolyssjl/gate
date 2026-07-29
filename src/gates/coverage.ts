import { existsSync, readFileSync } from "node:fs";
import { join, relative, isAbsolute } from "node:path";
import type { CoverageFormat } from "../core/config.js";

export interface FileCoverage {
  covered: Set<number>;
  uncovered: Set<number>;
}

/** relative POSIX path → line coverage. */
export type CoverageMap = Map<string, FileCoverage>;

/**
 * Load a coverage report. Milestone 1 shipped two parsers (jest/vitest +
 * generic JSON contract); Milestone 4 adds three more built-in formats so
 * non-Node stacks don't fall back to hand-rolling the generic contract:
 *   - istanbul `coverage/coverage-final.json` (jest, vitest --coverage)
 *   - generic contract: { files: [{ path, covered:[], uncovered:[] }] }
 *   - coverage.py `coverage/coverage.json` (`coverage json -o coverage/coverage.json`)
 *   - Go cover profile `coverage/go-cover.out` (`go test -coverprofile=coverage/go-cover.out`)
 *   - lcov `coverage/lcov.info` (nyc, gcov, and many others' standard output path)
 * Each path is a Gate convention (everything lives under `coverage/`), not
 * something the underlying tool assumes - point the configured coverage
 * command at it. Returns null if no report is found or it can't be parsed;
 * a coverage threshold with no parseable report fails closed (see TEST gate).
 */
export function loadCoverage(root: string, format: CoverageFormat = "auto"): CoverageMap | null {
  const istanbulPath = join(root, "coverage", "coverage-final.json");
  const genericPath = join(root, "coverage", "gate-coverage.json");
  const coveragePyPath = join(root, "coverage", "coverage.json");
  const goCoverPath = join(root, "coverage", "go-cover.out");
  const lcovPath = join(root, "coverage", "lcov.info");

  if ((format === "generic" || format === "auto") && existsSync(genericPath)) {
    const g = parseGeneric(readFileSync(genericPath, "utf8"), root);
    if (g) return g;
  }
  if ((format === "istanbul" || format === "auto") && existsSync(istanbulPath)) {
    const i = parseIstanbul(readFileSync(istanbulPath, "utf8"), root);
    if (i) return i;
  }
  if ((format === "coverage-py" || format === "auto") && existsSync(coveragePyPath)) {
    const p = parseCoveragePy(readFileSync(coveragePyPath, "utf8"), root);
    if (p) return p;
  }
  if ((format === "go-cover" || format === "auto") && existsSync(goCoverPath)) {
    const c = parseGoCover(readFileSync(goCoverPath, "utf8"), root);
    if (c) return c;
  }
  if ((format === "lcov" || format === "auto") && existsSync(lcovPath)) {
    const l = parseLcov(readFileSync(lcovPath, "utf8"));
    if (l) return l;
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

interface CoveragePyFile {
  executed_lines?: unknown;
  missing_lines?: unknown;
}

/** coverage.py's `coverage json` output: `{ files: { <path>: { executed_lines, missing_lines } } }`. */
function parseCoveragePy(raw: string, root: string): CoverageMap | null {
  let data: { files?: unknown };
  try {
    data = JSON.parse(raw) as { files?: unknown };
  } catch {
    return null;
  }
  if (!data.files || typeof data.files !== "object" || Array.isArray(data.files)) return null;
  const map: CoverageMap = new Map();
  for (const [path, file] of Object.entries(data.files as Record<string, unknown>)) {
    if (!file || typeof file !== "object") continue;
    const f = file as CoveragePyFile;
    map.set(toRel(root, path), {
      covered: new Set(Array.isArray(f.executed_lines) ? (f.executed_lines as number[]) : []),
      uncovered: new Set(Array.isArray(f.missing_lines) ? (f.missing_lines as number[]) : []),
    });
  }
  return map;
}

/** `module <name>` line of go.mod, with a trailing slash, so profile paths can be de-prefixed to repo-relative. Null when there's no go.mod to read. */
function readGoModulePrefix(root: string): string | null {
  try {
    const gomod = readFileSync(join(root, "go.mod"), "utf8");
    const m = /^module\s+(\S+)/m.exec(gomod);
    return m ? m[1] + "/" : null;
  } catch {
    return null;
  }
}

const GO_COVER_BLOCK = /^(.+):(\d+)\.\d+,(\d+)\.\d+\s+\d+\s+(\d+)$/;

/**
 * `go test -coverprofile` text profile: a `mode: <set|count|atomic>` header,
 * then one line per code block: `<file>:<startLine>.<col>,<endLine>.<col>
 * <numStatements> <count>`. File paths are Go import paths
 * (`<module>/<repo-relative path>`), not filesystem paths - stripped against
 * go.mod's module name so they line up with git's repo-relative paths;
 * without a go.mod (or a module path that doesn't match) they're kept as-is,
 * which degrades to "no coverage data for this file" rather than crashing
 * (the same fate as any file `diffCoverage` doesn't recognize).
 */
function parseGoCover(raw: string, root: string): CoverageMap | null {
  const lines = raw.split("\n").map((l) => l.trim()).filter((l) => l.length > 0);
  if (lines.length === 0 || !/^mode:\s*\S+$/.test(lines[0]!)) return null;

  const prefix = readGoModulePrefix(root);
  const map: CoverageMap = new Map();
  for (const line of lines.slice(1)) {
    const m = GO_COVER_BLOCK.exec(line);
    if (!m) continue; // tolerate a stray/blank line rather than failing the whole report
    const [, file, startStr, endStr, countStr] = m;
    const start = Number(startStr);
    const end = Number(endStr);
    if (!file || !Number.isFinite(start) || !Number.isFinite(end)) continue;
    const rel = prefix && file.startsWith(prefix) ? file.slice(prefix.length) : file;

    let entry = map.get(rel);
    if (!entry) {
      entry = { covered: new Set(), uncovered: new Set() };
      map.set(rel, entry);
    }
    const hit = Number(countStr) > 0;
    for (let ln = start; ln <= end; ln++) {
      if (hit) entry.covered.add(ln);
      else if (!entry.covered.has(ln)) entry.uncovered.add(ln);
    }
  }
  // A later block's covered hit wins over an earlier uncovered record of the
  // same line (blocks from separate packages in a merged `go test ./...`
  // profile can overlap) - same cleanup parseIstanbul/parseLcov do.
  for (const entry of map.values()) {
    for (const line of entry.covered) entry.uncovered.delete(line);
  }
  return map;
}

/**
 * Standard lcov `.info` text format: `SF:<path>` starts a file record,
 * `DA:<line>,<hits>` reports one line's hit count, `end_of_record` closes it.
 * Multiple records for the same `SF` (e.g. separate test suites) accumulate
 * into the same file entry. Function/branch lines (FN/FNDA/BRDA) are ignored
 * - Gate measures line coverage only.
 */
function parseLcov(raw: string): CoverageMap | null {
  const lines = raw.split("\n");
  if (!lines.some((l) => l.trim().startsWith("SF:"))) return null;

  const map: CoverageMap = new Map();
  let current: FileCoverage | null = null;
  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (line.startsWith("SF:")) {
      const path = line.slice(3).trim().replace(/\\/g, "/");
      current = map.get(path) ?? { covered: new Set(), uncovered: new Set() };
      map.set(path, current);
    } else if (line.startsWith("DA:") && current) {
      const [lineStr, hitsStr] = line.slice(3).split(",");
      const ln = Number(lineStr);
      const hits = Number(hitsStr);
      if (!Number.isFinite(ln) || !Number.isFinite(hits)) continue;
      if (hits > 0) current.covered.add(ln);
      else if (!current.covered.has(ln)) current.uncovered.add(ln);
    } else if (line === "end_of_record") {
      current = null;
    }
  }
  // A later block's covered hit wins over an earlier uncovered record of the
  // same line (e.g. one test suite exercises a line another doesn't).
  for (const entry of map.values()) {
    for (const line of entry.covered) entry.uncovered.delete(line);
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
 * covered by the tests. `changed` maps file → changed new-file line numbers
 * (untracked files arrive with every line enumerated); an empty set means no
 * added lines - e.g. a deletion-only diff - and contributes nothing.
 */
export function diffCoverage(changed: Map<string, Set<number>>, coverage: CoverageMap): DiffCoverage {
  let changedExecutable = 0;
  let covered = 0;
  const gaps: Array<{ file: string; uncovered: number[] }> = [];

  for (const [file, changedSet] of changed) {
    const cov = coverage.get(file);
    if (!cov) continue; // no coverage data for this file (e.g. non-source) — skip
    const executable = new Set<number>([...cov.covered, ...cov.uncovered]);
    const target = new Set([...changedSet].filter((l) => executable.has(l)));
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
