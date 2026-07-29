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
 * HEAD - its stated use case); otherwise the active run for the current
 * branch (Milestone 4). A detached HEAD has no branch to resolve "current"
 * from, so it refuses with a pointer to `--run` rather than guessing which
 * run the caller meant.
 *
 * `--run` is not a bare bypass: gates measure `changedFiles(root,
 * run.baseRef)` against whatever is actually checked out, so pointing it at
 * a run whose tree doesn't match the working directory lets TEST/REVIEW pass
 * on a vacuous empty diff (100% coverage, trivially in-scope) while IMPLEMENT
 * alone is protected by its non-empty-diff check. Refuse a non-active run
 * (done/abandoned) outright, and - when the run recorded a branch - refuse a
 * checked-out branch that doesn't match it; a detached HEAD skips that branch
 * check entirely, since there's no checked-out branch to compare against.
 */
export function requireActiveRun(args?: ParsedArgs): ActiveContext {
  const root = requireRoot();
  const config = loadConfig(root);

  const explicit = args && typeof args.flags.run === "string" ? args.flags.run : undefined;
  if (explicit) {
    const run = readRun(root, explicit);
    if (run.status !== "active") {
      throw new GateError(`run "${explicit}" is ${run.status}, not active — --run only selects an in-flight run`);
    }
    const resolved = resolveBranchKey(root);
    if (run.branch && resolved.kind === "key" && resolved.key !== run.branch) {
      throw new GateError(
        `run "${explicit}" was started on branch "${run.branch}", but "${resolved.key}" is checked out — ` +
          `checkout "${run.branch}" first (or detach HEAD) to act on this run`,
      );
    }
    return { root, run, config };
  }

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
