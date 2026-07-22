import { describe, expect, it } from "vitest";
import { parseArgs } from "../src/cli/args.js";

describe("arg parser", () => {
  it("reads the command, positionals, and flags", () => {
    const a = parseArgs(["start", "add greet", "--profile", "bugfix", "--json"]);
    expect(a.command).toBe("start");
    expect(a.positionals).toEqual(["add greet"]);
    expect(a.flags).toEqual({ profile: "bugfix", json: true });
  });

  it("supports --flag=value", () => {
    expect(parseArgs(["check", "--format=toon"]).flags).toEqual({ format: "toon" });
  });

  it("treats a leading long flag as no command (gate --version)", () => {
    const a = parseArgs(["--version"]);
    expect(a.command).toBeNull();
    expect(a.flags.version).toBe(true);
  });

  it("treats a leading short flag as no command (gate -v / -h)", () => {
    expect(parseArgs(["-v"])).toMatchObject({ command: null, flags: { v: true } });
    expect(parseArgs(["-h"])).toMatchObject({ command: null, flags: { h: true } });
  });

  it("returns a null command for empty argv", () => {
    expect(parseArgs([]).command).toBeNull();
  });
});
