import { execFileSync, spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { describe, expect, it } from "vitest";
import { makeRepo } from "./helpers.js";

const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const CLI = join(pkgRoot, "dist", "cli.js");

function gate(cwd: string, args: string[]): { code: number; stdout: string; stderr: string; json: () => unknown } {
  const res = spawnSync("node", [CLI, ...args], { cwd, encoding: "utf8" });
  return { code: res.status ?? 1, stdout: res.stdout, stderr: res.stderr, json: () => JSON.parse(res.stdout) };
}

/** Async spawn, for launching several `gate` invocations genuinely concurrently. */
function gateAsync(cwd: string, args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    const child = spawn("node", [CLI, ...args], { cwd });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d: Buffer) => (stdout += d));
    child.stderr.on("data", (d: Buffer) => (stderr += d));
    child.on("close", (code) => resolve({ code: code ?? 1, stdout, stderr }));
  });
}

function git(cwd: string, args: string[]): string {
  return execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
}

/** Write a minimal, valid, active schema-2 run.json directly (pre-Milestone-4). */
function seedLegacyRun(repo: string, id: string): void {
  const dir = join(repo, ".gate", "runs", id);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, "run.json"),
    JSON.stringify({
      schema: 2,
      id,
      title: id,
      profile: "docs",
      phase: "PLAN",
      status: "active",
      createdAt: "2026-01-01T00:00:00.000Z",
      updatedAt: "2026-01-01T00:00:00.000Z",
      baseRef: null,
      sessionId: null,
      history: [{ phase: "PLAN", event: "entered", at: "2026-01-01T00:00:00.000Z" }],
      overrides: [],
      artifacts: {},
    }),
  );
}

describe("branch-keyed run concurrency", () => {
  it("keys runs by branch: independent runs on separate branches, status shows the current one plus others in flight", () => {
    const repo = makeRepo({ "package.json": JSON.stringify({ name: "fx", scripts: { test: "node -e 0" } }) });
    gate(repo, ["init"]);
    gate(repo, ["trust"]);
    const mainBranch = git(repo, ["branch", "--show-current"]);

    const started = gate(repo, ["start", "run on main", "--profile", "docs", "--json"]);
    expect(started.code).toBe(0);
    const mainRun = started.json() as { id: string; branch: string };
    expect(mainRun.branch).toBe(mainBranch);

    git(repo, ["checkout", "-b", "other-branch"]);
    const startedOther = gate(repo, ["start", "run on other branch", "--profile", "docs", "--json"]);
    expect(startedOther.code).toBe(0);
    const otherRun = startedOther.json() as { id: string; branch: string };
    expect(otherRun.branch).toBe("other-branch");
    expect(otherRun.id).not.toBe(mainRun.id);

    const statusOther = gate(repo, ["status", "--json"]).json() as {
      id: string;
      branch: string;
      others: Array<{ branch: string; id: string }>;
    };
    expect(statusOther.id).toBe(otherRun.id);
    expect(statusOther.branch).toBe("other-branch");
    expect(statusOther.others).toEqual([{ branch: mainBranch, id: mainRun.id, phase: "PLAN" }]);

    git(repo, ["checkout", mainBranch]);
    const statusMain = gate(repo, ["status", "--json"]).json() as { id: string; branch: string };
    expect(statusMain.id).toBe(mainRun.id);
    expect(statusMain.branch).toBe(mainBranch);
  });

  it("gate start resumes a branch's existing active run instead of erroring", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    const first = gate(repo, ["start", "first title", "--profile", "docs", "--json"]).json() as { id: string };

    const second = gate(repo, ["start", "a different title", "--profile", "docs", "--json"]);
    expect(second.code).toBe(0);
    const resumed = second.json() as { id: string; resumed: boolean };
    expect(resumed.resumed).toBe(true);
    expect(resumed.id).toBe(first.id);
  });

  it("gate start refuses to resume when the plan changed since approval", () => {
    const repo = makeRepo({ "package.json": JSON.stringify({ name: "fx", scripts: { test: "node -e 0" } }) });
    gate(repo, ["init"]);
    gate(repo, ["trust"]);
    const started = gate(repo, ["start", "needs a plan", "--json"]).json() as { id: string; plan: string };

    const planContent = [
      "---",
      "goal: ship it",
      "files:",
      "  - a.txt",
      "criteria:",
      "  - id: c1",
      "    text: it works",
      '    verify: "manual"',
      "---",
      "",
    ].join("\n");
    writeFileSync(started.plan, planContent);
    expect(gate(repo, ["approve", "--by", "tester"]).code).toBe(0);

    // Edit the plan post-approval without re-approving.
    writeFileSync(started.plan, planContent + "\nextra line\n");

    const retry = gate(repo, ["start", "needs a plan again"]);
    expect(retry.code).not.toBe(0);
    expect(retry.stderr).toContain("changed since");
  });

  it("refuses to start on a detached HEAD (no branch to key the run by)", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    const sha = git(repo, ["rev-parse", "HEAD"]);
    git(repo, ["checkout", sha]);

    const res = gate(repo, ["start", "detached attempt"]);
    expect(res.code).not.toBe(0);
    expect(res.stderr).toContain("detached");
  });

  it("detached HEAD: gate status reports it without throwing; phase commands require --run", () => {
    const repo = makeRepo({ "package.json": JSON.stringify({ name: "fx", scripts: { test: "node -e 0" } }) });
    gate(repo, ["init"]);
    gate(repo, ["trust"]);
    const started = gate(repo, ["start", "run before detaching", "--json"]).json() as { id: string };

    const sha = git(repo, ["rev-parse", "HEAD"]);
    git(repo, ["checkout", sha]);

    const status = gate(repo, ["status", "--json"]).json() as { active: boolean; detached: boolean };
    expect(status.active).toBe(false);
    expect(status.detached).toBe(true);

    const withoutRun = gate(repo, ["check"]);
    expect(withoutRun.code).not.toBe(0);
    expect(withoutRun.stderr).toContain("detached");

    const withRun = gate(repo, ["check", "--run", started.id]);
    expect(withRun.stderr).not.toContain("detached");
  });

  it("--run refuses a non-active (done/abandoned) run instead of letting phase gates pass vacuously", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    seedLegacyRun(repo, "finished-run"); // schema 2, status: active by default - override below
    const runPath = join(repo, ".gate", "runs", "finished-run", "run.json");
    const run = JSON.parse(readFileSync(runPath, "utf8")) as { status: string };
    run.status = "done";
    writeFileSync(runPath, JSON.stringify(run));

    const res = gate(repo, ["check", "--run", "finished-run"]);
    expect(res.code).not.toBe(0);
    expect(res.stderr).toContain("not active");
  });

  it("--run refuses a run whose recorded branch doesn't match the checked-out branch (would otherwise diff against the wrong tree)", () => {
    const repo = makeRepo({ "package.json": JSON.stringify({ name: "fx", scripts: { test: "node -e 0" } }) });
    gate(repo, ["init"]);
    gate(repo, ["trust"]);
    const mainBranch = git(repo, ["branch", "--show-current"]);
    const started = gate(repo, ["start", "run on main", "--profile", "docs", "--json"]).json() as { id: string };

    git(repo, ["checkout", "-b", "other-branch"]);
    const res = gate(repo, ["check", "--run", started.id]);
    expect(res.code).not.toBe(0);
    expect(res.stderr).toContain(mainBranch);
    expect(res.stderr).toContain("other-branch");

    // Checking back out to the run's own branch, --run works again.
    git(repo, ["checkout", mainBranch]);
    const ok = gate(repo, ["check", "--run", started.id]);
    expect(ok.stderr).not.toContain("checked out");
  });

  it("migrates the legacy single-run .gate/current pointer into per-branch current.json", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    seedLegacyRun(repo, "legacy-run");
    const branch = git(repo, ["branch", "--show-current"]);
    writeFileSync(join(repo, ".gate", "current"), "legacy-run\n");

    const status = gate(repo, ["status", "--json"]).json() as { id: string; active: boolean };
    expect(status.active).toBe(true);
    expect(status.id).toBe("legacy-run");

    expect(existsSync(join(repo, ".gate", "current"))).toBe(false);
    const migrated = JSON.parse(readFileSync(join(repo, ".gate", "current.json"), "utf8")) as {
      branches: Record<string, string>;
    };
    expect(migrated.branches[branch]).toBe("legacy-run");
  });

  it("resolves the branch name on an unborn branch (git init, zero commits yet) - distinct from detached HEAD", () => {
    // Not makeRepo() - that helper always commits. A fresh `git init` repo has
    // no commits, so `git rev-parse --abbrev-ref HEAD` fails exactly as it
    // does on a real detached HEAD; this must still resolve the branch name.
    const repo = mkdtempSync(join(tmpdir(), "gate-unborn-"));
    execFileSync("git", ["init", "-q"], { cwd: repo });
    gate(repo, ["init"]);

    const started = gate(repo, ["start", "first run ever", "--json"]);
    expect(started.code).toBe(0);
    const data = started.json() as { branch: string | null };
    expect(data.branch).not.toBeNull();

    const status = gate(repo, ["status", "--json"]).json() as { detached?: boolean; active: boolean };
    expect(status.detached).toBeFalsy();
    expect(status.active).toBe(true);
  });

  it("works with no git repo at all: a single implicit key, no branch ambiguity", () => {
    // Not makeRepo() - a plain directory with no `git init`, exercising the
    // NO_GIT_BRANCH_KEY path (distinct from detached HEAD, which *is* a repo).
    const repo = mkdtempSync(join(tmpdir(), "gate-no-git-"));
    gate(repo, ["init"]);

    const started = gate(repo, ["start", "no-git run", "--profile", "docs", "--json"]);
    expect(started.code).toBe(0);
    const startedData = started.json() as { branch: string | null; id: string };
    expect(startedData.branch).toBeNull();

    const status = gate(repo, ["status", "--json"]).json() as { active: boolean; id: string; branch: string | null };
    expect(status.active).toBe(true);
    expect(status.id).toBe(startedData.id);
    expect(status.branch).toBeNull();

    // A second `gate start` on the same (non-)branch resumes rather than erroring.
    const again = gate(repo, ["start", "different title", "--json"]).json() as { resumed: boolean; id: string };
    expect(again.resumed).toBe(true);
    expect(again.id).toBe(startedData.id);
  });

  it("fails closed on a corrupt current.json instead of silently discarding every branch's mapping", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    gate(repo, ["start", "will be corrupted", "--profile", "docs"]);
    writeFileSync(join(repo, ".gate", "current.json"), "{ not: valid json");

    const status = gate(repo, ["status"]);
    expect(status.code).not.toBe(0);
    expect(status.stderr).toContain("corrupt");
    expect(status.stderr).toContain("current.json");
  });

  it("serializes concurrent `gate start` on the same branch: exactly one run wins, current.json never corrupts", async () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    const branch = git(repo, ["branch", "--show-current"]);

    const N = 8;
    const results = await Promise.all(
      Array.from({ length: N }, (_, i) =>
        gateAsync(repo, ["start", `concurrent run ${i}`, "--profile", "docs", "--json"]),
      ),
    );
    expect(results.every((r) => r.code === 0)).toBe(true);

    const parsed = results.map((r) => JSON.parse(r.stdout) as { id: string; resumed?: boolean });
    const created = parsed.filter((p) => !p.resumed);
    const resumed = parsed.filter((p) => p.resumed);
    // With the whole read-decide-write sequence under one lock, every
    // process but the first to acquire it must see the first's write and
    // resume - never two winners, never a lost mapping.
    expect(created.length).toBe(1);
    expect(resumed.length).toBe(N - 1);
    const winningId = created[0]!.id;
    expect(parsed.every((p) => p.id === winningId)).toBe(true);

    const state = JSON.parse(readFileSync(join(repo, ".gate", "current.json"), "utf8")) as {
      branches: Record<string, string>;
    };
    expect(state.branches[branch]).toBe(winningId);
    expect(Object.keys(state.branches)).toEqual([branch]);
  });
});
