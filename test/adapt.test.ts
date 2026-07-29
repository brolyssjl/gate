import { describe, expect, it } from "vitest";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { ADAPTERS, ADAPTER_KEYS } from "../src/adapters/index.js";
import { applyAdapter } from "../src/commands/adapt.js";
import { makeRepo } from "./helpers.js";

describe("applyAdapter (golden idempotence)", () => {
  it("creates CLAUDE.md and is idempotent (run twice = byte-identical)", () => {
    const root = makeRepo();
    const first = applyAdapter(root, ADAPTERS.claude!);
    expect(first.action).toBe("created");
    const contentA = readFileSync(join(root, "CLAUDE.md"), "utf8");

    const second = applyAdapter(root, ADAPTERS.claude!);
    expect(second.action).toBe("unchanged");
    const contentB = readFileSync(join(root, "CLAUDE.md"), "utf8");
    expect(contentA).toBe(contentB);
  });

  it("preserves user content outside the markers and survives edits above the block", () => {
    const root = makeRepo();
    const userText = "# House rules\n\nBe excellent to each other.\n";
    writeFileSync(join(root, "CLAUDE.md"), userText);

    applyAdapter(root, ADAPTERS.claude!);
    const out = readFileSync(join(root, "CLAUDE.md"), "utf8");
    expect(out.startsWith("# House rules\n\nBe excellent to each other.")).toBe(true);
    expect(out).toContain("Gate quality flow");

    const edited = "# House rules v2\n\nNew note.\n" + out.slice(userText.length);
    writeFileSync(join(root, "CLAUDE.md"), edited);
    applyAdapter(root, ADAPTERS.claude!);
    const out2 = readFileSync(join(root, "CLAUDE.md"), "utf8");
    expect(out2.startsWith("# House rules v2\n\nNew note.")).toBe(true);
  });

  it("writes every known adapter's dedicated file with its preamble intact and idempotently", () => {
    const root = makeRepo();
    for (const key of ADAPTER_KEYS) {
      const adapter = ADAPTERS[key]!;
      const first = applyAdapter(root, adapter);
      expect(first.action).toBe("created");
      const file = join(root, adapter.targetPath);
      expect(existsSync(file)).toBe(true);
      const contentA = readFileSync(file, "utf8");
      if (adapter.preamble) expect(contentA.startsWith(adapter.preamble)).toBe(true);
      expect(contentA).toContain("Gate quality flow");

      const second = applyAdapter(root, adapter);
      expect(second.action).toBe("unchanged");
      expect(readFileSync(file, "utf8")).toBe(contentA);
    }
  });

  it("cursor adapter writes a dedicated .mdc file with frontmatter", () => {
    const root = makeRepo();
    const res = applyAdapter(root, ADAPTERS.cursor!);
    expect(res.action).toBe("created");
    const content = readFileSync(join(root, ".cursor/rules/gate.mdc"), "utf8");
    expect(content).toContain("alwaysApply: true");
    expect(content).toContain("Gate quality flow");
  });
});
