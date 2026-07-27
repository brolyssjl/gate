import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
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
    expect(gate(repo, ["trust"]).code).toBe(0); // approve the inferred commands
    expect(gate(repo, ["start", "add greet"]).code).toBe(0);

    // PLAN gate fails on the empty scaffold.
    expect(gate(repo, ["check"]).code).toBe(1);

    // Write a real plan; it still fails until a separate `gate approve`.
    // Run id is date-based; discover it via status --json instead of hardcoding.
    const runId = (gate(repo, ["status", "--json"]).json() as { id: string }).id;
    writeFileSync(join(repo, `.gate/runs/${runId}/plan.md`), PLAN);
    expect(gate(repo, ["check"]).code).toBe(1); // schema ok now, but not approved
    expect(gate(repo, ["approve"]).code).toBe(0);
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

    // Green suite → TEST passes and the run enters REVIEW (feature profile).
    writeFileSync(
      join(repo, "test.js"),
      `const {greet}=require('./greet');const ok=greet('Sam')==='Hello, Sam';` +
        `console.log(JSON.stringify({tests:[{name:'greets by name',status:ok?'passed':'failed'}]}));` +
        `process.exit(ok?0:1)`,
    );
    expect(gate(repo, ["next"]).code).toBe(0);
    expect((gate(repo, ["status", "--json"]).json() as { phase: string }).phase).toBe("REVIEW");

    // REVIEW: blocks until a fresh packet is emitted, and the untouched
    // scaffold must NOT pass - a reviewer has to sign review.md.
    expect(gate(repo, ["check"]).code).toBe(1);
    expect(gate(repo, ["review", "--fresh"]).code).toBe(0);
    expect(gate(repo, ["next"]).code).toBe(1);
    const reviewRunId = (gate(repo, ["status", "--json"]).json() as { id: string }).id;
    writeFileSync(
      join(repo, `.gate/runs/${reviewRunId}/review.md`),
      `---\nreviewer: fresh-eyes\nfindings: []\n---\n# Review\n`,
    );
    expect(gate(repo, ["next"]).code).toBe(0);
    expect((gate(repo, ["status", "--json"]).json() as { phase: string }).phase).toBe("RETRO");

    // RETRO: the empty scaffold has no substance and must not pass; a filled-in
    // retro (no .agnosgram/ store in this fixture repo, so no journal sync is
    // required) reaches DONE.
    expect(gate(repo, ["check"]).code).toBe(1);
    writeFileSync(
      join(repo, `.gate/runs/${reviewRunId}/retro.md`),
      `---\nbroke: []\navoid:\n  - "Do not skip the reproduce step"\nconventions: []\n---\n# Retro\n`,
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

  it("records a human-authorized skip and advances", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    gate(repo, ["start", "skip demo"]);

    expect(gate(repo, ["skip", "PLAN"]).code).toBe(2); // --reason required
    expect(gate(repo, ["skip", "PLAN", "--reason", "trivial doc tweak"]).code).toBe(0);

    const status = gate(repo, ["status", "--json"]).json() as { phase: string };
    expect(status.phase).toBe("IMPLEMENT");
    const runId = "skip demo".replace(/[^a-z0-9]+/gi, "-").toLowerCase();
    const run = JSON.parse(
      readFileSync(join(repo, `.gate/runs/${todayPrefix()}-${runId}/run.json`), "utf8"),
    ) as { overrides: Array<{ phase: string; reason: string }> };
    expect(run.overrides).toEqual([expect.objectContaining({ phase: "PLAN", reason: "trivial doc tweak" })]);
  });

  it("registers an artifact with gate log", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    gate(repo, ["start", "log demo"]);
    writeFileSync(join(repo, "notes.txt"), "hello\n");
    expect(gate(repo, ["log", "notes.txt"]).code).toBe(0);
    const status = gate(repo, ["status", "--json"]).json() as { artifacts: string[] };
    expect(status.artifacts).toContain("notes.txt");
  });

  it("selects the phase set from the --profile flag", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);

    // bugfix routes through DEBUG (and skips IMPLEMENT); it scaffolds debug-log.md.
    expect(gate(repo, ["start", "fix login", "--profile", "bugfix"]).code).toBe(0);
    const status = gate(repo, ["status", "--json"]).json() as { profile: string; phases: string[] };
    expect(status.profile).toBe("bugfix");
    expect(status.phases).toEqual(["PLAN", "DEBUG", "TEST", "REVIEW", "RETRO", "DONE"]);
    const runId = (gate(repo, ["status", "--json"]).json() as { id: string }).id;
    expect(existsSync(join(repo, `.gate/runs/${runId}/debug-log.md`))).toBe(true);
  });

  it("rejects an unknown profile", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    expect(gate(repo, ["start", "x", "--profile", "bogus"]).code).toBe(2);
  });

  it("targets: --target override wins, and playbook overlays surface for affected targets", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    writeFileSync(
      join(repo, ".gate", "playbooks", "test.web.md"),
      "# Web overlay\n\nRun the visual regression suite.\n",
    );
    writeFileSync(
      join(repo, ".gate", "config.yml"),
      [
        "commands: {}",
        "thresholds: {}",
        "targets:",
        "  web:",
        "    match: [\"apps/web/**\"]",
        "    playbooks: { test: \".gate/playbooks/test.web.md\" }",
        "  api:",
        "    match: [\"apps/api/**\"]",
        "phases: {}",
        "integrations: { agnosgram: off, sdd: off }",
        "",
      ].join("\n"),
    );

    expect(gate(repo, ["start", "target override demo", "--target", "web"]).code).toBe(0);
    const playbook = gate(repo, ["playbook", "TEST", "--json"]).json() as { playbook: string };
    expect(playbook.playbook).toContain("## Target overlay: web");
    expect(playbook.playbook).toContain("Run the visual regression suite.");
  });

  it("refuses to reach DONE when a review fix breaks the code (staleness guard)", () => {
    const repo = makeRepo({
      "package.json": JSON.stringify({ name: "fx", scripts: { test: "node test.js" } }),
      "greet.js": "module.exports = (n) => 'Hello, ' + n\n",
      "test.js":
        `const ok = require('./greet')('Sam') === 'Hello, Sam';\n` +
        `console.log(JSON.stringify({tests:[{name:'greets by name',status:ok?'passed':'failed'}]}));\n` +
        `process.exit(ok ? 0 : 1)\n`,
    });
    gate(repo, ["init"]);
    gate(repo, ["trust"]);
    gate(repo, ["start", "demo"]);
    const runId = (gate(repo, ["status", "--json"]).json() as { id: string }).id;
    writeFileSync(
      join(repo, `.gate/runs/${runId}/plan.md`),
      `---\ngoal: demo\nfiles:\n  - "greet.js"\n  - "test.js"\ncriteria:\n  - id: c1\n    text: greets\n    verify: "test: greets by name"\n---\n# Plan\n`,
    );
    gate(repo, ["approve", "--by", "human"]);
    expect(gate(repo, ["next"]).code).toBe(0); // PLAN -> IMPLEMENT
    writeFileSync(join(repo, "greet.js"), "module.exports = (n) => 'Hello, ' + n // touched\n");
    expect(gate(repo, ["next"]).code).toBe(0); // IMPLEMENT -> TEST
    expect(gate(repo, ["next"]).code).toBe(0); // TEST -> REVIEW
    const review = gate(repo, ["review", "--fresh", "--json"]);
    expect(Object.keys(review.json() as Record<string, unknown>).sort()).toEqual([
      "diff",
      "findingsFile",
      "packet",
      "phase",
      "plan",
      "regenerated",
      "requestedBy",
      "rubric",
      "treeHash",
    ]);

    // Reviewer files a blocker; the implementer "fixes" it by breaking the
    // suite and flips the finding to resolved.
    writeFileSync(
      join(repo, `.gate/runs/${runId}/review.md`),
      `---\nreviewer: fresh-eyes\nfindings:\n  - id: f1\n    severity: blocker\n    status: resolved\n    note: crash on null\n---\n`,
    );
    writeFileSync(join(repo, "greet.js"), "throw new Error('boom')\n");

    // Stale packet: the reviewer never saw the "fix".
    expect(gate(repo, ["next"]).code).toBe(1);
    // Without --fresh an existing packet is not re-baselined.
    gate(repo, ["review"]);
    expect(gate(repo, ["next"]).code).toBe(1);
    // Even with a fresh packet, re-verification catches the red suite.
    gate(repo, ["review", "--fresh"]);
    expect(gate(repo, ["next"]).code).toBe(1);

    // A real fix, re-packeted, goes green and enters RETRO.
    writeFileSync(join(repo, "greet.js"), "module.exports = (n) => 'Hello, ' + n\n");
    gate(repo, ["review", "--fresh"]);
    expect(gate(repo, ["next"]).code).toBe(0);
    expect((gate(repo, ["status", "--json"]).json() as { phase: string }).phase).toBe("RETRO");
    writeFileSync(
      join(repo, `.gate/runs/${runId}/retro.md`),
      `---\nbroke:\n  - "Fixing a blocker by breaking the build almost shipped"\navoid: []\nconventions: []\n---\n# Retro\n`,
    );
    expect(gate(repo, ["next"]).code).toBe(0);
    expect((gate(repo, ["status", "--json"]).json() as { active: boolean }).active).toBe(false);
  });

  it("gate retro syncs the journal (fallback path — no agnosgram binary in this sandbox) and is idempotent", () => {
    const repo = makeRepo();
    mkdirSync(join(repo, ".agnosgram"), { recursive: true });
    writeFileSync(join(repo, ".agnosgram", "config.yml"), "version: 1\n");
    gate(repo, ["init"]);
    gate(repo, ["start", "retro sync demo"]);
    const runId = (gate(repo, ["status", "--json"]).json() as { id: string }).id;
    // Walk PLAN -> RETRO the short way: skip every gated phase up to RETRO.
    expect(gate(repo, ["skip", "PLAN", "--reason", "test"]).code).toBe(0);
    expect(gate(repo, ["skip", "IMPLEMENT", "--reason", "test"]).code).toBe(0);
    expect(gate(repo, ["skip", "TEST", "--reason", "test"]).code).toBe(0);
    expect(gate(repo, ["skip", "REVIEW", "--reason", "test"]).code).toBe(0);
    expect((gate(repo, ["status", "--json"]).json() as { phase: string }).phase).toBe("RETRO");

    // No substance yet: gate retro refuses.
    expect(gate(repo, ["retro"]).code).toBe(1);

    writeFileSync(
      join(repo, `.gate/runs/${runId}/retro.md`),
      `---\nbroke: []\navoid:\n  - "Do not skip reproduce"\nconventions: []\n---\n# Retro\n`,
    );
    const synced = gate(repo, ["retro", "--json"]);
    expect(synced.code).toBe(0);
    const syncedData = synced.json() as { synced: boolean; method: string; journalFile: string };
    expect(syncedData.synced).toBe(true);
    expect(syncedData.method).toBe("fallback");
    const journal = readFileSync(join(repo, syncedData.journalFile), "utf8");
    expect(journal).toContain(runId);
    expect(journal).toContain("- **Avoid:** Do not skip reproduce");

    // Idempotent: re-running does not duplicate the entry — byte-identical file.
    const again = gate(repo, ["retro", "--json"]).json() as { alreadySynced: boolean };
    expect(again.alreadySynced).toBe(true);
    const journalAfter = readFileSync(join(repo, syncedData.journalFile), "utf8");
    expect(journalAfter).toBe(journal);

    expect(gate(repo, ["next"]).code).toBe(0);
    expect((gate(repo, ["status", "--json"]).json() as { active: boolean }).active).toBe(false);
  });

  it("gate adapt writes every adapter by default and is idempotent across the CLI", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);

    const first = gate(repo, ["adapt", "--json"]);
    expect(first.code).toBe(0);
    const firstData = first.json() as { adapters: Array<{ path: string; action: string }> };
    expect(firstData.adapters.length).toBeGreaterThanOrEqual(6);
    expect(firstData.adapters.every((a) => a.action === "created")).toBe(true);
    for (const a of firstData.adapters) expect(existsSync(join(repo, a.path))).toBe(true);

    const second = gate(repo, ["adapt", "--json"]);
    const secondData = second.json() as { adapters: Array<{ action: string }> };
    expect(secondData.adapters.every((a) => a.action === "unchanged")).toBe(true);

    expect(gate(repo, ["adapt", "cursor", "--json"]).code).toBe(0);
    expect(gate(repo, ["adapt", "not-a-real-adapter"]).code).toBe(2);
  });

  it("reports per-run durations, gate failures, and findings", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    gate(repo, ["start", "report demo"]);
    // One failed advancement attempt (empty plan) is recorded for the report.
    expect(gate(repo, ["next"]).code).toBe(1);

    const report = gate(repo, ["report", "--json"]).json() as {
      id: string;
      profile: string;
      gateFailures: number;
      phases: Array<{ phase: string; gateFailures: number }>;
      findings: unknown;
    };
    expect(report.profile).toBe("feature");
    expect(report.gateFailures).toBeGreaterThanOrEqual(1);
    expect(report.phases.some((p) => p.phase === "PLAN")).toBe(true);
    expect(report.findings).toBeNull(); // no review.md yet
  });
});

describe("gate --json schema", () => {
  const repo = makeRepo({
    "package.json": JSON.stringify({ name: "fx", scripts: { test: "node -e 0" } }),
  });

  beforeAll(() => {
    gate(repo, ["init"]);
    gate(repo, ["trust"]);
  });

  const keys = (r: Run) => Object.keys(r.json() as Record<string, unknown>).sort();

  it("init/start/status expose stable top-level keys", () => {
    expect(keys(gate(repo, ["init", "--json"]))).toEqual(["detected", "initialized", "refreshed", "root"]);
    expect(keys(gate(repo, ["start", "schema demo", "--json"]))).toEqual([
      "hints",
      "id",
      "phase",
      "phases",
      "plan",
      "profile",
    ]);
    expect(keys(gate(repo, ["status", "--json"]))).toEqual([
      "active",
      "artifacts",
      "baseRef",
      "id",
      "nextAction",
      "phase",
      "phases",
      "profile",
      "sessionId",
      "status",
      "title",
    ]);
  });

  it("check/report/playbook expose stable top-level keys", () => {
    expect(keys(gate(repo, ["check", "--json"]))).toEqual(["checks", "ok", "phase"]);
    expect(keys(gate(repo, ["report", "--json"]))).toEqual([
      "artifacts",
      "findings",
      "gateFailures",
      "id",
      "overrides",
      "phase",
      "phases",
      "profile",
      "status",
      "title",
      "totalSeconds",
    ]);
    expect(keys(gate(repo, ["playbook", "PLAN", "--json"]))).toEqual(["hints", "phase", "playbook"]);
  });
});

function todayPrefix(): string {
  return new Date().toISOString().slice(0, 10);
}
