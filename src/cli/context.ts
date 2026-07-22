import { loadConfig, type GateConfig } from "../core/config.js";
import { readCurrentRunId } from "../core/current.js";
import { findGateRoot } from "../core/paths.js";
import { readRun, type Run } from "../core/run.js";
import { GateError } from "./output.js";

export function requireRoot(): string {
  const root = findGateRoot();
  if (!root) throw new GateError("no .gate/ found — run `gate init` first");
  return root;
}

export interface ActiveContext {
  root: string;
  run: Run;
  config: GateConfig;
}

export function requireActiveRun(): ActiveContext {
  const root = requireRoot();
  const id = readCurrentRunId(root);
  if (!id) throw new GateError("no active run — start one with `gate start \"<title>\"`");
  return { root, run: readRun(root, id), config: loadConfig(root) };
}
