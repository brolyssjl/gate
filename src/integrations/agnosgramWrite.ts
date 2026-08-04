import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { writeFileAtomic } from "../core/fsx.js";
import { GateError } from "../cli/output.js";
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
 * Fixed argv, binary resolved once per call from `$GATE_AGNOSGRAM_BIN` (or
 * `agnosgram` on PATH by default) - no arguments read from repo config, so
 * this sits outside Gate's command-trust regime (TOFU): there is nothing a
 * hostile `.gate/config.yml` could inject, because config never reaches this
 * call. The env override exists for test hermeticity (review finding F3):
 * whether a real `agnosgram` happens to be installed on the machine running
 * the suite must not change what these tests exercise - fallback tests point
 * it at a path that can't possibly exist, CLI-path tests at a stub script,
 * so the suite is green regardless of what's actually on PATH.
 */
function agnosgramBin(): string {
  return process.env.GATE_AGNOSGRAM_BIN || "agnosgram";
}
const AGNOSGRAM_ARGS = ["log", "--stdin", "--agent", "gate"];

export type JournalWriteMethod = "agnosgram-cli" | "fallback";

export interface JournalWriteResult {
  /** Repo-relative POSIX path to the journal file the entry landed in. */
  journalFile: string;
  method: JournalWriteMethod;
}

/**
 * The journal month, in UTC - matching the Agnosgram CLI's own contract
 * (`toISOString().slice(0, 7)`). Using local time here would pick a different
 * month than the CLI near a month boundary in any timezone ahead of or behind
 * UTC, producing wrong receipts (the file Gate records in run.json isn't the
 * one the CLI actually wrote to) and duplicate entries on retry. Must stay in
 * sync in both the CLI-success path and the ENOENT direct-append fallback
 * below - both funnel through this one function.
 */
function journalMonth(d: Date): string {
  return d.toISOString().slice(0, 7);
}

/** Repo-relative POSIX path to the month's journal file `when` falls in (UTC). */
export function journalFilePath(when: Date): string {
  return join(".agnosgram", "journal", `${journalMonth(when)}.md`)
    .split("\\")
    .join("/");
}

function monthHeader(month: string): string {
  return `# Journal - ${month}\n\nAppend-only. One file per month. Written by \`agnosgram log\` and, when the\nCLI is unavailable, by \`gate retro\`'s direct-append fallback.\n`;
}

/**
 * Append `entry` to the current month's journal file, creating the file (and
 * `.agnosgram/journal/`) if this is its first entry. Used only when spawning
 * the agnosgram CLI fails with ENOENT.
 */
function appendJournalDirect(root: string, entry: string, when: Date): string {
  const relPath = journalFilePath(when);
  const absPath = join(root, relPath);
  mkdirSync(dirname(absPath), { recursive: true });
  const existing = existsSync(absPath) ? readFileSync(absPath, "utf8") : monthHeader(journalMonth(when));
  const withTrailingNewline = existing.endsWith("\n") ? existing : existing + "\n";
  writeFileAtomic(absPath, withTrailingNewline + "\n" + entry);
  return relPath;
}

/**
 * Write a pre-formatted journal entry: prefer spawning the agnosgram CLI
 * (`agnosgram log --stdin --agent gate`, entry piped on stdin - agnosgram
 * appends stdin verbatim when it already starts with `##`); when the binary
 * isn't installed (ENOENT), fall back to appending directly in the same
 * frozen format. Any other spawn failure (binary present but errored) is
 * surfaced to the caller rather than silently falling back, since that
 * usually means something is actually wrong with the store.
 */
export function writeJournalEntry(root: string, entry: string, when: Date = new Date()): JournalWriteResult {
  const res = spawnSync(agnosgramBin(), AGNOSGRAM_ARGS, { cwd: root, input: entry, encoding: "utf8" });

  if (res.error && (res.error as NodeJS.ErrnoException).code === "ENOENT") {
    const journalFile = appendJournalDirect(root, entry, when);
    return { journalFile, method: "fallback" };
  }
  if (res.error || res.status !== 0) {
    const detail = res.error ? res.error.message : (res.stderr || res.stdout || "").trim();
    throw new GateError(`agnosgram log failed: ${detail}`);
  }

  return { journalFile: journalFilePath(when), method: "agnosgram-cli" };
}

/** Whether the journal file at `journalFile` (repo-relative) already records `runId`. */
export function journalContainsRunId(root: string, journalFile: string, runId: string): boolean {
  const path = join(root, journalFile);
  if (!existsSync(path)) return false;
  return readFileSync(path, "utf8").includes(runId);
}
