import { afterEach, describe, expect, it, vi } from "vitest";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "../src/cli/args.js";
import { loadConfig } from "../src/core/config.js";
import { makeRepo } from "./helpers.js";

/**
 * `bundledPlaybooksDir()` always resolves for real when tests run from within
 * this repo's own checkout (dev-from-source layout), so the "unexpected,
 * non-ENOENT error" branch in `cmdInit`'s bundled-playbooks fallback can't be
 * reached with a real filesystem fixture - it's mocked here instead.
 */
const bundledPlaybooksDirMock = vi.fn<() => string>();
vi.mock("../src/core/playbooks.js", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/core/playbooks.js")>();
  return { ...actual, bundledPlaybooksDir: () => bundledPlaybooksDirMock() };
});

const { cmdInit } = await import("../src/commands/init.js");

describe("gate init bundled-playbooks fallback", () => {
  const originalCwd = process.cwd();

  afterEach(() => {
    process.chdir(originalCwd);
    vi.clearAllMocks();
  });

  it("falls back to embedded playbooks silently on ENOENT (expected: single-file binary, no sibling playbooks/ dir)", () => {
    const root = makeRepo();
    process.chdir(root);
    bundledPlaybooksDirMock.mockImplementation(() => {
      throw new Error("bundled playbooks directory not found");
    });
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);

    cmdInit(parseArgs(["init"]));

    expect(stderr).not.toHaveBeenCalled();
    expect(existsSync(join(root, ".gate", "playbooks", "plan.md"))).toBe(true);
  });

  it("warns on stderr but still completes init on an unexpected (non-ENOENT-equivalent) error", () => {
    const root = makeRepo();
    process.chdir(root);
    bundledPlaybooksDirMock.mockImplementation(() => {
      throw new Error("EACCES: permission denied, scandir '/some/playbooks'");
    });
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);

    cmdInit(parseArgs(["init"]));

    expect(stderr).toHaveBeenCalled();
    const warned = stderr.mock.calls.map((c) => String(c[0])).join("");
    expect(warned).toContain("EACCES");
    // Still completes: playbooks land via the embedded fallback, not silence-and-abort.
    expect(existsSync(join(root, ".gate", "playbooks", "plan.md"))).toBe(true);
  });
});

describe("gate init seeds scope_ignore per detected stack", () => {
  const originalCwd = process.cwd();
  afterEach(() => process.chdir(originalCwd));

  it("seeds node cache globs for a package.json repo", () => {
    const root = makeRepo({ "package.json": JSON.stringify({ name: "fx" }) });
    process.chdir(root);
    cmdInit(parseArgs(["init"]));
    const scopeIgnore = loadConfig(root).scope_ignore;
    expect(scopeIgnore).toContain("node_modules/**");
    expect(scopeIgnore).toContain("coverage/**");
  });

  it("seeds go build dirs for a go.mod repo", () => {
    const root = makeRepo({ "go.mod": "module fx\n" });
    process.chdir(root);
    cmdInit(parseArgs(["init"]));
    const scopeIgnore = loadConfig(root).scope_ignore;
    expect(scopeIgnore).toContain("vendor/**");
    expect(scopeIgnore).not.toContain("node_modules/**");
  });

  it("never rewrites an existing config.yml's scope_ignore on --refresh", () => {
    const root = makeRepo({ "package.json": JSON.stringify({ name: "fx" }) });
    process.chdir(root);
    cmdInit(parseArgs(["init"]));
    const configPath = join(root, ".gate", "config.yml");
    writeFileSync(configPath, "commands: {}\nscope_ignore:\n  - hand-edited/**\n");
    cmdInit(parseArgs(["init", "--refresh"]));
    expect(loadConfig(root).scope_ignore).toEqual(["hand-edited/**"]);
  });
});

describe("gate init writes .gate/ run-state gitignore entries", () => {
  const originalCwd = process.cwd();
  afterEach(() => process.chdir(originalCwd));

  const ENTRIES = [".gate/runs/", ".gate/current", ".gate/current.json", ".gate/archive/"];

  it("creates .gitignore with the run-state entries when none exists", () => {
    const root = makeRepo();
    process.chdir(root);
    cmdInit(parseArgs(["init"]));
    const gitignore = readFileSync(join(root, ".gitignore"), "utf8");
    for (const entry of ENTRIES) expect(gitignore).toContain(entry);
    // config.yml and trust.json are tracked, never gitignored.
    expect(gitignore).not.toContain("config.yml");
    expect(gitignore).not.toContain("trust.json");
  });

  it("appends missing entries to an existing .gitignore without touching the rest", () => {
    const root = makeRepo();
    writeFileSync(join(root, ".gitignore"), "node_modules/\ndist/\n");
    process.chdir(root);
    cmdInit(parseArgs(["init"]));
    const gitignore = readFileSync(join(root, ".gitignore"), "utf8");
    expect(gitignore).toContain("node_modules/");
    expect(gitignore).toContain("dist/");
    for (const entry of ENTRIES) expect(gitignore).toContain(entry);
  });

  it("is idempotent: a second init/--refresh doesn't duplicate entries", () => {
    const root = makeRepo();
    process.chdir(root);
    cmdInit(parseArgs(["init"]));
    const first = readFileSync(join(root, ".gitignore"), "utf8");
    cmdInit(parseArgs(["init", "--refresh"]));
    const second = readFileSync(join(root, ".gitignore"), "utf8");
    expect(second).toBe(first);
    for (const entry of ENTRIES) {
      expect(second.split("\n").filter((l) => l.trim() === entry)).toHaveLength(1);
    }
  });
});
