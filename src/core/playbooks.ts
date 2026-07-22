import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gatePaths } from "./paths.js";
import type { Phase } from "./stateMachine.js";

/** Locate the bundled default playbooks by walking up to the package root. */
export function bundledPlaybooksDir(): string {
  let dir = dirname(fileURLToPath(import.meta.url));
  for (;;) {
    const candidate = join(dir, "playbooks");
    if (existsSync(join(dir, "package.json")) && existsSync(candidate)) return candidate;
    const parent = dirname(dir);
    if (parent === dir) throw new Error("bundled playbooks directory not found");
    dir = parent;
  }
}

export function bundledPlaybookPath(phase: Phase): string {
  return join(bundledPlaybooksDir(), `${phase.toLowerCase()}.md`);
}

/**
 * Resolve the active playbook for a phase: the user's editable copy under
 * `.gate/playbooks/` wins; otherwise the bundled default. Returns null for
 * phases without a playbook (e.g. DONE).
 */
export function resolvePlaybook(root: string, phase: Phase): string | null {
  const name = `${phase.toLowerCase()}.md`;
  const userPath = join(gatePaths(root).playbooks, name);
  if (existsSync(userPath)) return readFileSync(userPath, "utf8");
  const bundled = bundledPlaybookPath(phase);
  if (existsSync(bundled)) return readFileSync(bundled, "utf8");
  return null;
}
