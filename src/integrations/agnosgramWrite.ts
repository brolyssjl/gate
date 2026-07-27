import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { writeFileAtomic } from "../core/fsx.js";
import type { RetroLog } from "../artifacts/retro.js";

/**
 * Writes a `gate retro` journal entry into `.agnosgram/journal/`, in the
 * frozen Agnosgram journal format v1 (pinned cross-project contract):
 *
 *   ## YYYY-MM-DD HH:MM · gate · <branch>
 *   - **Did:** Completed gate run "<title>" (<run-id>, profile <profile>)
 *   - **Learned:** <broke entries; joined "; ">
 *   - **Decided:** <conventions entries>
 *   - **Avoid:** <avoid entries>
 *   - **Source:** .gate/runs/<run-id>
 *
 * Empty slots are omitted. `formatJournalEntry` is pure and golden-tested
 * against that exact block; everything else here is I/O.
 */

export interface JournalEntryParams {
  run: { id: string; title: string; profile: string };
  retro: RetroLog;
  branch: string | null;
  when?: Date;
}

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

function localTimestamp(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

export function formatJournalEntry(params: JournalEntryParams): string {
  const { run, retro, branch } = params;
  const when = params.when ?? new Date();

  const headingParts = [`## ${localTimestamp(when)}`, "·", "gate"];
  if (branch) headingParts.push("·", branch);

  const lines = [headingParts.join(" ")];
  lines.push(`- **Did:** Completed gate run "${run.title}" (${run.id}, profile ${run.profile})`);
  if (retro.broke.length > 0) lines.push(`- **Learned:** ${retro.broke.join("; ")}`);
  if (retro.conventions.length > 0) lines.push(`- **Decided:** ${retro.conventions.join("; ")}`);
  if (retro.avoid.length > 0) lines.push(`- **Avoid:** ${retro.avoid.join("; ")}`);
  lines.push(`- **Source:** .gate/runs/${run.id}`);

  return lines.join("\n") + "\n";
}

/**
 * Fixed binary, fixed argv — spawns exactly `agnosgram log --stdin --agent
 * gate` with no arguments read from repo config, so this sits outside Gate's
 * command-trust regime (TOFU): there is nothing a hostile `.gate/config.yml`
 * could inject, because config never reaches this call.
 */
const AGNOSGRAM_BIN = "agnosgram";
const AGNOSGRAM_ARGS = ["log", "--stdin", "--agent", "gate"];

export type JournalWriteMethod = "agnosgram-cli" | "fallback";

export interface JournalWriteResult {
  /** Repo-relative POSIX path to the journal file the entry landed in. */
  journalFile: string;
  method: JournalWriteMethod;
}

function journalMonth(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}`;
}

function monthHeader(month: string): string {
  return `# Journal — ${month}\n\nAppend-only. One file per month. Written by \`agnosgram log\` and, when the\nCLI is unavailable, by \`gate retro\`'s direct-append fallback.\n`;
}

/**
 * Append `entry` to the current month's journal file, creating the file (and
 * `.agnosgram/journal/`) if this is its first entry. Used only when spawning
 * the agnosgram CLI fails with ENOENT.
 */
function appendJournalDirect(root: string, entry: string, when: Date): string {
  const month = journalMonth(when);
  const relPath = join(".agnosgram", "journal", `${month}.md`);
  const absPath = join(root, relPath);
  mkdirSync(dirname(absPath), { recursive: true });
  const existing = existsSync(absPath) ? readFileSync(absPath, "utf8") : monthHeader(month);
  const withTrailingNewline = existing.endsWith("\n") ? existing : existing + "\n";
  writeFileAtomic(absPath, withTrailingNewline + "\n" + entry);
  return relPath.split("\\").join("/");
}

/**
 * Write a pre-formatted journal entry: prefer spawning the agnosgram CLI
 * (`agnosgram log --stdin --agent gate`, entry piped on stdin — agnosgram
 * appends stdin verbatim when it already starts with `##`); when the binary
 * isn't installed (ENOENT), fall back to appending directly in the same
 * frozen format. Any other spawn failure (binary present but errored) is
 * surfaced to the caller rather than silently falling back, since that
 * usually means something is actually wrong with the store.
 */
export function writeJournalEntry(root: string, entry: string, when: Date = new Date()): JournalWriteResult {
  const res = spawnSync(AGNOSGRAM_BIN, AGNOSGRAM_ARGS, { cwd: root, input: entry, encoding: "utf8" });

  if (res.error && (res.error as NodeJS.ErrnoException).code === "ENOENT") {
    const journalFile = appendJournalDirect(root, entry, when);
    return { journalFile, method: "fallback" };
  }
  if (res.error || res.status !== 0) {
    const detail = res.error ? res.error.message : (res.stderr || res.stdout || "").trim();
    throw new Error(`agnosgram log failed: ${detail}`);
  }

  const journalFile = join(".agnosgram", "journal", `${journalMonth(when)}.md`)
    .split("\\")
    .join("/");
  return { journalFile, method: "agnosgram-cli" };
}
