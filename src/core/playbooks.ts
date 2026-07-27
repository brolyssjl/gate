import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gatePaths } from "./paths.js";
import type { GateConfig } from "./config.js";
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

/**
 * The base playbook for `phase`, with one `## Target overlay: <name>` section
 * appended per affected target that declares an overlay for this phase in its
 * `config.yml` `playbooks:` map (`targets.<name>.playbooks.<phase>`, e.g.
 * `playbooks.test`). Shared discipline lives in the base playbook; stack
 * specifics live in the overlay (proposal §4.1). No affected targets, or none
 * with an overlay for this phase, returns the base playbook unchanged.
 */
export function resolvePlaybookWithOverlays(
  root: string,
  phase: Phase,
  config: GateConfig,
  targetNames: string[],
): string | null {
  const base = resolvePlaybook(root, phase);
  if (!base) return null;

  const overlays: string[] = [];
  for (const name of targetNames) {
    const overlayRel = config.targets[name]?.playbooks?.[phase.toLowerCase()];
    if (!overlayRel) continue;
    const overlayPath = join(root, overlayRel);
    if (!existsSync(overlayPath)) continue;
    const content = readFileSync(overlayPath, "utf8").trim();
    if (content) overlays.push(`## Target overlay: ${name}\n\n${content}`);
  }
  if (overlays.length === 0) return base;
  return base.trimEnd() + "\n\n" + overlays.join("\n\n") + "\n";
}
