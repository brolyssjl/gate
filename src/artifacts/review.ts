import { existsSync, readFileSync } from "node:fs";
import { stringify } from "yaml";
import { splitFrontmatter } from "./frontmatter.js";

/**
 * Severity ordering matters: `blocker` and `major` are *load-bearing* - the
 * REVIEW gate refuses to advance while any is still open. `minor`/`nit` are
 * advisory and never block.
 */
export const SEVERITIES = ["blocker", "major", "minor", "nit"] as const;
export type Severity = (typeof SEVERITIES)[number];
export const STATUSES = ["open", "resolved", "waived"] as const;
export type FindingStatus = (typeof STATUSES)[number];

export interface Finding {
  id: string;
  severity: Severity;
  status: FindingStatus;
  note: string;
  /** Human waiver rationale; required when status is `waived`. */
  waiver: string;
}

export interface Review {
  /** Reviewer identity as recorded in the findings file (advisory; run.json is authoritative). */
  reviewer: string;
  findings: Finding[];
  body: string;
}

export interface ReviewParse {
  review: Review | null;
  errors: string[];
}

export function parseReviewFile(path: string): ReviewParse {
  if (!existsSync(path)) return { review: null, errors: ["review.md does not exist"] };
  return parseReview(readFileSync(path, "utf8"));
}

export function parseReview(raw: string): ReviewParse {
  const split = splitFrontmatter(raw, "review.md");
  if (!split.ok) return { review: null, errors: split.errors };
  const { data } = split.value;
  const errors: string[] = [];

  const reviewer = typeof data.reviewer === "string" ? data.reviewer.trim() : "";

  const findings: Finding[] = [];
  const rawFindings = Array.isArray(data.findings) ? data.findings : [];
  const seenIds = new Set<string>();
  rawFindings.forEach((f, i) => {
    if (!f || typeof f !== "object") {
      errors.push(`findings[${i}] must be a mapping with id, severity, status`);
      return;
    }
    const obj = f as Record<string, unknown>;
    const id = typeof obj.id === "string" ? obj.id.trim() : "";
    const severity = typeof obj.severity === "string" ? obj.severity.trim() : "";
    const status = typeof obj.status === "string" ? obj.status.trim() : "";
    const note = typeof obj.note === "string" ? obj.note.trim() : "";
    const waiver = typeof obj.waiver === "string" ? obj.waiver.trim() : "";
    if (!id) errors.push(`findings[${i}].id is required`);
    else if (seenIds.has(id)) errors.push(`findings[${i}].id "${id}" is duplicated`);
    else seenIds.add(id);
    if (!isSeverity(severity)) errors.push(`findings[${id || i}].severity must be one of ${SEVERITIES.join("/")}`);
    if (!isStatus(status)) errors.push(`findings[${id || i}].status must be one of ${STATUSES.join("/")}`);
    if (id && isSeverity(severity) && isStatus(status)) {
      findings.push({ id, severity, status, note, waiver });
    }
  });

  if (errors.length > 0) return { review: null, errors };
  return { review: { reviewer, findings, body: split.value.body }, errors: [] };
}

function isSeverity(v: string): v is Severity {
  return (SEVERITIES as readonly string[]).includes(v);
}
function isStatus(v: string): v is FindingStatus {
  return (STATUSES as readonly string[]).includes(v);
}

/** Findings that block advancement: blocker/major that are still open. */
export function blockingFindings(review: Review): Finding[] {
  return review.findings.filter(
    (f) => (f.severity === "blocker" || f.severity === "major") && f.status === "open",
  );
}

/** Waived blocker/major findings missing the required human rationale. */
export function unjustifiedWaivers(review: Review): Finding[] {
  return review.findings.filter(
    (f) => (f.severity === "blocker" || f.severity === "major") && f.status === "waived" && !f.waiver,
  );
}

/**
 * Render review.md from a reviewer + findings list (Milestone 4: `gate review
 * --human` writes this after walking the rubric interactively) - the same
 * shape the REVIEW gate parses via `parseReviewFile`, so a human-recorded
 * review satisfies it exactly like an agent-edited one.
 */
export function serializeReview(reviewer: string, findings: Finding[]): string {
  const frontmatter = stringify({ reviewer, findings: findings.map(({ id, severity, status, note, waiver }) => ({
    id,
    severity,
    status,
    note,
    ...(status === "waived" ? { waiver } : {}),
  })) });
  return [
    "---",
    frontmatter.trimEnd(),
    "---",
    "",
    "# Review",
    "",
    "Recorded via `gate review --human`.",
    "",
  ].join("\n");
}
