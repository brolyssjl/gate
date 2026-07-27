import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { runPaths } from "../core/paths.js";
import { hasSubstance, parseRetroFile } from "../artifacts/retro.js";
import { type Check, type GateContext, type GateResult, fail, pass, result } from "./types.js";

/**
 * RETRO gate — deterministic:
 *  - retro.md exists and follows the schema (broke/avoid/conventions lists)
 *  - at least one of the three questions was actually answered
 *  - when an Agnosgram store is present and the integration is not "off", the
 *    entry was synced to the journal (`gate retro`); otherwise this check is
 *    advisory (pass-with-note — there is nowhere to sync to)
 *
 * Whether the retro is *insightful* is judgment and lives in the RETRO
 * playbook; the gate only checks that the ritual happened and, where a store
 * exists, that the journal actually received it.
 */
export function retroGate(ctx: GateContext): GateResult {
  const checks: Check[] = [];
  const { retro: retroPath } = runPaths(ctx.root, ctx.run.id);

  const { retro, errors } = parseRetroFile(retroPath);
  if (!retro) {
    checks.push(fail("retro.schema", `retro.md invalid: ${errors.join("; ")}`));
    return result("RETRO", checks);
  }
  checks.push(pass("retro.schema", "retro.md matches the required schema"));

  checks.push(
    hasSubstance(retro)
      ? pass("retro.substance", "at least one of broke/avoid/conventions was answered")
      : fail("retro.substance", "retro.md is empty — answer at least one of broke/avoid/conventions"),
  );

  checks.push(journalCheck(ctx));

  return result("RETRO", checks);
}

function journalCheck(ctx: GateContext): Check {
  if (ctx.config.integrations.agnosgram === "off") {
    return pass("retro.journal", "agnosgram integration is off — journal sync not required");
  }
  if (!existsSync(join(ctx.root, ".agnosgram"))) {
    return pass("retro.journal", "no .agnosgram store detected — journal sync not required");
  }

  const sync = ctx.run.retro;
  if (!sync) {
    return fail("retro.journal", "no journal sync recorded — run `gate retro` to write the journal entry");
  }
  const journalPath = join(ctx.root, sync.journalFile);
  if (!existsSync(journalPath)) {
    return fail("retro.journal", `journal file not found: ${sync.journalFile} — re-run \`gate retro\``);
  }
  const content = readFileSync(journalPath, "utf8");
  if (!content.includes(ctx.run.id)) {
    return fail(
      "retro.journal",
      `journal file ${sync.journalFile} does not contain run id "${ctx.run.id}" — re-run \`gate retro\``,
    );
  }
  return pass("retro.journal", `journal entry recorded in ${sync.journalFile}`);
}
