import { describe, expect, it } from "vitest";
import { join } from "node:path";
import { newRun, nowIso, type Run } from "../src/core/run.js";
import { treeFingerprint } from "../src/core/git.js";
import { implementGate } from "../src/gates/implement.js";
import { testGate } from "../src/gates/test.js";
import { debugGate } from "../src/gates/debug.js";
import { reviewGate } from "../src/gates/review.js";
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

  it("asserts the triggering test against the COMBINED report across all affected targets, not each target's own report", () => {
    // Two targets are affected; the triggering test's name only appears in
    // web's suite. Before the fix, api's own per-target check compared the
    // triggering test against api's report alone and could never find it
    // there, so a multi-target bugfix run could never pass even with both
    // suites green and the fix actually verified.
    const root = makeRepo();
    writeFile(
      root,
      join(".gate", "runs", "r1", "plan.md"),
      `---\ngoal: g\nfiles:\n  - apps/api/**\n  - apps/web/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan`,
    );
    writeFile(root, join(".gate", "runs", "r1", "debug-log.md"), GOOD_DEBUG);
    const config = writeConfig(root, {
      targets: {
        api: { match: ["apps/api/**"], commands: { test: `node -e "console.log(JSON.stringify({tests:[{name:'unrelated api test',status:'passed'}]}))"` } },
        web: { match: ["apps/web/**"], commands: { test: `node -e "console.log(JSON.stringify({tests:[{name:'greets by name',status:'passed'}]}))"` } },
      },
    });
    writeFile(root, "apps/api/x.py", "x\n");
    writeFile(root, "apps/web/x.ts", "x\n");

    const res = debugGate({ root, run: runOn("r1", "DEBUG", null), config });
    const checkNames = names(res.checks);
    expect(checkNames).toContain("debug.test-green[api]");
    expect(checkNames).toContain("debug.test-green[web]");
    expect(checkNames).toContain("debug.triggering-test");
    expect(res.checks.find((c) => c.name === "debug.test-green[api]")?.ok).toBe(true);
    expect(res.checks.find((c) => c.name === "debug.test-green[web]")?.ok).toBe(true);
    expect(res.checks.find((c) => c.name === "debug.triggering-test")?.ok).toBe(true);
    expect(res.ok).toBe(true);
  });

  it("still fails a target whose own suite regresses, even though the aggregate triggering-test check passes", () => {
    const root = makeRepo();
    writeFile(
      root,
      join(".gate", "runs", "r1", "plan.md"),
      `---\ngoal: g\nfiles:\n  - apps/api/**\n  - apps/web/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan`,
    );
    writeFile(root, join(".gate", "runs", "r1", "debug-log.md"), GOOD_DEBUG);
    const config = writeConfig(root, {
      targets: {
        api: { match: ["apps/api/**"], commands: { test: `node -e "process.exit(1)"` } },
        web: { match: ["apps/web/**"], commands: { test: `node -e "console.log(JSON.stringify({tests:[{name:'greets by name',status:'passed'}]}))"` } },
      },
    });
    writeFile(root, "apps/api/x.py", "x\n");
    writeFile(root, "apps/web/x.ts", "x\n");

    const res = debugGate({ root, run: runOn("r1", "DEBUG", null), config });
    expect(res.checks.find((c) => c.name === "debug.test-green[api]")?.ok).toBe(false);
    expect(res.checks.find((c) => c.name === "debug.triggering-test")?.ok).toBe(true);
    expect(res.ok).toBe(false); // api's own regression still blocks, despite the aggregate match
  });
});

describe("TEST gate residual coverage (files matching no target)", () => {
  it("still enforces diff coverage over changed files that match no target's glob", () => {
    const root = makeRepo();
    writeFile(root, join(".gate", "runs", "r1", "plan.md"), `---\ngoal: g\nfiles:\n  - apps/**\n  - shared/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan`);
    // A coverage command that (re)writes a fixed report: apps/api/x.py fully
    // covered, shared/util.ts's changed line NOT covered.
    const covCmd =
      `node -e "require('fs').mkdirSync('coverage',{recursive:true});` +
      `require('fs').writeFileSync('coverage/gate-coverage.json', JSON.stringify({files:[` +
      `{path:'apps/api/x.py',covered:[1],uncovered:[]},` +
      `{path:'shared/util.ts',covered:[],uncovered:[1]}` +
      `]}))"`;
    const config = writeConfig(root, {
      commands: { coverage: covCmd },
      thresholds: { diff_coverage: 80 },
      targets: {
        api: { match: ["apps/api/**"], commands: { test: `node -e "console.log(JSON.stringify({tests:[]}))"` } },
      },
    });
    writeFile(root, "apps/api/x.py", "x\n");
    writeFile(root, "shared/util.ts", "y\n"); // matches no target's glob

    const res = testGate({ root, run: runOn("r1", "TEST", null), config });
    const checkNames = names(res.checks);
    expect(checkNames).toContain("test.coverage[api]");
    expect(checkNames).toContain("test.coverage"); // residual check, unbracketed
    expect(res.checks.find((c) => c.name === "test.coverage[api]")?.ok).toBe(true);
    expect(res.checks.find((c) => c.name === "test.coverage")?.ok).toBe(false);
    expect(res.ok).toBe(false); // the whole diff must stay covered, not just targeted files
  });

  it("adds no residual check when every changed file matches an affected target", () => {
    const root = makeRepo();
    writeFile(root, join(".gate", "runs", "r1", "plan.md"), `---\ngoal: g\nfiles:\n  - apps/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan`);
    writeFile(root, "apps/api/x.py", "x\n");
    // Writing coverage/gate-coverage.json BEFORE the gate runs (rather than
    // via a live `commands.coverage` invoked during the run) would itself
    // become an untracked "changed" file matching no target - write it via a
    // live coverage command instead, exactly like a real coverage tool run
    // by Gate would, so `touched` (computed before any command runs) stays
    // limited to the actual source change.
    const covCmd =
      `node -e "require('fs').mkdirSync('coverage',{recursive:true});` +
      `require('fs').writeFileSync('coverage/gate-coverage.json', JSON.stringify({files:[` +
      `{path:'apps/api/x.py',covered:[1],uncovered:[]}` +
      `]}))"`;
    const config = writeConfig(root, {
      commands: { coverage: covCmd },
      thresholds: { diff_coverage: 80 },
      targets: {
        api: { match: ["apps/api/**"], commands: { test: `node -e "console.log(JSON.stringify({tests:[]}))"` } },
      },
    });

    const res = testGate({ root, run: runOn("r1", "TEST", null), config });
    expect(names(res.checks)).not.toContain("test.coverage");
  });
});

describe("TEST gate fails closed when an affected target's green suite yields no parseable report", () => {
  it("fails test.report[target] instead of silently dropping the target from criteria/no-skips", () => {
    const root = makeRepo();
    writeFile(
      root,
      join(".gate", "runs", "r1", "plan.md"),
      `---\ngoal: g\nfiles:\n  - apps/api/**\n  - apps/web/**\ncriteria:\n  - id: c1\n    text: t\n    verify: "test: web works"\n---\n# Plan`,
    );
    const config = writeConfig(root, {
      targets: {
        // Green exit, but no parseable report - the bug this guards against.
        api: { match: ["apps/api/**"], commands: { test: `node -e "process.exit(0)"` } },
        web: {
          match: ["apps/web/**"],
          commands: { test: `node -e "console.log(JSON.stringify({tests:[{name:'web works',status:'passed'}]}))"` },
        },
      },
    });
    writeFile(root, "apps/api/x.py", "x\n");
    writeFile(root, "apps/web/x.ts", "x\n");

    const res = testGate({ root, run: runOn("r1", "TEST", null), config });
    expect(res.checks.find((c) => c.name === "test.command[api]")?.ok).toBe(true); // exit 0
    expect(res.checks.find((c) => c.name === "test.report[api]")?.ok).toBe(false);
    expect(res.ok).toBe(false);
  });

  it("does not fail test.report when the target's suite is red (already caught by test.command)", () => {
    const root = makeRepo();
    writeFile(root, join(".gate", "runs", "r1", "plan.md"), `---\ngoal: g\nfiles:\n  - apps/api/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan`);
    const config = writeConfig(root, {
      targets: { api: { match: ["apps/api/**"], commands: { test: `node -e "process.exit(1)"` } } },
    });
    writeFile(root, "apps/api/x.py", "x\n");

    const res = testGate({ root, run: runOn("r1", "TEST", null), config });
    expect(names(res.checks)).not.toContain("test.report[api]");
  });
});

describe("REVIEW gate evidence re-verification with targets", () => {
  function reviewRunOn(root: string, sessionId: string | null): Run {
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId });
    run.phase = "REVIEW";
    run.review = { requestedBy: null, requestedAt: nowIso(), treeHash: treeFingerprint(root) };
    return run;
  }

  it("actually re-runs a per-target-only repo's commands instead of passing vacuously", () => {
    const root = makeRepo();
    // No top-level build/lint/test at all - only the api target has commands.
    const config = writeConfig(root, {
      targets: {
        api: { match: ["apps/api/**"], commands: { test: `node -e "process.exit(1)"` } },
      },
    });
    writeFile(root, "apps/api/x.py", "changed\n");
    const run = reviewRunOn(root, null);
    // Force a re-verify: the last passed treeHash differs from the current tree.
    run.history.push({ phase: "TEST", event: "passed", at: nowIso(), treeHash: "sha256:stale" });
    writeFile(root, join(".gate", "runs", "r1", "review.md"), `---\nreviewer: rev-1\nfindings: []\n---\n# Review`);
    writeFile(root, join(".gate", "runs", "r1", "review-packet.md"), "# packet");

    const res = reviewGate({ root, run, config });
    const checkNames = names(res.checks);
    expect(checkNames).toContain("review.evidence[api]");
    expect(checkNames).not.toContain("review.evidence");
    expect(res.checks.find((c) => c.name === "review.evidence[api]")?.ok).toBe(false);
    expect(res.ok).toBe(false);
  });
});
