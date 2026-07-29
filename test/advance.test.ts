import { describe, expect, it } from "vitest";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { advance } from "../src/commands/advance.js";
import { runPaths } from "../src/core/paths.js";
import { newRun } from "../src/core/run.js";
import { makeRepo } from "./helpers.js";

describe("advance() scaffolds retro.md on entry into RETRO when missing", () => {
  it("writes retro.md for a run started before RETRO scaffolding existed", () => {
    const root = makeRepo();
    const run = newRun({ id: "r1", title: "pre-upgrade run", profile: "feature", baseRef: null, sessionId: null });
    run.phase = "REVIEW"; // about to advance into RETRO
    const retroPath = runPaths(root, "r1").retro;
    expect(existsSync(retroPath)).toBe(false); // simulates a run.json with no retro.md scaffold

    const { to } = advance(root, run);

    expect(to).toBe("RETRO");
    expect(existsSync(retroPath)).toBe(true);
    const content = readFileSync(retroPath, "utf8");
    expect(content).toContain("broke: []");
    expect(content).toContain("pre-upgrade run");
  });

  it("does not clobber an existing retro.md", () => {
    const root = makeRepo();
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    run.phase = "REVIEW";
    const retroPath = runPaths(root, "r1").retro;
    const custom = "---\nbroke:\n  - already filled in\navoid: []\nconventions: []\n---\n# Retro\n";
    mkdirSync(runPaths(root, "r1").dir, { recursive: true });
    writeFileSync(retroPath, custom);

    advance(root, run);

    expect(readFileSync(retroPath, "utf8")).toBe(custom);
  });

  it("does not scaffold retro.md when the transition isn't into RETRO", () => {
    const root = makeRepo();
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    run.phase = "PLAN"; // → IMPLEMENT, not RETRO
    advance(root, run);
    expect(existsSync(runPaths(root, "r1").retro)).toBe(false);
  });
});
