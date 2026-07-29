import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * Build `dist/` exactly once before any test file runs. The e2e suites each
 * spawn `node dist/cli.js` and used to rebuild it themselves in a per-file
 * `beforeAll` - harmless when vitest ran one file at a time, but vitest runs
 * files in parallel worker processes by default, so two suites' concurrent
 * `tsc` invocations could race and hand a third suite a dist/cli.js mid-write
 * (a rare but real source of flaky "exit code 1, no stderr" failures). A
 * single `globalSetup` build removes the race instead of papering over it.
 */
export default function setup(): void {
  execFileSync("npm", ["run", "build"], { cwd: pkgRoot, stdio: "inherit" });
}
