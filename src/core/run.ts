import { existsSync, readFileSync } from "node:fs";
import { writeFileAtomic } from "./fsx.js";
import { runPaths } from "./paths.js";
import type { Phase } from "./stateMachine.js";

export type RunStatus = "active" | "done" | "abandoned";

export interface HistoryEntry {
  phase: Phase;
  event: "entered" | "passed" | "failed" | "skipped";
  at: string;
  /** For failed/passed: short summary of the gate verdict. */
  detail?: string;
  /**
   * For passed: fingerprint of the working tree the gate certified. The REVIEW
   * gate compares this against the current tree to detect code that changed
   * after its evidence was gathered (staleness).
   */
  treeHash?: string;
}

export interface OverrideEntry {
  phase: Phase;
  action: "skip";
  reason: string;
  at: string;
  /** Who authorized (from --by or GATE_SESSION_ID); best-effort, not proof of a human. */
  by?: string | null;
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

/**
 * Recorded by `gate amend` (Milestone 5): intent to re-approve a plan that
 * drifted from its approved hash. A separate, deliberate step from `gate
 * approve --amend` itself - the same discipline that keeps approval from
 * being a one-step self-sign-off applies to re-approving a delta: a human
 * must have seen the diff (`gate amend` prints it) before the drifted plan
 * can be re-approved. Cleared once `gate approve --amend` re-approves.
 */
export interface Amendment {
  /** Hash of plan.md when `gate amend` last recorded intent to amend. */
  planHash: string;
  at: string;
  /** Who recorded the amendment (from --by or GATE_SESSION_ID); best-effort. */
  by: string | null;
}

/**
 * Journal-sync receipt written by `gate retro` (Milestone 3, additive). Absent
 * until `gate retro` runs; the RETRO gate's `retro.journal` check reads it to
 * confirm the sync actually landed, rather than trusting a bare claim.
 */
export interface RetroSync {
  /** Repo-relative path to the `.agnosgram/journal/*.md` file the entry landed in. */
  journalFile: string;
  syncedAt: string;
  /** "agnosgram-cli" when `agnosgram log --stdin` ran; "fallback" on ENOENT direct-append. */
  method: "agnosgram-cli" | "fallback";
}

export interface ReviewRequest {
  /**
   * Who emitted the packet (from --by or GATE_SESSION_ID); audit only. The
   * reviewer of record is the `reviewer:` field the reviewer writes into
   * review.md - identity is claimed at sign-off time, not at packet time.
   */
  requestedBy: string | null;
  requestedAt: string;
  /**
   * Fingerprint of the working tree the packet was generated from. The REVIEW
   * gate refuses to pass while the current tree differs - a reviewer must have
   * seen the code that actually ships.
   */
  treeHash: string | null;
}

export interface Run {
  /** Schema version of this run.json, for forward migration. */
  schema: 4;
  id: string;
  title: string;
  profile: string;
  phase: Phase;
  status: RunStatus;
  createdAt: string;
  updatedAt: string;
  /**
   * Branch this run was started on (Milestone 4, additive); null when there
   * was no branch to record (not a git repo, or detached HEAD at start time).
   * Runs are keyed by branch for concurrency (`core/current.ts`) - this field
   * is the audit trail of that key, not the key itself.
   */
  branch: string | null;
  /** git ref (sha) captured at `gate start`; the IMPLEMENT diff is measured from here. */
  baseRef: string | null;
  /** Explicit `gate start --target` override (Milestone 3, additive); wins over file-based resolution. */
  targetOverride?: string[];
  /** Session id of the implementer, when the agent supplies one (GATE_SESSION_ID). */
  sessionId: string | null;
  history: HistoryEntry[];
  overrides: OverrideEntry[];
  /** Registered artifacts keyed by relative filename within the run folder. */
  artifacts: Record<string, ArtifactEntry>;
  /** PLAN approval, recorded by `gate approve` (absent until approved). */
  approval?: Approval;
  /** Pending re-approval intent, recorded by `gate amend` (absent unless the plan has drifted and `gate amend` ran). */
  amendment?: Amendment;
  /** REVIEW packet request, recorded by `gate review` (absent until requested). */
  review?: ReviewRequest;
  /** RETRO journal-sync receipt, recorded by `gate retro` (absent until synced). */
  retro?: RetroSync;
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
  branch?: string | null;
  baseRef: string | null;
  sessionId: string | null;
  targetOverride?: string[];
}): Run {
  const at = nowIso();
  return {
    schema: 4,
    id: params.id,
    title: params.title,
    profile: params.profile,
    phase: "PLAN",
    status: "active",
    createdAt: at,
    updatedAt: at,
    branch: params.branch ?? null,
    baseRef: params.baseRef,
    ...(params.targetOverride && params.targetOverride.length > 0 ? { targetOverride: params.targetOverride } : {}),
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
  const parsed = JSON.parse(readFileSync(runJson, "utf8")) as Run & { schema: number };
  return migrateRun(parsed, runId);
}

/**
 * Migrate older run.json schemas in place. Schema 1 (Milestone 1) predates
 * profiles and review requests: it gains `profile: "feature"` - deliberate:
 * a legacy run past TEST now owes the REVIEW phase the feature profile added -
 * and any schema-1 `review.reviewer` becomes `requestedBy`. Schema 2
 * (Milestone 1-3) predates branch-keyed concurrency: it gains `branch: null` -
 * honest, not a guess; Gate has no record of which branch a pre-Milestone-4
 * run started on (contrast `core/current.ts`'s legacy-pointer migration, which
 * *can* infer a key from the branch checked out at migration time). Schema 3
 * (Milestone 1-4) predates amend/drift tracking (Milestone 5): `amendment` is
 * optional and simply absent on a migrated run, so no field needs
 * backfilling - just the version bump. The migrated run is persisted on the
 * next writeRun.
 */
function migrateRun(raw: Run & { schema: number }, runId: string): Run {
  const parsed = raw as Omit<Run, "schema"> & { schema: number };
  if (parsed.schema === 1) {
    parsed.profile = parsed.profile ?? "feature";
    const legacy = parsed.review as (ReviewRequest & { reviewer?: string | null }) | undefined;
    if (legacy) {
      parsed.review = {
        requestedBy: legacy.requestedBy ?? legacy.reviewer ?? null,
        requestedAt: legacy.requestedAt,
        treeHash: legacy.treeHash ?? null,
      };
    }
    parsed.schema = 2;
  }
  if (parsed.schema === 2) {
    parsed.branch = parsed.branch ?? null;
    parsed.schema = 3;
  }
  if (parsed.schema === 3) {
    parsed.schema = 4;
  }
  if (parsed.schema !== 4) {
    throw new Error(`Unsupported run.json schema ${String(parsed.schema)} in ${runId}.`);
  }
  return parsed as Run;
}

export function writeRun(root: string, run: Run): void {
  const { runJson } = runPaths(root, run.id);
  run.updatedAt = nowIso();
  // Atomic: a crash mid-write must never corrupt the run (all state on disk).
  writeFileAtomic(runJson, JSON.stringify(run, null, 2) + "\n");
}
