import { existsSync, readFileSync } from "node:fs";
import { parse as parseYaml } from "yaml";

/**
 * The RETRO phase asks three questions (proposal §3): what broke, what to
 * avoid next time, what convention emerged. All three lists may be empty
 * individually — `hasSubstance` is the gate's real bar: at least one answer
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

const FRONTMATTER = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/;

export function parseRetroFile(path: string): RetroParse {
  if (!existsSync(path)) return { retro: null, errors: ["retro.md does not exist"] };
  return parseRetro(readFileSync(path, "utf8"));
}

export function parseRetro(raw: string): RetroParse {
  const m = FRONTMATTER.exec(raw);
  if (!m) {
    return { retro: null, errors: ["retro.md must start with a YAML frontmatter block (---)"] };
  }
  let fm: unknown;
  try {
    fm = parseYaml(m[1]!);
  } catch (e) {
    return { retro: null, errors: [`frontmatter is not valid YAML: ${(e as Error).message}`] };
  }
  if (!fm || typeof fm !== "object") {
    return { retro: null, errors: ["frontmatter must be a YAML mapping"] };
  }
  const data = fm as Record<string, unknown>;

  const broke = asStringArray(data.broke);
  const avoid = asStringArray(data.avoid);
  const conventions = asStringArray(data.conventions);

  return {
    retro: { broke, avoid, conventions, body: (m[2] ?? "").trim() },
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
