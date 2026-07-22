import { describe, expect, it } from "vitest";
import { encodeToon } from "../src/serialize/toon.js";
import { serialize } from "../src/serialize/index.js";

describe("TOON encoder", () => {
  it("encodes primitives", () => {
    expect(encodeToon(42)).toBe("42");
    expect(encodeToon(true)).toBe("true");
    expect(encodeToon(null)).toBe("null");
  });

  it("encodes a flat object as key: value lines", () => {
    expect(encodeToon({ phase: "PLAN", ok: false })).toBe("phase: PLAN\nok: false");
  });

  it("renders a uniform array of flat objects as a table (the token win)", () => {
    const value = {
      checks: [
        { name: "a", ok: true },
        { name: "b", ok: false },
      ],
    };
    expect(encodeToon(value)).toBe(
      ["checks[2]{name,ok}:", "  a,true", "  b,false"].join("\n"),
    );
  });

  it("inlines a primitive array", () => {
    expect(encodeToon({ files: ["a.ts", "b.ts"] })).toBe('files[2]: a.ts,b.ts');
  });

  it("renders an empty array with a zero count", () => {
    expect(encodeToon({ risks: [] })).toBe("risks[0]:");
  });

  it("falls back to list form for non-uniform arrays", () => {
    const out = encodeToon({ items: [{ a: 1 }, { a: 1, b: 2 }] });
    expect(out).toContain("items[2]:");
    expect(out).toContain("- ");
  });

  it("quotes values containing separators", () => {
    expect(encodeToon({ msg: "a, b: c" })).toBe('msg: "a, b: c"');
  });

  it("is selected via the serializer facade", () => {
    expect(serialize({ n: 1 }, "toon")).toBe("n: 1");
    expect(serialize({ n: 1 }, "json")).toBe('{\n  "n": 1\n}');
  });
});
