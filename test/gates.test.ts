import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { loadConfig, type GateConfig } from "../src/core/config.js";
import { newRun, nowIso, type Run } from "../src/core/run.js";
import { treeFingerprint } from "../src/core/git.js";
import { hashPlanFile } from "../src/artifacts/plan.js";
import { runPaths } from "../src/core/paths.js";
import { planGate } from "../src/gates/plan.js";
import { implementGate } from "../src/gates/implement.js";
import { testGate } from "../src/gates/test.js";
import { debugGate } from "../src/gates/debug.js";
import { reviewGate } from "../src/gates/review.js";
import { retroGate } from "../src/gates/retro.js";
import type { GateResult } from "../src/gates/types.js";
import { headSha, makeRepo, writeConfig, writeFile } from "./helpers.js";

const EMPTY_CONFIG: GateConfig = {
  commands: {},
  thresholds: {},
  targets: {},
  phases: {},
  integrations: {},
  coverage_format: "auto",
  retention: {},
  scope_ignore: [],
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

  it("does not add plan.spec when no SDD directory is detected", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const run = runOn("PLAN", null);
    approve(root, run);
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(res.checks.some((c) => c.name === "plan.spec")).toBe(false);
  });

  it("requires a spec citation once an SDD directory is detected", () => {
    const root = makeRepo();
    writeFile(root, "openspec/changes/x/spec.md", "# spec\n");
    writePlan(root, GOOD_PLAN);
    const run = runOn("PLAN", null);
    approve(root, run);
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "plan.spec")).toBe(false);
  });

  it("passes plan.spec when the cited path exists under the detected SDD dir", () => {
    const root = makeRepo();
    writeFile(root, "openspec/changes/x/spec.md", "# spec\n");
    writePlan(root, GOOD_PLAN.replace("goal: Add greet", "goal: Add greet\nspec: openspec/changes/x/spec.md"));
    const run = runOn("PLAN", null);
    approve(root, run);
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "plan.spec")).toBe(true);
  });

  it("fails plan.spec when the cited path is outside the SDD dir", () => {
    const root = makeRepo();
    writeFile(root, "openspec/changes/x/spec.md", "# spec\n");
    writeFile(root, "elsewhere.md", "not a spec\n");
    writePlan(root, GOOD_PLAN.replace("goal: Add greet", "goal: Add greet\nspec: elsewhere.md"));
    const run = runOn("PLAN", null);
    approve(root, run);
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "plan.spec")).toBe(false);
  });

  it("fails plan.spec when '..' would escape the SDD dir after normalization", () => {
    const root = makeRepo();
    writeFile(root, "openspec/changes/x/spec.md", "# spec\n");
    writeFile(root, "README.md", "not a spec\n");
    // "openspec/../README.md" string-starts-with "openspec/" but normalizes
    // to "README.md" - outside the detected SDD dir entirely.
    writePlan(root, GOOD_PLAN.replace("goal: Add greet", "goal: Add greet\nspec: openspec/../README.md"));
    const run = runOn("PLAN", null);
    approve(root, run);
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "plan.spec")).toBe(false);
  });

  it("skips plan.spec entirely when integrations.sdd is off", () => {
    const root = makeRepo();
    writeFile(root, "openspec/changes/x/spec.md", "# spec\n");
    writePlan(root, GOOD_PLAN);
    const run = runOn("PLAN", null);
    approve(root, run);
    const config = writeConfig(root, { integrations: { sdd: "off" } });
    const res = planGate({ root, run, config });
    expect(res.checks.some((c) => c.name === "plan.spec")).toBe(false);
  });

  it("does not add plan.advise without an .agnosgram store", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const run = runOn("PLAN", null);
    approve(root, run);
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(res.checks.some((c) => c.name === "plan.advise")).toBe(false);
  });

  it("passes plan.advise with a note when a store exists but no report was generated", () => {
    const root = makeRepo();
    writeFile(root, ".agnosgram/config.yml", "version: 1\n");
    writePlan(root, GOOD_PLAN);
    const run = runOn("PLAN", null);
    approve(root, run);
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "plan.advise")).toBe(true);
  });

  it("fails plan.advise on an unacknowledged contradiction", () => {
    const root = makeRepo();
    writeFile(root, ".agnosgram/config.yml", "version: 1\n");
    writePlan(root, GOOD_PLAN);
    writeFile(
      root,
      join(".gate", "runs", "r1", "plan.md.advise.json"),
      JSON.stringify({
        agnosgram_advise: 1,
        plan: ".gate/runs/r1/plan.md",
        generated: "2026-07-27",
        checked_ids: ["LES-002"],
        contradictions: [{ record_id: "LES-002", severity: "blocker", kind: "empirical" }],
        clear: false,
      }),
    );
    const run = runOn("PLAN", null);
    approve(root, run);
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "plan.advise")).toBe(false);
  });

  it("passes plan.advise once every contradiction is acknowledged in plan.md", () => {
    const root = makeRepo();
    writeFile(root, ".agnosgram/config.yml", "version: 1\n");
    writePlan(root, GOOD_PLAN.replace("goal: Add greet", "goal: Add greet\nacknowledgments: [LES-002]"));
    writeFile(
      root,
      join(".gate", "runs", "r1", "plan.md.advise.json"),
      JSON.stringify({
        agnosgram_advise: 1,
        plan: ".gate/runs/r1/plan.md",
        generated: "2026-07-27",
        checked_ids: ["LES-002"],
        contradictions: [{ record_id: "LES-002", severity: "blocker", kind: "empirical" }],
        clear: false,
      }),
    );
    const run = runOn("PLAN", null);
    approve(root, run);
    const res = planGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "plan.advise")).toBe(true);
  });

  it("skips plan.advise entirely when integrations.agnosgram is off", () => {
    const root = makeRepo();
    writeFile(root, ".agnosgram/config.yml", "version: 1\n");
    writePlan(root, GOOD_PLAN);
    const run = runOn("PLAN", null);
    approve(root, run);
    const config = writeConfig(root, { integrations: { agnosgram: "off" } });
    const res = planGate({ root, run, config });
    expect(res.checks.some((c) => c.name === "plan.advise")).toBe(false);
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

describe("scope_ignore", () => {
  function setup() {
    const root = makeRepo({ "greet.js": "" });
    const base = headSha(root);
    writePlan(root, GOOD_PLAN); // declares greet.js, test.js only
    return { root, base };
  }

  it("false-noise scenario: a scope_ignore match goes green instead of hard-blocking the gate", () => {
    const { root, base } = setup();
    writeFile(root, "greet.js", "x\n"); // in-plan
    writeFile(root, ".cache/build-info.json", "{}\n"); // noise, matches scope_ignore
    const config = writeConfig(root, { scope_ignore: [".cache/**"] });
    const res = implementGate({ root, run: runOn("IMPLEMENT", base), config });
    expect(res.ok).toBe(true);
    expect(check(res, "implement.scope")).toBe(true);
    expect(check(res, "implement.scope.ignored")).toBe(true);
    const ignoredCheck = res.checks.find((c) => c.name === "implement.scope.ignored");
    expect(ignoredCheck?.detail).toContain(".cache/build-info.json");
    // F5: the passing implement.scope detail must be truthful about *why* -
    // it must not claim "all ... declared in plan.md" when a file only
    // passed because it matched scope_ignore, not because it was declared.
    const scopeCheckDetail = res.checks.find((c) => c.name === "implement.scope")?.detail;
    expect(scopeCheckDetail).toContain("scope_ignore");
  });

  it("a real undeclared source file still fails even with scope_ignore configured", () => {
    const { root, base } = setup();
    writeFile(root, "greet.js", "x\n");
    writeFile(root, "secret.js", "y\n"); // real undeclared source file, not scope_ignore
    const config = writeConfig(root, { scope_ignore: [".cache/**"] });
    const res = implementGate({ root, run: runOn("IMPLEMENT", base), config });
    expect(check(res, "implement.scope")).toBe(false);
    const failing = res.checks.find((c) => c.name === "implement.scope");
    expect(failing?.detail).toContain("secret.js");
    expect(res.checks.some((c) => c.name === "implement.scope.ignored")).toBe(false);
  });

  it("an untrusted scope_ignore edit refuses to apply", () => {
    const { root, base } = setup();
    writeFile(root, "greet.js", "x\n");
    writeFile(root, ".cache/build-info.json", "{}\n");
    writeConfig(root, { scope_ignore: [] }); // trusted, without the ignore glob
    // Edit the ignore glob in directly, without re-running `gate trust`.
    writeFile(root, ".gate/config.yml", 'commands: {}\nscope_ignore:\n  - ".cache/**"\n');
    const config = loadConfig(root);
    const res = implementGate({ root, run: runOn("IMPLEMENT", base), config });
    expect(check(res, "implement.scope")).toBe(false);
    const failing = res.checks.find((c) => c.name === "implement.scope");
    expect(failing?.detail).toContain(".cache/build-info.json");
    expect(failing?.detail).toContain("untrusted");
    expect(res.checks.some((c) => c.name === "implement.scope.ignored")).toBe(false);
  });
});

describe("plan drift (Milestone 5)", () => {
  it("is silent when the plan was never approved, or is unchanged since approval", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const unapproved = runOn("IMPLEMENT", null);
    expect(
      implementGate({ root, run: unapproved, config: EMPTY_CONFIG }).checks.some((c) => c.name === "implement.plan-drift"),
    ).toBe(false);

    const approved = runOn("IMPLEMENT", null);
    approve(root, approved);
    expect(
      implementGate({ root, run: approved, config: EMPTY_CONFIG }).checks.some((c) => c.name === "implement.plan-drift"),
    ).toBe(false);
  });

  it("fails every later gate once plan.md drifts from its approved hash (restores void-on-edit for the whole run)", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const run = runOn("IMPLEMENT", null);
    approve(root, run);
    writePlan(root, GOOD_PLAN + "\nscope widened after approval\n");

    const implementRes = implementGate({ root, run: { ...run, phase: "IMPLEMENT" }, config: EMPTY_CONFIG });
    expect(implementRes.checks.find((c) => c.name === "implement.plan-drift")?.ok).toBe(false);

    const debugRes = debugGate({ root, run: { ...run, phase: "DEBUG" }, config: EMPTY_CONFIG });
    expect(debugRes.checks.find((c) => c.name === "debug.plan-drift")?.ok).toBe(false);

    const testRes = testGate({ root, run: { ...run, phase: "TEST" }, config: EMPTY_CONFIG });
    expect(testRes.checks.find((c) => c.name === "test.plan-drift")?.ok).toBe(false);

    const reviewRes = reviewGate({ root, run: { ...run, phase: "REVIEW" }, config: EMPTY_CONFIG });
    expect(reviewRes.checks.find((c) => c.name === "review.plan-drift")?.ok).toBe(false);

    const retroRes = retroGate({ root, run: { ...run, phase: "RETRO" }, config: EMPTY_CONFIG });
    expect(retroRes.checks.find((c) => c.name === "retro.plan-drift")?.ok).toBe(false);
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

  it("runs build and lint too (the bugfix profile has no IMPLEMENT phase)", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const config = writeConfig(root, {
      commands: { test: `node -e "console.log(JSON.stringify({tests:[{name:'greets by name',status:'passed'}]}))"`, build: "exit 3" },
    });
    const res = testGate({ root, run: runOn("TEST", null), config });
    expect(check(res, "test.build")).toBe(false);
    expect(res.ok).toBe(false);
  });

  it("ignores a test report staged before the run (evidence must come from the run itself)", () => {
    // The suite is green but prints no report; a pre-staged file claims the
    // criterion's test passed. The gate must not trust it.
    const { root, config } = setup(`node -e "process.exit(0)"`);
    writeFile(root, join(".gate", "runs", "r1", "test-report.json"), REPORT_PASS);
    const res = testGate({ root, run: runOn("TEST", null), config });
    expect(check(res, "test.command")).toBe(true);
    expect(check(res, "test.criteria")).toBe(false);
  });

  it("accepts a report the test command itself writes to $GATE_TEST_REPORT", () => {
    const { root, config } = setup(
      `node -e "require('fs').writeFileSync(process.env.GATE_TEST_REPORT, process.env.R)"`,
    );
    process.env.R = REPORT_PASS;
    const res = testGate({ root, run: runOn("TEST", null), config });
    delete process.env.R;
    expect(check(res, "test.criteria")).toBe(true);
  });

  it("persists the normalized report it parsed from stdout (Gate-owned evidence)", () => {
    const { root, config } = setup(`node -e "console.log(process.env.R)"`);
    process.env.R = REPORT_PASS;
    testGate({ root, run: runOn("TEST", null), config });
    delete process.env.R;
    const written = JSON.parse(
      readFileSync(join(root, ".gate", "runs", "r1", "test-report.json"), "utf8"),
    ) as { source: string; tests: unknown[] };
    expect(written.source).toContain("gate");
    expect(written.tests).toHaveLength(1);
  });
});

const GREEN_SUITE = `node -e "console.log(JSON.stringify({tests:[{name:'greets by name',status:'passed'}]}))"`;

function writeDebugLog(root: string, content: string): void {
  writeFile(root, join(".gate", "runs", "r1", "debug-log.md"), content);
}

const GOOD_DEBUG = `---
triggering_test: "greets by name"
reproduced: true
cycles:
  - hypothesis: "units mismatch"
    prediction: "logs show a 1000x gap"
    experiment: "logged both sides"
    observation: "off by 1000x"
    conclusion: "confirmed units bug"
    status: complete
---
# Debug log`;

describe("DEBUG gate", () => {
  function setup(debug: string, testCmd = GREEN_SUITE, trust = true) {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    writeDebugLog(root, debug);
    const config = writeConfig(root, { commands: { test: testCmd } }, trust);
    return { root, config };
  }

  it("passes a reproduced bug with a complete cycle and a green triggering test", () => {
    const { root, config } = setup(GOOD_DEBUG);
    const res = debugGate({ root, run: runOn("DEBUG", null), config });
    expect(res.ok).toBe(true);
    expect(check(res, "debug.reproduced")).toBe(true);
    expect(check(res, "debug.cycle")).toBe(true);
    expect(check(res, "debug.test-green")).toBe(true);
  });

  it("fails when the log is missing", () => {
    const root = makeRepo();
    writePlan(root, GOOD_PLAN);
    const config = writeConfig(root, { commands: { test: GREEN_SUITE } });
    const res = debugGate({ root, run: runOn("DEBUG", null), config });
    expect(check(res, "debug.log")).toBe(false);
  });

  it("fails when the bug was not reproduced", () => {
    const { root, config } = setup(GOOD_DEBUG.replace("reproduced: true", "reproduced: false"));
    const res = debugGate({ root, run: runOn("DEBUG", null), config });
    expect(check(res, "debug.reproduced")).toBe(false);
  });

  it("fails when no cycle is complete", () => {
    const { root, config } = setup(GOOD_DEBUG.replace("status: complete", "status: in-progress"));
    const res = debugGate({ root, run: runOn("DEBUG", null), config });
    expect(check(res, "debug.cycle")).toBe(false);
  });

  it("fails when the suite is still red (regression guard)", () => {
    const { root, config } = setup(GOOD_DEBUG, `node -e "process.exit(1)"`);
    const res = debugGate({ root, run: runOn("DEBUG", null), config });
    expect(check(res, "debug.test-green")).toBe(false);
  });

  it("does NOT run an untrusted test command", () => {
    const { root, config } = setup(GOOD_DEBUG, "touch ran.txt", false);
    const res = debugGate({ root, run: runOn("DEBUG", null), config });
    expect(check(res, "debug.test-green")).toBe(false);
    expect(existsSync(join(root, "ran.txt"))).toBe(false);
  });

  it("fails closed when the green suite emits no parseable report", () => {
    // Exit 0 alone cannot name the triggering test; without a report the
    // "triggering test passes" claim is unverifiable and must fail.
    const { root, config } = setup(GOOD_DEBUG, `node -e "process.exit(0)"`);
    const res = debugGate({ root, run: runOn("DEBUG", null), config });
    expect(check(res, "debug.test-green")).toBe(false);
  });
});

function writeReview(root: string, content: string): void {
  writeFile(root, join(".gate", "runs", "r1", "review.md"), content);
}
function writePacket(root: string): void {
  writeFile(root, join(".gate", "runs", "r1", "review-packet.md"), "# packet");
}

const SIGNED_EMPTY_REVIEW = `---\nreviewer: rev-1\nfindings: []\n---\n# Review`;

/**
 * A run in REVIEW with a packet already requested (as `gate review` would),
 * fingerprinted against the repo's current tree so the packet reads as fresh.
 */
function reviewRun(root: string, sessionId: string | null): Run {
  const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId });
  run.phase = "REVIEW";
  run.review = { requestedBy: null, requestedAt: nowIso(), treeHash: treeFingerprint(root) };
  return run;
}

describe("REVIEW gate", () => {
  it("passes with a fresh packet, a signed review, and no blocking findings", () => {
    const root = makeRepo();
    const run = reviewRun(root, null);
    writePacket(root);
    writeReview(root, SIGNED_EMPTY_REVIEW);
    const res = reviewGate({ root, run, config: EMPTY_CONFIG });
    expect(res.ok).toBe(true);
    expect(check(res, "review.packet")).toBe(true);
    expect(check(res, "review.findings")).toBe(true);
    expect(check(res, "review.reviewer")).toBe(true);
  });

  it("fails when no packet was emitted", () => {
    const root = makeRepo();
    writeReview(root, SIGNED_EMPTY_REVIEW);
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    run.phase = "REVIEW"; // no run.review set
    const res = reviewGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "review.packet")).toBe(false);
  });

  it("fails when the packet is stale (code changed after it was emitted)", () => {
    const root = makeRepo();
    const run = reviewRun(root, null); // fingerprints the tree as it stands
    writePacket(root);
    writeReview(root, SIGNED_EMPTY_REVIEW);
    writeFile(root, "sneaky.js", "changed after the reviewer looked\n");
    const res = reviewGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "review.packet")).toBe(false);
  });

  it("fails an unsigned review (the untouched scaffold must not pass)", () => {
    const root = makeRepo();
    const run = reviewRun(root, null);
    writePacket(root);
    writeReview(root, `---\nreviewer:\nfindings: []\n---\n# Review`);
    const res = reviewGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "review.reviewer")).toBe(false);
    expect(res.ok).toBe(false);
  });

  it("blocks on an open blocker finding", () => {
    const root = makeRepo();
    const run = reviewRun(root, null);
    writePacket(root);
    writeReview(
      root,
      `---\nreviewer: rev-1\nfindings:\n  - id: f1\n    severity: blocker\n    status: open\n    note: bad\n---\n`,
    );
    const res = reviewGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "review.findings")).toBe(false);
  });

  it("accepts a waived major finding with a rationale", () => {
    const root = makeRepo();
    const run = reviewRun(root, null);
    writePacket(root);
    writeReview(
      root,
      `---\nreviewer: rev-1\nfindings:\n  - id: f1\n    severity: major\n    status: waived\n    note: n\n    waiver: "accepted for now"\n---\n`,
    );
    const res = reviewGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "review.findings")).toBe(true);
  });

  it("rejects a waived major finding with no rationale", () => {
    const root = makeRepo();
    const run = reviewRun(root, null);
    writePacket(root);
    writeReview(
      root,
      `---\nreviewer: rev-1\nfindings:\n  - id: f1\n    severity: major\n    status: waived\n    note: n\n---\n`,
    );
    const res = reviewGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "review.findings")).toBe(false);
  });

  it("fails self-review when the signing reviewer equals the implementer", () => {
    const root = makeRepo();
    const run = reviewRun(root, "agent-1");
    writePacket(root);
    writeReview(root, `---\nreviewer: agent-1\nfindings: []\n---\n`);
    const res = reviewGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "review.reviewer")).toBe(false);
  });

  it("passes when the signing reviewer differs from the implementer", () => {
    const root = makeRepo();
    const run = reviewRun(root, "agent-1");
    writePacket(root);
    writeReview(root, `---\nreviewer: agent-2\nfindings: []\n---\n`);
    const res = reviewGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "review.reviewer")).toBe(true);
  });

  it("skips re-verification while the tree matches the last passed gate", () => {
    const root = makeRepo();
    const run = reviewRun(root, null);
    // TEST passed on exactly this tree; the sentinel command must not run.
    run.history.push({ phase: "TEST", event: "passed", at: nowIso(), treeHash: treeFingerprint(root)! });
    const config = writeConfig(root, { commands: { test: "touch ran.txt" } });
    writePacket(root);
    writeReview(root, SIGNED_EMPTY_REVIEW);
    const res = reviewGate({ root, run, config });
    expect(check(res, "review.evidence")).toBe(true);
    expect(existsSync(join(root, "ran.txt"))).toBe(false);
  });

  it("re-verifies and fails when a review fix left the suite red", () => {
    const root = makeRepo();
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    run.phase = "REVIEW";
    run.history.push({ phase: "TEST", event: "passed", at: nowIso(), treeHash: "sha256:before-the-fix" });
    const config = writeConfig(root, { commands: { test: `node -e "process.exit(1)"` } });
    writeFile(root, "fix.js", "the review fix\n");
    run.review = { requestedBy: null, requestedAt: nowIso(), treeHash: treeFingerprint(root) };
    writePacket(root);
    writeReview(root, SIGNED_EMPTY_REVIEW);
    const res = reviewGate({ root, run, config });
    expect(check(res, "review.evidence")).toBe(false);
    expect(res.ok).toBe(false);
  });

  it("re-verifies and passes when the review fix keeps everything green", () => {
    const root = makeRepo();
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    run.phase = "REVIEW";
    run.history.push({ phase: "TEST", event: "passed", at: nowIso(), treeHash: "sha256:before-the-fix" });
    const config = writeConfig(root, { commands: { test: `node -e "process.exit(0)"` } });
    writeFile(root, "fix.js", "the review fix\n");
    run.review = { requestedBy: null, requestedAt: nowIso(), treeHash: treeFingerprint(root) };
    writePacket(root);
    writeReview(root, SIGNED_EMPTY_REVIEW);
    const res = reviewGate({ root, run, config });
    expect(check(res, "review.evidence")).toBe(true);
    expect(res.ok).toBe(true);
  });

  it("does NOT re-verify with untrusted commands (fails closed)", () => {
    const root = makeRepo();
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    run.phase = "REVIEW";
    run.history.push({ phase: "TEST", event: "passed", at: nowIso(), treeHash: "sha256:before-the-fix" });
    const config = writeConfig(root, { commands: { test: "touch ran.txt" } }, false);
    run.review = { requestedBy: null, requestedAt: nowIso(), treeHash: treeFingerprint(root) };
    writePacket(root);
    writeReview(root, SIGNED_EMPTY_REVIEW);
    const res = reviewGate({ root, run, config });
    expect(check(res, "review.evidence")).toBe(false);
    expect(existsSync(join(root, "ran.txt"))).toBe(false);
  });
});

function writeRetro(root: string, content: string): void {
  writeFile(root, join(".gate", "runs", "r1", "retro.md"), content);
}

const SUBSTANTIVE_RETRO = `---\nbroke: []\navoid:\n  - "Do not skip reproduce"\nconventions: []\n---\n# Retro`;
const EMPTY_RETRO = `---\nbroke: []\navoid: []\nconventions: []\n---\n# Retro`;

describe("RETRO gate", () => {
  it("fails when retro.md is missing", () => {
    const root = makeRepo();
    const res = retroGate({ root, run: runOn("RETRO", null), config: EMPTY_CONFIG });
    expect(check(res, "retro.schema")).toBe(false);
  });

  it("fails retro.substance on an empty scaffold", () => {
    const root = makeRepo();
    writeRetro(root, EMPTY_RETRO);
    const res = retroGate({ root, run: runOn("RETRO", null), config: EMPTY_CONFIG });
    expect(check(res, "retro.substance")).toBe(false);
  });

  it("passes with no .agnosgram store (journal sync not required)", () => {
    const root = makeRepo();
    writeRetro(root, SUBSTANTIVE_RETRO);
    const res = retroGate({ root, run: runOn("RETRO", null), config: EMPTY_CONFIG });
    expect(res.ok).toBe(true);
    expect(check(res, "retro.journal")).toBe(true);
  });

  it("passes when the agnosgram integration is off, even with a store present", () => {
    const root = makeRepo();
    writeFile(root, ".agnosgram/journal/2026-01.md", "# Journal\n");
    writeRetro(root, SUBSTANTIVE_RETRO);
    const config = writeConfig(root, { integrations: { agnosgram: "off" } });
    const res = retroGate({ root, run: runOn("RETRO", null), config });
    expect(check(res, "retro.journal")).toBe(true);
  });

  it("fails retro.journal when a store is present but no sync was recorded", () => {
    const root = makeRepo();
    writeFile(root, ".agnosgram/journal/2026-01.md", "# Journal\n");
    writeRetro(root, SUBSTANTIVE_RETRO);
    const res = retroGate({ root, run: runOn("RETRO", null), config: EMPTY_CONFIG });
    expect(check(res, "retro.journal")).toBe(false);
  });

  it("passes retro.journal once the run.retro receipt points at a journal entry containing the run id", () => {
    const root = makeRepo();
    writeFile(root, ".agnosgram/journal/2026-01.md", "# Journal\n\n## entry\n- **Source:** .gate/runs/r1\n");
    writeRetro(root, SUBSTANTIVE_RETRO);
    const run = runOn("RETRO", null);
    run.retro = { journalFile: ".agnosgram/journal/2026-01.md", syncedAt: nowIso(), method: "fallback" };
    const res = retroGate({ root, run, config: EMPTY_CONFIG });
    expect(res.ok).toBe(true);
    expect(check(res, "retro.journal")).toBe(true);
  });

  it("fails retro.journal when the recorded journal file no longer contains the run id", () => {
    const root = makeRepo();
    writeFile(root, ".agnosgram/journal/2026-01.md", "# Journal\n\n## entry\n- **Did:** unrelated\n");
    writeRetro(root, SUBSTANTIVE_RETRO);
    const run = runOn("RETRO", null);
    run.retro = { journalFile: ".agnosgram/journal/2026-01.md", syncedAt: nowIso(), method: "fallback" };
    const res = retroGate({ root, run, config: EMPTY_CONFIG });
    expect(check(res, "retro.journal")).toBe(false);
  });
});
