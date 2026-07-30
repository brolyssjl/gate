import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { archivePath, gatePaths, runPaths } from "../core/paths.js";
import { readCurrentRunId, resolveBranchKey } from "../core/current.js";
import { readRun, type HistoryEntry, type Run } from "../core/run.js";
import { parseReviewFile, SEVERITIES, STATUSES } from "../artifacts/review.js";
import { emit, GateError, requireRoot, type ParsedArgs } from "./shared.js";

interface PhaseReport {
  phase: string;
  seconds: number;
  gateFailures: number;
}

export interface ReportData {
  id: string;
  title: string;
  profile: string;
  phase: string;
  status: string;
  totalSeconds: number;
  gateFailures: number;
  phases: PhaseReport[];
  findings: ReturnType<typeof summarizeFindings>;
  overrides: Array<{ phase: string; reason: string; at: string; by: string | null }>;
  artifacts: string[];
}

/**
 * Pure summary of a run: everything `gate report` prints, with no I/O beyond
 * what the caller already did (reading run.json and review.md). Shared by the
 * live path (`cmdReport`) and `gate prune`, which persists exactly this shape
 * to `.gate/archive/<id>.json` before deleting the run folder - the archive
 * summary and a live report are the same data, computed the same way.
 */
export function buildReportData(root: string, run: Run): ReportData {
  const phases = phaseReports(run.history);
  const totalSeconds = phases.reduce((a, p) => a + p.seconds, 0);
  const gateFailures = phases.reduce((a, p) => a + p.gateFailures, 0);
  const findings = summarizeFindings(root, run.id);
  const overrides = run.overrides.map((o) => ({ phase: o.phase, reason: o.reason, at: o.at, by: o.by ?? null }));
  const artifacts = Object.keys(run.artifacts);

  return {
    id: run.id,
    title: run.title,
    profile: run.profile,
    phase: run.phase,
    status: run.status,
    totalSeconds,
    gateFailures,
    phases,
    findings,
    overrides,
    artifacts,
  };
}

/** Render `buildReportData`'s output as the human-readable report text. */
export function renderReportHuman(data: ReportData, archived: boolean): string {
  return [
    `Report: ${data.id}${archived ? " (archived)" : ""}`,
    `  Title:    ${data.title}`,
    `  Profile:  ${data.profile}    Phase: ${data.phase}    Status: ${data.status}`,
    `  Duration: ${formatDuration(data.totalSeconds)} total, ${data.gateFailures} gate failure(s)`,
    "",
    "  Phase durations:",
    ...data.phases.map(
      (p) => `    ${p.phase.padEnd(10)} ${formatDuration(p.seconds).padStart(8)}` +
        (p.gateFailures ? `   (${p.gateFailures} failed attempt(s))` : ""),
    ),
    "",
    data.findings
      ? `  Findings: ${data.findings.total} total - ` +
        `${data.findings.blocker} blocker, ${data.findings.major} major, ${data.findings.minor} minor, ${data.findings.nit} nit ` +
        `(${data.findings.open} open, ${data.findings.resolved} resolved, ${data.findings.waived} waived)`
      : "  Findings: (no review.md)",
    data.overrides.length
      ? `  Overrides: ${data.overrides.map((o) => `${o.phase} (${o.reason}${o.by ? `, by ${o.by}` : ""})`).join(", ")}`
      : "  Overrides: none",
    data.artifacts.length ? `  Artifacts: ${data.artifacts.join(", ")}` : "  Artifacts: none",
  ].join("\n");
}

/**
 * `gate report [runId]` - a per-run summary: how long each phase took, how many
 * gate attempts failed, and any review findings. Read-only; it never runs a gate
 * or a command. Defaults to the active run, else the most recently updated live
 * one, so `gate report` works right after a run reaches DONE. Falls back to the
 * newest archived summary (`.gate/archive/<id>.json`) when `gate prune` has
 * already removed every live run folder - otherwise an argless `gate report`
 * right after a full prune would error even though summaries still exist.
 */
export function cmdReport(args: ParsedArgs): void {
  const root = requireRoot();
  const explicitId = args.positionals[0];
  const resolved = resolveBranchKey(root);
  // A detached HEAD has no branch to resolve a default run from - same
  // policy as the phase commands (check/next/review/...): refuse rather than
  // silently falling back to mostRecentRunId, which could describe a
  // completely different branch's run with nothing indicating that's what
  // happened.
  if (!explicitId && resolved.kind === "detached") {
    throw new GateError("HEAD is detached - no branch to resolve a default run from; pass a run id: gate report <id>");
  }
  const currentId = resolved.kind === "key" ? readCurrentRunId(root, resolved.key) : null;
  const runId = explicitId ?? currentId ?? mostRecentRunId(root) ?? mostRecentArchivedRunId(root);
  if (!runId) throw new GateError("no run to report on - pass a run id: gate report <id>");

  const { data, archived } = loadReportData(root, runId);
  emit(renderReportHuman(data, archived), data, args.flags);
}

function loadReportData(root: string, runId: string): { data: ReportData; archived: boolean } {
  if (existsSync(runPaths(root, runId).runJson)) {
    return { data: buildReportData(root, readRun(root, runId)), archived: false };
  }
  const archived = archivePath(root, runId);
  if (existsSync(archived)) {
    return { data: JSON.parse(readFileSync(archived, "utf8")) as ReportData, archived: true };
  }
  throw new GateError(`run "${runId}" not found (not live, and no archive summary at ${archived})`);
}

/** Per-phase wall-clock, summed across re-entries, plus failed-attempt counts. */
export function phaseReports(history: HistoryEntry[]): PhaseReport[] {
  const seconds = new Map<string, number>();
  const failures = new Map<string, number>();
  const order: string[] = [];
  const note = (phase: string) => {
    if (!order.includes(phase)) order.push(phase);
  };
  const lastAt = history[history.length - 1]?.at;

  for (let i = 0; i < history.length; i++) {
    const entry = history[i]!;
    note(entry.phase);
    if (entry.event === "failed") {
      failures.set(entry.phase, (failures.get(entry.phase) ?? 0) + 1);
    }
    if (entry.event === "entered") {
      // A phase spans from its `entered` event to the next phase entry (failed
      // and passed attempts in between belong to it), or to the last recorded
      // event while it is still the current phase.
      let end = lastAt ?? entry.at;
      for (let j = i + 1; j < history.length; j++) {
        if (history[j]!.event === "entered") {
          end = history[j]!.at;
          break;
        }
      }
      const delta = (Date.parse(end) - Date.parse(entry.at)) / 1000;
      if (Number.isFinite(delta) && delta > 0) {
        seconds.set(entry.phase, (seconds.get(entry.phase) ?? 0) + delta);
      } else if (!seconds.has(entry.phase)) {
        seconds.set(entry.phase, 0);
      }
    }
  }
  return order.map((phase) => ({
    phase,
    seconds: Math.round(seconds.get(phase) ?? 0),
    gateFailures: failures.get(phase) ?? 0,
  }));
}

function summarizeFindings(root: string, runId: string) {
  const { review } = parseReviewFile(runPaths(root, runId).review);
  if (!review) return null;
  const counts: Record<string, number> = { total: review.findings.length };
  for (const s of SEVERITIES) counts[s] = 0;
  for (const s of STATUSES) counts[s] = 0;
  for (const f of review.findings) {
    counts[f.severity] = (counts[f.severity] ?? 0) + 1;
    counts[f.status] = (counts[f.status] ?? 0) + 1;
  }
  return counts as {
    total: number;
    blocker: number;
    major: number;
    minor: number;
    nit: number;
    open: number;
    resolved: number;
    waived: number;
  };
}

function mostRecentRunId(root: string): string | null {
  const runsDir = gatePaths(root).runs;
  if (!existsSync(runsDir)) return null;
  let best: { id: string; at: number } | null = null;
  for (const id of readdirSync(runsDir)) {
    const runJson = join(runsDir, id, "run.json");
    if (!existsSync(runJson)) continue;
    try {
      const run = JSON.parse(readFileSync(runJson, "utf8")) as Run;
      const at = Date.parse(run.updatedAt);
      if (!best || at > best.at) best = { id, at };
    } catch {
      // ignore unreadable runs
    }
  }
  return best?.id ?? null;
}

/**
 * The most recently archived run's id, by archive-file mtime (`ReportData`
 * carries no timestamp of its own - `gate prune` writes archives in order, so
 * mtime is a faithful recency proxy). Used only when no live run exists at
 * all (e.g. `gate report` with no args right after a full `gate prune`).
 */
function mostRecentArchivedRunId(root: string): string | null {
  const archiveDir = gatePaths(root).archive;
  if (!existsSync(archiveDir)) return null;
  let best: { id: string; at: number } | null = null;
  for (const file of readdirSync(archiveDir)) {
    if (!file.endsWith(".json")) continue;
    const at = statSync(join(archiveDir, file)).mtimeMs;
    const id = file.slice(0, -".json".length);
    if (!best || at > best.at) best = { id, at };
  }
  return best?.id ?? null;
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m < 60) return s ? `${m}m${s}s` : `${m}m`;
  const h = Math.floor(m / 60);
  return `${h}h${m % 60}m`;
}
