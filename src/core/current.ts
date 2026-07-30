import { closeSync, existsSync, openSync, readFileSync, rmSync, unlinkSync } from "node:fs";
import { writeFileAtomic } from "./fsx.js";
import { gatePaths } from "./paths.js";
import { branchState } from "./git.js";
import { readRun, writeRun } from "./run.js";
import { GateError } from "../cli/output.js";

/**
 * The active run is tracked per branch (Milestone 4): `.gate/current.json`
 * maps a branch key to its active run id, so concurrent work-in-progress on
 * separate branches doesn't collide (proposal open question: "runs keyed by
 * branch name"). Milestone 1-3 tracked a single global run in a plain-text
 * `.gate/current` file; that file is migrated in place the first time it's
 * read after upgrade (see `migrateLegacy`).
 *
 * The milestone's own use case - multiple agents in flight at once - means
 * `current.json` is a file two `gate` processes can genuinely race to
 * read-modify-write at the same time. Every mutation goes through
 * `withCurrentLock`, an O_EXCL lockfile with retry, so "read, decide, write"
 * is one atomic-with-respect-to-other-processes step instead of racing.
 */

/**
 * Sentinel key for repos with no git branch concept at all (not a git repo -
 * `branchState` returns `"none"`). Contains `~`, a character forbidden in
 * real git ref names, so it can never collide with an actual branch.
 */
export const NO_GIT_BRANCH_KEY = "~no-git~";

export type BranchKeyResolution = { kind: "key"; key: string } | { kind: "detached" };

/**
 * Resolve the branch key `current.json` is keyed by for this working tree.
 * Detached HEAD is genuinely ambiguous - it returns `{ kind: "detached" }` so
 * callers can require an explicit `--run` instead of guessing.
 */
export function resolveBranchKey(root: string): BranchKeyResolution {
  const state = branchState(root);
  if (state.kind === "detached") return { kind: "detached" };
  return { kind: "key", key: state.kind === "branch" ? state.name : NO_GIT_BRANCH_KEY };
}

interface CurrentState {
  schema: 1;
  branches: Record<string, string>;
}

const LOCK_RETRY_MS = 20;
const LOCK_TIMEOUT_MS = 5000;

/**
 * Acquire an exclusive lock on `current.json` via an `O_EXCL`-created
 * lockfile (atomic create-if-absent at the filesystem level), retrying with
 * a short blocking sleep until another process's lock is released or the
 * timeout is hit. Returns a release function; callers must call it exactly
 * once, in a `finally`.
 */
function acquireLock(root: string): () => void {
  const lockPath = gatePaths(root).current + ".lock";
  const deadline = Date.now() + LOCK_TIMEOUT_MS;
  for (;;) {
    try {
      closeSync(openSync(lockPath, "wx"));
      return () => {
        try {
          unlinkSync(lockPath);
        } catch {
          // Already gone - nothing left to release.
        }
      };
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== "EEXIST") throw err;
      if (Date.now() > deadline) {
        throw new GateError(
          `timed out waiting for the lock on ${lockPath} - remove it manually if you're sure no other gate process is running`,
        );
      }
      blockingSleep(LOCK_RETRY_MS);
    }
  }
}

/** A synchronous sleep - Gate is a short-lived CLI with no event loop to usefully yield to while waiting on another process's lock. */
function blockingSleep(ms: number): void {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function readState(root: string): CurrentState {
  migrateLegacy(root);
  const { current } = gatePaths(root);
  if (!existsSync(current)) return { schema: 1, branches: {} };
  let parsed: Partial<CurrentState>;
  try {
    parsed = JSON.parse(readFileSync(current, "utf8")) as Partial<CurrentState>;
  } catch (err) {
    // Fail closed: silently treating corrupt state as "no active run
    // anywhere" would let the next write quietly discard every branch's
    // mapping. A human needs to look at the file.
    throw new GateError(`${current} is corrupt (${(err as Error).message}) - fix or remove it by hand`);
  }
  const branches = parsed.branches && typeof parsed.branches === "object" ? parsed.branches : {};
  return { schema: 1, branches };
}

function writeState(root: string, state: CurrentState): void {
  writeFileAtomic(gatePaths(root).current, JSON.stringify(state, null, 2) + "\n");
}

/**
 * One-time migration: the old single-run `.gate/current` file maps onto
 * whatever branch is checked out right now (best-effort - Gate has no record
 * of which branch a pre-Milestone-4 run actually started on), or the "no git
 * branch" sentinel when that can't be determined (detached HEAD or no repo
 * at migration time). The migrated run's own `branch` field is backfilled to
 * match (when it's still null, i.e. its schema-2->3 migration couldn't infer
 * it) - belt-and-suspenders with `clearRunEverywhere`, which doesn't trust
 * `run.branch` for exactly this reason, but a correct record is still worth
 * having for `gate status`'s display and future lookups.
 */
function migrateLegacy(root: string): void {
  const { legacyCurrent, current } = gatePaths(root);
  if (existsSync(current) || !existsSync(legacyCurrent)) return;
  const runId = readFileSync(legacyCurrent, "utf8").trim();
  if (runId) {
    const resolved = resolveBranchKey(root);
    const key = resolved.kind === "key" ? resolved.key : NO_GIT_BRANCH_KEY;
    writeState(root, { schema: 1, branches: { [key]: runId } });
    backfillMigratedBranch(root, runId, key);
  }
  // force: a second gate invocation racing through this same one-time
  // migration can pass the existsSync guard above right before the first
  // process's rmSync runs; without force, the second process's rmSync would
  // throw ENOENT on a file that's already gone.
  rmSync(legacyCurrent, { force: true });
}

function backfillMigratedBranch(root: string, runId: string, key: string): void {
  try {
    const run = readRun(root, runId);
    if (run.branch == null) {
      run.branch = key === NO_GIT_BRANCH_KEY ? null : key;
      writeRun(root, run);
    }
  } catch {
    // Run unreadable or missing - nothing to backfill; the current.json
    // mapping itself is still migrated above regardless.
  }
}

export function readCurrentRunId(root: string, key: string): string | null {
  return readState(root).branches[key] ?? null;
}

/**
 * The current branch's mapped run id, or null - including on a detached HEAD
 * (nothing "current" to resolve a key from). Convenience for callers that
 * only want "whatever's current, if anything" and don't otherwise need the
 * resolved branch key or the detached/key distinction; callers that do
 * (e.g. `gate status`, or anything that must react differently to a
 * detached HEAD) should call `resolveBranchKey` themselves instead of this,
 * to avoid resolving it twice.
 */
export function currentRunIdOrNull(root: string): string | null {
  const resolved = resolveBranchKey(root);
  return resolved.kind === "key" ? readCurrentRunId(root, resolved.key) : null;
}

/**
 * Run `fn` with exclusive access to `current.json`'s branch map: locks,
 * reads (fail-closed on corruption, migrating the legacy pointer first),
 * lets `fn` mutate the map in place and return a result, then persists and
 * unlocks - all as one critical section. `fn` may also perform other,
 * unrelated filesystem work (e.g. `gate start` creating the new run's
 * `run.json`) before deciding how to mutate the map; a thrown error skips
 * the write entirely (no partial state persisted) but the lock is always
 * released. This is the single load-mutate-persist path `setCurrentRunId`/
 * `clearCurrentRunId` build on, and what `gate start` uses directly so its
 * whole "read existing -> decide resume/create -> point at it" sequence is
 * one lock window instead of three separate ones a concurrent process could
 * interleave with.
 */
export function withCurrentLock<T>(root: string, fn: (branches: Record<string, string>) => T): T {
  const release = acquireLock(root);
  try {
    const state = readState(root);
    const result = fn(state.branches);
    writeState(root, state);
    return result;
  } finally {
    release();
  }
}

export function setCurrentRunId(root: string, key: string, runId: string): void {
  withCurrentLock(root, (branches) => {
    branches[key] = runId;
  });
}

export function clearCurrentRunId(root: string, key: string): void {
  withCurrentLock(root, (branches) => {
    delete branches[key];
  });
}

/**
 * Remove every mapping pointing at `runId`, regardless of which branch key
 * it's filed under - used when a run reaches a terminal phase. Deliberately
 * does not trust `run.branch` to compute the one key to clear: a run's own
 * branch record can be stale or never backfilled (a schema-2 run migrated
 * from the legacy single-run pointer before `migrateLegacy`'s backfill
 * existed, for instance), in which case clearing by the wrong key leaves the
 * real mapping - and the finished run - resolvable forever. The mapping
 * itself, not the run's memory of its own branch, is the source of truth for
 * where it's filed.
 */
export function clearRunEverywhere(root: string, runId: string): void {
  withCurrentLock(root, (branches) => {
    for (const [key, id] of Object.entries(branches)) {
      if (id === runId) delete branches[key];
    }
  });
}

/** Every branch with an active-run mapping, for `gate status`'s "other in-flight runs" list. */
export function listActiveBranches(root: string): Array<{ key: string; runId: string }> {
  return Object.entries(readState(root).branches).map(([key, runId]) => ({ key, runId }));
}
