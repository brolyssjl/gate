import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { writeFileAtomic } from "./fsx.js";
import { gatePaths } from "./paths.js";

/**
 * Records what `gate init` last left the repo-root `.gitignore` containing,
 * so the scope check can tell "gate's own bookkeeping write, untouched since"
 * apart from "someone edited .gitignore after that" (review finding: an
 * unconditional `.gitignore` exemption is a scope-check bypass - appending an
 * ignore pattern there hides whatever files it now matches from every
 * git-based diff Gate takes, including the scope check itself). Only an
 * exact content match to the hash recorded right after `gate init` wrote the
 * file counts as bookkeeping; any other edit - including widening the ignore
 * list - is a normal touched file, declared in `plan.md` or it fails the
 * scope check like anything else.
 */
export interface GitignoreState {
  hash: string;
  writtenAt: string;
}

function contentHash(content: string): string {
  return "sha256:" + createHash("sha256").update(content).digest("hex");
}

/** Record the hash of `.gitignore`'s full content immediately after `gate init` writes it. */
export function recordGitignoreState(root: string, content: string): void {
  const record: GitignoreState = { hash: contentHash(content), writtenAt: new Date().toISOString() };
  writeFileAtomic(gatePaths(root).gitignoreState, JSON.stringify(record, null, 2) + "\n");
}

/**
 * True only when `.gitignore` exists and its current content byte-for-byte
 * matches what `gate init` recorded - i.e. nothing has touched it since.
 * False when no record exists (init never ran, or predates this feature),
 * the file was deleted, or its content has changed at all - fails closed:
 * an unrecognized or edited `.gitignore` is never silently exempted.
 */
export function isGitignoreUnchangedSinceInit(root: string): boolean {
  const statePath = gatePaths(root).gitignoreState;
  if (!existsSync(statePath)) return false;
  let record: GitignoreState;
  try {
    record = JSON.parse(readFileSync(statePath, "utf8")) as GitignoreState;
  } catch {
    return false;
  }
  const gitignorePath = join(root, ".gitignore");
  if (!existsSync(gitignorePath)) return false;
  return contentHash(readFileSync(gitignorePath, "utf8")) === record.hash;
}
