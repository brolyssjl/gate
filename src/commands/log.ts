import { copyFileSync, existsSync } from "node:fs";
import { basename, resolve } from "node:path";
import { runPaths } from "../core/paths.js";
import { nowIso, writeRun } from "../core/run.js";
import { emit, GateError, requireActiveRun, UsageError, type ParsedArgs } from "./shared.js";

/**
 * `gate log <file>` - register an artifact against the current phase. The file
 * is copied into the run folder under its basename (if not already there) and
 * recorded in run.artifacts, so gates and `gate report` can find it. Artifacts
 * are context for humans and reviewers - never gate evidence: in particular a
 * hand-registered test-report.json is ignored by the TEST/DEBUG gates, which
 * only trust reports produced by the test run Gate executes itself.
 */
export function cmdLog(args: ParsedArgs): void {
  const { root, run } = requireActiveRun(args);

  const arg = args.positionals[0];
  if (!arg) throw new UsageError("gate log needs a file: gate log <file>");
  const src = resolve(process.cwd(), arg);
  if (!existsSync(src)) throw new GateError(`file not found: ${arg}`);

  const name = basename(src);
  const dest = resolve(runPaths(root, run.id).dir, name);
  if (src !== dest) copyFileSync(src, dest);

  run.artifacts[name] = { phase: run.phase, at: nowIso() };
  writeRun(root, run);

  const note = looksLikeTestReport(name)
    ? " (note: gates ignore hand-registered test reports - evidence comes only from the run Gate itself " +
      "executes: the test command's stdout, or a file it writes to $GATE_TEST_REPORT during the run; " +
      "see `gate playbook TEST`)"
    : "";
  emit(`Registered artifact "${name}" against ${run.phase}${note}`, { artifact: name, phase: run.phase }, args.flags);
}

/**
 * Heuristic for "this file is probably someone trying to hand-register test
 * evidence" - broader than the literal `test-report.json` name Gate itself
 * writes, so `gate log jest-results.json` (or `results.json`, `junit.xml`,
 * ...) also gets pointed at the real mechanism instead of registering
 * silently and leaving the author to discover the gate ignores it.
 */
function looksLikeTestReport(name: string): boolean {
  return /\.(json|xml)$/i.test(name) && /(test|report|result|junit)/i.test(name);
}
