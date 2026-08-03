import { afterEach, describe, expect, it } from "vitest";
import { chmodSync, existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { formatJournalEntry, journalFilePath, writeJournalEntry } from "../src/integrations/agnosgramWrite.js";
import { makeRepo } from "./helpers.js";

const WHEN = new Date(2026, 6, 27, 14, 5); // 2026-07-27 14:05, local

/**
 * A path that cannot possibly resolve to a real binary, for the fallback
 * (ENOENT) tests - hermetic regardless of whether an actual `agnosgram` is
 * installed on the machine running the suite (review finding F3).
 */
const NONEXISTENT_AGNOSGRAM_BIN = "/nonexistent/gate-agnosgram-test-stub";

/** Run `fn` with `GATE_AGNOSGRAM_BIN` set, restoring the prior value (or its absence) afterward. */
function withAgnosgramBin<T>(bin: string, fn: () => T): T {
  const prior = process.env.GATE_AGNOSGRAM_BIN;
  process.env.GATE_AGNOSGRAM_BIN = bin;
  try {
    return fn();
  } finally {
    if (prior === undefined) delete process.env.GATE_AGNOSGRAM_BIN;
    else process.env.GATE_AGNOSGRAM_BIN = prior;
  }
}

/**
 * A stub `agnosgram` binary that always succeeds, for exercising the
 * "agnosgram-cli" success method without depending on the real CLI being
 * installed. Verifies it was invoked as documented: `log --stdin --agent
 * gate`, with the entry piped on stdin.
 */
function makeAgnosgramStub(recordPath: string): string {
  const binDir = mkdtempSync(join(tmpdir(), "gate-agnosgram-stub-"));
  const stub = join(binDir, "agnosgram");
  writeFileSync(
    stub,
    `#!/bin/sh\nif [ "$1" != "log" ] || [ "$2" != "--stdin" ] || [ "$3" != "--agent" ] || [ "$4" != "gate" ]; then\n  echo "unexpected args: $@" >&2\n  exit 1\nfi\ncat > "${recordPath}"\nexit 0\n`,
  );
  chmodSync(stub, 0o755);
  return stub;
}

describe("formatJournalEntry (golden - frozen Agnosgram journal format v1)", () => {
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

describe("writeJournalEntry fallback (agnosgram binary genuinely absent - hermetic via GATE_AGNOSGRAM_BIN)", () => {
  // F3: pointed at a path that cannot resolve to a real binary, so this
  // suite exercises the ENOENT fallback regardless of whether an actual
  // `agnosgram` happens to be installed on PATH on the machine running it.
  it("creates the month's journal file and appends the entry", () => {
    const root = makeRepo();
    const entry = formatJournalEntry({
      run: { id: "r1", title: "demo", profile: "feature" },
      retro: { broke: ["x"], avoid: [], conventions: [], body: "" },
      branch: "main",
      when: WHEN,
    });

    const result = withAgnosgramBin(NONEXISTENT_AGNOSGRAM_BIN, () => writeJournalEntry(root, entry, WHEN));
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

    const [r1, r2] = withAgnosgramBin(NONEXISTENT_AGNOSGRAM_BIN, () => [
      writeJournalEntry(root, first, WHEN),
      writeJournalEntry(root, second, WHEN),
    ]);
    expect(r1.journalFile).toBe(r2.journalFile);

    const content = readFileSync(join(root, r1.journalFile), "utf8");
    expect(content).toContain(".gate/runs/r1");
    expect(content).toContain(".gate/runs/r2");
  });
});

describe("writeJournalEntry via a stub agnosgram CLI (GATE_AGNOSGRAM_BIN, review finding F3)", () => {
  it("spawns `agnosgram log --stdin --agent gate` with the entry on stdin, and reports method agnosgram-cli", () => {
    const root = makeRepo();
    const recordPath = join(mkdtempSync(join(tmpdir(), "gate-agnosgram-record-")), "received.md");
    const stub = makeAgnosgramStub(recordPath);
    const entry = formatJournalEntry({
      run: { id: "r1", title: "demo", profile: "feature" },
      retro: { broke: ["x"], avoid: [], conventions: [], body: "" },
      branch: "main",
      when: WHEN,
    });

    const result = withAgnosgramBin(stub, () => writeJournalEntry(root, entry, WHEN));
    expect(result.method).toBe("agnosgram-cli");
    expect(result.journalFile).toBe(".agnosgram/journal/2026-07.md");
    expect(readFileSync(recordPath, "utf8")).toBe(entry);
  });
});

describe("journalFilePath pins the month to UTC (matches the agnosgram CLI's toISOString().slice(0,7))", () => {
  const originalTZ = process.env.TZ;
  afterEach(() => {
    if (originalTZ === undefined) delete process.env.TZ;
    else process.env.TZ = originalTZ;
  });

  it("uses the UTC month even when local wall-clock time has already rolled into the next month", () => {
    // 2026-07-31 23:30 UTC. In a timezone well ahead of UTC, local wall-clock
    // time here is already 2026-08-01 - a local-time-based derivation would
    // (wrongly) file this under August, disagreeing with the agnosgram CLI's
    // own UTC-based month, which is exactly the "wrong receipt" bug.
    process.env.TZ = "Etc/GMT-14"; // POSIX TZ sign is inverted: GMT-14 means UTC+14
    const when = new Date(Date.UTC(2026, 6, 31, 23, 30));
    expect(journalFilePath(when)).toBe(".agnosgram/journal/2026-07.md");
  });

  it("uses the UTC month even when local wall-clock time is still in the previous month", () => {
    // 2026-08-01 00:30 UTC. In a timezone well behind UTC, local wall-clock
    // time here is still 2026-07-31 - same contract, opposite direction.
    process.env.TZ = "Etc/GMT+12"; // POSIX TZ sign is inverted: GMT+12 means UTC-12
    const when = new Date(Date.UTC(2026, 7, 1, 0, 30));
    expect(journalFilePath(when)).toBe(".agnosgram/journal/2026-08.md");
  });
});
