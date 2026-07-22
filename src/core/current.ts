import { existsSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { gatePaths } from "./paths.js";

/**
 * The active run is tracked by a single `.gate/current` file holding the run id.
 * Milestone 1 supports one active run at a time; branch-keyed concurrency is a
 * later milestone (proposal open question 2).
 */
export function readCurrentRunId(root: string): string | null {
  const { current } = gatePaths(root);
  if (!existsSync(current)) return null;
  const id = readFileSync(current, "utf8").trim();
  return id.length > 0 ? id : null;
}

export function setCurrentRunId(root: string, runId: string): void {
  writeFileSync(gatePaths(root).current, runId + "\n");
}

export function clearCurrentRunId(root: string): void {
  const { current } = gatePaths(root);
  if (existsSync(current)) rmSync(current);
}
