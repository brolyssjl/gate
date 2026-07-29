import { existsSync, readFileSync, rmSync } from "node:fs";
import { writeFileAtomic } from "./fsx.js";
import { gatePaths } from "./paths.js";
import { branchState } from "./git.js";

/**
 * The active run is tracked per branch (Milestone 4): `.gate/current.json`
 * maps a branch key to its active run id, so concurrent work-in-progress on
 * separate branches doesn't collide (proposal open question: "runs keyed by
 * branch name"). Milestone 1-3 tracked a single global run in a plain-text
 * `.gate/current` file; that file is migrated in place the first time it's
 * read after upgrade (see `migrateLegacy`).
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

const EMPTY_STATE: CurrentState = { schema: 1, branches: {} };

function readState(root: string): CurrentState {
  migrateLegacy(root);
  const { current } = gatePaths(root);
  if (!existsSync(current)) return { ...EMPTY_STATE, branches: {} };
  try {
    const parsed = JSON.parse(readFileSync(current, "utf8")) as Partial<CurrentState>;
    const branches = parsed.branches && typeof parsed.branches === "object" ? parsed.branches : {};
    return { schema: 1, branches };
  } catch {
    // Corrupt current.json must not wedge every command; fail open to "no
    // active run anywhere" rather than crash the CLI on its hottest path.
    return { ...EMPTY_STATE, branches: {} };
  }
}

function writeState(root: string, state: CurrentState): void {
  writeFileAtomic(gatePaths(root).current, JSON.stringify(state, null, 2) + "\n");
}

/**
 * One-time migration: the old single-run `.gate/current` file maps onto
 * whatever branch is checked out right now (best-effort - Gate has no record
 * of which branch a pre-Milestone-4 run actually started on), or the "no git
 * branch" sentinel when that can't be determined (detached HEAD or no repo
 * at migration time).
 */
function migrateLegacy(root: string): void {
  const { legacyCurrent, current } = gatePaths(root);
  if (existsSync(current) || !existsSync(legacyCurrent)) return;
  const runId = readFileSync(legacyCurrent, "utf8").trim();
  if (runId) {
    const resolved = resolveBranchKey(root);
    const key = resolved.kind === "key" ? resolved.key : NO_GIT_BRANCH_KEY;
    writeState(root, { schema: 1, branches: { [key]: runId } });
  }
  rmSync(legacyCurrent);
}

export function readCurrentRunId(root: string, key: string): string | null {
  return readState(root).branches[key] ?? null;
}

export function setCurrentRunId(root: string, key: string, runId: string): void {
  const state = readState(root);
  state.branches[key] = runId;
  writeState(root, state);
}

export function clearCurrentRunId(root: string, key: string): void {
  const state = readState(root);
  if (key in state.branches) {
    delete state.branches[key];
    writeState(root, state);
  }
}

/** Every branch with an active-run mapping, for `gate status`'s "other in-flight runs" list. */
export function listActiveBranches(root: string): Array<{ key: string; runId: string }> {
  return Object.entries(readState(root).branches).map(([key, runId]) => ({ key, runId }));
}
