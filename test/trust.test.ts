import { describe, expect, it } from "vitest";
import { currentCommandsHash, hasNoCommands, isCommandsTrusted, writeTrust } from "../src/core/trust.js";
import { makeRepo, writeConfig } from "./helpers.js";

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
});
