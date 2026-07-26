import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { gatePaths, runPaths } from "../core/paths.js";
import { readCurrentRunId } from "../core/current.js";
import { readRun, type HistoryEntry, type Run } from "../core/run.js";
import { parseReviewFile, SEVERITIES, STATUSES } from "../artifacts/review.js";
import { emit, GateError, requireRoot, type ParsedArgs } from "./shared.js";

interface PhaseReport {
  phase: string;
  seconds: number;
  gateFailures: number;
}

/**
 * `gate report [runId]` — a per-run summary: how long each phase took, how many
 * gate attempts failed, and any review findings. Read-only; it never runs a gate
 * or a command. Defaults to the active run, else the most recently updated one,
 * so `gate report` works right after a run reaches DONE.
 */
export function cmdReport(args: ParsedArgs): void {
  const root = requireRoot();
  const runId = args.positionals[0] ?? readCurrentRunId(root) ?? mostRecentRunId(root);
  if (!runId) throw new GateError("no run to report on — pass a run id: gate report <id>");

  const run = readRun(root, runId);
  const phases = phaseReports(run.history);
  const totalSeconds = phases.reduce((a, p) => a + p.seconds, 0);
  const gateFailures = phases.reduce((a, p) => a + p.gateFailures, 0);
  const findings = summarizeFindings(root, runId);
  const overrides = run.overrides.map((o) => ({ phase: o.phase, reason: o.reason, at: o.at, by: o.by ?? null }));
  const artifacts = Object.keys(run.artifacts);

  const data = {
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

  const human = [
    `Report: ${run.id}`,
    `  Title:    ${run.title}`,
    `  Profile:  ${run.profile}    Phase: ${run.phase}    Status: ${run.status}`,
    `  Duration: ${formatDuration(totalSeconds)} total, ${gateFailures} gate failure(s)`,
    "",
    "  Phase durations:",
    ...phases.map(
      (p) => `    ${p.phase.padEnd(10)} ${formatDuration(p.seconds).padStart(8)}` +
        (p.gateFailures ? `   (${p.gateFailures} failed attempt(s))` : ""),
    ),
    "",
    findings
      ? `  Findings: ${findings.total} total — ` +
        `${findings.blocker} blocker, ${findings.major} major, ${findings.minor} minor, ${findings.nit} nit ` +
        `(${findings.open} open, ${findings.resolved} resolved, ${findings.waived} waived)`
      : "  Findings: (no review.md)",
    overrides.length
      ? `  Overrides: ${overrides.map((o) => `${o.phase} (${o.reason}${o.by ? `, by ${o.by}` : ""})`).join(", ")}`
      : "  Overrides: none",
    artifacts.length ? `  Artifacts: ${artifacts.join(", ")}` : "  Artifacts: none",
  ].join("\n");

  emit(human, data, args.flags);
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

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m < 60) return s ? `${m}m${s}s` : `${m}m`;
  const h = Math.floor(m / 60);
  return `${h}h${m % 60}m`;
}
