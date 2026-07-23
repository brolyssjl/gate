import { existsSync, readFileSync } from "node:fs";
import { parse as parseYaml } from "yaml";

/**
 * A single pass of the debugging protocol: reproduce → hypothesize → predict →
 * test → conclude. A cycle is *complete* only when every field is filled in and
 * `status: complete`; the DEBUG gate requires at least one complete cycle so the
 * agent can't claim a fix without recording how it got there.
 */
export interface DebugCycle {
  hypothesis: string;
  prediction: string;
  experiment: string;
  observation: string;
  conclusion: string;
  status: "complete" | "in-progress";
}

export interface DebugLog {
  /** The failing test this session set out to fix (substring of its title). */
  triggeringTest: string;
  /** Whether the bug was reproduced before diagnosing it. */
  reproduced: boolean;
  cycles: DebugCycle[];
  body: string;
}

export interface DebugLogParse {
  log: DebugLog | null;
  errors: string[];
}

const FRONTMATTER = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/;
const CYCLE_FIELDS = ["hypothesis", "prediction", "experiment", "observation", "conclusion"] as const;

export function parseDebugLogFile(path: string): DebugLogParse {
  if (!existsSync(path)) return { log: null, errors: ["debug-log.md does not exist"] };
  return parseDebugLog(readFileSync(path, "utf8"));
}

export function parseDebugLog(raw: string): DebugLogParse {
  const m = FRONTMATTER.exec(raw);
  if (!m) {
    return { log: null, errors: ["debug-log.md must start with a YAML frontmatter block (---)"] };
  }
  let fm: unknown;
  try {
    fm = parseYaml(m[1]!);
  } catch (e) {
    return { log: null, errors: [`frontmatter is not valid YAML: ${(e as Error).message}`] };
  }
  if (!fm || typeof fm !== "object") {
    return { log: null, errors: ["frontmatter must be a YAML mapping"] };
  }
  const data = fm as Record<string, unknown>;
  const errors: string[] = [];

  const triggeringTest = typeof data.triggering_test === "string" ? data.triggering_test.trim() : "";
  if (!triggeringTest) errors.push("`triggering_test` is required (name/substring of the failing test)");

  const reproduced = data.reproduced === true;

  const cycles: DebugCycle[] = [];
  const rawCycles = Array.isArray(data.cycles) ? data.cycles : [];
  if (rawCycles.length === 0) errors.push("`cycles` must list at least one debugging cycle");
  rawCycles.forEach((c, i) => {
    if (!c || typeof c !== "object") {
      errors.push(`cycles[${i}] must be a mapping`);
      return;
    }
    const obj = c as Record<string, unknown>;
    const fields = Object.fromEntries(
      CYCLE_FIELDS.map((f) => [f, typeof obj[f] === "string" ? (obj[f] as string).trim() : ""]),
    ) as Record<(typeof CYCLE_FIELDS)[number], string>;
    const status = obj.status === "complete" ? "complete" : "in-progress";
    cycles.push({ ...fields, status });
  });

  if (errors.length > 0) return { log: null, errors };
  return {
    log: { triggeringTest, reproduced, cycles, body: (m[2] ?? "").trim() },
    errors: [],
  };
}

/** A cycle counts as complete when every protocol field is filled and status is complete. */
export function isCycleComplete(cycle: DebugCycle): boolean {
  return cycle.status === "complete" && CYCLE_FIELDS.every((f) => cycle[f].length > 0);
}
