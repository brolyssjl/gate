import { describe, expect, it } from "vitest";
import { loadConfig } from "../src/core/config.js";
import { GateError } from "../src/cli/output.js";
import { matchesAny } from "../src/core/glob.js";
import { makeRepo, writeFile } from "./helpers.js";

describe("loadConfig target validation", () => {
  it("loads a well-formed targets block unchanged", () => {
    const root = makeRepo();
    writeFile(
      root,
      ".gate/config.yml",
      "targets:\n  api:\n    match:\n      - apps/api/**\n    commands:\n      test: pytest\n",
    );
    const config = loadConfig(root);
    expect(config.targets.api?.match).toEqual(["apps/api/**"]);
  });

  it("throws a friendly GateError naming the target when `match` is missing", () => {
    const root = makeRepo();
    writeFile(root, ".gate/config.yml", "targets:\n  api:\n    commands:\n      test: pytest\n");
    expect(() => loadConfig(root)).toThrow(GateError);
    expect(() => loadConfig(root)).toThrow(/"api"/);
  });

  it("throws when `match` is present but empty", () => {
    const root = makeRepo();
    writeFile(root, ".gate/config.yml", "targets:\n  api:\n    match: []\n");
    expect(() => loadConfig(root)).toThrow(GateError);
  });

  it("throws when `match` contains a non-string entry", () => {
    const root = makeRepo();
    writeFile(root, ".gate/config.yml", "targets:\n  api:\n    match:\n      - 5\n");
    expect(() => loadConfig(root)).toThrow(GateError);
  });

  it("throws when a target's commands block is not a mapping", () => {
    const root = makeRepo();
    writeFile(root, ".gate/config.yml", "targets:\n  api:\n    match: [apps/api/**]\n    commands: nope\n");
    expect(() => loadConfig(root)).toThrow(GateError);
  });

  it("previously crashed gates deep inside glob matching - matchesAny is never reached with a missing `match`", () => {
    const root = makeRepo();
    writeFile(root, ".gate/config.yml", "targets:\n  api:\n    commands:\n      test: pytest\n");
    // Before validation, this reached matchesAny(file, undefined) → "globs is
    // not iterable". Now loadConfig itself refuses the config, so no caller
    // ever gets a TargetConfig with a missing `match`.
    expect(() => loadConfig(root)).toThrow(GateError);
    expect(() => matchesAny("x", undefined as unknown as string[])).toThrow(TypeError);
  });
});

describe("loadConfig scope_ignore", () => {
  it("defaults to an empty list", () => {
    const root = makeRepo();
    writeFile(root, ".gate/config.yml", "commands: {}\n");
    expect(loadConfig(root).scope_ignore).toEqual([]);
  });

  it("loads a well-formed glob list", () => {
    const root = makeRepo();
    writeFile(root, ".gate/config.yml", 'scope_ignore:\n  - "node_modules/**"\n  - "coverage/**"\n');
    expect(loadConfig(root).scope_ignore).toEqual(["node_modules/**", "coverage/**"]);
  });

  it("throws when scope_ignore is not a list of strings", () => {
    const root = makeRepo();
    writeFile(root, ".gate/config.yml", "scope_ignore: not-a-list\n");
    expect(() => loadConfig(root)).toThrow(GateError);
  });

  it("throws when scope_ignore contains a non-string entry", () => {
    const root = makeRepo();
    writeFile(root, ".gate/config.yml", "scope_ignore:\n  - 5\n");
    expect(() => loadConfig(root)).toThrow(GateError);
  });
});
