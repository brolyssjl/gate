import { existsSync, readFileSync, statSync } from "node:fs";
import { runPaths } from "../core/paths.js";
import { changedLines } from "../core/git.js";
import { runCommand } from "../core/exec.js";
import { writeFileAtomic } from "../core/fsx.js";
import { isCommandsTrusted } from "../core/trust.js";
import { parsePlanFile } from "../artifacts/plan.js";
import { parseTestReport, type NormalizedReport } from "./testReport.js";
import { diffCoverage, loadCoverage } from "./coverage.js";
import { commandCheck } from "./implement.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * TEST gate — deterministic:
 *  - `build` and `lint` exit 0 (parity with IMPLEMENT; the bugfix profile has
 *    no IMPLEMENT phase, so this is where its build/lint discipline lives)
 *  - the test command exits 0 (no green claim over a red suite)
 *  - every `test:`-verified acceptance criterion maps to a named passing test
 *  - no skipped tests
 *  - diff coverage ≥ threshold (only when coverage command + threshold are set)
 *
 * Machine evidence is produced by Gate itself: the report comes from the test
 * command Gate just ran (stdout, or a file that command wrote), never from a
 * file an agent staged in advance.
 */
export function testGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];
  const { dir, testReport: reportPath } = runPaths(ctx.root, ctx.run.id);

  const testCmd = ctx.config.commands.test;
  if (!testCmd) {
    checks.push(fail("test.command", "no test command configured (commands.test)"));
    return result("TEST", checks);
  }
  // Untrusted config never spawns a process (proposal §9).
  if (!isCommandsTrusted(ctx.root)) {
    checks.push(
      fail("test.command", "test command not trusted — review .gate/config.yml and run `gate trust`"),
    );
    return result("TEST", checks);
  }

  // 1. Build + lint. Redundant for the feature profile (IMPLEMENT ran them on
  //    an older tree) but load-bearing for bugfix, which never walks IMPLEMENT.
  checks.push(commandCheck("test.build", ctx.config.commands.build, ctx.root, "build", true));
  checks.push(commandCheck("test.lint", ctx.config.commands.lint, ctx.root, "lint", true));

  // 2. Test command exit code — the load-bearing check.
  const reportBefore = statReport(reportPath);
  const run = runCommand(testCmd, ctx.root, { GATE_RUN_DIR: dir, GATE_TEST_REPORT: reportPath });
  const stdout = run.stdout;
  checks.push(
    run.code === 0
      ? pass("test.command", "test suite passed (exit 0)")
      : fail(
          "test.command",
          `test suite failed (exit ${run.code}): ${(run.stderr || run.stdout).trim().split("\n").slice(-3).join(" ⏎ ")}`,
        ),
  );

  // 3. Normalized report from the run Gate just executed.
  const report = loadReport(reportPath, stdout, reportBefore);

  // 4. Criterion → test mapping.
  const { plan } = parsePlanFile(runPaths(ctx.root, ctx.run.id).plan);
  const testCriteria = (plan?.criteria ?? []).filter((c) => c.verify.toLowerCase().startsWith("test:"));
  if (testCriteria.length === 0) {
    checks.push(pass("test.criteria", "no test-verified criteria to map"));
  } else if (!report) {
    checks.push(
      fail("test.criteria", "cannot verify criterion→test mapping: no parseable test report found"),
    );
  } else {
    const passing = report.tests.filter((t) => t.status === "passed").map((t) => t.name.toLowerCase());
    const unmapped = testCriteria.filter((c) => {
      const needle = c.verify.slice(c.verify.indexOf(":") + 1).trim().toLowerCase();
      return !passing.some((name) => name.includes(needle));
    });
    checks.push(
      unmapped.length === 0
        ? pass("test.criteria", `all ${testCriteria.length} test-verified criteria map to a passing test`)
        : fail(
            "test.criteria",
            `criteria without a passing named test: ${unmapped.map((c) => `${c.id} (${c.verify})`).join(", ")}`,
          ),
    );
  }

  // 5. No skipped tests.
  if (report) {
    checks.push(
      report.skipped === 0
        ? pass("test.no-skips", "no skipped tests")
        : fail("test.no-skips", `${report.skipped} skipped test(s); un-skip or split into a separate run`),
    );
  }

  // 6. Diff coverage (only when configured).
  const threshold = ctx.config.thresholds.diff_coverage;
  if (threshold !== undefined) {
    checks.push(coverageCheck(ctx, threshold));
  }

  return result("TEST", checks);
}

/** Identity of a report file's on-disk state, for was-it-rewritten detection. */
export interface ReportStat {
  mtimeMs: number;
  size: number;
}

export function statReport(reportPath: string): ReportStat | null {
  if (!existsSync(reportPath)) return null;
  const s = statSync(reportPath);
  return { mtimeMs: s.mtimeMs, size: s.size };
}

/**
 * Normalized report for the test run Gate just executed. Evidence integrity:
 * if the agent could hand Gate the report, enforcement would be fiction, so
 * the sources are, in order:
 *  1. the command's stdout (captured by Gate itself) — Gate then persists the
 *     normalized report to `test-report.json` for the audit trail;
 *  2. a `test-report.json` the command wrote *during this run* (runners can
 *     target it via $GATE_TEST_REPORT), detected by the file's stat changing
 *     across the run (`before` is the caller's stat from just before spawning).
 * A pre-existing file staged before the run (e.g. via `gate log`) is ignored.
 * Returns null when neither source yields a parseable report.
 */
export function loadReport(reportPath: string, stdout: string, before: ReportStat | null): NormalizedReport | null {
  const fromStdout = parseTestReport(stdout);
  if (fromStdout) {
    writeFileAtomic(
      reportPath,
      JSON.stringify(
        { source: "gate: normalized from the test command's stdout", tests: fromStdout.tests },
        null,
        2,
      ) + "\n",
    );
    return fromStdout;
  }
  const after = statReport(reportPath);
  const writtenDuringRun =
    after !== null && (before === null || after.mtimeMs !== before.mtimeMs || after.size !== before.size);
  if (writtenDuringRun) {
    return parseTestReport(readFileSync(reportPath, "utf8"));
  }
  return null;
}

function coverageCheck(ctx: GateContext, threshold: number): Check {
  const covCmd = ctx.config.commands.coverage;
  if (covCmd) runCommand(covCmd, ctx.root);
  const coverage = loadCoverage(ctx.root, ctx.config.coverage_format);
  if (!coverage) {
    return fail(
      "test.coverage",
      `diff coverage threshold is ${threshold}% but no coverage report was found ` +
        `(expected coverage/coverage-final.json or coverage/gate-coverage.json)`,
    );
  }
  const changed = changedLines(ctx.root, ctx.run.baseRef);
  const dc = diffCoverage(changed, coverage);
  if (dc.percent >= threshold) {
    return pass("test.coverage", `diff coverage ${dc.percent}% ≥ ${threshold}%`);
  }
  const gap = dc.gaps
    .slice(0, 5)
    .map((g) => `${g.file}:${g.uncovered.slice(0, 10).join(",")}`)
    .join("; ");
  return fail("test.coverage", `diff coverage ${dc.percent}% < ${threshold}% — uncovered changed lines: ${gap}`);
}
