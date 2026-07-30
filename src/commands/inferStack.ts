import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { Commands } from "../core/config.js";

/**
 * Best-effort command inference for common ecosystems. `gate init` writes these
 * as a starting point; anything unusual is hand-edited. Never fails - an unknown
 * stack simply yields empty commands with commented placeholders.
 */
export function inferCommands(root: string): Commands {
  const pkgPath = join(root, "package.json");
  if (existsSync(pkgPath)) return inferNode(pkgPath);
  if (existsSync(join(root, "pyproject.toml"))) {
    return { test: "pytest", coverage: "pytest --cov --cov-report=json" };
  }
  if (existsSync(join(root, "go.mod"))) {
    return { build: "go build ./...", test: "go test ./...", lint: "go vet ./..." };
  }
  if (existsSync(join(root, "Cargo.toml"))) {
    return { build: "cargo build", test: "cargo test", lint: "cargo clippy" };
  }
  return {};
}

function inferNode(pkgPath: string): Commands {
  let scripts: Record<string, unknown> = {};
  try {
    scripts = (JSON.parse(readFileSync(pkgPath, "utf8")) as { scripts?: Record<string, unknown> }).scripts ?? {};
  } catch {
    scripts = {};
  }
  const has = (name: string) => typeof scripts[name] === "string";
  const cmds: Commands = {};
  if (has("build")) cmds.build = "npm run build";
  if (has("test")) cmds.test = "npm test";
  if (has("lint")) cmds.lint = "npm run lint";
  if (has("coverage")) cmds.coverage = "npm run coverage";
  return cmds;
}
