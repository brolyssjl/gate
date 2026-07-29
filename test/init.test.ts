import { afterEach, describe, expect, it, vi } from "vitest";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "../src/cli/args.js";
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
