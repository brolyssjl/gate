import { describe, expect, it } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { newRun, readRun, writeRun, type Run } from "../src/core/run.js";
import { phaseReports } from "../src/commands/report.js";
import { makeRepo, writeFile } from "./helpers.js";

describe("run.json persistence", () => {
  it("writes atomically: valid JSON on disk, no temp file left behind", () => {
    const root = makeRepo();
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    writeRun(root, run);
    const path = join(root, ".gate", "runs", "r1", "run.json");
    expect(existsSync(path + ".tmp")).toBe(false);
    expect((JSON.parse(readFileSync(path, "utf8")) as Run).id).toBe("r1");
  });

  it("migrates a schema-1 (Milestone 1) run all the way to schema 4: profile defaults, reviewer becomes requestedBy, branch is null", () => {
    const root = makeRepo();
    const legacy = {
      schema: 1,
      id: "legacy",
      title: "old run",
      phase: "TEST",
      status: "active",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
      baseRef: null,
      sessionId: null,
      history: [{ phase: "PLAN", event: "entered", at: "2026-01-01T00:00:00.000Z" }],
      overrides: [],
      artifacts: {},
      review: { reviewer: "rev-1", requestedAt: "2026-01-02T00:00:00.000Z" },
    };
    writeFile(root, join(".gate", "runs", "legacy", "run.json"), JSON.stringify(legacy));
    const run = readRun(root, "legacy");
    expect(run.schema).toBe(4);
    expect(run.profile).toBe("feature");
    expect(run.branch).toBeNull();
    expect(run.review).toEqual({
      requestedBy: "rev-1",
      requestedAt: "2026-01-02T00:00:00.000Z",
      treeHash: null,
    });
  });

  it("migrates a schema-2 (Milestone 1-3) run to schema 4: gains branch: null, everything else untouched", () => {
    const root = makeRepo();
    const legacy = {
      schema: 2,
      id: "pre-m4",
      title: "concurrency-less run",
      profile: "bugfix",
      phase: "REVIEW",
      status: "active",
      createdAt: "2026-02-01T00:00:00.000Z",
      updatedAt: "2026-02-01T00:00:00.000Z",
      baseRef: "abc123",
      sessionId: "sess-1",
      history: [{ phase: "PLAN", event: "entered", at: "2026-02-01T00:00:00.000Z" }],
      overrides: [],
      artifacts: {},
    };
    writeFile(root, join(".gate", "runs", "pre-m4", "run.json"), JSON.stringify(legacy));
    const run = readRun(root, "pre-m4");
    expect(run.schema).toBe(4);
    expect(run.branch).toBeNull();
    expect(run.profile).toBe("bugfix");
    expect(run.baseRef).toBe("abc123");
    expect(run.sessionId).toBe("sess-1");
  });

  it("migrates a schema-3 (Milestone 1-4) run to schema 4: no amendment, everything else untouched", () => {
    const root = makeRepo();
    const legacy = {
      schema: 3,
      id: "pre-m5",
      title: "drift-less run",
      profile: "feature",
      phase: "IMPLEMENT",
      status: "active",
      createdAt: "2026-07-01T00:00:00.000Z",
      updatedAt: "2026-07-01T00:00:00.000Z",
      branch: "main",
      baseRef: "def456",
      sessionId: "sess-2",
      history: [{ phase: "PLAN", event: "entered", at: "2026-07-01T00:00:00.000Z" }],
      overrides: [],
      artifacts: {},
      approval: { by: "human", at: "2026-07-01T00:00:00.000Z", reason: null, planHash: "sha256:abc" },
    };
    writeFile(root, join(".gate", "runs", "pre-m5", "run.json"), JSON.stringify(legacy));
    const run = readRun(root, "pre-m5");
    expect(run.schema).toBe(4);
    expect(run.amendment).toBeUndefined();
    expect(run.branch).toBe("main");
    expect(run.approval?.planHash).toBe("sha256:abc");
  });

  it("rejects unknown future schemas", () => {
    const root = makeRepo();
    writeFile(root, join(".gate", "runs", "future", "run.json"), JSON.stringify({ schema: 99, id: "future" }));
    expect(() => readRun(root, "future")).toThrow(/schema 99/);
  });
});

describe("report phase durations", () => {
  it("spans a phase from entry to the next phase entry, including failed attempts", () => {
    const t = (s: number) => new Date(s * 1000).toISOString();
    const phases = phaseReports([
      { phase: "PLAN", event: "entered", at: t(0) },
      { phase: "PLAN", event: "failed", at: t(10) },
      { phase: "PLAN", event: "passed", at: t(25) },
      { phase: "IMPLEMENT", event: "entered", at: t(25) },
      { phase: "IMPLEMENT", event: "passed", at: t(40) },
    ]);
    const plan = phases.find((p) => p.phase === "PLAN")!;
    expect(plan.seconds).toBe(25); // not truncated at the failed attempt
    expect(plan.gateFailures).toBe(1);
    const impl = phases.find((p) => p.phase === "IMPLEMENT")!;
    expect(impl.seconds).toBe(15); // still open: up to the last recorded event
  });
});
