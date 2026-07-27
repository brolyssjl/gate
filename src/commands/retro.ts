import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { runPaths } from "../core/paths.js";
import { currentBranch } from "../core/git.js";
import { nowIso, writeRun } from "../core/run.js";
import { hasSubstance, parseRetroFile } from "../artifacts/retro.js";
import { formatJournalEntry, writeJournalEntry } from "../integrations/agnosgramWrite.js";
import { emit, GateError, requireActiveRun, type ParsedArgs } from "./shared.js";

/**
 * `gate retro` — sync the current run's retro.md into the Agnosgram journal
 * (source-linked to this run), when a store is present. Phase-guarded to
 * RETRO; refuses a substance-less retro (nothing worth syncing). Idempotent:
 * re-running after a successful sync is a no-op as long as the journal entry
 * is still there. Advisory by design — with no `.agnosgram/` store, this is a
 * no-op and the RETRO gate does not require it.
 */
export function cmdRetro(args: ParsedArgs): void {
  const { root, run, config } = requireActiveRun();
  if (run.phase !== "RETRO") {
    throw new GateError(`nothing to sync — run is in ${run.phase}, not RETRO`);
  }

  const paths = runPaths(root, run.id);
  const { retro, errors } = parseRetroFile(paths.retro);
  if (!retro) {
    throw new GateError(`cannot sync an invalid retro: ${errors.join("; ")}`);
  }
  if (!hasSubstance(retro)) {
    throw new GateError("retro.md has no substance — answer at least one of broke/avoid/conventions before syncing");
  }

  if (config.integrations.agnosgram === "off" || !existsSync(join(root, ".agnosgram"))) {
    emit(
      "No .agnosgram/ store detected — nothing to sync. The RETRO gate does not require a journal entry.",
      { synced: false, reason: "no-store" },
      args.flags,
    );
    return;
  }

  if (run.retro && journalContainsRunId(root, run.retro.journalFile, run.id)) {
    emit(
      `Already synced to ${run.retro.journalFile} — nothing to do.`,
      { synced: true, journalFile: run.retro.journalFile, method: run.retro.method, alreadySynced: true },
      args.flags,
    );
    return;
  }

  const entry = formatJournalEntry({
    run: { id: run.id, title: run.title, profile: run.profile },
    retro,
    branch: currentBranch(root),
  });
  const { journalFile, method } = writeJournalEntry(root, entry);

  run.retro = { journalFile, syncedAt: nowIso(), method };
  writeRun(root, run);

  emit(
    `Synced retro to ${journalFile} (${method})`,
    { synced: true, journalFile, method, alreadySynced: false },
    args.flags,
  );
}

function journalContainsRunId(root: string, journalFile: string, runId: string): boolean {
  const path = join(root, journalFile);
  if (!existsSync(path)) return false;
  return readFileSync(path, "utf8").includes(runId);
}
