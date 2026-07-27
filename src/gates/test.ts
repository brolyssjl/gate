import { existsSync, readFileSync, statSync } from "node:fs";
import { runPaths } from "../core/paths.js";
import { changedLines, changedFiles } from "../core/git.js";
import { runCommand } from "../core/exec.js";
import { writeFileAtomic } from "../core/fsx.js";
import { isCommandsTrusted } from "../core/trust.js";
import { parsePlanFile } from "../artifacts/plan.js";
import {
  checkName,
  filesForTarget,
  resolvePhaseTargets,
  type ResolvedTarget,
} from "../core/targets.js";
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
 *
 * Targets (Milestone 3): when `config.targets` resolves to a single bare,
 * top-level command set (no targets configured, or none affected by the
 * touched files), this delegates to `runBareTargetGate` — the exact same
 * check sequence and names Gate produced before targets existed. Once ≥1
 * named target is affected, each gets its own `test.build[name]` /
 * `test.lint[name]` / `test.command[name]` / `test.coverage[name]` checks;
 * `test.criteria` / `test.no-skips` stay single, aggregate checks over every
 * affected target's combined report (a criterion is a property of the plan,
 * not of one stack).
 */
export function testGate(ctx: GateContext): GateResult {
  const { testReport: reportPath } = runPaths(ctx.root, ctx.run.id);
  const touched = changedFiles(ctx.root, ctx.run.baseRef).filter((f) => !f.startsWith(".gate/"));

  // Untrusted config never spawns a process, for any target (proposal §9) —
  // the trust hash already covers the targets block (commandsBlockHashSource).
  if (!isCommandsTrusted(ctx.root)) {
    return result("TEST", [
      fail("test.command", "test command not trusted — review .gate/config.yml and run `gate trust`"),
    ]);
  }

  const resolved = resolvePhaseTargets(ctx.config, ctx.run, touched);
  if (resolved.length === 1 && resolved[0]!.target === null) {
    return result("TEST", runBareTargetGate(ctx, resolved[0]!, reportPath));
  }
  return result("TEST", runTargetedGate(ctx, resolved, reportPath, touched));
}

/**
 * Byte-identical to Gate's pre-targets TEST gate: no `targets:` configured,
 * or the touched files matched none. `bare.commands`/`bare.thresholds` are
 * always exactly `ctx.config.commands`/`ctx.config.thresholds` in this case.
 */
function runBareTargetGate(ctx: GateContext, bare: ResolvedTarget, reportPath: string): Check[] {
  const checks: Check[] = [];
  const { dir } = runPaths(ctx.root, ctx.run.id);

  const testCmd = bare.commands.test;
  if (!testCmd) {
    checks.push(fail("test.command", "no test command configured (commands.test)"));
    return checks;
  }

  checks.push(commandCheck("test.build", bare.commands.build, ctx.root, "build", true));
  checks.push(commandCheck("test.lint", bare.commands.lint, ctx.root, "lint", true));

  const reportBefore = statReport(reportPath);
  const run = runCommand(testCmd, ctx.root, { GATE_RUN_DIR: dir, GATE_TEST_REPORT: reportPath });
  checks.push(
    run.code === 0
      ? pass("test.command", "test suite passed (exit 0)")
      : fail("test.command", `test suite failed (exit ${run.code}): ${tail(run.stderr || run.stdout)}`),
  );

  const report = loadReport(reportPath, run.stdout, reportBefore);

  const { plan } = parsePlanFile(runPaths(ctx.root, ctx.run.id).plan);
  const testCriteria = (plan?.criteria ?? []).filter((c) => c.verify.toLowerCase().startsWith("test:"));
  if (testCriteria.length === 0) {
    checks.push(pass("test.criteria", "no test-verified criteria to map"));
  } else if (!report) {
    checks.push(fail("test.criteria", "cannot verify criterion→test mapping: no parseable test report found"));
  } else {
    checks.push(criteriaCheck("test.criteria", testCriteria, report));
  }

  if (report) {
    checks.push(
      report.skipped === 0
        ? pass("test.no-skips", "no skipped tests")
        : fail("test.no-skips", `${report.skipped} skipped test(s); un-skip or split into a separate run`),
    );
  }

  const threshold = bare.thresholds.diff_coverage;
  if (threshold !== undefined) {
    checks.push(coverageCheck(ctx, threshold, "test.coverage", bare.commands.coverage, null, []));
  }

  return checks;
}

/** One or more named targets are affected — bracketed per-target checks, aggregate criteria/skips. */
function runTargetedGate(
  ctx: GateContext,
  resolved: ResolvedTarget[],
  reportPath: string,
  touched: string[],
): Check[] {
  const checks: Check[] = [];
  const reports: NormalizedReport[] = [];
  const { dir } = runPaths(ctx.root, ctx.run.id);

  for (const t of resolved) {
    const testCmd = t.commands.test;
    if (!testCmd) {
      checks.push(fail(checkName("test.command", t.target), "no test command configured (commands.test)"));
      continue;
    }
    checks.push(commandCheck(checkName("test.build", t.target), t.commands.build, ctx.root, "build", true));
    checks.push(commandCheck(checkName("test.lint", t.target), t.commands.lint, ctx.root, "lint", true));

    const path = targetReportPath(reportPath, t.target);
    const reportBefore = statReport(path);
    const run = runCommand(testCmd, ctx.root, { GATE_RUN_DIR: dir, GATE_TEST_REPORT: path });
    checks.push(
      run.code === 0
        ? pass(checkName("test.command", t.target), "test suite passed (exit 0)")
        : fail(checkName("test.command", t.target), `test suite failed (exit ${run.code}): ${tail(run.stderr || run.stdout)}`),
    );

    const report = loadReport(path, run.stdout, reportBefore);
    if (report) reports.push(report);

    const threshold = t.thresholds.diff_coverage;
    if (threshold !== undefined && t.target) {
      checks.push(coverageCheck(ctx, threshold, checkName("test.coverage", t.target), t.commands.coverage, t.target, touched));
    }
  }

  const combined: NormalizedReport | null =
    reports.length > 0
      ? { tests: reports.flatMap((r) => r.tests), skipped: reports.reduce((a, r) => a + r.skipped, 0) }
      : null;

  const { plan } = parsePlanFile(runPaths(ctx.root, ctx.run.id).plan);
  const testCriteria = (plan?.criteria ?? []).filter((c) => c.verify.toLowerCase().startsWith("test:"));
  if (testCriteria.length === 0) {
    checks.push(pass("test.criteria", "no test-verified criteria to map"));
  } else if (!combined) {
    checks.push(fail("test.criteria", "cannot verify criterion→test mapping: no parseable test report found"));
  } else {
    checks.push(criteriaCheck("test.criteria", testCriteria, combined));
  }

  if (combined) {
    checks.push(
      combined.skipped === 0
        ? pass("test.no-skips", "no skipped tests")
        : fail("test.no-skips", `${combined.skipped} skipped test(s); un-skip or split into a separate run`),
    );
  }

  return checks;
}

function criteriaCheck(
  name: string,
  testCriteria: Array<{ id: string; verify: string }>,
  report: NormalizedReport,
): Check {
  const passing = report.tests.filter((t) => t.status === "passed").map((t) => t.name.toLowerCase());
  const unmapped = testCriteria.filter((c) => {
    const needle = c.verify.slice(c.verify.indexOf(":") + 1).trim().toLowerCase();
    return !passing.some((n) => n.includes(needle));
  });
  return unmapped.length === 0
    ? pass(name, `all ${testCriteria.length} test-verified criteria map to a passing test`)
    : fail(name, `criteria without a passing named test: ${unmapped.map((c) => `${c.id} (${c.verify})`).join(", ")}`);
}

function tail(s: string): string {
  return s.trim().split("\n").slice(-3).join(" ⏎ ");
}

/** Per-target report path: unchanged for the bare case, `<base>.<target>.json` otherwise. */
function targetReportPath(basePath: string, target: string | null): string {
  return target ? basePath.replace(/\.json$/, `.${target}.json`) : basePath;
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

/**
 * Diff coverage for the bare/legacy case (`target === null`): scoped to every
 * changed line, coverage command run unscoped — identical to pre-targets
 * behavior. For a named target, scoped to only the changed lines under that
 * target's `match` globs, using that target's own coverage command.
 */
function coverageCheck(
  ctx: GateContext,
  threshold: number,
  name: string,
  covCmd: string | undefined,
  target: string | null,
  touched: string[],
): Check {
  if (covCmd) runCommand(covCmd, ctx.root);
  const coverage = loadCoverage(ctx.root, ctx.config.coverage_format);
  if (!coverage) {
    return fail(
      name,
      `diff coverage threshold is ${threshold}% but no coverage report was found ` +
        `(expected coverage/coverage-final.json or coverage/gate-coverage.json)`,
    );
  }
  const allChanged = changedLines(ctx.root, ctx.run.baseRef);
  const changed = target
    ? new Map([...allChanged].filter(([f]) => filesForTarget(ctx.config, target, touched).includes(f)))
    : allChanged;
  const dc = diffCoverage(changed, coverage);
  if (dc.percent >= threshold) {
    return pass(name, `diff coverage ${dc.percent}% ≥ ${threshold}%`);
  }
  const gap = dc.gaps
    .slice(0, 5)
    .map((g) => `${g.file}:${g.uncovered.slice(0, 10).join(",")}`)
    .join("; ");
  return fail(name, `diff coverage ${dc.percent}% < ${threshold}% — uncovered changed lines: ${gap}`);
}
