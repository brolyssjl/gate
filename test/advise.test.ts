import { describe, expect, it } from "vitest";
import { adviseReportPath, loadAdviseReport, parseAdviseReport } from "../src/integrations/advise.js";

const VALID = JSON.stringify({
  agnosgram_advise: 1,
  plan: ".gate/runs/r1/plan.md",
  generated: "2026-07-27",
  checked_ids: ["LES-001", "LES-002"],
  contradictions: [
    {
      record_id: "LES-002",
      kind: "empirical",
      severity: "blocker",
      plan_excerpt: "retry on 500",
      record_excerpt: "retries caused double-charges",
      confidence: "high",
      last_verified: "2026-07-20",
      explanation: "the plan reintroduces a fix that already failed",
    },
  ],
  clear: false,
});

describe("advise report parser (pinned agnosgram_advise schema)", () => {
  it("parses a well-formed report", () => {
    const report = parseAdviseReport(VALID);
    expect(report).not.toBeNull();
    expect(report?.version).toBe(1);
    expect(report?.contradictions).toHaveLength(1);
    expect(report?.contradictions[0]?.record_id).toBe("LES-002");
    expect(report?.clear).toBe(false);
  });

  it("tolerates unknown extra fields (forward compatibility)", () => {
    const withExtra = JSON.stringify({ ...JSON.parse(VALID), future_field: "whatever", extra: { nested: true } });
    const report = parseAdviseReport(withExtra);
    expect(report).not.toBeNull();
    expect(report?.contradictions).toHaveLength(1);
  });

  it("returns null for JSON missing the agnosgram_advise version tag", () => {
    expect(parseAdviseReport(JSON.stringify({ contradictions: [] }))).toBeNull();
  });

  it("returns null for unparseable JSON (treated as 'no report', never an error)", () => {
    expect(parseAdviseReport("not json")).toBeNull();
  });

  it("returns null for a clear report with no contradictions array required", () => {
    const report = parseAdviseReport(JSON.stringify({ agnosgram_advise: 1, clear: true }));
    expect(report?.clear).toBe(true);
    expect(report?.contradictions).toEqual([]);
  });

  it("derives the report path as <plan>.advise.json", () => {
    expect(adviseReportPath(".gate/runs/r1/plan.md")).toBe(".gate/runs/r1/plan.md.advise.json");
  });

  it("loadAdviseReport returns null when the file does not exist", () => {
    expect(loadAdviseReport("/nonexistent/plan.md")).toBeNull();
  });
});
