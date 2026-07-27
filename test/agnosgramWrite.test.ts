import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { formatJournalEntry, writeJournalEntry } from "../src/integrations/agnosgramWrite.js";
import { makeRepo } from "./helpers.js";

const WHEN = new Date(2026, 6, 27, 14, 5); // 2026-07-27 14:05, local

describe("formatJournalEntry (golden — frozen Agnosgram journal format v1)", () => {
  it("renders every slot when broke/avoid/conventions are all answered", () => {
    const entry = formatJournalEntry({
      run: { id: "2026-07-27-add-greet", title: "add greet", profile: "feature" },
      retro: {
        broke: ["assumed the API was idempotent"],
        avoid: ["skipping the reproduce step"],
        conventions: ["always add --dry-run to destructive commands"],
        body: "",
      },
      branch: "milestone-3-ecosystem",
      when: WHEN,
    });

    expect(entry).toBe(
      [
        "## 2026-07-27 14:05 · gate · milestone-3-ecosystem",
        '- **Did:** Completed gate run "add greet" (2026-07-27-add-greet, profile feature)',
        "- **Learned:** assumed the API was idempotent",
        "- **Decided:** always add --dry-run to destructive commands",
        "- **Avoid:** skipping the reproduce step",
        "- **Source:** .gate/runs/2026-07-27-add-greet",
        "",
      ].join("\n"),
    );
  });

  it("omits empty slots and the branch segment when unknown", () => {
    const entry = formatJournalEntry({
      run: { id: "2026-07-27-fix-login", title: "fix login", profile: "bugfix" },
      retro: { broke: [], avoid: ["forgetting to check the token expiry edge case"], conventions: [], body: "" },
      branch: null,
      when: WHEN,
    });

    expect(entry).toBe(
      [
        "## 2026-07-27 14:05 · gate",
        '- **Did:** Completed gate run "fix login" (2026-07-27-fix-login, profile bugfix)',
        "- **Avoid:** forgetting to check the token expiry edge case",
        "- **Source:** .gate/runs/2026-07-27-fix-login",
        "",
      ].join("\n"),
    );
  });

  it("joins multiple entries in a slot with '; '", () => {
    const entry = formatJournalEntry({
      run: { id: "r1", title: "t", profile: "feature" },
      retro: { broke: ["one thing broke", "another thing broke"], avoid: [], conventions: [], body: "" },
      branch: "main",
      when: WHEN,
    });
    expect(entry).toContain("- **Learned:** one thing broke; another thing broke");
  });
});

describe("writeJournalEntry fallback (agnosgram binary not on PATH in this sandbox)", () => {
  it("creates the month's journal file and appends the entry", () => {
    const root = makeRepo();
    const entry = formatJournalEntry({
      run: { id: "r1", title: "demo", profile: "feature" },
      retro: { broke: ["x"], avoid: [], conventions: [], body: "" },
      branch: "main",
      when: WHEN,
    });

    const result = writeJournalEntry(root, entry, WHEN);
    expect(result.method).toBe("fallback");
    expect(result.journalFile).toBe(".agnosgram/journal/2026-07.md");

    const abs = join(root, result.journalFile);
    expect(existsSync(abs)).toBe(true);
    const content = readFileSync(abs, "utf8");
    expect(content).toContain("## 2026-07-27 14:05 · gate · main");
    expect(content).toContain("- **Source:** .gate/runs/r1");
  });

  it("appends a second entry to an existing month file without clobbering the first", () => {
    const root = makeRepo();
    const first = formatJournalEntry({
      run: { id: "r1", title: "first", profile: "feature" },
      retro: { broke: ["a"], avoid: [], conventions: [], body: "" },
      branch: "main",
      when: WHEN,
    });
    const second = formatJournalEntry({
      run: { id: "r2", title: "second", profile: "feature" },
      retro: { broke: ["b"], avoid: [], conventions: [], body: "" },
      branch: "main",
      when: WHEN,
    });

    const r1 = writeJournalEntry(root, first, WHEN);
    const r2 = writeJournalEntry(root, second, WHEN);
    expect(r1.journalFile).toBe(r2.journalFile);

    const content = readFileSync(join(root, r1.journalFile), "utf8");
    expect(content).toContain(".gate/runs/r1");
    expect(content).toContain(".gate/runs/r2");
  });
});
