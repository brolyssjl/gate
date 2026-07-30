import { existsSync, readFileSync } from "node:fs";
import { join, relative, isAbsolute, normalize } from "node:path";
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
/**
 * One registry entry per format Gate knows, driving both auto-detection
 * order and (via `coverageReportPaths`) the "no report found" failure
 * message - previously the format list, the path-per-format + if-chain
 * here, and the hardcoded path list in the TEST gate's failure message were
 * three separate places that had to be kept in lockstep by hand. Keyed by
 * every `CoverageFormat` except `"auto"` (not a real format to look for, just
 * "try them all") - a format added to `CoverageFormat` without a matching
 * entry here is a compile error, and vice versa.
 */
type ParseableFormat = Exclude<CoverageFormat, "auto">;
interface FormatSpec {
  /** Path relative to the repo root this format's report is expected at (a Gate convention, not something the underlying tool assumes). */
  path: string;
  parse: (raw: string, root: string) => CoverageMap | null;
}
const FORMATS: Record<ParseableFormat, FormatSpec> = {
  generic: { path: "coverage/gate-coverage.json", parse: parseGeneric },
  istanbul: { path: "coverage/coverage-final.json", parse: parseIstanbul },
  "coverage-py": { path: "coverage/coverage.json", parse: parseCoveragePy },
  "go-cover": { path: "coverage/go-cover.out", parse: parseGoCover },
  lcov: { path: "coverage/lcov.info", parse: parseLcov },
};

/** Every format's expected report path, for the "no coverage report was found" failure message. */
export function coverageReportPaths(): string[] {
  return Object.values(FORMATS).map((spec) => spec.path);
}

export function loadCoverage(root: string, format: CoverageFormat = "auto"): CoverageMap | null {
  const candidates = (format === "auto" ? Object.keys(FORMATS) : [format]) as ParseableFormat[];
  for (const name of candidates) {
    const spec = FORMATS[name];
    const full = join(root, spec.path);
    if (!existsSync(full)) continue;
    const parsed = spec.parse(readFileSync(full, "utf8"), root);
    if (parsed) return parsed;
    // Exists but didn't parse: fall through and keep trying other formats in
    // "auto" mode (an explicit single format has nothing left to fall back to).
  }
  return null;
}

function toRel(root: string, p: string): string {
  // normalize() collapses a leading "./" (some lcov writers - genhtml,
  // certain nyc/cargo-llvm-cov configs - emit `SF:./src/file.ts`), not just
  // the absolute-path case every other parser already handled.
  const rel = isAbsolute(p) ? relative(root, p) : normalize(p);
  return rel.split("\\").join("/");
}

/**
 * Record one line's hit/miss into a file's coverage entry. A covered hit is
 * never overwritten by a later uncovered record of the same line (an earlier
 * uncovered mark yields to a later covered one instead - see
 * `reconcileCoverage`), so callers don't need to check `covered.has(line)`
 * themselves before adding to `uncovered`.
 */
function markLine(entry: FileCoverage, line: number, hit: boolean): void {
  if (hit) entry.covered.add(line);
  else if (!entry.covered.has(line)) entry.uncovered.add(line);
}

/**
 * Final reconcile pass, shared by every parser that can see the same line
 * more than once under different verdicts (istanbul: sibling statements on
 * one line; go-cover: overlapping blocks from a merged multi-package
 * profile; lcov: separate test-suite records for one SF). A covered hit
 * anywhere always wins over an uncovered record of the same line recorded
 * earlier, regardless of order.
 */
function reconcileCoverage(map: CoverageMap): void {
  for (const entry of map.values()) {
    for (const line of entry.covered) entry.uncovered.delete(line);
  }
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
    const entry: FileCoverage = { covered: new Set(), uncovered: new Set() };
    for (const [id, stmt] of Object.entries(file.statementMap)) {
      const hits = file.s[id] ?? 0;
      for (let line = stmt.start.line; line <= stmt.end.line; line++) {
        markLine(entry, line, hits > 0);
      }
    }
    map.set(rel, entry);
  }
  reconcileCoverage(map);
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
    for (let ln = start; ln <= end; ln++) markLine(entry, ln, hit);
  }
  reconcileCoverage(map);
  return map;
}

/**
 * Standard lcov `.info` text format: `SF:<path>` starts a file record,
 * `DA:<line>,<hits>` reports one line's hit count, `end_of_record` closes it.
 * Multiple records for the same `SF` (e.g. separate test suites) accumulate
 * into the same file entry. Function/branch lines (FN/FNDA/BRDA) are ignored
 * - Gate measures line coverage only. `SF:` paths are relativized via `toRel`
 * like every other parser: gcov, genhtml, cargo-llvm-cov, and some nyc
 * configs write absolute (or `./`-prefixed) paths, which otherwise never
 * match git's repo-relative changed files - `diffCoverage` would then skip
 * every file and report a vacuous 100%, the diff-coverage gate failing open
 * at any threshold.
 */
function parseLcov(raw: string, root: string): CoverageMap | null {
  const lines = raw.split("\n");
  if (!lines.some((l) => l.trim().startsWith("SF:"))) return null;

  const map: CoverageMap = new Map();
  let current: FileCoverage | null = null;
  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (line.startsWith("SF:")) {
      const path = toRel(root, line.slice(3).trim());
      current = map.get(path) ?? { covered: new Set(), uncovered: new Set() };
      map.set(path, current);
    } else if (line.startsWith("DA:") && current) {
      const [lineStr, hitsStr] = line.slice(3).split(",");
      const ln = Number(lineStr);
      const hits = Number(hitsStr);
      if (!Number.isFinite(ln) || !Number.isFinite(hits)) continue;
      markLine(current, ln, hits > 0);
    } else if (line === "end_of_record") {
      current = null;
    }
  }
  reconcileCoverage(map);
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
