import { spawnSync } from "node:child_process";

export interface ExecResult {
  code: number;
  stdout: string;
  stderr: string;
}

/**
 * Run a configured command string through the shell, from `cwd`. Gate executes
 * commands from config.yml - the same trust class as npm scripts (proposal §9)
 * - and only after `gate trust` has pinned them. `env` entries are layered over
 * the process env so gates can hand runners well-known paths (GATE_TEST_REPORT).
 */
export function runCommand(command: string, cwd: string, env?: Record<string, string>): ExecResult {
  const res = spawnSync(command, {
    cwd,
    shell: true,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
    env: env ? { ...process.env, ...env } : process.env,
  });
  return {
    code: res.status ?? (res.error ? 127 : 1),
    stdout: res.stdout ?? "",
    stderr: res.stderr ?? "",
  };
}
