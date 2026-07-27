import { existsSync, readFileSync } from "node:fs";
import { parse as parseYaml } from "yaml";
import { gatePaths } from "./paths.js";

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

export interface TargetConfig {
  match: string[];
  commands?: Commands;
  thresholds?: Thresholds;
  playbooks?: Record<string, string>;
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
   * `generic` = the documented JSON contract. Defaults to auto-detect.
   */
  coverage_format?: "istanbul" | "generic" | "auto";
  /**
   * `gate prune` retention defaults. Deliberately NOT part of the trust hash
   * (`commandsBlockHashSource`) — it configures which run folders get
   * archived, never a command that executes.
   */
  retention: RetentionConfig;
}

const DEFAULT_CONFIG: GateConfig = {
  commands: {},
  thresholds: {},
  targets: {},
  phases: {},
  integrations: {},
  coverage_format: "auto",
  retention: {},
};

export function loadConfig(root: string): GateConfig {
  const { config } = gatePaths(root);
  if (!existsSync(config)) return { ...DEFAULT_CONFIG };
  const raw = parseYaml(readFileSync(config, "utf8")) as Partial<GateConfig> | null;
  if (!raw || typeof raw !== "object") return { ...DEFAULT_CONFIG };
  return {
    commands: raw.commands ?? {},
    thresholds: raw.thresholds ?? {},
    targets: raw.targets ?? {},
    phases: raw.phases ?? {},
    integrations: raw.integrations ?? {},
    coverage_format: raw.coverage_format ?? "auto",
    retention: raw.retention ?? {},
  };
}

/** Raw commands block text, used by trust hashing in a later milestone. */
export function commandsBlockHashSource(root: string): string {
  const { config } = gatePaths(root);
  if (!existsSync(config)) return "";
  const raw = parseYaml(readFileSync(config, "utf8")) as Partial<GateConfig> | null;
  return JSON.stringify({ commands: raw?.commands ?? {}, targets: raw?.targets ?? {} });
}
