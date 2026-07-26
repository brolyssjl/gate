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

  it("migrates a schema-1 (Milestone 1) run: profile defaults, reviewer becomes requestedBy", () => {
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
    expect(run.schema).toBe(2);
    expect(run.profile).toBe("feature");
    expect(run.review).toEqual({
      requestedBy: "rev-1",
      requestedAt: "2026-01-02T00:00:00.000Z",
      treeHash: null,
    });
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
