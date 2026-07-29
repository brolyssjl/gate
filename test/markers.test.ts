import { describe, expect, it } from "vitest";
import { END_MARKER, hasManagedBlock, START_MARKER, upsertManagedBlock } from "../src/core/markers.js";

describe("managed-block markers", () => {
  it("appends a managed block to user content, preserving it", () => {
    const existing = "# My rules\n\nAlways be nice.\n";
    const out = upsertManagedBlock(existing, "BODY");
    expect(out.startsWith("# My rules\n\nAlways be nice.")).toBe(true);
    expect(out).toContain(START_MARKER);
    expect(out).toContain(END_MARKER);
    expect(out).toContain("BODY");
  });

  it("is idempotent - running twice yields identical output (golden invariant)", () => {
    const existing = "# My rules\n\nkeep me\n";
    const once = upsertManagedBlock(existing, "BODY v1");
    const twice = upsertManagedBlock(once, "BODY v1");
    expect(once).toBe(twice);
  });

  it("replaces the block body without touching surrounding user content", () => {
    const existing = "top\n";
    const v1 = upsertManagedBlock(existing, "OLD");
    const v2 = upsertManagedBlock(v1 + "\nuser added this later\n", "NEW");
    expect(v2).toContain("NEW");
    expect(v2).not.toContain("OLD");
    expect(v2.startsWith("top")).toBe(true);
    expect(v2).toContain("user added this later");
  });

  it("collapses accidental duplicate blocks into one", () => {
    const block = `${START_MARKER}\nx\n${END_MARKER}`;
    const existing = `a\n\n${block}\n\nb\n\n${block}\n`;
    const out = upsertManagedBlock(existing, "ONE");
    const count = out.split(START_MARKER).length - 1;
    expect(count).toBe(1);
    expect(out).toContain("a");
    expect(out).toContain("b");
  });

  it("handles empty input", () => {
    const out = upsertManagedBlock("", "BODY");
    expect(hasManagedBlock(out)).toBe(true);
  });

  it("never deletes user content around an orphaned start marker (no matching end)", () => {
    const existing = `keep this\n\n${START_MARKER}\nnot a real block, no end marker\nstill user content\n`;

    const once = upsertManagedBlock(existing, "BODY v1");
    // The orphan and everything after it is untouched - only appended to.
    expect(once).toContain("keep this");
    expect(once).toContain("not a real block, no end marker");
    expect(once).toContain("still user content");
    expect(once.startsWith(existing.replace(/\s*$/, ""))).toBe(true);

    const twice = upsertManagedBlock(once, "BODY v2");
    // Running it again must not delete anything either - byte-for-byte the
    // original orphan and its trailing user content still survive verbatim.
    expect(twice).toContain("keep this");
    expect(twice).toContain("not a real block, no end marker");
    expect(twice).toContain("still user content");
    expect(twice.startsWith(existing.replace(/\s*$/, ""))).toBe(true);
  });
});
