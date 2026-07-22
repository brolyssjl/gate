import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { commandsBlockHashSource } from "./config.js";
import { gatePaths } from "./paths.js";

/**
 * Trust-on-first-use for the `commands:` block (proposal §9). Gate executes
 * shell commands from config.yml — the same trust class as npm scripts — so the
 * IMPLEMENT/TEST gates refuse to run them until a human has run `gate trust`.
 * The approved hash lives in `.gate/trust.json`, which is tracked: changing a
 * command and re-trusting land in the same diff, giving PR review the checkpoint.
 */
export interface TrustRecord {
  commandsHash: string;
  trustedAt: string;
  trustedBy: string | null;
}

function trustPath(root: string): string {
  return join(gatePaths(root).gate, "trust.json");
}

export function currentCommandsHash(root: string): string {
  const source = commandsBlockHashSource(root);
  return "sha256:" + createHash("sha256").update(source).digest("hex");
}

/** True when the commands block is empty (nothing executes, e.g. `{}`). */
export function hasNoCommands(root: string): boolean {
  const source = commandsBlockHashSource(root);
  return source === JSON.stringify({ commands: {}, targets: {} });
}

export function readTrust(root: string): TrustRecord | null {
  const path = trustPath(root);
  if (!existsSync(path)) return null;
  try {
    return JSON.parse(readFileSync(path, "utf8")) as TrustRecord;
  } catch {
    return null;
  }
}

export function writeTrust(root: string, by: string | null): TrustRecord {
  const record: TrustRecord = {
    commandsHash: currentCommandsHash(root),
    trustedAt: new Date().toISOString(),
    trustedBy: by,
  };
  writeFileSync(trustPath(root), JSON.stringify(record, null, 2) + "\n");
  return record;
}

/**
 * Whether the current commands block is trusted: trivially true when there are
 * no commands to run, otherwise the stored hash must match the current one.
 */
export function isCommandsTrusted(root: string): boolean {
  if (hasNoCommands(root)) return true;
  const record = readTrust(root);
  return record !== null && record.commandsHash === currentCommandsHash(root);
}
