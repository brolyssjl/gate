import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

/**
 * Resolve the root of a Gate project by walking up from `start` until a
 * `.gate/` directory is found. Returns null if none exists (not yet init'd).
 */
export function findGateRoot(start: string = process.cwd()): string | null {
  let dir = resolve(start);
  // Walk up to filesystem root.
  for (;;) {
    if (existsSync(join(dir, ".gate"))) return dir;
    const parent = dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

/** Paths inside a resolved Gate project root. */
export function gatePaths(root: string) {
  const gate = join(root, ".gate");
  return {
    root,
    gate,
    config: join(gate, "config.yml"),
    playbooks: join(gate, "playbooks"),
    runs: join(gate, "runs"),
    current: join(gate, "current.json"), // per-branch map of branch key -> active run id
    legacyCurrent: join(gate, "current"), // Milestone 1-3 single-run pointer; migrated on first read
    archive: join(gate, "archive"), // gate prune: summaries of pruned runs
  };
}

/** Where `gate prune` archives a run's summary once its folder is removed. */
export function archivePath(root: string, runId: string): string {
  return join(gatePaths(root).archive, `${runId}.json`);
}

export type GatePaths = ReturnType<typeof gatePaths>;

/** Paths inside a single run folder. */
export function runPaths(root: string, runId: string) {
  const dir = join(root, ".gate", "runs", runId);
  return {
    dir,
    runJson: join(dir, "run.json"),
    plan: join(dir, "plan.md"),
    /** Snapshot of plan.md as of the last approval/amendment - `gate amend` diffs the current plan against this. */
    planApproved: join(dir, "plan.approved.md"),
    worklog: join(dir, "worklog.md"),
    testReport: join(dir, "test-report.json"),
    debugLog: join(dir, "debug-log.md"),
    reviewPacket: join(dir, "review-packet.md"),
    review: join(dir, "review.md"),
    retro: join(dir, "retro.md"),
  };
}
