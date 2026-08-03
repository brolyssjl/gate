import { describe, expect, it } from "vitest";
import { currentCommandsHash, hasNoCommands, isCommandsTrusted, writeTrust } from "../src/core/trust.js";
import { makeRepo, writeConfig, writeFile } from "./helpers.js";

describe("command trust (TOFU)", () => {
  it("treats an empty commands block as trusted (nothing runs)", () => {
    const root = makeRepo();
    writeConfig(root, { commands: {} }, false);
    expect(hasNoCommands(root)).toBe(true);
    expect(isCommandsTrusted(root)).toBe(true);
  });

  it("is untrusted before, trusted after `writeTrust`", () => {
    const root = makeRepo();
    writeConfig(root, { commands: { test: "echo hi" } }, false);
    expect(isCommandsTrusted(root)).toBe(false);
    writeTrust(root, "tester");
    expect(isCommandsTrusted(root)).toBe(true);
  });

  it("invalidates trust when the commands change", () => {
    const root = makeRepo();
    writeConfig(root, { commands: { test: "echo one" } }); // trusts
    expect(isCommandsTrusted(root)).toBe(true);
    writeConfig(root, { commands: { test: "echo two" } }, false); // change, no re-trust
    expect(isCommandsTrusted(root)).toBe(false);
  });

  it("produces a stable hash for the same commands block", () => {
    const root = makeRepo();
    writeConfig(root, { commands: { test: "echo hi" } }, false);
    expect(currentCommandsHash(root)).toBe(currentCommandsHash(root));
  });

  it("invalidates trust when scope_ignore changes, even with commands untouched (Milestone 5)", () => {
    const root = makeRepo();
    writeConfig(root, { commands: { test: "echo hi" }, scope_ignore: ["node_modules/**"] }); // trusts
    expect(isCommandsTrusted(root)).toBe(true);
    writeConfig(root, { commands: { test: "echo hi" }, scope_ignore: ["node_modules/**", "coverage/**"] }, false);
    expect(isCommandsTrusted(root)).toBe(false);
  });

  it("F4: an empty scope_ignore does not invalidate a trust hash computed before the key existed", () => {
    const root = makeRepo();
    // A 0.3.0-era config/trust: no scope_ignore key at all.
    writeFile(root, ".gate/config.yml", 'commands:\n  test: "echo hi"\n');
    writeTrust(root, null);
    expect(isCommandsTrusted(root)).toBe(true);
    const preUpgradeHash = currentCommandsHash(root);

    // "Upgrading" gate now understands scope_ignore, but none is configured
    // (or it's explicitly empty) - trust must survive untouched, not force
    // a surprise re-trust nobody asked for.
    writeFile(root, ".gate/config.yml", 'commands:\n  test: "echo hi"\nscope_ignore: []\n');
    expect(currentCommandsHash(root)).toBe(preUpgradeHash);
    expect(isCommandsTrusted(root)).toBe(true);
  });

  it("F4: a populated scope_ignore still forces a real re-trust, even with commands/targets both empty", () => {
    const root = makeRepo();
    writeConfig(root, { commands: {}, scope_ignore: [] }, false);
    expect(hasNoCommands(root)).toBe(true);
    expect(isCommandsTrusted(root)).toBe(true); // nothing to trust yet

    writeConfig(root, { commands: {}, scope_ignore: ["node_modules/**"] }, false);
    expect(hasNoCommands(root)).toBe(false);
    expect(isCommandsTrusted(root)).toBe(false); // scope_ignore alone is not trivially trusted
  });
});
