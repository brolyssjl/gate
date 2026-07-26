import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";

function git(root: string, args: string[]): { ok: boolean; stdout: string } {
  const res = spawnSync("git", args, { cwd: root, encoding: "utf8" });
  return { ok: res.status === 0, stdout: res.stdout ?? "" };
}

/**
 * Untracked (not-ignored) files, excluding `.gate/` bookkeeping. Repo-relative
 * POSIX paths, as git reports them.
 */
export function untrackedFiles(root: string): string[] {
  return git(root, ["ls-files", "--others", "--exclude-standard"])
    .stdout.split("\n")
    .map((l) => l.trim())
    .filter((f) => f.length > 0 && !f.startsWith(".gate/"));
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
 * Unified diff text since `baseRef` (committed + uncommitted + untracked), for
 * the review packet. `git diff` alone omits untracked files - and nothing in
 * the Gate flow requires committing - so each untracked file is appended as a
 * synthesized new-file diff (`git diff --no-index /dev/null <f>`); without this
 * a reviewer would silently see none of the run's new files. `.gate/`
 * bookkeeping is excluded so the packet shows only real code. Returns "" when
 * not a repo or there is nothing to show.
 */
export function diffText(root: string, baseRef: string | null): string {
  if (!isGitRepo(root)) return "";
  const args = baseRef
    ? ["diff", "--no-color", baseRef, "--", ".", ":(exclude).gate/**"]
    : ["diff", "--no-color", "HEAD", "--", ".", ":(exclude).gate/**"];
  const parts = [git(root, args).stdout];
  for (const f of untrackedFiles(root)) {
    // --no-index exits non-zero when the files differ; the diff is still on stdout.
    parts.push(git(root, ["diff", "--no-color", "--no-index", "--", "/dev/null", f]).stdout);
  }
  return parts.filter((p) => p.length > 0).join("");
}

/**
 * A content fingerprint of the working tree: HEAD plus every uncommitted and
 * untracked change (excluding `.gate/`). Recorded when a gate passes and when a
 * review packet is emitted, so the REVIEW gate can detect code that changed
 * after its evidence was gathered. Committing between phases changes HEAD and
 * therefore the fingerprint - that errs toward re-verification, never under it.
 * Returns null when not a git repo (nothing to fingerprint).
 */
export function treeFingerprint(root: string): string | null {
  if (!isGitRepo(root)) return null;
  const parts = [
    headSha(root) ?? "(no-head)",
    git(root, ["diff", "--no-color", "HEAD", "--", ".", ":(exclude).gate/**"]).stdout,
  ];
  for (const f of untrackedFiles(root)) {
    const h = git(root, ["hash-object", "--", f]);
    parts.push(`${f}\0${h.ok ? h.stdout.trim() : "(unhashable)"}`);
  }
  return "sha256:" + createHash("sha256").update(parts.join("\0")).digest("hex");
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
  // Untracked files: every line counts as added. Enumerated explicitly - an
  // empty set must keep meaning "no added lines" (e.g. a deletion-only diff),
  // never double as a wholly-new-file sentinel.
  for (const f of untrackedFiles(root)) {
    if (result.has(f)) continue;
    const lines = new Set<number>();
    for (let n = 1; n <= countLines(join(root, f)); n++) lines.add(n);
    result.set(f, lines);
  }
  return result;
}

function countLines(abs: string): number {
  try {
    const text = readFileSync(abs, "utf8");
    if (text.length === 0) return 0;
    const parts = text.split("\n").length;
    return text.endsWith("\n") ? parts - 1 : parts;
  } catch {
    return 0; // unreadable (e.g. binary garbage) - contributes no measurable lines
  }
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
