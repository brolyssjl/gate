import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { stringify } from "yaml";
import { loadConfig, type GateConfig } from "../src/core/config.js";
import { writeTrust } from "../src/core/trust.js";

/** Create an isolated temp git repo with an initial commit. Returns its path. */
export function makeRepo(files: Record<string, string> = {}): string {
  const dir = mkdtempSync(join(tmpdir(), "gate-test-"));
  const git = (...args: string[]) => execFileSync("git", args, { cwd: dir, stdio: "pipe" });
  git("init", "-q");
  git("config", "user.email", "test@test.co");
  git("config", "user.name", "test");
  git("config", "commit.gpgsign", "false");
  writeFile(dir, "README.md", "seed\n");
  for (const [path, content] of Object.entries(files)) writeFile(dir, path, content);
  git("add", "-A");
  git("commit", "-qm", "init");
  return dir;
}

export function writeFile(root: string, rel: string, content: string): void {
  const abs = join(root, rel);
  mkdirSync(dirname(abs), { recursive: true });
  writeFileSync(abs, content);
}

export function headSha(root: string): string {
  return execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
}

/**
 * Write `.gate/config.yml` from a partial config so trust hashes the same
 * commands the gate will run. Returns the loaded config. When `trust` is true
 * (default) the commands block is trusted so gates will execute it.
 */
export function writeConfig(
  root: string,
  partial: Partial<GateConfig>,
  trust = true,
): GateConfig {
  mkdirSync(join(root, ".gate"), { recursive: true });
  writeFileSync(join(root, ".gate", "config.yml"), stringify(partial));
  if (trust) writeTrust(root, null);
  return loadConfig(root);
}
