import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { readFileSync, writeFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { makeRepo, writeFile } from "./helpers.js";

const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const CLI = join(pkgRoot, "dist", "cli.js");

function gate(
  cwd: string,
  args: string[],
  input?: string,
): { code: number; stdout: string; stderr: string; json: () => unknown } {
  const res = spawnSync("node", [CLI, ...args], { cwd, encoding: "utf8", input });
  return { code: res.status ?? 1, stdout: res.stdout, stderr: res.stderr, json: () => JSON.parse(res.stdout) };
}

/** Walk a fresh repo from init to an active REVIEW-phase run. */
function setupReviewRun(repo: string): string {
  writeFile(repo, "run-tests.sh", "#!/bin/sh\nexit 0\n");
  execFileSync("chmod", ["+x", join(repo, "run-tests.sh")]);
  gate(repo, ["init"]);
  const configPath = join(repo, ".gate", "config.yml");
  writeFileSync(configPath, readFileSync(configPath, "utf8").replace('# test: "<command>"', 'test: "./run-tests.sh"'));
  gate(repo, ["trust"]);

  const started = gate(repo, ["start", "human review test", "--profile", "feature", "--json"]).json() as {
    id: string;
    plan: string;
  };
  writeFileSync(
    started.plan,
    [
      "---",
      "goal: exercise human review",
      "files:",
      "  - a.txt",
      "  - run-tests.sh",
      "criteria:",
      "  - id: c1",
      "    text: it works",
      '    verify: "manual"',
      "---",
      "",
    ].join("\n"),
  );
  gate(repo, ["approve", "--by", "implementer"]);
  gate(repo, ["next"]); // -> IMPLEMENT
  writeFile(repo, "a.txt", "hi\n");
  gate(repo, ["next"]); // -> TEST
  gate(repo, ["next"]); // -> REVIEW
  return started.id;
}

describe("gate review --human", () => {
  it("records a waived blocker with a rationale, satisfying the same REVIEW gate as an agent review", () => {
    const repo = makeRepo();
    setupReviewRun(repo);

    const input = ["y", "", "blocker", "off by one in the loop", "waived", "acceptable for this run", "n", "tester"].join(
      "\n",
    ) + "\n";
    const res = gate(repo, ["review", "--human"], input);
    expect(res.code).toBe(0);

    const check = gate(repo, ["check", "--json"]);
    expect(check.code).toBe(0);
    const data = check.json() as { ok: boolean };
    expect(data.ok).toBe(true);
  });

  it("blocks the REVIEW gate on an open blocker finding, same as an agent-recorded one", () => {
    const repo = makeRepo();
    setupReviewRun(repo);

    const input = ["y", "", "blocker", "this is unresolved", "open", "n", "tester"].join("\n") + "\n";
    const res = gate(repo, ["review", "--human"], input);
    expect(res.code).toBe(0);

    const check = gate(repo, ["check", "--json"]);
    expect(check.code).toBe(1);
    const data = check.json() as { ok: boolean; checks: Array<{ name: string; ok: boolean }> };
    expect(data.ok).toBe(false);
    expect(data.checks.some((c) => c.name === "review.findings" && !c.ok)).toBe(true);
  });

  it("records multiple findings and requires a reviewer name before writing review.md", () => {
    const repo = makeRepo();
    setupReviewRun(repo);

    const input =
      ["y", "f-typo", "nit", "typo in a comment", "resolved", "y", "f-perf", "minor", "could be faster", "open", "n", "reviewer-two"].join(
        "\n",
      ) + "\n";
    const res = gate(repo, ["review", "--human"], input);
    expect(res.code).toBe(0);

    const check = gate(repo, ["check", "--json"]);
    expect(check.code).toBe(0); // minor/nit never block
  });

  it("reuses an existing packet unless --fresh is passed, matching the non-human review command", () => {
    const repo = makeRepo();
    setupReviewRun(repo);

    const first = gate(repo, ["review", "--human"], ["n", "tester"].join("\n") + "\n");
    expect(first.stdout).toContain("freshly generated");

    const second = gate(repo, ["review", "--human"], ["n", "tester"].join("\n") + "\n");
    expect(second.stdout).toContain("reusing an existing packet");
  });

  it("errors clearly instead of hanging when input ends before the review is recorded", () => {
    const repo = makeRepo();
    setupReviewRun(repo);

    const res = gate(repo, ["review", "--human"], "n\n"); // stops right before the reviewer-name prompt
    expect(res.code).not.toBe(0);
    expect(res.stderr).toContain("input ended");
  });

  const seedFindingF2 = (reviewPath: string): void =>
    writeFileSync(
      reviewPath,
      [
        "---",
        "reviewer:",
        "findings:",
        "  - id: f2",
        "    severity: minor",
        "    status: resolved",
        "    note: pre-existing - f1 was deleted by hand",
        "---",
        "",
        "# Review",
        "",
      ].join("\n"),
    );

  it("defaults to a non-colliding finding id when an earlier one was deleted by hand (only f2 remains)", () => {
    const repo = makeRepo();
    const id = setupReviewRun(repo);
    const reviewPath = join(repo, ".gate", "runs", id, "review.md");
    seedFindingF2(reviewPath);

    // Accept the default id (empty input) for one new finding. The old bug:
    // the default was existing.length + 1 = "f2" again (length 1, not the
    // actual id present), producing a duplicate id that made review.md
    // permanently unparseable on every subsequent --human run.
    const input = ["y", "", "nit", "trivial", "resolved", "n", "tester"].join("\n") + "\n";
    const res = gate(repo, ["review", "--human"], input);
    expect(res.code).toBe(0);

    const content = readFileSync(reviewPath, "utf8");
    expect((content.match(/id: f2/g) ?? []).length).toBe(1);

    const check = gate(repo, ["check", "--json"]);
    expect(check.code).toBe(0);

    // Wedged in the old code: this second --human run would have thrown
    // "existing review.md is invalid" the moment it tried to parse the
    // duplicate-id file the first run just wrote.
    const again = gate(repo, ["review", "--human"], ["n", "tester"].join("\n") + "\n");
    expect(again.code).toBe(0);
    expect(again.stderr).not.toContain("invalid");
  });

  it("re-prompts when the reviewer explicitly types a finding id that's already used", () => {
    const repo = makeRepo();
    const id = setupReviewRun(repo);
    const reviewPath = join(repo, ".gate", "runs", id, "review.md");
    seedFindingF2(reviewPath);

    const input = ["y", "f2", "f9", "minor", "typo", "resolved", "n", "tester"].join("\n") + "\n";
    const res = gate(repo, ["review", "--human"], input);
    expect(res.code).toBe(0);
    expect(res.stdout).toContain('already used');

    const content = readFileSync(reviewPath, "utf8");
    expect(content).toContain("id: f9");
    expect((content.match(/id: f2/g) ?? []).length).toBe(1);
    expect(gate(repo, ["check", "--json"]).code).toBe(0);
  });
});
