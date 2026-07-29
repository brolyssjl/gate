import { describe, expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { rmSync } from "node:fs";
import { join } from "node:path";
import { changedLines, diffText, headSha as head, stagedFiles, treeFingerprint } from "../src/core/git.js";
import { makeRepo, writeFile } from "./helpers.js";

describe("diffText", () => {
  it("includes untracked files as new-file diffs (nothing requires committing)", () => {
    const root = makeRepo();
    writeFile(root, "brand-new.js", "console.log('new module')\n");
    const diff = diffText(root, head(root));
    expect(diff).toContain("brand-new.js");
    expect(diff).toContain("console.log('new module')");
  });

  it("excludes .gate/ bookkeeping, tracked or untracked", () => {
    const root = makeRepo();
    writeFile(root, ".gate/runs/r1/run.json", "{}\n");
    writeFile(root, "code.js", "x\n");
    const diff = diffText(root, head(root));
    expect(diff).toContain("code.js");
    expect(diff).not.toContain("run.json");
  });
});

describe("changedLines", () => {
  it("enumerates every line of an untracked file", () => {
    const root = makeRepo();
    writeFile(root, "new.js", "a\nb\nc\n");
    const lines = changedLines(root, head(root)).get("new.js");
    expect(lines).toEqual(new Set([1, 2, 3]));
  });

  it("keeps a deletion-only diff as an empty set (not whole-file)", () => {
    const root = makeRepo({ "old.js": "a\nb\nc\n" });
    writeFile(root, "old.js", "a\n"); // delete two lines, add none
    const lines = changedLines(root, head(root)).get("old.js");
    expect(lines).toEqual(new Set());
  });
});

describe("treeFingerprint", () => {
  it("is stable while the tree is unchanged", () => {
    const root = makeRepo();
    expect(treeFingerprint(root)).toBe(treeFingerprint(root));
  });

  it("changes when a tracked file is edited and when an untracked file appears", () => {
    const root = makeRepo({ "a.js": "one\n" });
    const clean = treeFingerprint(root);
    writeFile(root, "a.js", "two\n");
    const edited = treeFingerprint(root);
    expect(edited).not.toBe(clean);
    writeFile(root, "extra.js", "three\n");
    const withUntracked = treeFingerprint(root);
    expect(withUntracked).not.toBe(edited);
    rmSync(join(root, "extra.js"));
    expect(treeFingerprint(root)).toBe(edited);
  });

  it("ignores .gate/ bookkeeping (gate's own writes must not read as drift)", () => {
    const root = makeRepo();
    const before = treeFingerprint(root);
    writeFile(root, ".gate/runs/r1/review.md", "reviewer notes\n");
    expect(treeFingerprint(root)).toBe(before);
  });
});

describe("stagedFiles", () => {
  it("returns exact, unquoted filenames for a staged non-ASCII path (core.quotepath C-quotes them by default without -z)", () => {
    const root = makeRepo();
    const name = "café.ts";
    writeFile(root, name, "export const x = 1;\n");
    execFileSync("git", ["add", name], { cwd: root });
    expect(stagedFiles(root)).toEqual([name]);
  });

  it("excludes .gate/ bookkeeping even when staged", () => {
    const root = makeRepo();
    writeFile(root, ".gate/runs/r1/run.json", "{}\n");
    writeFile(root, "code.js", "x\n");
    execFileSync("git", ["add", "-A"], { cwd: root });
    const files = stagedFiles(root);
    expect(files).toContain("code.js");
    expect(files.some((f) => f.includes("run.json"))).toBe(false);
  });
});
