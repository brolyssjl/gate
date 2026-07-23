import { spawnSync } from "node:child_process";

function git(root: string, args: string[]): { ok: boolean; stdout: string } {
  const res = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  return { ok: res.status === 0, stdout: res.stdout ?? "" };
}

export function isGitRepo(root: string): boolean {
  return git(root, ["rev-parse", "--is-inside-work-tree"]).ok;
}

/** Current HEAD sha, or null if there are no commits yet / not a repo. */
export function headSha(root: string): string | null {
  const res = git(root, ["rev-parse", "HEAD"]);
  return res.ok ? res.stdout.trim() : null;
}

/**
 * Files changed since `baseRef` (committed diff) plus uncommitted and untracked
 * changes in the working tree. Returns repo-relative POSIX paths, deduped.
 * When `baseRef` is null, only the working-tree state is considered.
 */
export function changedFiles(root: string, baseRef: string | null): string[] {
  const set = new Set<string>();
  const add = (out: string) => {
    for (const line of out.split("\n")) {
      const f = line.trim();
      if (f) set.add(f);
    }
  };
  if (baseRef) add(git(root, ["diff", "--name-only", baseRef, "--"]).stdout);
  add(git(root, ["diff", "--name-only", "--"]).stdout); // unstaged vs index
  add(git(root, ["diff", "--name-only", "--cached", "--"]).stdout); // staged
  add(git(root, ["ls-files", "--others", "--exclude-standard"]).stdout); // untracked
  return [...set];
}

/**
 * Unified diff text since `baseRef` (committed + uncommitted), for the review
 * packet. `.gate/` bookkeeping is excluded so the packet shows only real code.
 * Returns "" when not a repo or there is nothing to show.
 */
export function diffText(root: string, baseRef: string | null): string {
  if (!isGitRepo(root)) return "";
  const args = baseRef
    ? ["diff", "--no-color", baseRef, "--", ".", ":(exclude).gate/**"]
    : ["diff", "--no-color", "HEAD", "--", ".", ":(exclude).gate/**"];
  return git(root, args).stdout;
}

/**
 * Map of file → set of added/modified line numbers (new-file line numbers)
 * since `baseRef`, including uncommitted work. Used for diff coverage.
 */
export function changedLines(root: string, baseRef: string | null): Map<string, Set<number>> {
  const result = new Map<string, Set<number>>();
  const diffArgs = baseRef
    ? ["diff", "--unified=0", "--no-color", baseRef, "--"]
    : ["diff", "--unified=0", "--no-color", "HEAD", "--"];
  const committed = git(root, diffArgs).stdout;
  parseUnifiedDiff(committed, result);
  // Untracked files: every line counts as added.
  const untracked = git(root, ["ls-files", "--others", "--exclude-standard"]).stdout;
  for (const line of untracked.split("\n")) {
    const f = line.trim();
    if (!f) continue;
    // Represented as fully-added; the coverage check treats all lines as changed.
    if (!result.has(f)) result.set(f, new Set());
  }
  return result;
}

function parseUnifiedDiff(diff: string, out: Map<string, Set<number>>): void {
  let file: string | null = null;
  let newLine = 0;
  let remaining = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+++ ")) {
      const path = line.slice(4).trim();
      file = path === "/dev/null" ? null : path.replace(/^b\//, "");
      if (file && !out.has(file)) out.set(file, new Set());
      continue;
    }
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@/.exec(line);
    if (hunk) {
      newLine = Number(hunk[1]);
      remaining = hunk[2] === undefined ? 1 : Number(hunk[2]);
      continue;
    }
    if (file && remaining > 0 && line.startsWith("+") && !line.startsWith("+++")) {
      out.get(file)!.add(newLine);
      newLine++;
      remaining--;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      // deleted line: does not consume a new-file line number
    }
  }
}
