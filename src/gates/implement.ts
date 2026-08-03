import { runPaths } from "../core/paths.js";
import { changedFiles, isGateBookkeeping, isGitRepo } from "../core/git.js";
import { matchesAny } from "../core/glob.js";
import { runCommand } from "../core/exec.js";
import { isCommandsTrusted } from "../core/trust.js";
import { parsePlanFile } from "../artifacts/plan.js";
import { checkName, resolvePhaseTargets } from "../core/targets.js";
import { planDriftCheck } from "./planDrift.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * IMPLEMENT gate - deterministic:
 *  - the working diff is non-empty
 *  - every touched file is covered by a `plan.files` glob (scope discipline)
 *  - build passes (if a build command is configured)
 *  - lint passes (if a lint command is configured)
 *
 * Changes under `.gate/` (run bookkeeping) are never counted as code touched.
 *
 * Targets (Milestone 3): with no `targets:` configured, or none affected by
 * the touched files, `resolvePhaseTargets` collapses to a single bare entry
 * using the top-level commands - byte-identical to pre-targets behavior,
 * including the check names. Once ≥1 named target is affected, each target
 * gets its own `implement.build[name]` / `implement.lint[name]` checks using
 * that target's effective commands.
 */
export function implementGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];
  const drift = planDriftCheck(ctx, "implement.plan-drift");
  if (drift) checks.push(drift);

  if (!isGitRepo(ctx.root)) {
    checks.push(fail("implement.git", "not a git repository - cannot measure the diff"));
    return result("IMPLEMENT", checks);
  }

  const touched = changedFiles(ctx.root, ctx.run.baseRef).filter((f) => !isGateBookkeeping(ctx.root, f));

  checks.push(
    touched.length > 0
      ? pass("implement.diff", `${touched.length} file(s) changed`)
      : fail("implement.diff", "no code changes detected since the run started"),
  );

  checks.push(...scopeCheck("implement.scope", ctx, touched));

  const trusted = isCommandsTrusted(ctx.root);
  for (const t of resolvePhaseTargets(ctx.config, ctx.run, touched)) {
    checks.push(commandCheck(checkName("implement.build", t.target), t.commands.build, ctx.root, "build", trusted));
    checks.push(commandCheck(checkName("implement.lint", t.target), t.commands.lint, ctx.root, "lint", trusted));
  }

  return result("IMPLEMENT", checks);
}

/**
 * Scope discipline: every touched file (outside `.gate/`) must be covered by a
 * `plan.files` glob, or by `config.scope_ignore` (Milestone 5) - environment
 * cruft (a node compile cache, a go build dir, coverage output) that isn't
 * part of the plan but shouldn't hard-block the gate either. Shared by the
 * IMPLEMENT and DEBUG gates, both of which produce a diff that must stay
 * within the declared plan.
 *
 * `scope_ignore` only takes effect when the commands block is trusted (it
 * rides in the same TOFU hash) - an untrusted edit to it never silently
 * widens what the scope check accepts. A match is always surfaced as its own
 * info-level check line, never folded silently into a passing result.
 */
export function scopeCheck(name: string, ctx: GateContext, touched: string[]): Check[] {
  const { plan } = parsePlanFile(runPaths(ctx.root, ctx.run.id).plan);
  const declared = plan?.files ?? [];
  const ignoreGlobs = ctx.config.scope_ignore;
  const trusted = isCommandsTrusted(ctx.root);

  const outOfPlan = touched.filter((f) => !matchesAny(f, declared));
  const ignored = trusted ? outOfPlan.filter((f) => matchesAny(f, ignoreGlobs)) : [];
  const undeclared = outOfPlan.filter((f) => !ignored.includes(f));

  const checks: Check[] = [];
  if (undeclared.length === 0) {
    checks.push(pass(name, "all touched files are declared in plan.md"));
  } else {
    const untrustedHint =
      !trusted && ignoreGlobs.length > 0 && outOfPlan.some((f) => matchesAny(f, ignoreGlobs))
        ? " (scope_ignore is configured but untrusted - run `gate trust` to apply it)"
        : "";
    checks.push(
      fail(
        name,
        `files touched but not declared in plan.md (add them or narrow scope): ${undeclared.join(", ")}${untrustedHint}`,
      ),
    );
  }
  if (ignored.length > 0) {
    checks.push(pass(`${name}.ignored`, `matched scope_ignore, excluded from the scope check: ${ignored.join(", ")}`));
  }
  return checks;
}

/**
 * Runs a configured command; a missing command passes with a skip note. When a
 * command is set but the commands block is untrusted, it fails *without running*
 * - untrusted config never spawns a process.
 */
export function commandCheck(
  name: string,
  command: string | undefined,
  cwd: string,
  label: string,
  trusted: boolean,
): Check {
  if (!command) return pass(name, `no ${label} command configured (skipped)`);
  if (!trusted) {
    return fail(name, `${label} command not trusted - review .gate/config.yml and run \`gate trust\``);
  }
  const res = runCommand(command, cwd);
  if (res.code === 0) return pass(name, `${label} passed`);
  const tail = (res.stderr || res.stdout).trim().split("\n").slice(-3).join(" ⏎ ");
  return fail(name, `${label} failed (exit ${res.code}): ${tail}`);
}
