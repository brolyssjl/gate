import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { parse as parseYaml } from "yaml";

export interface Criterion {
  id: string;
  text: string;
  /**
   * How the criterion is verified. Mechanically meaningful prefixes:
   *   `test: <substring>` — a named test whose title contains <substring> must
   *                          run and pass (checked by the TEST gate).
   *   `manual`            — verified by a human; the TEST gate accepts it as-is.
   * Any other value is treated as free-form and satisfies "checkable" only by
   * being non-empty (a verification method was declared).
   */
  verify: string;
}

export interface Plan {
  goal: string;
  /** Repo-relative path to an SDD spec this plan cites, e.g. "openspec/changes/x/spec.md". */
  spec: string | null;
  files: string[];
  out_of_scope: string[];
  criteria: Criterion[];
  risks: string[];
  /** Free-form markdown body (may cite an SDD spec path). */
  body: string;
}

export interface PlanParse {
  plan: Plan | null;
  errors: string[];
}

const FRONTMATTER = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/;

export function parsePlanFile(path: string): PlanParse {
  if (!existsSync(path)) return { plan: null, errors: ["plan.md does not exist"] };
  return parsePlan(readFileSync(path, "utf8"));
}

export function parsePlan(raw: string): PlanParse {
  const m = FRONTMATTER.exec(raw);
  if (!m) {
    return { plan: null, errors: ["plan.md must start with a YAML frontmatter block (---)"] };
  }
  let fm: unknown;
  try {
    fm = parseYaml(m[1]!);
  } catch (e) {
    return { plan: null, errors: [`frontmatter is not valid YAML: ${(e as Error).message}`] };
  }
  if (!fm || typeof fm !== "object") {
    return { plan: null, errors: ["frontmatter must be a YAML mapping"] };
  }
  const data = fm as Record<string, unknown>;
  const errors: string[] = [];

  const goal = typeof data.goal === "string" ? data.goal.trim() : "";
  if (!goal) errors.push("`goal` is required and must be a non-empty string");

  const specRaw = typeof data.spec === "string" ? data.spec.trim() : "";
  const spec = specRaw.length > 0 ? specRaw : null;

  const files = asStringArray(data.files);
  if (files.length === 0) errors.push("`files` must list at least one path or glob");

  const out_of_scope = asStringArray(data.out_of_scope);
  const risks = asStringArray(data.risks);

  const criteria: Criterion[] = [];
  const rawCriteria = Array.isArray(data.criteria) ? data.criteria : [];
  if (rawCriteria.length === 0) {
    errors.push("`criteria` must list at least one acceptance criterion");
  }
  const seenIds = new Set<string>();
  rawCriteria.forEach((c, i) => {
    if (!c || typeof c !== "object") {
      errors.push(`criteria[${i}] must be a mapping with id, text, verify`);
      return;
    }
    const obj = c as Record<string, unknown>;
    const id = typeof obj.id === "string" ? obj.id.trim() : "";
    const text = typeof obj.text === "string" ? obj.text.trim() : "";
    const verify = typeof obj.verify === "string" ? obj.verify.trim() : "";
    if (!id) errors.push(`criteria[${i}].id is required`);
    else if (seenIds.has(id)) errors.push(`criteria[${i}].id "${id}" is duplicated`);
    else seenIds.add(id);
    if (!text) errors.push(`criteria[${i}].text is required`);
    // "each criterion is checkable" is enforced mechanically as: a verification
    // method must be declared (non-empty `verify`).
    if (!verify) errors.push(`criteria[${id || i}].verify is required (how is this checked?)`);
    if (id && text && verify) criteria.push({ id, text, verify });
  });

  if (errors.length > 0) return { plan: null, errors };
  return {
    plan: { goal, spec, files, out_of_scope, criteria, risks, body: (m[2] ?? "").trim() },
    errors: [],
  };
}

function asStringArray(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  return v.filter((x): x is string => typeof x === "string" && x.trim().length > 0).map((x) => x.trim());
}

/** Content hash of a plan, binding an approval to the exact plan it approved. */
export function hashPlan(raw: string): string {
  return "sha256:" + createHash("sha256").update(raw).digest("hex");
}

export function hashPlanFile(path: string): string | null {
  if (!existsSync(path)) return null;
  return hashPlan(readFileSync(path, "utf8"));
}
