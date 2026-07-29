import { copyFileSync, existsSync } from "node:fs";
import { basename, resolve } from "node:path";
import { runPaths } from "../core/paths.js";
import { nowIso, writeRun } from "../core/run.js";
import { emit, GateError, requireActiveRun, UsageError, type ParsedArgs } from "./shared.js";

/**
 * `gate log <file>` — register an artifact against the current phase. The file
 * is copied into the run folder under its basename (if not already there) and
 * recorded in run.artifacts, so gates and `gate report` can find it. Artifacts
 * are context for humans and reviewers — never gate evidence: in particular a
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

  const note =
    name === "test-report.json"
      ? " (note: gates ignore hand-registered test reports; evidence comes from the run Gate executes)"
      : "";
  emit(`Registered artifact "${name}" against ${run.phase}${note}`, { artifact: name, phase: run.phase }, args.flags);
}
