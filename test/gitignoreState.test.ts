import { describe, expect, it } from "vitest";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { isGateBookkeeping } from "../src/core/git.js";
import { isGitignoreUnchangedSinceInit, recordGitignoreState } from "../src/core/gitignoreState.js";
import { makeRepo } from "./helpers.js";

describe("gitignore bookkeeping exemption (review finding F1)", () => {
  it("is not exempt before gate init ever records a write", () => {
    const root = makeRepo();
    writeFileSync(join(root, ".gitignore"), "node_modules/\n");
    expect(isGitignoreUnchangedSinceInit(root)).toBe(false);
    expect(isGateBookkeeping(root, ".gitignore")).toBe(false);
  });

  it("is exempt only while the file matches exactly what was recorded", () => {
    const root = makeRepo();
    const content = "node_modules/\n# gate\n.gate/runs/\n";
    writeFileSync(join(root, ".gitignore"), content);
    recordGitignoreState(root, content);
    expect(isGitignoreUnchangedSinceInit(root)).toBe(true);
    expect(isGateBookkeeping(root, ".gitignore")).toBe(true);

    // Any edit at all - even appending something that looks harmless -
    // revokes the exemption; it does not try to distinguish "safe" edits.
    writeFileSync(join(root, ".gitignore"), content + "payload/\n");
    expect(isGitignoreUnchangedSinceInit(root)).toBe(false);
    expect(isGateBookkeeping(root, ".gitignore")).toBe(false);
  });

  it("is not exempt when the file was deleted after being recorded", () => {
    const root = makeRepo();
    const content = ".gate/runs/\n";
    writeFileSync(join(root, ".gitignore"), content);
    recordGitignoreState(root, content);
    expect(isGitignoreUnchangedSinceInit(root)).toBe(true);

    writeFileSync(join(root, ".gitignore"), "something else entirely\n");
    expect(isGitignoreUnchangedSinceInit(root)).toBe(false);
  });

  it("`.gate/` bookkeeping stays unconditionally exempt regardless of gitignore state", () => {
    const root = makeRepo();
    expect(isGateBookkeeping(root, ".gate/runs/x/run.json")).toBe(true);
    expect(isGateBookkeeping(root, ".gate/trust.json")).toBe(true);
  });
});
