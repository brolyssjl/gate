import { existsSync, readFileSync } from "node:fs";

/**
 * Parser for Agnosgram's `agnosgram_advise` report schema (pinned cross-project
 * contract, owned by Agnosgram). Agnosgram writes the report to
 * `<plan>.advise.json` by default. Gate only *reads* this file - it never
 * shells out to `agnosgram advise` itself (integrations are advisory, never
 * load-bearing): a missing or unparseable report is treated as "no report",
 * never as an error.
 */
export interface AdviseContradiction {
  record_id: string;
  kind: string;
  severity: string;
  plan_excerpt: string;
  record_excerpt: string;
  confidence: string;
  last_verified: string;
  explanation: string;
}

export interface AdviseReport {
  version: number;
  plan: string;
  generated: string;
  checked_ids: string[];
  contradictions: AdviseContradiction[];
  clear: boolean;
}

/** Where `agnosgram advise` writes its report for a given plan.md path. */
export function adviseReportPath(planPath: string): string {
  return planPath + ".advise.json";
}

/**
 * Parse the pinned `agnosgram_advise` schema. Tolerant of extra fields (forward
 * compatibility); returns null on anything that doesn't look like a report at
 * all - Gate treats that identically to a missing file.
 */
/** `v` if it's a string, else "" - the report's fields are all optional/untrusted input. */
function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

export function parseAdviseReport(raw: string): AdviseReport | null {
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!data || typeof data !== "object") return null;
  const obj = data as Record<string, unknown>;
  if (typeof obj.agnosgram_advise !== "number") return null;

  const contradictions: AdviseContradiction[] = [];
  if (Array.isArray(obj.contradictions)) {
    for (const c of obj.contradictions) {
      if (!c || typeof c !== "object") continue;
      const rec = c as Record<string, unknown>;
      const record_id = str(rec.record_id);
      if (!record_id) continue;
      contradictions.push({
        record_id,
        kind: str(rec.kind),
        severity: str(rec.severity),
        plan_excerpt: str(rec.plan_excerpt),
        record_excerpt: str(rec.record_excerpt),
        confidence: str(rec.confidence),
        last_verified: str(rec.last_verified),
        explanation: str(rec.explanation),
      });
    }
  }

  return {
    version: obj.agnosgram_advise,
    plan: str(obj.plan),
    generated: str(obj.generated),
    checked_ids: Array.isArray(obj.checked_ids) ? obj.checked_ids.filter((x): x is string => typeof x === "string") : [],
    contradictions,
    clear: obj.clear === true,
  };
}

/** Load and parse the advise report for `planPath`; null when absent or unparseable ("no report"). */
export function loadAdviseReport(planPath: string): AdviseReport | null {
  const path = adviseReportPath(planPath);
  if (!existsSync(path)) return null;
  try {
    return parseAdviseReport(readFileSync(path, "utf8"));
  } catch {
    return null;
  }
}
