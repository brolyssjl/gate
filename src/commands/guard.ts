import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { matchesAny } from "../core/glob.js";
import { gitHooksDir, stagedFiles } from "../core/git.js";
import { NO_GIT_BRANCH_KEY, readCurrentRunId, resolveBranchKey } from "../core/current.js";
import { runPaths } from "../core/paths.js";
import { readRun, type Run } from "../core/run.js";
import { parsePlanFile } from "../artifacts/plan.js";
import { emit, GateError, requireRoot, UsageError, type ParsedArgs } from "./shared.js";

/**
 * `gate guard` - an opt-in `.git/hooks/pre-commit` hook (Milestone 4). Never
 * installed by `init` or any other command - only the explicit `gate guard
 * install` writes it, and only `gate guard uninstall` removes it. Advisory by
 * design: the hook runs only cheap deterministic checks (never a build/test
 * command), and is always escapable (`git commit --no-verify`, `GATE_GUARD=0`)
 * - a local git hook is not the enforcement boundary, `gate check`/`gate next`
 * are.
 */
export function cmdGuard(args: ParsedArgs): void {
  const root = requireRoot();
  const sub = args.positionals[0];
  if (sub === "install") return guardInstall(root, args);
  if (sub === "uninstall") return guardUninstall(root, args);
  if (sub === "run") return guardRun(root, args);
  throw new UsageError("gate guard needs a subcommand: install | uninstall | run");
}

const MARKER = "# gate-guard: v1";

function hookScript(chained: boolean): string {
  const lines = [
    "#!/bin/sh",
    MARKER,
    "# Installed by `gate guard install` - advisory only, never load-bearing.",
    "# Skip once: git commit --no-verify. Disable just the gate check: GATE_GUARD=0",
    "# (a chained pre-existing hook, if any, still runs either way).",
    'if [ "$GATE_GUARD" != "0" ]; then',
    "  if command -v gate >/dev/null 2>&1; then",
    "    gate guard run",
    "    status=$?",
    "    if [ $status -ne 0 ]; then",
    "      exit $status",
    "    fi",
    "  else",
    '    echo "gate guard: \\`gate\\` not found on PATH - skipping (advisory only)" >&2',
    "  fi",
    "fi",
  ];
  if (chained) {
    lines.push('exec "$(dirname "$0")/pre-commit.gate-backup" "$@"');
  }
  lines.push("exit 0", "");
  return lines.join("\n");
}

function guardInstall(root: string, args: ParsedArgs): void {
  const hooksDir = gitHooksDir(root);
  if (!hooksDir) throw new GateError("not a git repository (or no git hooks directory) - gate guard needs git");
  mkdirSync(hooksDir, { recursive: true });
  const hookPath = join(hooksDir, "pre-commit");
  const backupPath = join(hooksDir, "pre-commit.gate-backup");

  if (existsSync(hookPath)) {
    const existing = readFileSync(hookPath, "utf8");
    if (existing.includes(MARKER)) {
      emit("gate guard is already installed.", { installed: true, alreadyInstalled: true, chained: false }, args.flags);
      return;
    }
    if (existsSync(backupPath)) {
      throw new GateError(
        `refusing to overwrite - a backup already exists at ${backupPath}; ` +
          "resolve it manually (a previous install may have failed partway through)",
      );
    }
    renameSync(hookPath, backupPath);
    writeFileSync(hookPath, hookScript(true));
    chmodSync(hookPath, 0o755);
    emit(
      [
        "Installed .git/hooks/pre-commit",
        `Chained the existing hook (backed up to ${backupPath}).`,
        "Advisory only: bypass with `git commit --no-verify` or `GATE_GUARD=0`.",
      ].join("\n"),
      { installed: true, alreadyInstalled: false, chained: true },
      args.flags,
    );
    return;
  }

  writeFileSync(hookPath, hookScript(false));
  chmodSync(hookPath, 0o755);
  emit(
    [
      "Installed .git/hooks/pre-commit",
      "Advisory only: bypass with `git commit --no-verify` or `GATE_GUARD=0`.",
    ].join("\n"),
    { installed: true, alreadyInstalled: false, chained: false },
    args.flags,
  );
}

function guardUninstall(root: string, args: ParsedArgs): void {
  const hooksDir = gitHooksDir(root);
  if (!hooksDir) throw new GateError("not a git repository (or no git hooks directory)");
  const hookPath = join(hooksDir, "pre-commit");
  const backupPath = join(hooksDir, "pre-commit.gate-backup");

  if (!existsSync(hookPath)) {
    emit("Nothing to uninstall - no .git/hooks/pre-commit.", { uninstalled: false, restored: false }, args.flags);
    return;
  }
  const existing = readFileSync(hookPath, "utf8");
  if (!existing.includes(MARKER)) {
    throw new GateError(
      ".git/hooks/pre-commit wasn't installed by `gate guard` - remove it manually if you want it gone",
    );
  }
  if (existsSync(backupPath)) {
    renameSync(backupPath, hookPath);
    chmodSync(hookPath, 0o755);
    emit("Restored the pre-existing pre-commit hook.", { uninstalled: true, restored: true }, args.flags);
    return;
  }
  rmSync(hookPath);
  emit("Removed .git/hooks/pre-commit.", { uninstalled: true, restored: false }, args.flags);
}

export interface GuardResult {
  ok: boolean;
  reasons: string[];
}

/**
 * The hook's actual checks - cheap and deterministic only, never a build/test
 * command: an active run exists for the branch, staged files stay within the
 * plan's declared scope, and the run isn't still in PLAN (nothing should be
 * committed before IMPLEMENT starts). Exported so `gate guard run` and its
 * tests share the exact same logic the installed hook executes.
 */
export function runGuardChecks(root: string): GuardResult {
  const resolved = resolveBranchKey(root);
  if (resolved.kind === "detached") {
    return { ok: true, reasons: ["HEAD is detached - gate guard has no branch to check against (advisory pass)"] };
  }

  const id = readCurrentRunId(root, resolved.key);
  if (!id) {
    const where = resolved.key === NO_GIT_BRANCH_KEY ? "" : ` on branch "${resolved.key}"`;
    return {
      ok: false,
      reasons: [`no active run${where} - start one with \`gate start "<title>"\` or bypass with --no-verify`],
    };
  }

  let run: Run;
  try {
    run = readRun(root, id);
  } catch {
    return { ok: false, reasons: [`active run "${id}" has an unreadable run.json`] };
  }

  const reasons: string[] = [];
  if (run.phase === "PLAN") {
    reasons.push(`run "${id}" is still in PLAN - nothing should be committed until IMPLEMENT starts`);
  }

  const staged = stagedFiles(root);
  const { plan } = parsePlanFile(runPaths(root, id).plan);
  const declared = plan?.files ?? [];
  const undeclared = staged.filter((f) => !matchesAny(f, declared));
  if (undeclared.length > 0) {
    reasons.push(`staged files outside the declared plan scope: ${undeclared.join(", ")}`);
  }

  return { ok: reasons.length === 0, reasons };
}

function guardRun(root: string, args: ParsedArgs): void {
  const res = runGuardChecks(root);
  const human = res.ok
    ? res.reasons.length
      ? `gate guard: ok (${res.reasons.join("; ")})`
      : "gate guard: ok"
    : [
        "gate guard: blocked",
        ...res.reasons.map((r) => `  - ${r}`),
        "",
        "Bypass: `git commit --no-verify`, or `GATE_GUARD=0 git commit`.",
      ].join("\n");
  emit(human, { ok: res.ok, reasons: res.reasons }, args.flags);
  process.exitCode = res.ok ? 0 : 1;
}
