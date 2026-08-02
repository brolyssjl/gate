import { runPaths } from "../core/paths.js";
import { changedFiles, isGateBookkeeping } from "../core/git.js";
import { runCommand } from "../core/exec.js";
import { isCommandsTrusted } from "../core/trust.js";
import { parseDebugLogFile, isCycleComplete } from "../artifacts/debugLog.js";
import { checkName, resolvePhaseTargets, type ResolvedTarget } from "../core/targets.js";
import { scopeCheck } from "./implement.js";
import { planDriftCheck } from "./planDrift.js";
import { loadReport, statReport, targetReportPath } from "./test.js";
import type { NormalizedReport } from "./testReport.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * DEBUG gate - deterministic proof that a bug was diagnosed, not guessed at:
 *  - debug-log.md exists and follows the protocol schema
 *  - reproduction is attested (`reproduced: true` - an agent claim, not proof;
 *    the mechanical proof is the triggering test below)
 *  - at least one *complete* cycle (hypothesis → prediction → experiment →
 *    observation → conclusion) is recorded
 *  - the fix stayed within the declared plan scope
 *  - the triggering test is green AND the whole suite is green (no regressions)
 *
 * The protocol itself - asking good questions, ruling causes out - is judgment
 * and lives in the DEBUG playbook. The gate only checks that the ritual happened
 * and the evidence is real.
 */
export function debugGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];
  const drift = planDriftCheck(ctx, "debug.plan-drift");
  if (drift) checks.push(drift);
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
          "no complete cycle - each needs hypothesis, prediction, experiment, observation, conclusion and status: complete",
        ),
  );

  const touched = changedFiles(ctx.root, ctx.run.baseRef).filter((f) => !isGateBookkeeping(f));
  checks.push(...scopeCheck("debug.scope", ctx, touched));

  const resolved = resolvePhaseTargets(ctx.config, ctx.run, touched);
  if (resolved.length === 1 && resolved[0]!.target === null) {
    checks.push(triggeringTestCheck(ctx, log.triggeringTest, reportPath, resolved[0]!));
  } else {
    checks.push(...targetedTriggeringTestChecks(ctx, log.triggeringTest, reportPath, resolved));
  }

  return result("DEBUG", checks);
}

/**
 * Runs the test suite and asserts the triggering test is now green with no
 * regressions: the command exits 0 (whole suite green) and the report contains a
 * passing test whose name matches the triggering test. Mirrors the TEST gate's
 * trust discipline - untrusted config never spawns a process - and its evidence
 * discipline: the report must come from the run Gate just executed, and its
 * absence fails closed (a bugfix whose fix cannot be named is not verified).
 *
 * Bare/legacy case only (no targets configured, or none affected) - reproduces
 * the pre-targets check byte-for-byte, including the "debug.test-green" name
 * and report path.
 */
function triggeringTestCheck(ctx: GateContext, triggering: string, reportPath: string, t: ResolvedTarget): Check {
  const name = checkName("debug.test-green", t.target);
  const testCmd = t.commands.test;
  if (!testCmd) return fail(name, "no test command configured (commands.test)");
  if (!isCommandsTrusted(ctx.root)) {
    return fail(name, "test command not trusted - review .gate/config.yml and run `gate trust`");
  }
  const { dir } = runPaths(ctx.root, ctx.run.id);
  const path = targetReportPath(reportPath, t.target);
  const reportBefore = statReport(path);
  const run = runCommand(testCmd, ctx.root, { GATE_RUN_DIR: dir, GATE_TEST_REPORT: path });
  if (run.code !== 0) {
    const tail = (run.stderr || run.stdout).trim().split("\n").slice(-3).join(" ⏎ ");
    return fail(name, `suite still red (exit ${run.code}) - no regressions allowed: ${tail}`);
  }
  const report = loadReport(path, run.stdout, reportBefore);
  if (!report) {
    return fail(
      name,
      "suite green, but no parseable report to confirm the triggering test - " +
        "emit a JSON report on stdout or write it to $GATE_TEST_REPORT",
    );
  }
  const needle = triggering.toLowerCase();
  const hit = report.tests.some((r) => r.status === "passed" && r.name.toLowerCase().includes(needle));
  return hit
    ? pass(name, `triggering test "${triggering}" passes; suite green`)
    : fail(name, `suite is green but no passing test matches the triggering test "${triggering}"`);
}

/**
 * ≥1 named target is affected. Each target's suite must be green - checked
 * per target so one target's regression doesn't hide behind another's pass -
 * but the triggering test's name is a property of the bugfix as a whole, not
 * of any single stack, so it's asserted once against the COMBINED report
 * across every affected target (mirrors the TEST gate's aggregate
 * test.criteria / test.no-skips over `runTargetedGate`'s combined report).
 * Before this split, each target's report was checked against the triggering
 * test in isolation, so a bugfix that only touched one target's suite could
 * never pass once a second, unrelated target was also affected - its
 * unrelated report would never contain the triggering test's name.
 */
function targetedTriggeringTestChecks(
  ctx: GateContext,
  triggering: string,
  reportPath: string,
  resolved: ResolvedTarget[],
): Check[] {
  const checks: Check[] = [];
  const reports: NormalizedReport[] = [];
  const { dir } = runPaths(ctx.root, ctx.run.id);
  const trusted = isCommandsTrusted(ctx.root);

  for (const t of resolved) {
    const name = checkName("debug.test-green", t.target);
    const testCmd = t.commands.test;
    if (!testCmd) {
      checks.push(fail(name, "no test command configured (commands.test)"));
      continue;
    }
    if (!trusted) {
      checks.push(fail(name, "test command not trusted - review .gate/config.yml and run `gate trust`"));
      continue;
    }
    const path = targetReportPath(reportPath, t.target);
    const reportBefore = statReport(path);
    const run = runCommand(testCmd, ctx.root, { GATE_RUN_DIR: dir, GATE_TEST_REPORT: path });
    if (run.code !== 0) {
      const tail = (run.stderr || run.stdout).trim().split("\n").slice(-3).join(" ⏎ ");
      checks.push(fail(name, `suite still red (exit ${run.code}) - no regressions allowed: ${tail}`));
      continue;
    }
    checks.push(pass(name, "suite green (exit 0)"));
    const report = loadReport(path, run.stdout, reportBefore);
    if (report) {
      reports.push(report);
    } else {
      checks.push(
        fail(
          checkName("debug.report", t.target),
          "suite green, but no parseable report to confirm the triggering test - " +
            "emit a JSON report on stdout or write it to $GATE_TEST_REPORT",
        ),
      );
    }
  }

  const combined: NormalizedReport | null =
    reports.length > 0
      ? { tests: reports.flatMap((r) => r.tests), skipped: reports.reduce((a, r) => a + r.skipped, 0) }
      : null;

  const needle = triggering.toLowerCase();
  if (!combined) {
    checks.push(
      fail(
        "debug.triggering-test",
        "no parseable test report across the affected targets to confirm the triggering test",
      ),
    );
  } else {
    const hit = combined.tests.some((r) => r.status === "passed" && r.name.toLowerCase().includes(needle));
    checks.push(
      hit
        ? pass("debug.triggering-test", `triggering test "${triggering}" passes across the affected targets`)
        : fail(
            "debug.triggering-test",
            `no passing test across the affected targets matches the triggering test "${triggering}"`,
          ),
    );
  }

  return checks;
}
