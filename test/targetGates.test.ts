import { describe, expect, it } from "vitest";
import { join } from "node:path";
import { newRun, type Run } from "../src/core/run.js";
import { implementGate } from "../src/gates/implement.js";
import { testGate } from "../src/gates/test.js";
import { debugGate } from "../src/gates/debug.js";
import { headSha, makeRepo, writeConfig, writeFile } from "./helpers.js";

const PLAN_BOTH = `---
goal: touch both stacks
files:
  - apps/api/**
  - apps/web/**
criteria:
  - id: c1
    text: "does the thing"
    verify: "test: does the thing"
---
# Plan`;

function runOn(id: string, phase: Run["phase"], baseRef: string | null): Run {
  const run = newRun({ id, title: "t", profile: "feature", baseRef, sessionId: null });
  run.phase = phase;
  return run;
}

function names(checks: Array<{ name: string }>): string[] {
  return checks.map((c) => c.name).sort();
}

describe("IMPLEMENT gate with targets", () => {
  const TARGET_CONFIG = {
    targets: {
      api: { match: ["apps/api/**"], commands: { build: "node -e 0", lint: "node -e 0" } },
      web: { match: ["apps/web/**"], commands: { build: "node -e 0", lint: "node -e 0" } },
    },
  };

  it("emits bracketed checks per affected target, no bare implement.build/lint", () => {
    const root = makeRepo({ "apps/api/x.py": "", "apps/web/x.ts": "" });
    const base = headSha(root);
    writeFile(root, join(".gate", "runs", "r1", "plan.md"), PLAN_BOTH);
    const config = writeConfig(root, TARGET_CONFIG);
    writeFile(root, "apps/api/x.py", "changed\n");
    writeFile(root, "apps/web/x.ts", "changed\n");

    const res = implementGate({ root, run: runOn("r1", "IMPLEMENT", base), config });
    const checkNames = names(res.checks);
    expect(checkNames).toContain("implement.build[api]");
    expect(checkNames).toContain("implement.lint[api]");
    expect(checkNames).toContain("implement.build[web]");
    expect(checkNames).toContain("implement.lint[web]");
    expect(checkNames).not.toContain("implement.build");
    expect(checkNames).not.toContain("implement.lint");
  });

  it("brackets even a single affected target", () => {
    const root = makeRepo({ "apps/api/x.py": "" });
    const base = headSha(root);
    writeFile(root, join(".gate", "runs", "r1", "plan.md"), PLAN_BOTH.replace("- apps/web/**\n", ""));
    const config = writeConfig(root, TARGET_CONFIG);
    writeFile(root, "apps/api/x.py", "changed\n");

    const res = implementGate({ root, run: runOn("r1", "IMPLEMENT", base), config });
    const checkNames = names(res.checks);
    expect(checkNames).toContain("implement.build[api]");
    expect(checkNames).not.toContain("implement.build[web]");
    expect(checkNames).not.toContain("implement.build");
  });

  it("falls back to bare top-level checks when targets are configured but unaffected", () => {
    const root = makeRepo({ "README.md": "seed\n" });
    const base = headSha(root);
    writeFile(root, join(".gate", "runs", "r1", "plan.md"), PLAN_BOTH.replace(/apps\/(api|web)\/\*\*/g, "README.md"));
    const config = writeConfig(root, {
      commands: { build: "node -e 0", lint: "node -e 0" },
      ...TARGET_CONFIG,
    });
    writeFile(root, "README.md", "changed\n");

    const res = implementGate({ root, run: runOn("r1", "IMPLEMENT", base), config });
    const checkNames = names(res.checks);
    expect(checkNames).toContain("implement.build");
    expect(checkNames).toContain("implement.lint");
    expect(checkNames.some((n) => n.includes("["))).toBe(false);
  });
});

describe("TEST gate with targets", () => {
  const testCmd = (name: string) =>
    `node -e "console.log(JSON.stringify({tests:[{name:'${name}',status:'passed'}]}))"`;

  it("runs each affected target's test command and aggregates criteria/no-skips", () => {
    const root = makeRepo();
    writeFile(
      root,
      join(".gate", "runs", "r1", "plan.md"),
      `---\ngoal: g\nfiles:\n  - apps/api/**\n  - apps/web/**\ncriteria:\n  - id: c1\n    text: t\n    verify: "test: api works"\n  - id: c2\n    text: t2\n    verify: "test: web works"\n---\n# Plan`,
    );
    const config = writeConfig(root, {
      targets: {
        api: { match: ["apps/api/**"], commands: { test: testCmd("api works") } },
        web: { match: ["apps/web/**"], commands: { test: testCmd("web works") } },
      },
    });
    writeFile(root, "apps/api/x.py", "x\n");
    writeFile(root, "apps/web/x.ts", "x\n");

    const res = testGate({ root, run: runOn("r1", "TEST", null), config });
    const checkNames = names(res.checks);
    expect(checkNames).toContain("test.command[api]");
    expect(checkNames).toContain("test.command[web]");
    expect(checkNames).toContain("test.criteria");
    expect(checkNames).toContain("test.no-skips");
    expect(res.checks.find((c) => c.name === "test.criteria")?.ok).toBe(true);
    expect(res.ok).toBe(true);
  });

  it("fails only the affected target whose suite is red", () => {
    const root = makeRepo();
    writeFile(root, join(".gate", "runs", "r1", "plan.md"), `---\ngoal: g\nfiles:\n  - apps/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan`);
    const config = writeConfig(root, {
      targets: {
        api: { match: ["apps/api/**"], commands: { test: `node -e "process.exit(1)"` } },
        web: { match: ["apps/web/**"], commands: { test: `node -e "process.exit(0)"` } },
      },
    });
    writeFile(root, "apps/api/x.py", "x\n");
    writeFile(root, "apps/web/x.ts", "x\n");

    const res = testGate({ root, run: runOn("r1", "TEST", null), config });
    expect(res.checks.find((c) => c.name === "test.command[api]")?.ok).toBe(false);
    expect(res.checks.find((c) => c.name === "test.command[web]")?.ok).toBe(true);
    expect(res.ok).toBe(false);
  });
});

describe("DEBUG gate with targets", () => {
  const GOOD_DEBUG = `---
triggering_test: "greets by name"
reproduced: true
cycles:
  - hypothesis: "x"
    prediction: "y"
    experiment: "z"
    observation: "w"
    conclusion: "confirmed"
    status: complete
---
# Debug log`;

  it("brackets debug.test-green per affected target", () => {
    const root = makeRepo();
    writeFile(root, join(".gate", "runs", "r1", "plan.md"), `---\ngoal: g\nfiles:\n  - apps/api/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan`);
    writeFile(root, join(".gate", "runs", "r1", "debug-log.md"), GOOD_DEBUG);
    const config = writeConfig(root, {
      targets: {
        api: {
          match: ["apps/api/**"],
          commands: { test: `node -e "console.log(JSON.stringify({tests:[{name:'greets by name',status:'passed'}]}))"` },
        },
      },
    });
    writeFile(root, "apps/api/x.py", "x\n");

    const res = debugGate({ root, run: runOn("r1", "DEBUG", null), config });
    const checkNames = names(res.checks);
    expect(checkNames).toContain("debug.test-green[api]");
    expect(checkNames).not.toContain("debug.test-green");
    expect(res.checks.find((c) => c.name === "debug.test-green[api]")?.ok).toBe(true);
  });
});
