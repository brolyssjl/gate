import { existsSync, readFileSync } from "node:fs";
import { runPaths } from "../core/paths.js";
import { changedLines } from "../core/git.js";
import { runCommand } from "../core/exec.js";
import { parsePlanFile } from "../artifacts/plan.js";
import { parseTestReport, type NormalizedReport } from "./testReport.js";
import { diffCoverage, loadCoverage } from "./coverage.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * TEST gate — deterministic:
 *  - the test command exits 0 (no green claim over a red suite)
 *  - every `test:`-verified acceptance criterion maps to a named passing test
 *  - no skipped tests (unless the report is absent)
 *  - diff coverage ≥ threshold (only when coverage command + threshold are set)
 */
export function testGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];
  const { testReport: reportPath } = runPaths(ctx.root, ctx.run.id);

  // 1. Test command exit code — the load-bearing check.
  const testCmd = ctx.config.commands.test;
  let stdout = "";
  if (!testCmd) {
    checks.push(fail("test.command", "no test command configured (commands.test)"));
    return result("TEST", checks);
  }
  const run = runCommand(testCmd, ctx.root);
  stdout = run.stdout;
  checks.push(
    run.code === 0
      ? pass("test.command", "test suite passed (exit 0)")
      : fail(
          "test.command",
          `test suite failed (exit ${run.code}): ${(run.stderr || run.stdout).trim().split("\n").slice(-3).join(" ⏎ ")}`,
        ),
  );

  // 2. Load the normalized report: prefer a registered test-report.json,
  //    else parse the command's stdout as JSON.
  const report = loadReport(reportPath, stdout);

  // 3. Criterion → test mapping.
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

  // 4. No skipped tests.
  if (report) {
    checks.push(
      report.skipped === 0
        ? pass("test.no-skips", "no skipped tests")
        : fail("test.no-skips", `${report.skipped} skipped test(s); un-skip or split into a separate run`),
    );
  }

  // 5. Diff coverage (only when configured).
  const threshold = ctx.config.thresholds.diff_coverage;
  if (threshold !== undefined) {
    checks.push(coverageCheck(ctx, threshold));
  }

  return result("TEST", checks);
}

function loadReport(reportPath: string, stdout: string): NormalizedReport | null {
  if (existsSync(reportPath)) {
    const fromFile = parseTestReport(readFileSync(reportPath, "utf8"));
    if (fromFile) return fromFile;
  }
  return parseTestReport(stdout);
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
