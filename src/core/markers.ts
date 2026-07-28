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
 * Insert or replace the managed block in `existing`. When no complete block is
 * present the managed block is appended (with one blank-line separator) so user
 * content stays on top. When present, the first block is replaced in place and
 * any stray duplicate blocks are removed.
 *
 * A *complete* block requires its start marker's nearest following end marker to
 * close it before any other start marker begins. Without that guard, an orphaned
 * start marker (start with no matching end - e.g. a file with an old, manually
 * truncated block) plus a later, correctly-appended block would make the lazy
 * `BLOCK_RE` match span from the orphan all the way through the real block's end
 * marker, and the intervening user content between them would be deleted on
 * replace. Treating that case as "no complete block" and only ever appending
 * means user content is never deleted, only ever added after.
 */
export function upsertManagedBlock(existing: string, body: string): string {
  const managed = wrapManagedBlock(body);
  if (!hasCompleteFirstBlock(existing)) {
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

/** Whether the first START_MARKER in `text` closes with an END_MARKER before any other START_MARKER begins. */
function hasCompleteFirstBlock(text: string): boolean {
  const start = text.indexOf(START_MARKER);
  if (start === -1) return false;
  const end = text.indexOf(END_MARKER, start);
  if (end === -1) return false; // orphaned start marker: no end anywhere after it
  const nextStart = text.indexOf(START_MARKER, start + START_MARKER.length);
  return nextStart === -1 || nextStart > end;
}

export function hasManagedBlock(existing: string): boolean {
  return new RegExp(BLOCK_RE.source).test(existing);
}
