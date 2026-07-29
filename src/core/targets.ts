import { changedFiles } from "./git.js";
import { matchesAny } from "./glob.js";
import { runPaths } from "./paths.js";
import type { Commands, CoverageFormat, GateConfig, Thresholds } from "./config.js";
import type { Run } from "./run.js";
import { parsePlanFile } from "../artifacts/plan.js";

/**
 * Targets let a multi-stack repo declare named per-stack blocks in
 * `config.yml` (match globs, commands, thresholds, playbook overlays).
 * Composition rule (proposal §4.1): **profiles choose which phases run;
 * targets choose how each phase runs.** Everything here is additive - a repo
 * with no `targets:` block resolves to a single bare, top-level command set,
 * byte-identical to Gate's behavior before targets existed.
 */

/** Names of every target whose `match` globs cover at least one of `files`. */
export function resolveAffectedTargets(config: GateConfig, files: string[]): string[] {
  const names: string[] = [];
  for (const [name, target] of Object.entries(config.targets)) {
    if (files.some((f) => matchesAny(f, target.match))) names.push(name);
  }
  return names;
}

/** A target's effective commands: its own commands layered over the top-level defaults. */
export function targetCommands(config: GateConfig, name: string): Commands {
  return { ...config.commands, ...(config.targets[name]?.commands ?? {}) };
}

/** A target's effective thresholds: its own thresholds layered over the top-level defaults. */
export function targetThresholds(config: GateConfig, name: string): Thresholds {
  return { ...config.thresholds, ...(config.targets[name]?.thresholds ?? {}) };
}

/** A target's effective coverage format: its own override, or the top-level default. */
export function targetCoverageFormat(config: GateConfig, name: string): CoverageFormat {
  return config.targets[name]?.coverage_format ?? config.coverage_format ?? "auto";
}

/** `base` for the untargeted case (JSON stability), `base[name]` once a target applies. */
export function checkName(base: string, target: string | null): string {
  return target ? `${base}[${target}]` : base;
}

/** Files among `files` that fall under target `name`'s `match` globs. */
export function filesForTarget(config: GateConfig, name: string, files: string[]): string[] {
  const target = config.targets[name];
  if (!target) return [];
  return files.filter((f) => matchesAny(f, target.match));
}

/**
 * The run's resolved targets for gate execution: an explicit `gate start
 * --target` override (stored in run.json) always wins over file-based
 * resolution. Unknown override names (a target since removed from config) are
 * dropped rather than silently treated as an active target.
 */
export function resolveRunTargets(config: GateConfig, run: Pick<Run, "targetOverride">, files: string[]): string[] {
  if (run.targetOverride && run.targetOverride.length > 0) {
    return run.targetOverride.filter((t) => t in config.targets);
  }
  return resolveAffectedTargets(config, files);
}

export interface ResolvedTarget {
  /** null = no targets configured, or none affected - run the bare, top-level command set. */
  target: string | null;
  commands: Commands;
  thresholds: Thresholds;
  coverageFormat: CoverageFormat;
}

/**
 * The command sets a gated phase should run this pass. Two cases collapse to
 * a single bare entry using the top-level commands/thresholds unchanged (the
 * critical invariant: no `targets:` configured, or none of `files` matched
 * any target, byte-identical to pre-targets behavior). Otherwise one entry per
 * affected target, each carrying its own effective commands/thresholds and
 * yielding bracketed check names via `checkName`.
 */
export function resolvePhaseTargets(config: GateConfig, run: Pick<Run, "targetOverride">, files: string[]): ResolvedTarget[] {
  const bare: ResolvedTarget = {
    target: null,
    commands: config.commands,
    thresholds: config.thresholds,
    coverageFormat: config.coverage_format ?? "auto",
  };
  if (Object.keys(config.targets).length === 0) return [bare];
  const affected = resolveRunTargets(config, run, files);
  if (affected.length === 0) return [bare];
  return affected.map((name) => ({
    target: name,
    commands: targetCommands(config, name),
    thresholds: targetThresholds(config, name),
    coverageFormat: targetCoverageFormat(config, name),
  }));
}

/** The glob's literal prefix up to its first wildcard, directory-separator trimmed. */
function staticPrefix(glob: string): string {
  const idx = glob.search(/[*?]/);
  const cut = idx === -1 ? glob : glob.slice(0, idx);
  return cut.replace(/\/+$/, "");
}

/**
 * Whether a plan-declared file/glob entry could plausibly touch a target's
 * match glob, without a real diff to check against yet. Errs toward inclusion
 * (a wildcard-rooted glob on either side is treated as covering everything;
 * otherwise two globs overlap when one's static path prefix contains the
 * other's) - a false positive only shows an extra, harmless playbook overlay.
 */
function globsOverlap(a: string, b: string): boolean {
  const pa = staticPrefix(a);
  const pb = staticPrefix(b);
  if (pa === "" || pb === "") return true;
  return pa === pb || pa.startsWith(pb + "/") || pb.startsWith(pa + "/");
}

/** Targets whose match globs plausibly overlap any of a plan's declared `files` entries. */
export function resolveTargetsFromPlanFiles(config: GateConfig, planFiles: string[]): string[] {
  const names: string[] = [];
  for (const [name, target] of Object.entries(config.targets)) {
    if (planFiles.some((pf) => target.match.some((g) => globsOverlap(pf, g)))) names.push(name);
  }
  return names;
}

/**
 * Best-effort target set for *display* purposes (playbook overlays), where a
 * real diff may not exist yet (e.g. still in PLAN before any code is
 * touched). Prefers the real changed-files resolution; falls back to the
 * plan's declared `files` when the diff is empty, then to nothing.
 */
export function resolveDisplayTargets(root: string, run: Run, config: GateConfig): string[] {
  if (Object.keys(config.targets).length === 0) return [];
  // An explicit override always wins - delegate to resolveRunTargets instead
  // of re-implementing that branch here; files are irrelevant once an
  // override is set, so it doesn't matter that none are passed.
  if (run.targetOverride && run.targetOverride.length > 0) {
    return resolveRunTargets(config, run, []);
  }
  const changed = changedFiles(root, run.baseRef).filter((f) => !f.startsWith(".gate/"));
  const fromDiff = resolveRunTargets(config, run, changed);
  if (fromDiff.length > 0) return fromDiff;
  const { plan } = parsePlanFile(runPaths(root, run.id).plan);
  return plan ? resolveTargetsFromPlanFiles(config, plan.files) : [];
}
