import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { GateConfig } from "../src/core/config.js";
import { newRun, nowIso, type Run } from "../src/core/run.js";
import { treeFingerprint } from "../src/core/git.js";
import { hashPlanFile } from "../src/artifacts/plan.js";
import { runPaths } from "../src/core/paths.js";
import { planGate } from "../src/gates/plan.js";
import { implementGate } from "../src/gates/implement.js";
import { testGate } from "../src/gates/test.js";
import { debugGate } from "../src/gates/debug.js";
import { reviewGate } from "../src/gates/review.js";
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
