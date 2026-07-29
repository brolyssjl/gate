import { loadConfig, type GateConfig } from "../core/config.js";
import { NO_GIT_BRANCH_KEY, readCurrentRunId, resolveBranchKey } from "../core/current.js";
import { findGateRoot } from "../core/paths.js";
import { readRun, type Run } from "../core/run.js";
import type { ParsedArgs } from "./args.js";
import { GateError } from "./output.js";

export function requireRoot(): string {
  const root = findGateRoot();
  if (!root) throw new GateError("no .gate/ found — run `gate init` first");
  return root;
}

export interface ActiveContext {
  root: string;
  run: Run;
  config: GateConfig;
}

/**
 * Resolve the run a phase-scoped command (check/next/review/retro/skip/log/
 * playbook) operates on: `--run <id>` always wins (works even in detached
 * HEAD); otherwise the active run for the current branch (Milestone 4). A
 * detached HEAD has no branch to resolve "current" from, so it refuses with a
 * pointer to `--run` rather than guessing which run the caller meant.
 */
export function requireActiveRun(args?: ParsedArgs): ActiveContext {
  const root = requireRoot();
  const config = loadConfig(root);

  const explicit = args && typeof args.flags.run === "string" ? args.flags.run : undefined;
  if (explicit) return { root, run: readRun(root, explicit), config };

  const resolved = resolveBranchKey(root);
  if (resolved.kind === "detached") {
    throw new GateError(
      "HEAD is detached — no branch to resolve the active run from; pass --run <id> (see `gate status` for runs in flight)",
    );
  }
  const id = readCurrentRunId(root, resolved.key);
  if (!id) {
    throw new GateError(
      resolved.key === NO_GIT_BRANCH_KEY
        ? "no active run — start one with `gate start \"<title>\"`"
        : `no active run on branch "${resolved.key}" — start one with \`gate start "<title>"\``,
    );
  }
  return { root, run: readRun(root, id), config };
}
