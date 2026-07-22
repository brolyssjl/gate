import { runPaths } from "../core/paths.js";
import { changedFiles, isGitRepo } from "../core/git.js";
import { matchesAny } from "../core/glob.js";
import { runCommand } from "../core/exec.js";
import { isCommandsTrusted } from "../core/trust.js";
import { parsePlanFile } from "../artifacts/plan.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * IMPLEMENT gate — deterministic:
 *  - the working diff is non-empty
 *  - every touched file is covered by a `plan.files` glob (scope discipline)
 *  - build passes (if a build command is configured)
 *  - lint passes (if a lint command is configured)
 *
 * Changes under `.gate/` (run bookkeeping) are never counted as code touched.
 */
export function implementGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];

  if (!isGitRepo(ctx.root)) {
    checks.push(fail("implement.git", "not a git repository — cannot measure the diff"));
    return result("IMPLEMENT", checks);
  }

  const touched = changedFiles(ctx.root, ctx.run.baseRef).filter(
    (f) => !f.startsWith(".gate/"),
  );

  checks.push(
    touched.length > 0
      ? pass("implement.diff", `${touched.length} file(s) changed`)
      : fail("implement.diff", "no code changes detected since the run started"),
  );

  const { plan } = parsePlanFile(runPaths(ctx.root, ctx.run.id).plan);
  const declared = plan?.files ?? [];
  const undeclared = touched.filter((f) => !matchesAny(f, declared));
  checks.push(
    undeclared.length === 0
      ? pass("implement.scope", "all touched files are declared in plan.md")
      : fail(
          "implement.scope",
          `files touched but not declared in plan.md (add them or narrow scope): ${undeclared.join(", ")}`,
        ),
  );

  const trusted = isCommandsTrusted(ctx.root);
  checks.push(commandCheck("implement.build", ctx.config.commands.build, ctx.root, "build", trusted));
  checks.push(commandCheck("implement.lint", ctx.config.commands.lint, ctx.root, "lint", trusted));

  return result("IMPLEMENT", checks);
}

/**
 * Runs a configured command; a missing command passes with a skip note. When a
 * command is set but the commands block is untrusted, it fails *without running*
 * — untrusted config never spawns a process.
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
    return fail(name, `${label} command not trusted — review .gate/config.yml and run \`gate trust\``);
  }
  const res = runCommand(command, cwd);
  if (res.code === 0) return pass(name, `${label} passed`);
  const tail = (res.stderr || res.stdout).trim().split("\n").slice(-3).join(" ⏎ ");
  return fail(name, `${label} failed (exit ${res.code}): ${tail}`);
}
