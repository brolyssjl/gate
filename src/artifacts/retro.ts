import { existsSync, readFileSync } from "node:fs";
import { splitFrontmatter } from "./frontmatter.js";

/**
 * The RETRO phase asks three questions (proposal §3): what broke, what to
 * avoid next time, what convention emerged. All three lists may be empty
 * individually - `hasSubstance` is the gate's real bar: at least one answer
 * across the three, so a run can't coast through on an untouched scaffold.
 */
export interface RetroLog {
  broke: string[];
  avoid: string[];
  conventions: string[];
  body: string;
}

export interface RetroParse {
  retro: RetroLog | null;
  errors: string[];
}

/**
 * The scaffold `gate start` writes for a run whose profile walks through
 * RETRO, and that `advance()` also writes when a pre-Milestone-3 run (started
 * before RETRO scaffolding existed) transitions into RETRO and finds no
 * retro.md waiting - otherwise that run stalls forever with no way to satisfy
 * the RETRO gate. `%TITLE%` is replaced by the caller.
 */
export const RETRO_TEMPLATE = `---
broke: []
avoid: []
conventions: []
---

# Retro: %TITLE%

`;

export function parseRetroFile(path: string): RetroParse {
  if (!existsSync(path)) return { retro: null, errors: ["retro.md does not exist"] };
  return parseRetro(readFileSync(path, "utf8"));
}

export function parseRetro(raw: string): RetroParse {
  const split = splitFrontmatter(raw, "retro.md");
  if (!split.ok) return { retro: null, errors: split.errors };
  const { data, body } = split.value;

  const broke = asStringArray(data.broke);
  const avoid = asStringArray(data.avoid);
  const conventions = asStringArray(data.conventions);

  return {
    retro: { broke, avoid, conventions, body },
    errors: [],
  };
}

function asStringArray(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  return v.filter((x): x is string => typeof x === "string" && x.trim().length > 0).map((x) => x.trim());
}

/** At least one of the three questions was actually answered. */
export function hasSubstance(retro: RetroLog): boolean {
  return retro.broke.length + retro.avoid.length + retro.conventions.length > 0;
}
