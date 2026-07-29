import { describe, expect, it } from "vitest";
import type { GateConfig } from "../src/core/config.js";
import { newRun } from "../src/core/run.js";
import {
  checkName,
  filesForTarget,
  resolveAffectedTargets,
  resolveDisplayTargets,
  resolvePhaseTargets,
  resolveRunTargets,
  resolveTargetsFromPlanFiles,
  targetCommands,
  targetCoverageFormat,
  targetThresholds,
} from "../src/core/targets.js";
import { makeRepo, writeFile } from "./helpers.js";

const EMPTY_CONFIG: GateConfig = {
  commands: { test: "top-level-test", build: "top-level-build" },
  thresholds: { diff_coverage: 50 },
  targets: {},
  phases: {},
  integrations: {},
  coverage_format: "auto",
  retention: {},
};

const TWO_TARGETS: GateConfig = {
  ...EMPTY_CONFIG,
  targets: {
    api: {
      match: ["apps/api/**"],
      commands: { test: "pytest" },
      thresholds: { diff_coverage: 85 },
      coverage_format: "coverage-py",
    },
    web: { match: ["apps/web/**"], commands: { test: "vitest run" } },
  },
};

describe("resolveAffectedTargets", () => {
  it("returns nothing when no targets are configured", () => {
    expect(resolveAffectedTargets(EMPTY_CONFIG, ["apps/api/foo.py"])).toEqual([]);
  });

  it("returns targets whose match globs cover at least one file", () => {
    expect(resolveAffectedTargets(TWO_TARGETS, ["apps/api/foo.py"])).toEqual(["api"]);
    expect(resolveAffectedTargets(TWO_TARGETS, ["apps/web/foo.ts", "apps/api/foo.py"]).sort()).toEqual(["api", "web"]);
  });

  it("returns nothing when files match no target", () => {
    expect(resolveAffectedTargets(TWO_TARGETS, ["README.md"])).toEqual([]);
  });
});

describe("targetCommands / targetThresholds / targetCoverageFormat", () => {
  it("layers a target's own values over the top-level defaults", () => {
    expect(targetCommands(TWO_TARGETS, "api")).toEqual({ test: "pytest", build: "top-level-build" });
    expect(targetThresholds(TWO_TARGETS, "api")).toEqual({ diff_coverage: 85 });
    // web has no thresholds override - inherits the top-level value.
    expect(targetThresholds(TWO_TARGETS, "web")).toEqual({ diff_coverage: 50 });
  });

  it("returns the top-level values unchanged for an unknown target name", () => {
    expect(targetCommands(TWO_TARGETS, "nope")).toEqual(EMPTY_CONFIG.commands);
  });

  it("a target's own coverage_format wins over the top-level default; falls back without one", () => {
    expect(targetCoverageFormat(TWO_TARGETS, "api")).toBe("coverage-py");
    expect(targetCoverageFormat(TWO_TARGETS, "web")).toBe("auto");
  });
});

describe("checkName", () => {
  it("is bare for a null target and bracketed otherwise", () => {
    expect(checkName("test.command", null)).toBe("test.command");
    expect(checkName("test.command", "api")).toBe("test.command[api]");
  });
});

describe("filesForTarget", () => {
  it("filters to files under the target's match globs", () => {
    const files = ["apps/api/a.py", "apps/web/b.ts", "README.md"];
    expect(filesForTarget(TWO_TARGETS, "api", files)).toEqual(["apps/api/a.py"]);
  });
});

describe("resolveRunTargets", () => {
  it("prefers an explicit override over file-based resolution", () => {
    const run = { targetOverride: ["web"] };
    expect(resolveRunTargets(TWO_TARGETS, run, ["apps/api/a.py"])).toEqual(["web"]);
  });

  it("drops override names that no longer exist in config", () => {
    const run = { targetOverride: ["ghost", "api"] };
    expect(resolveRunTargets(TWO_TARGETS, run, [])).toEqual(["api"]);
  });

  it("falls back to file-based resolution with no override", () => {
    expect(resolveRunTargets(TWO_TARGETS, {}, ["apps/web/x.ts"])).toEqual(["web"]);
  });
});

describe("resolvePhaseTargets (the critical byte-identical invariant)", () => {
  it("collapses to a single bare entry with the top-level commands when no targets are configured", () => {
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    const resolved = resolvePhaseTargets(EMPTY_CONFIG, run, ["src/x.ts"]);
    expect(resolved).toEqual([
      { target: null, commands: EMPTY_CONFIG.commands, thresholds: EMPTY_CONFIG.thresholds, coverageFormat: "auto" },
    ]);
  });

  it("collapses to a single bare entry when targets are configured but none are affected", () => {
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    const resolved = resolvePhaseTargets(TWO_TARGETS, run, ["README.md"]);
    expect(resolved).toEqual([
      { target: null, commands: TWO_TARGETS.commands, thresholds: TWO_TARGETS.thresholds, coverageFormat: "auto" },
    ]);
  });

  it("returns one entry per affected target, each with its own effective commands", () => {
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    const resolved = resolvePhaseTargets(TWO_TARGETS, run, ["apps/api/a.py", "apps/web/b.ts"]);
    expect(resolved.map((r) => r.target).sort()).toEqual(["api", "web"]);
    const api = resolved.find((r) => r.target === "api")!;
    expect(api.commands.test).toBe("pytest");
    expect(api.thresholds.diff_coverage).toBe(85);
  });

  it("honors a --target override even when no files match it", () => {
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null, targetOverride: ["api"] });
    const resolved = resolvePhaseTargets(TWO_TARGETS, run, []);
    expect(resolved).toEqual([
      {
        target: "api",
        commands: targetCommands(TWO_TARGETS, "api"),
        thresholds: targetThresholds(TWO_TARGETS, "api"),
        coverageFormat: "coverage-py",
      },
    ]);
  });
});

describe("resolveTargetsFromPlanFiles", () => {
  it("matches a plan file entry against target match globs", () => {
    expect(resolveTargetsFromPlanFiles(TWO_TARGETS, ["apps/api/**"])).toEqual(["api"]);
  });

  it("errs toward inclusion for a bare wildcard plan entry", () => {
    expect(resolveTargetsFromPlanFiles(TWO_TARGETS, ["**"]).sort()).toEqual(["api", "web"]);
  });

  it("finds no overlap for a plan entry outside every target's directory", () => {
    expect(resolveTargetsFromPlanFiles(TWO_TARGETS, ["docs/readme.md"])).toEqual([]);
  });
});

describe("resolveDisplayTargets", () => {
  it("returns nothing when no targets are configured", () => {
    const root = makeRepo();
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    expect(resolveDisplayTargets(root, run, EMPTY_CONFIG)).toEqual([]);
  });

  it("falls back to the plan's declared files when there is no diff yet (PLAN phase)", () => {
    const root = makeRepo();
    writeFile(root, ".gate/runs/r1/plan.md", "---\ngoal: g\nfiles:\n  - apps/api/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n");
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    expect(resolveDisplayTargets(root, run, TWO_TARGETS)).toEqual(["api"]);
  });

  it("prefers real changed files over the plan once code exists", () => {
    const root = makeRepo();
    writeFile(root, "apps/web/new.ts", "x\n");
    const run = newRun({ id: "r1", title: "t", profile: "feature", baseRef: null, sessionId: null });
    expect(resolveDisplayTargets(root, run, TWO_TARGETS)).toEqual(["web"]);
  });
});
