import { existsSync, readFileSync } from "node:fs";
import { parse as parseYaml } from "yaml";
import { gatePaths } from "./paths.js";
import { GateError } from "../cli/output.js";

/** Machine actions per phase. Missing commands mean "no such action here". */
export interface Commands {
  build?: string;
  test?: string;
  lint?: string;
  coverage?: string;
}

export interface Thresholds {
  /** Minimum percentage of changed lines that must be covered. */
  diff_coverage?: number;
}

/** Diff-coverage report formats Gate can parse (Milestone 4 adds coverage-py/go-cover/lcov). */
export type CoverageFormat = "istanbul" | "generic" | "coverage-py" | "go-cover" | "lcov" | "auto";
const COVERAGE_FORMATS: readonly CoverageFormat[] = [
  "istanbul",
  "generic",
  "coverage-py",
  "go-cover",
  "lcov",
  "auto",
];

export interface TargetConfig {
  match: string[];
  commands?: Commands;
  thresholds?: Thresholds;
  playbooks?: Record<string, string>;
  /** Overrides the top-level `coverage_format` for this target only. */
  coverage_format?: CoverageFormat;
}

export type PhaseMode = "required" | "optional" | "off";

/** `gate prune` retention defaults (Milestone 3, additive). CLI flags win when given. */
export interface RetentionConfig {
  /** Keep the N most recently updated non-active runs (default 10). */
  keep?: number;
  /** Additionally require a candidate to be older than N days to be pruned. */
  days?: number;
}

export interface GateConfig {
  commands: Commands;
  thresholds: Thresholds;
  targets: Record<string, TargetConfig>;
  phases: Record<string, PhaseMode>;
  integrations: Record<string, string>;
  /**
   * Coverage report format hint. `istanbul` = coverage-final.json (jest/vitest),
   * `generic` = the documented JSON contract, `coverage-py` = coverage.py's
   * `coverage json` output, `go-cover` = `go test -coverprofile` text profiles,
   * `lcov` = the standard lcov.info text format. Defaults to auto-detect.
   */
  coverage_format?: CoverageFormat;
  /**
   * `gate prune` retention defaults. Deliberately NOT part of the trust hash
   * (`commandsBlockHashSource`) - it configures which run folders get
   * archived, never a command that executes.
   */
  retention: RetentionConfig;
  /**
   * Glob list of paths the scope check (IMPLEMENT/DEBUG) treats as noise
   * rather than an undeclared file - environment cruft that gets rewritten by
   * the commands Gate itself runs (a node compile cache, a go build dir,
   * coverage output) and would otherwise hard-block the gate with false scope
   * violations. Part of the trust hash (`commandsBlockHashSource`): it
   * changes what the scope check accepts, so widening it needs the same
   * re-trust as changing a command.
   */
  scope_ignore: string[];
}

const DEFAULT_CONFIG: GateConfig = {
  commands: {},
  thresholds: {},
  targets: {},
  phases: {},
  integrations: {},
  coverage_format: "auto",
  retention: {},
  scope_ignore: [],
};

export function loadConfig(root: string): GateConfig {
  const { config } = gatePaths(root);
  if (!existsSync(config)) return { ...DEFAULT_CONFIG };
  const raw = parseYaml(readFileSync(config, "utf8")) as Partial<GateConfig> | null;
  if (!raw || typeof raw !== "object") return { ...DEFAULT_CONFIG };
  const targets = raw.targets ?? {};
  validateTargets(targets);
  if (raw.coverage_format !== undefined && !COVERAGE_FORMATS.includes(raw.coverage_format)) {
    throw new GateError(`.gate/config.yml: coverage_format must be one of ${COVERAGE_FORMATS.join(", ")}`);
  }
  if (
    raw.scope_ignore !== undefined &&
    (!Array.isArray(raw.scope_ignore) ||
      !raw.scope_ignore.every((g) => typeof g === "string" && g.trim().length > 0))
  ) {
    throw new GateError(".gate/config.yml: scope_ignore must be a list of glob strings");
  }
  return {
    commands: raw.commands ?? {},
    thresholds: raw.thresholds ?? {},
    targets,
    phases: raw.phases ?? {},
    integrations: raw.integrations ?? {},
    coverage_format: raw.coverage_format ?? "auto",
    retention: raw.retention ?? {},
    scope_ignore: raw.scope_ignore ?? [],
  };
}

/**
 * A malformed target block (most dangerously a missing/empty `match`) used to
 * reach the gates and blow up deep inside glob matching ("globs is not
 * iterable" - `matchesAny` does `for (const g of globs)`). Fail fast and
 * friendly here instead, naming the offending target so the fix is obvious.
 */
function validateTargets(targets: Record<string, unknown>): void {
  for (const [name, raw] of Object.entries(targets)) {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
      throw new GateError(`.gate/config.yml: target "${name}" must be a mapping`);
    }
    const t = raw as Record<string, unknown>;

    if (
      !Array.isArray(t.match) ||
      t.match.length === 0 ||
      !t.match.every((g) => typeof g === "string" && g.trim().length > 0)
    ) {
      throw new GateError(
        `.gate/config.yml: target "${name}" must declare a non-empty \`match\` list of glob strings`,
      );
    }

    if (t.commands !== undefined && (typeof t.commands !== "object" || t.commands === null || Array.isArray(t.commands))) {
      throw new GateError(`.gate/config.yml: target "${name}".commands must be a mapping`);
    }
    if (
      t.thresholds !== undefined &&
      (typeof t.thresholds !== "object" || t.thresholds === null || Array.isArray(t.thresholds))
    ) {
      throw new GateError(`.gate/config.yml: target "${name}".thresholds must be a mapping`);
    }
    if (
      t.playbooks !== undefined &&
      (typeof t.playbooks !== "object" || t.playbooks === null || Array.isArray(t.playbooks))
    ) {
      throw new GateError(`.gate/config.yml: target "${name}".playbooks must be a mapping of phase → path`);
    }
    if (t.coverage_format !== undefined && !COVERAGE_FORMATS.includes(t.coverage_format as CoverageFormat)) {
      throw new GateError(
        `.gate/config.yml: target "${name}".coverage_format must be one of ${COVERAGE_FORMATS.join(", ")}`,
      );
    }
  }
}

/**
 * Raw commands-block text, used by TOFU trust hashing (`core/trust.ts`).
 * `scope_ignore` rides in the same hash (Milestone 5): it changes what the
 * scope check accepts as noise rather than an undeclared file, the same
 * trust class as a command - widening it silently would be as dangerous as
 * editing a command without re-trusting.
 */
export function commandsBlockHashSource(root: string): string {
  const { config } = gatePaths(root);
  if (!existsSync(config)) return "";
  const raw = parseYaml(readFileSync(config, "utf8")) as Partial<GateConfig> | null;
  return JSON.stringify({
    commands: raw?.commands ?? {},
    targets: raw?.targets ?? {},
    scope_ignore: raw?.scope_ignore ?? [],
  });
}
