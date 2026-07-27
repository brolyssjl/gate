/**
 * Managed-block injection (ported from Agnosgram's `src/core/markers.ts`,
 * same design). Gate owns only the text between its markers in an agent
 * config file; everything else the user wrote is preserved byte-for-byte.
 * Rewriting is idempotent: running it twice yields an identical file.
 */

export const START_MARKER = "<!-- gate:start -->";
export const END_MARKER = "<!-- gate:end -->";

const BLOCK_RE = new RegExp(
  `${escapeRegExp(START_MARKER)}[\\s\\S]*?${escapeRegExp(END_MARKER)}`,
  "g",
);

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Wrap block body in markers with a note that the region is tool-managed. */
export function wrapManagedBlock(body: string): string {
  return `${START_MARKER}\n<!-- Managed by gate. Edits inside this block are overwritten on \`gate adapt\`. -->\n${body.trimEnd()}\n${END_MARKER}`;
}

/**
 * Insert or replace the managed block in `existing`. When no block is present the
 * managed block is appended (with one blank-line separator) so user content stays
 * on top. When present, the first block is replaced in place and any stray
 * duplicate blocks are removed.
 */
export function upsertManagedBlock(existing: string, body: string): string {
  const managed = wrapManagedBlock(body);
  if (!BLOCK_RE.test(existing)) {
    const base = existing.replace(/\s*$/, "");
    return base === "" ? managed + "\n" : `${base}\n\n${managed}\n`;
  }
  let replaced = false;
  const result = existing.replace(BLOCK_RE, () => {
    if (replaced) return ""; // collapse accidental duplicate blocks
    replaced = true;
    return managed;
  });
  // Clean up any blank runs left by removed duplicates, keep a trailing newline.
  return result.replace(/\n{3,}/g, "\n\n").replace(/\s*$/, "") + "\n";
}

export function hasManagedBlock(existing: string): boolean {
  return new RegExp(BLOCK_RE.source).test(existing);
}
