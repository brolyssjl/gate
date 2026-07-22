import { spawnSync } from "node:child_process";

export interface ExecResult {
  code: number;
  stdout: string;
  stderr: string;
}

/**
 * Run a configured command string through the shell, from `cwd`. Gate executes
 * commands from config.yml — the same trust class as npm scripts (proposal §9).
 * Milestone 1 runs them directly; trust-on-first-use hashing lands in M2.
 */
export function runCommand(command: string, cwd: string): ExecResult {
  const res = spawnSync(command, {
    cwd,
    shell: true,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  return {
    code: res.status ?? (res.error ? 127 : 1),
    stdout: res.stdout ?? "",
    stderr: res.stderr ?? "",
  };
}
