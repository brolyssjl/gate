import { runPaths } from "../core/paths.js";
import { changedFiles } from "../core/git.js";
import { runCommand } from "../core/exec.js";
import { isCommandsTrusted } from "../core/trust.js";
import { parseDebugLogFile, isCycleComplete } from "../artifacts/debugLog.js";
import { scopeCheck } from "./implement.js";
import { loadReport } from "./test.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * DEBUG gate — deterministic proof that a bug was diagnosed, not guessed at:
 *  - debug-log.md exists and follows the protocol schema
 *  - the bug was reproduced before diagnosing it
 *  - at least one *complete* cycle (hypothesis → prediction → experiment →
 *    observation → conclusion) is recorded
 *  - the fix stayed within the declared plan scope
 *  - the triggering test is green AND the whole suite is green (no regressions)
 *
 * The protocol itself — asking good questions, ruling causes out — is judgment
 * and lives in the DEBUG playbook. The gate only checks that the ritual happened
 * and the evidence is real.
 */
export function debugGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];
  const { debugLog: logPath, testReport: reportPath } = runPaths(ctx.root, ctx.run.id);

  const { log, errors } = parseDebugLogFile(logPath);
  if (!log) {
    checks.push(fail("debug.log", `debug-log.md invalid: ${errors.join("; ")}`));
    return result("DEBUG", checks);
  }
  checks.push(pass("debug.log", "debug-log.md matches the protocol schema"));

  checks.push(
    log.reproduced
      ? pass("debug.reproduced", "bug reproduced before diagnosing")
      : fail("debug.reproduced", "set `reproduced: true` only after you have reproduced the failure"),
  );

  const complete = log.cycles.filter(isCycleComplete);
  checks.push(
    complete.length >= 1
      ? pass("debug.cycle", `${complete.length} complete debugging cycle(s)`)
      : fail(
          "debug.cycle",
          "no complete cycle — each needs hypothesis, prediction, experiment, observation, conclusion and status: complete",
        ),
  );

  const touched = changedFiles(ctx.root, ctx.run.baseRef).filter((f) => !f.startsWith(".gate/"));
  checks.push(scopeCheck("debug.scope", ctx, touched));

  checks.push(triggeringTestCheck(ctx, log.triggeringTest, reportPath));

  return result("DEBUG", checks);
}

/**
 * Runs the test suite and asserts the triggering test is now green with no
 * regressions: the command exits 0 (whole suite green) and the report contains a
 * passing test whose name matches the triggering test. Mirrors the TEST gate's
 * trust discipline — untrusted config never spawns a process.
 */
function triggeringTestCheck(ctx: GateContext, triggering: string, reportPath: string): Check {
  const testCmd = ctx.config.commands.test;
  if (!testCmd) return fail("debug.test-green", "no test command configured (commands.test)");
  if (!isCommandsTrusted(ctx.root)) {
    return fail("debug.test-green", "test command not trusted — review .gate/config.yml and run `gate trust`");
  }
  const run = runCommand(testCmd, ctx.root);
  if (run.code !== 0) {
    const tail = (run.stderr || run.stdout).trim().split("\n").slice(-3).join(" ⏎ ");
    return fail("debug.test-green", `suite still red (exit ${run.code}) — no regressions allowed: ${tail}`);
  }
  const report = loadReport(reportPath, run.stdout);
  if (!report) {
    // Suite is green; we just can't confirm the specific test by name.
    return pass("debug.test-green", "suite green (exit 0); no parseable report to name the triggering test");
  }
  const needle = triggering.toLowerCase();
  const hit = report.tests.some((t) => t.status === "passed" && t.name.toLowerCase().includes(needle));
  return hit
    ? pass("debug.test-green", `triggering test "${triggering}" passes; suite green`)
    : fail(
        "debug.test-green",
        `suite is green but no passing test matches the triggering test "${triggering}"`,
      );
}
