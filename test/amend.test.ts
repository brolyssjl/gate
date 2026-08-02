import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { gate, makeRepo } from "./helpers.js";

function planWithFiles(files: string[]): string {
  return [
    "---",
    "goal: exercise drift",
    "files:",
    ...files.map((f) => `  - ${f}`),
    "criteria:",
    "  - id: c1",
    "    text: it works",
    '    verify: "manual"',
    "---",
    "",
  ].join("\n");
}

/** Start a run, write and approve a plan declaring `files`. Returns the run id and plan path. */
function setupApprovedRun(repo: string, files: string[]): { runId: string; planPath: string } {
  gate(repo, ["init"]);
  const started = gate(repo, ["start", "drift demo", "--json"]).json() as { id: string; plan: string };
  writeFileSync(started.plan, planWithFiles(files));
  expect(gate(repo, ["approve", "--by", "human"]).code).toBe(0);
  return { runId: started.id, planPath: started.plan };
}

describe("approved-plan drift + gate amend", () => {
  it("post-PLAN scope widening fails closed until amended (previously accepted with no re-approval)", () => {
    const repo = makeRepo();
    const { planPath } = setupApprovedRun(repo, ["a.txt"]);
    expect(gate(repo, ["next"]).code).toBe(0); // PLAN -> IMPLEMENT

    writeFileSync(planPath, planWithFiles(["a.txt", "b.txt"])); // scope widened, no re-approval

    const check = gate(repo, ["check", "--json"]);
    expect(check.code).toBe(1);
    const data = check.json() as { checks: Array<{ name: string; ok: boolean }> };
    expect(data.checks.some((c) => c.name === "implement.plan-drift" && !c.ok)).toBe(true);
  });

  it("gate amend refuses without a prior approval, and refuses when nothing has drifted", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    const started = gate(repo, ["start", "no approval yet", "--json"]).json() as { plan: string };
    writeFileSync(started.plan, planWithFiles(["a.txt"]));

    const beforeApproval = gate(repo, ["amend"]);
    expect(beforeApproval.code).not.toBe(0);
    expect(beforeApproval.stderr).toContain("never approved");

    expect(gate(repo, ["approve"]).code).toBe(0);
    const noDrift = gate(repo, ["amend"]);
    expect(noDrift.code).not.toBe(0);
    expect(noDrift.stderr).toContain("nothing to amend");
  });

  it("gate approve --amend refuses without gate amend having recorded intent first", () => {
    const repo = makeRepo();
    const { planPath } = setupApprovedRun(repo, ["a.txt"]);
    writeFileSync(planPath, planWithFiles(["a.txt", "b.txt"]));

    const res = gate(repo, ["approve", "--amend"]);
    expect(res.code).not.toBe(0);
    expect(res.stderr).toContain("gate amend");
  });

  it("amend + approve --amend goes green: shows the diff, re-approves, and the drift check clears", () => {
    const repo = makeRepo();
    const { runId, planPath } = setupApprovedRun(repo, ["a.txt"]);
    expect(gate(repo, ["next"]).code).toBe(0); // PLAN -> IMPLEMENT

    writeFileSync(planPath, planWithFiles(["a.txt", "b.txt"]));
    expect(gate(repo, ["check"]).code).toBe(1);

    const amend = gate(repo, ["amend", "--json"]);
    expect(amend.code).toBe(0);
    const amendData = amend.json() as { amended: boolean; diff: string };
    expect(amendData.amended).toBe(true);
    expect(amendData.diff).toContain("b.txt");

    const approveAmend = gate(repo, ["approve", "--amend", "--by", "human"]);
    expect(approveAmend.code).toBe(0);

    const afterAmend = gate(repo, ["check", "--json"]).json() as { checks: Array<{ name: string }> };
    expect(afterAmend.checks.some((c) => c.name === "implement.plan-drift")).toBe(false);

    // Continue the flow with the newly widened scope - it goes all the way green.
    writeFileSync(join(repo, "a.txt"), "hi\n");
    writeFileSync(join(repo, "b.txt"), "hi\n");
    expect(gate(repo, ["next"]).code).toBe(0); // IMPLEMENT -> TEST

    const run = JSON.parse(readFileSync(join(repo, ".gate", "runs", runId, "run.json"), "utf8")) as {
      amendment?: unknown;
      approval: { planHash: string };
    };
    expect(run.amendment).toBeUndefined();
  });

  it("editing the plan again after `gate amend` invalidates that amendment - approve --amend refuses", () => {
    const repo = makeRepo();
    const { planPath } = setupApprovedRun(repo, ["a.txt"]);
    expect(gate(repo, ["next"]).code).toBe(0);

    writeFileSync(planPath, planWithFiles(["a.txt", "b.txt"]));
    expect(gate(repo, ["amend"]).code).toBe(0);

    writeFileSync(planPath, planWithFiles(["a.txt", "b.txt", "c.txt"]));
    const res = gate(repo, ["approve", "--amend"]);
    expect(res.code).not.toBe(0);
    expect(res.stderr).toContain("gate amend");
  });
});
