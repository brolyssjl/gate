import { parse as parseYaml } from "yaml";

/**
 * Shared YAML-frontmatter split, used by every artifact parser (plan.md,
 * debug-log.md, review.md, retro.md): a leading `---`-fenced YAML block
 * followed by a free-form markdown body. Each artifact's own parser still
 * owns its field-level validation - this only owns the mechanical split and
 * its three failure modes, which were previously copy-pasted four times.
 */
const FRONTMATTER_RE = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/;

export interface FrontmatterSplit {
  data: Record<string, unknown>;
  body: string;
}

export type FrontmatterResult =
  | { ok: true; value: FrontmatterSplit }
  | { ok: false; errors: string[] };

/** `fileLabel` (e.g. "plan.md") names the file in the "must start with..." error. */
export function splitFrontmatter(raw: string, fileLabel: string): FrontmatterResult {
  const m = FRONTMATTER_RE.exec(raw);
  if (!m) {
    return { ok: false, errors: [`${fileLabel} must start with a YAML frontmatter block (---)`] };
  }
  let fm: unknown;
  try {
    fm = parseYaml(m[1]!);
  } catch (e) {
    return { ok: false, errors: [`frontmatter is not valid YAML: ${(e as Error).message}`] };
  }
  if (!fm || typeof fm !== "object") {
    return { ok: false, errors: ["frontmatter must be a YAML mapping"] };
  }
  return { ok: true, value: { data: fm as Record<string, unknown>, body: (m[2] ?? "").trim() } };
}
