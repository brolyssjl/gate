import { mkdirSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

/**
 * Write a file atomically: write to a sibling temp file, then rename over the
 * target. A crash mid-write leaves the old content (or a stray .tmp), never a
 * truncated file. Run state must survive process death (proposal: all state on
 * disk), so every state file goes through here. Gate is a single-process CLI,
 * so a fixed temp name per target is safe.
 */
export function writeFileAtomic(path: string, content: string): void {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = path + ".tmp";
  writeFileSync(tmp, content);
  renameSync(tmp, path);
}
