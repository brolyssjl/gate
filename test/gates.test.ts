import { describe, expect, it } from "vitest";
import { existsSync } from "node:fs";
import { join } from "node:path";
import type { GateConfig } from "../src/core/config.js";
import { newRun, nowIso, type Run } from "../src/core/run.js";
import { hashPlanFile } from "../src/artifacts/plan.js";
import { runPaths } from "../src/core/paths.js";
import { planGate } from "../src/gates/plan.js";
import { implementGate } from "../src/gates/implement.js";
import { testGate } from "../src/gates/test.js";
import type { GateResult } from "../src/gates/types.js";
import { headSha, makeRepo, writeConfig, writeFile } from "./helpers.js";

const EMPTY_CONFIG: GateConfig = {
  commands: {},
  thresholds: {},
  targets: {},
  phases: {},
  integrations: {},
  coverage_format: "auto",
};

function runOn(phase: Run["phase"], baseRef: string | null): Run {
  const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef, sessionId: null });
  run.phase = phase;
  return run;
}

function writePlan(root: string, content: string): void {
  writeFile(root, join(".gate", "runs", "r1", "plan.md"), content);
}

/** Approve a run against the current on-disk plan (as `gate approve` would). */
function approve(root: string, run: Run): void {
  run.approval = { by: null, at: nowIso(), reason: null, planHash: hashPlanFile(runPaths(root, "r1").plan)! };
}

function check(res: GateResult, name: string): boolean {
  return res.checks.find((c) => c.name === name)?.ok ?? false;
}

const GOOD_PLAN = `---
goal: Add greet
files:
  - greet.js
  - test.js
criteria:
  - id: c1
    text: "greets by name"
    verify: "test: greets by name"
---
# Plan`;

describe("PLAN gate", () => {
  it("passes a valid, approved plan", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const run = runOn("PLAN", null);
    approve(root, run);
    expect(planGate({ root, run, config: EMPTY_CONFIG }).ok).toBe(true);
  });

  it("fails when the plan is missing", () => {
    const root = makeRepo();
    const res = planGate({ root, run: runOn("PLAN", null), config: EMPTY_CONFIG });
    expect(check(res, "plan.schema")).toBe(false);
  });

  it("fails when not approved", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const res = planGate({ root, run: runOn("PLAN", null), config: EMPTY_CONFIG });
    expect(check(res, "plan.approved")).toBe(false);
  });

  it("voids approval when the plan changes after approval", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const run = runOn("PLAN", null);
    approve(root, run);
    writePlan(root, GOOD_PLAN + "\nedited after approval\n"); // plan hash now differs
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "plan.approved")).toBe(false);
  });
});

describe("IMPLEMENT gate", () => {
  function setup() {
    const root = makeRepo({ "greet.js": "" });
    const base = headSha(root);
    writePlan(root, GOOD_PLAN);
    return { root, base };
  }

  it("fails with no diff", () => {
    const { root, base } = setup();
    const res = implementGate({ root, run: runOn("IMPLEMENT", base), config: EMPTY_CONFIG });
    expect(check(res, "implement.diff")).toBe(false);
  });

  it("passes with an in-scope change", () => {
    const { root, base } = setup();
    writeFile(root, "greet.js", "module.exports.greet = (n) => 'Hi ' + n;\n");
    const res = implementGate({ root, run: runOn("IMPLEMENT", base), config: EMPTY_CONFIG });
    expect(res.ok).toBe(true);
    expect(check(res, "implement.diff")).toBe(true);
    expect(check(res, "implement.scope")).toBe(true);
  });

  it("fails an out-of-scope change", () => {
    const { root, base } = setup();
    writeFile(root, "greet.js", "x\n");
    writeFile(root, "secret.js", "y\n");
    const res = implementGate({ root, run: runOn("IMPLEMENT", base), config: EMPTY_CONFIG });
    expect(check(res, "implement.scope")).toBe(false);
  });

  it("ignores .gate/ bookkeeping when measuring the diff", () => {
    const { root, base } = setup();
    writeFile(root, ".gate/runs/r1/worklog.md", "note\n");
    const res = implementGate({ root, run: runOn("IMPLEMENT", base), config: EMPTY_CONFIG });
    expect(check(res, "implement.diff")).toBe(false);
  });

  it("fails when a trusted build command fails", () => {
    const { root, base } = setup();
    writeFile(root, "greet.js", "x\n");
    const config = writeConfig(root, { commands: { build: "exit 3" } });
    const res = implementGate({ root, run: runOn("IMPLEMENT", base), config });
    expect(check(res, "implement.build")).toBe(false);
  });

  it("does NOT run an untrusted build command", () => {
    const { root, base } = setup();
    writeFile(root, "greet.js", "x\n");
    // Sentinel: the command would create ran.txt if it executed.
    const config = writeConfig(root, { commands: { build: "touch ran.txt" } }, false);
    const res = implementGate({ root, run: runOn("IMPLEMENT", base), config });
    expect(check(res, "implement.build")).toBe(false);
    expect(existsSync(join(root, "ran.txt"))).toBe(false);
  });
});

describe("TEST gate", () => {
  const REPORT_PASS = `{"tests":[{"name":"greets by name","status":"passed"}]}`;
  const REPORT_FAIL = `{"tests":[{"name":"greets by name","status":"failed"}]}`;

  function setup(testCmd: string, trust = true) {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const config = writeConfig(root, { commands: { test: testCmd } }, trust);
    return { root, config };
  }

  it("passes a green suite with mapped criteria", () => {
    const { root, config } = setup(`node -e "console.log(process.env.R)"`);
    process.env.R = REPORT_PASS;
    const res = testGate({ root, run: runOn("TEST", null), config });
    delete process.env.R;
    expect(res.ok).toBe(true);
    expect(check(res, "test.command")).toBe(true);
    expect(check(res, "test.criteria")).toBe(true);
  });

  it("refuses a red suite (non-zero exit)", () => {
    const { root, config } = setup(`node -e "console.log('${REPORT_FAIL}'); process.exit(1)"`);
    const res = testGate({ root, run: runOn("TEST", null), config });
    expect(check(res, "test.command")).toBe(false);
    expect(res.ok).toBe(false);
  });

  it("fails when a criterion maps to no passing test", () => {
    const { root, config } = setup(
      `node -e "console.log(JSON.stringify({tests:[{name:'unrelated',status:'passed'}]}))"`,
    );
    const res = testGate({ root, run: runOn("TEST", null), config });
    expect(check(res, "test.criteria")).toBe(false);
  });

  it("fails on skipped tests", () => {
    const { root, config } = setup(
      `node -e "console.log(JSON.stringify({tests:[{name:'greets by name',status:'passed'},{name:'later',status:'skipped'}]}))"`,
    );
    const res = testGate({ root, run: runOn("TEST", null), config });
    expect(check(res, "test.no-skips")).toBe(false);
  });

  it("fails when no test command is configured", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const res = testGate({ root, run: runOn("TEST", null), config: EMPTY_CONFIG });
    expect(check(res, "test.command")).toBe(false);
  });

  it("does NOT run an untrusted test command", () => {
    const { root, config } = setup("touch ran.txt", false);
    const res = testGate({ root, run: runOn("TEST", null), config });
    expect(check(res, "test.command")).toBe(false);
    expect(existsSync(join(root, "ran.txt"))).toBe(false);
  });
});
