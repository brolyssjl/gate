import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { writeFileSync } from "node:fs";
import { beforeAll, describe, expect, it } from "vitest";
import { makeRepo } from "./helpers.js";

const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const CLI = join(pkgRoot, "dist", "cli.js");

interface Run {
  code: number;
  stdout: string;
  json: () => unknown;
}

function gate(cwd: string, args: string[]): Run {
  const res = spawnSync("node", [CLI, ...args], { cwd, encoding: "utf8" });
  return {
    code: res.status ?? 1,
    stdout: res.stdout,
    json: () => JSON.parse(res.stdout),
  };
}

const PLAN = `---
goal: Add greet
approved: true
files:
  - greet.js
  - test.js
criteria:
  - id: c1
    text: "greets by name"
    verify: "test: greets by name"
---
# Plan
`;

describe("gate CLI end-to-end", () => {
  beforeAll(() => {
    // Ensure the binary is built before spawning it.
    execFileSync("npm", ["run", "build"], { cwd: pkgRoot, stdio: "pipe" });
  }, 120_000);

  it("walks a run from PLAN to DONE, refusing every hollow gate", () => {
    const repo = makeRepo({
      "package.json": JSON.stringify({ name: "fx", scripts: { test: "node test.js" } }),
    });

    expect(gate(repo, ["init"]).code).toBe(0);
    expect(gate(repo, ["start", "add greet"]).code).toBe(0);

    // PLAN gate fails on the empty scaffold.
    expect(gate(repo, ["check"]).code).toBe(1);

    // Approve a real plan → PLAN passes, enter IMPLEMENT.
    // Run id is date-based; discover it via status --json instead of hardcoding.
    const runId = (gate(repo, ["status", "--json"]).json() as { id: string }).id;
    writeFileSync(join(repo, `.gate/runs/${runId}/plan.md`), PLAN);
    expect(gate(repo, ["next"]).code).toBe(0);
    expect((gate(repo, ["status", "--json"]).json() as { phase: string }).phase).toBe("IMPLEMENT");

    // IMPLEMENT: empty diff fails; in-scope code passes.
    expect(gate(repo, ["check"]).code).toBe(1);
    writeFileSync(join(repo, "greet.js"), "module.exports.greet = (n) => 'Hello, ' + n;\n");
    expect(gate(repo, ["next"]).code).toBe(0);

    // TEST: red suite fails.
    writeFileSync(
      join(repo, "test.js"),
      `const {greet}=require('./greet');const ok=greet('Sam')==='Bye';` +
        `console.log(JSON.stringify({tests:[{name:'greets by name',status:ok?'passed':'failed'}]}));` +
        `process.exit(ok?0:1)`,
    );
    expect(gate(repo, ["check"]).code).toBe(1);

    // Green suite → TEST passes and the run reaches DONE.
    writeFileSync(
      join(repo, "test.js"),
      `const {greet}=require('./greet');const ok=greet('Sam')==='Hello, Sam';` +
        `console.log(JSON.stringify({tests:[{name:'greets by name',status:ok?'passed':'failed'}]}));` +
        `process.exit(ok?0:1)`,
    );
    expect(gate(repo, ["next"]).code).toBe(0);

    const done = gate(repo, ["status", "--json"]).json() as { active: boolean };
    expect(done.active).toBe(false);
  }, 60_000);

  it("emits a stable --json schema for check", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    gate(repo, ["start", "schema snap"]);
    const res = gate(repo, ["check", "--json"]);
    const parsed = res.json() as Record<string, unknown>;
    expect(Object.keys(parsed).sort()).toEqual(["checks", "ok", "phase"]);
    expect(parsed.phase).toBe("PLAN");
    expect(parsed.ok).toBe(false);
    expect(Array.isArray(parsed.checks)).toBe(true);
    const check0 = (parsed.checks as Array<Record<string, unknown>>)[0]!;
    expect(Object.keys(check0).sort()).toEqual(["detail", "name", "ok"]);
  });
});
