import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

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
