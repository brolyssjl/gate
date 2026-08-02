import { join } from "node:path";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { gate, makeRepo } from "./helpers.js";

/** Write a minimal, valid, non-active run.json directly (skip the full state-machine walk). */
function seedRun(repo: string, id: string, updatedAt: string, status: "done" | "abandoned" = "done"): void {
  const dir = join(repo, ".gate", "runs", id);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, "run.json"),
    JSON.stringify({
      schema: 2,
      id,
      title: id,
      profile: "docs",
      phase: "DONE",
      status,
      createdAt: updatedAt,
      updatedAt,
      baseRef: null,
      sessionId: null,
      history: [{ phase: "PLAN", event: "entered", at: updatedAt }],
      overrides: [],
      artifacts: {},
    }),
  );
}

function daysAgo(n: number): string {
  return new Date(Date.now() - n * 24 * 60 * 60 * 1000).toISOString();
}

describe("gate prune", () => {
  it("keeps the newest --keep runs and prunes the rest, archiving summaries", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    for (let i = 0; i < 5; i++) seedRun(repo, `run-${i}`, daysAgo(i));

    const dry = gate(repo, ["prune", "--keep", "2", "--dry-run", "--json"]);
    expect(dry.code).toBe(0);
    const dryData = dry.json() as { dryRun: boolean; pruned: string[] };
    expect(dryData.dryRun).toBe(true);
    expect(dryData.pruned.sort()).toEqual(["run-2", "run-3", "run-4"]);
    // Dry run touches nothing.
    expect(existsSync(join(repo, ".gate", "runs", "run-4"))).toBe(true);
    expect(existsSync(join(repo, ".gate", "archive"))).toBe(false);

    const real = gate(repo, ["prune", "--keep", "2", "--json"]);
    expect(real.code).toBe(0);
    const realData = real.json() as { pruned: string[] };
    expect(realData.pruned.sort()).toEqual(["run-2", "run-3", "run-4"]);

    // Newest 2 survive untouched.
    expect(existsSync(join(repo, ".gate", "runs", "run-0"))).toBe(true);
    expect(existsSync(join(repo, ".gate", "runs", "run-1"))).toBe(true);
    // Pruned run folders are gone, but archived.
    for (const id of ["run-2", "run-3", "run-4"]) {
      expect(existsSync(join(repo, ".gate", "runs", id))).toBe(false);
      expect(existsSync(join(repo, ".gate", "archive", `${id}.json`))).toBe(true);
      const archived = JSON.parse(readFileSync(join(repo, ".gate", "archive", `${id}.json`), "utf8")) as { id: string };
      expect(archived.id).toBe(id);
    }
  });

  it("never prunes the active run, even if it's the oldest", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    gate(repo, ["start", "active one", "--profile", "docs"]);
    const activeId = (gate(repo, ["status", "--json"]).json() as { id: string }).id;
    // Backdate it so it would otherwise be the top prune candidate.
    const runJsonPath = join(repo, ".gate", "runs", activeId, "run.json");
    const run = JSON.parse(readFileSync(runJsonPath, "utf8")) as { updatedAt: string };
    run.updatedAt = daysAgo(30);
    writeFileSync(runJsonPath, JSON.stringify(run));
    seedRun(repo, "finished-run", daysAgo(1));

    const res = gate(repo, ["prune", "--keep", "0", "--json"]);
    const data = res.json() as { pruned: string[] };
    expect(data.pruned).toEqual(["finished-run"]);
    expect(data.pruned).not.toContain(activeId);
    expect(existsSync(join(repo, ".gate", "runs", activeId))).toBe(true);
  });

  it("protects every run current.json maps to, even a done run mapped under a branch other than the one checked out", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    // A done-but-still-mapped run (the failure mode a migration/advance bug
    // could produce): status is "done", so nothing about its own record
    // marks it active, but current.json still points a branch at it.
    seedRun(repo, "stale-mapped-run", daysAgo(30), "done");
    writeFileSync(
      join(repo, ".gate", "current.json"),
      JSON.stringify({ schema: 1, branches: { "some-other-branch": "stale-mapped-run" } }),
    );
    seedRun(repo, "genuinely-finished", daysAgo(1));

    const res = gate(repo, ["prune", "--keep", "0", "--json"]);
    const data = res.json() as { pruned: string[] };
    expect(data.pruned).toEqual(["genuinely-finished"]);
    expect(data.pruned).not.toContain("stale-mapped-run");
    expect(existsSync(join(repo, ".gate", "runs", "stale-mapped-run"))).toBe(true);
  });

  it("--days additionally requires a candidate to be older than N days", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    seedRun(repo, "recent", daysAgo(1));
    seedRun(repo, "old", daysAgo(40));

    // keep=0 alone would prune both; --days 30 spares the recent one.
    const res = gate(repo, ["prune", "--keep", "0", "--days", "30", "--json"]);
    const data = res.json() as { pruned: string[] };
    expect(data.pruned).toEqual(["old"]);
    expect(existsSync(join(repo, ".gate", "runs", "recent"))).toBe(true);
    expect(existsSync(join(repo, ".gate", "runs", "old"))).toBe(false);
  });

  it("gate report falls back to the archived summary after a run is pruned", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    seedRun(repo, "archived-run", daysAgo(10));
    gate(repo, ["prune", "--keep", "0"]);
    expect(existsSync(join(repo, ".gate", "runs", "archived-run"))).toBe(false);

    const report = gate(repo, ["report", "archived-run", "--json"]);
    expect(report.code).toBe(0);
    const data = report.json() as { id: string; profile: string };
    expect(data.id).toBe("archived-run");
    expect(data.profile).toBe("docs");
    expect(report.stdout).toBeTruthy();

    const human = gate(repo, ["report", "archived-run"]);
    expect(human.stdout).toContain("(archived)");
  });

  it("gate report with no run id falls back to the newest archived summary after a full prune", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    seedRun(repo, "older-run", daysAgo(20));
    seedRun(repo, "newer-run", daysAgo(2));
    // Prune both, oldest archived first so mtime recency actually distinguishes them.
    gate(repo, ["prune", "--keep", "0"]);
    expect(existsSync(join(repo, ".gate", "runs", "older-run"))).toBe(false);
    expect(existsSync(join(repo, ".gate", "runs", "newer-run"))).toBe(false);

    // No positional arg, no active run - must not error just because the live
    // runs folder is empty; both summaries are still on disk under archive/.
    const report = gate(repo, ["report", "--json"]);
    expect(report.code).toBe(0);
    const data = report.json() as { id: string };
    expect(["older-run", "newer-run"]).toContain(data.id);
  });
});
