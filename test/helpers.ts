import { execFileSync, spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { stringify } from "yaml";
import { loadConfig, type GateConfig } from "../src/core/config.js";
import { writeTrust } from "../src/core/trust.js";

const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_CLI = join(pkgRoot, "dist", "cli.js");

export interface GateInvocation {
  code: number;
  stdout: string;
  stderr: string;
  json: () => unknown;
}

export interface GateOptions {
  env?: NodeJS.ProcessEnv;
  input?: string;
}

/**
 * Resolve how to invoke the `gate` binary under test: `$GATE_BIN` when set -
 * conformance mode (`npm run conformance`), point it at any binary
 * implementing the CLI contract these black-box tests assert (e.g. a future
 * Rust port) - otherwise the local build, `node dist/cli.js`. Centralized
 * here so every CLI-driving suite (cli.e2e, concurrency, humanReview, prune,
 * guard) is conformance-capable, not just one file.
 */
function gateCommand(args: string[]): { cmd: string; cmdArgs: string[] } {
  const bin = process.env.GATE_BIN;
  return bin ? { cmd: bin, cmdArgs: args } : { cmd: "node", cmdArgs: [DEFAULT_CLI, ...args] };
}

/** Spawn the `gate` CLI under test and capture its result. See `gateCommand` re: $GATE_BIN. */
export function gate(cwd: string, args: string[], opts: GateOptions = {}): GateInvocation {
  const { cmd, cmdArgs } = gateCommand(args);
  const res = spawnSync(cmd, cmdArgs, { cwd, encoding: "utf8", env: opts.env ?? process.env, input: opts.input });
  return { code: res.status ?? 1, stdout: res.stdout, stderr: res.stderr, json: () => JSON.parse(res.stdout) };
}

/** Async spawn of the CLI under test, for launching several invocations genuinely concurrently. */
export function gateAsync(cwd: string, args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
  const { cmd, cmdArgs } = gateCommand(args);
  return new Promise((resolve) => {
    const child = spawn(cmd, cmdArgs, { cwd });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d: Buffer) => (stdout += d));
    child.stderr.on("data", (d: Buffer) => (stderr += d));
    child.on("close", (code) => resolve({ code: code ?? 1, stdout, stderr }));
  });
}

/**
 * Shell command line that invokes the CLI under test, for embedding in a
 * shim script (e.g. a `gate` on PATH so an installed git hook resolves it in
 * tests). Mirrors `gateCommand`'s $GATE_BIN resolution.
 */
export function gateShellCommand(): string {
  const bin = process.env.GATE_BIN;
  return bin ? `"${bin}"` : `node "${DEFAULT_CLI}"`;
}

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
