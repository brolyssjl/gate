import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { runPaths } from "./paths.js";
import type { Phase } from "./stateMachine.js";

export type RunStatus = "active" | "done" | "abandoned";

export interface HistoryEntry {
  phase: Phase;
  event: "entered" | "passed" | "failed" | "skipped";
  at: string;
  /** For failed/passed: short summary of the gate verdict. */
  detail?: string;
}

export interface OverrideEntry {
  phase: Phase;
  action: "skip";
  reason: string;
  at: string;
}

export interface ArtifactEntry {
  phase: Phase;
  at: string;
}

export interface Approval {
  /** Who approved (from --by or GATE_SESSION_ID); best-effort, not proof of a human. */
  by: string | null;
  at: string;
  reason: string | null;
  /** Hash of the plan at approval time; a later plan edit voids the approval. */
  planHash: string;
}

export interface ReviewRequest {
  /** Reviewer identity (from --by or GATE_SESSION_ID); best-effort, not proof of a human. */
  reviewer: string | null;
  requestedAt: string;
}

export interface Run {
  /** Schema version of this run.json, for forward migration. */
  schema: 1;
  id: string;
  title: string;
  profile: string;
  phase: Phase;
  status: RunStatus;
  createdAt: string;
  updatedAt: string;
  /** git ref (sha) captured at `gate start`; the IMPLEMENT diff is measured from here. */
  baseRef: string | null;
  /** Session id of the implementer, when the agent supplies one (GATE_SESSION_ID). */
  sessionId: string | null;
  history: HistoryEntry[];
  overrides: OverrideEntry[];
  /** Registered artifacts keyed by relative filename within the run folder. */
  artifacts: Record<string, ArtifactEntry>;
  /** PLAN approval, recorded by `gate approve` (absent until approved). */
  approval?: Approval;
  /** REVIEW packet request, recorded by `gate review` (absent until requested). */
  review?: ReviewRequest;
}

export function nowIso(): string {
  return new Date().toISOString();
}

/** Deterministic run id: `YYYY-MM-DD-<slug>`, deduped by caller if needed. */
export function makeRunId(title: string, date: Date = new Date()): string {
  const day = date.toISOString().slice(0, 10);
  const slug =
    title
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || "run";
  return `${day}-${slug}`;
}

export function newRun(params: {
  id: string;
  title: string;
  profile: string;
  baseRef: string | null;
  sessionId: string | null;
}): Run {
  const at = nowIso();
  return {
    schema: 1,
    id: params.id,
    title: params.title,
    profile: params.profile,
    phase: "PLAN",
    status: "active",
    createdAt: at,
    updatedAt: at,
    baseRef: params.baseRef,
    sessionId: params.sessionId,
    history: [{ phase: "PLAN", event: "entered", at }],
    overrides: [],
    artifacts: {},
  };
}

export function readRun(root: string, runId: string): Run {
  const { runJson } = runPaths(root, runId);
  if (!existsSync(runJson)) {
    throw new Error(`Run "${runId}" not found (${runJson}).`);
  }
  const parsed = JSON.parse(readFileSync(runJson, "utf8")) as Run;
  if (parsed.schema !== 1) {
    throw new Error(`Unsupported run.json schema ${String(parsed.schema)} in ${runId}.`);
  }
  return parsed;
}

export function writeRun(root: string, run: Run): void {
  const { dir, runJson } = runPaths(root, run.id);
  mkdirSync(dir, { recursive: true });
  run.updatedAt = nowIso();
  writeFileSync(runJson, JSON.stringify(run, null, 2) + "\n");
}
