import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { loadConfig } from "../core/config.js";
import { readCurrentRunId } from "../core/current.js";
import { archivePath, gatePaths, runPaths } from "../core/paths.js";
import { readRun, type Run } from "../core/run.js";
import { buildReportData, type ReportData } from "./report.js";
import { emit, requireRoot, UsageError, type ParsedArgs } from "./shared.js";

const DEFAULT_KEEP = 10;

interface Candidate {
  id: string;
  updatedAt: string;
}

/**
 * `gate prune` — archive finished runs past the retention window (proposal
 * §4: "run folders are ephemeral by design"). Candidates are every non-active
 * run except the currently active one. Keeps the most recently updated
 * `--keep` (default 10, or `retention.keep` in config.yml); with `--days`,
 * a candidate must *also* be older than that many days to be pruned — `--keep`
 * alone is a count-based floor, `--days` an additional age gate. `--dry-run`
 * reports what would be pruned without touching disk. A pruned run's summary
 * (the same shape `gate report` prints) is written to
 * `.gate/archive/<id>.json` before its folder is deleted, so `gate report`
 * keeps working on a pruned run.
 */
export function cmdPrune(args: ParsedArgs): void {
  const root = requireRoot();
  const config = loadConfig(root);

  const keep = parseIntFlag(args.flags.keep, config.retention.keep ?? DEFAULT_KEEP, "--keep");
  const days = args.flags.days !== undefined ? parseIntFlag(args.flags.days, 0, "--days") : (config.retention.days ?? null);
  const dryRun = args.flags["dry-run"] === true;

  const activeId = readCurrentRunId(root);
  const candidates = listCandidates(root, activeId);
  const toPrune = selectForPrune(candidates, keep, days);

  if (dryRun) {
    const human = toPrune.length
      ? `Would prune ${toPrune.length} run(s):\n` + toPrune.map((c) => `  ${c.id}`).join("\n")
      : "Nothing to prune.";
    emit(human, { dryRun: true, pruned: toPrune.map((c) => c.id) }, args.flags);
    return;
  }

  const archiveDir = gatePaths(root).archive;
  mkdirSync(archiveDir, { recursive: true });
  const pruned: string[] = [];
  for (const c of toPrune) {
    const run = readRun(root, c.id);
    const data: ReportData = buildReportData(root, run);
    writeFileSync(archivePath(root, c.id), JSON.stringify(data, null, 2) + "\n");
    rmSync(runPaths(root, c.id).dir, { recursive: true, force: true });
    pruned.push(c.id);
  }

  const human = pruned.length
    ? `Pruned ${pruned.length} run(s), archived to ${archiveDir}:\n` + pruned.map((id) => `  ${id}`).join("\n")
    : "Nothing to prune.";
  emit(human, { dryRun: false, pruned }, args.flags);
}

function listCandidates(root: string, activeId: string | null): Candidate[] {
  const runsDir = gatePaths(root).runs;
  if (!existsSync(runsDir)) return [];
  const candidates: Candidate[] = [];
  for (const id of readdirSync(runsDir)) {
    if (id === activeId) continue;
    if (!existsSync(runPaths(root, id).runJson)) continue;
    let run: Run;
    try {
      run = readRun(root, id);
    } catch {
      continue; // unreadable run.json — leave it alone, don't guess
    }
    if (run.status === "active") continue;
    candidates.push({ id, updatedAt: run.updatedAt });
  }
  return candidates;
}

/** Newest `keep` candidates are always spared; the rest are pruned unless `days` excludes them. */
function selectForPrune(candidates: Candidate[], keep: number, days: number | null): Candidate[] {
  const sorted = [...candidates].sort((a, b) => {
    const byDate = Date.parse(b.updatedAt) - Date.parse(a.updatedAt);
    return byDate !== 0 ? byDate : a.id.localeCompare(b.id);
  });
  const beyondKeep = sorted.slice(Math.max(keep, 0));
  if (days === null) return beyondKeep;
  const cutoff = Date.now() - days * 24 * 60 * 60 * 1000;
  return beyondKeep.filter((c) => Date.parse(c.updatedAt) < cutoff);
}

function parseIntFlag(value: string | boolean | undefined, fallback: number, flagName: string): number {
  if (value === undefined) return fallback;
  const n = typeof value === "string" ? Number(value) : NaN;
  if (!Number.isFinite(n) || n < 0 || !Number.isInteger(n)) {
    throw new UsageError(`${flagName} expects a non-negative integer, got "${String(value)}"`);
  }
  return n;
}
