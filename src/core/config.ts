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
}

const DEFAULT_CONFIG: GateConfig = {
  commands: {},
  thresholds: {},
  targets: {},
  phases: {},
  integrations: {},
  coverage_format: "auto",
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
  };
}

/** Raw commands block text, used by trust hashing in a later milestone. */
export function commandsBlockHashSource(root: string): string {
  const { config } = gatePaths(root);
  if (!existsSync(config)) return "";
  const raw = parseYaml(readFileSync(config, "utf8")) as Partial<GateConfig> | null;
  return JSON.stringify({ commands: raw?.commands ?? {}, targets: raw?.targets ?? {} });
}
