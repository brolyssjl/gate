import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { describe, expect, it } from "vitest";
import { makeRepo, writeFile } from "./helpers.js";

const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const CLI = join(pkgRoot, "dist", "cli.js");

function gate(cwd: string, args: string[], env?: NodeJS.ProcessEnv): { code: number; stdout: string; stderr: string; json: () => unknown } {
  const res = spawnSync("node", [CLI, ...args], { cwd, encoding: "utf8", env: env ?? process.env });
  return { code: res.status ?? 1, stdout: res.stdout, stderr: res.stderr, json: () => JSON.parse(res.stdout) };
}

function git(cwd: string, args: string[], env?: NodeJS.ProcessEnv): { code: number; stdout: string; stderr: string } {
  const res = spawnSync("git", args, { cwd, encoding: "utf8", env: env ?? process.env });
  return { code: res.status ?? 1, stdout: res.stdout, stderr: res.stderr };
}

/** A `gate` shim on PATH so the installed hook (which looks up `gate` via PATH) resolves in tests. */
function gateShimPath(): string {
  const binDir = mkdtempSync(join(tmpdir(), "gate-bin-"));
  const shim = join(binDir, "gate");
  writeFileSync(shim, `#!/bin/sh\nexec node "${CLI}" "$@"\n`);
  chmodSync(shim, 0o755);
  return binDir;
}

function withGateOnPath(): NodeJS.ProcessEnv {
  return { ...process.env, PATH: `${gateShimPath()}:${process.env.PATH ?? ""}` };
}

function startActiveRun(repo: string, files: string[]): string {
  gate(repo, ["init"]);
  gate(repo, ["trust"]);
  const started = gate(repo, ["start", "guarded work", "--profile", "docs", "--json"]).json() as {
    id: string;
    plan: string;
  };
  const plan = [
    "---",
    "goal: guarded work",
    "files:",
    ...files.map((f) => `  - ${f}`),
    "criteria:",
    "  - id: c1",
    "    text: it works",
    '    verify: "manual"',
    "---",
    "",
  ].join("\n");
  writeFileSync(started.plan, plan);
  return started.id;
}

describe("gate guard", () => {
  it("install writes an executable pre-commit hook carrying the gate-guard marker", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    const res = gate(repo, ["guard", "install", "--json"]);
    expect(res.code).toBe(0);
    expect((res.json() as { installed: boolean }).installed).toBe(true);

    const hookPath = join(repo, ".git", "hooks", "pre-commit");
    expect(existsSync(hookPath)).toBe(true);
    expect(readFileSync(hookPath, "utf8")).toContain("gate-guard: v1");
    expect(statSync(hookPath).mode & 0o111).not.toBe(0); // executable
  });

  it("install is idempotent: a second install reports already-installed instead of re-wrapping", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    gate(repo, ["guard", "install"]);
    const again = gate(repo, ["guard", "install", "--json"]).json() as { alreadyInstalled: boolean };
    expect(again.alreadyInstalled).toBe(true);
  });

  it("install backs up and chains a pre-existing foreign hook", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    const hookPath = join(repo, ".git", "hooks", "pre-commit");
    mkdirSync(dirname(hookPath), { recursive: true });
    writeFileSync(hookPath, "#!/bin/sh\necho original-hook-ran\nexit 0\n");
    chmodSync(hookPath, 0o755);

    const res = gate(repo, ["guard", "install", "--json"]).json() as { chained: boolean };
    expect(res.chained).toBe(true);

    const backupPath = join(repo, ".git", "hooks", "pre-commit.gate-backup");
    expect(existsSync(backupPath)).toBe(true);
    expect(readFileSync(backupPath, "utf8")).toContain("original-hook-ran");
    expect(readFileSync(hookPath, "utf8")).toContain("pre-commit.gate-backup");
  });

  it("uninstall restores a backed-up foreign hook", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    const hookPath = join(repo, ".git", "hooks", "pre-commit");
    mkdirSync(dirname(hookPath), { recursive: true });
    writeFileSync(hookPath, "#!/bin/sh\necho original-hook-ran\nexit 0\n");
    gate(repo, ["guard", "install"]);

    const res = gate(repo, ["guard", "uninstall", "--json"]).json() as { restored: boolean };
    expect(res.restored).toBe(true);
    expect(readFileSync(hookPath, "utf8")).toContain("original-hook-ran");
    expect(existsSync(join(repo, ".git", "hooks", "pre-commit.gate-backup"))).toBe(false);
  });

  it("uninstall removes a hook it installed with nothing to restore", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    gate(repo, ["guard", "install"]);
    const hookPath = join(repo, ".git", "hooks", "pre-commit");
    expect(existsSync(hookPath)).toBe(true);

    gate(repo, ["guard", "uninstall"]);
    expect(existsSync(hookPath)).toBe(false);
  });

  it("uninstall refuses to touch a pre-commit hook gate didn't install", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    const hookPath = join(repo, ".git", "hooks", "pre-commit");
    mkdirSync(dirname(hookPath), { recursive: true });
    writeFileSync(hookPath, "#!/bin/sh\necho someone-elses-hook\nexit 0\n");

    const res = gate(repo, ["guard", "uninstall"]);
    expect(res.code).not.toBe(0);
    expect(res.stderr).toContain("wasn't installed by");
    expect(readFileSync(hookPath, "utf8")).toContain("someone-elses-hook");
  });

  it("gate guard run blocks with no active run on the branch", () => {
    const repo = makeRepo();
    gate(repo, ["init"]);
    const res = gate(repo, ["guard", "run", "--json"]);
    expect(res.code).toBe(1);
    const data = res.json() as { ok: boolean; reasons: string[] };
    expect(data.ok).toBe(false);
    expect(data.reasons.join(" ")).toContain("no active run");
  });

  it("gate guard run blocks while still in PLAN", () => {
    const repo = makeRepo();
    startActiveRun(repo, ["a.txt"]);
    const res = gate(repo, ["guard", "run", "--json"]);
    expect(res.code).toBe(1);
    expect((res.json() as { reasons: string[] }).reasons.join(" ")).toContain("still in PLAN");
  });

  it("gate guard run blocks staged files outside the declared plan scope, passes for in-scope files", () => {
    const repo = makeRepo();
    startActiveRun(repo, ["a.txt"]);
    gate(repo, ["approve"]);
    gate(repo, ["next"]); // -> IMPLEMENT

    writeFile(repo, "out-of-scope.txt", "nope\n");
    git(repo, ["add", "out-of-scope.txt"]);
    const blocked = gate(repo, ["guard", "run", "--json"]);
    expect(blocked.code).toBe(1);
    expect((blocked.json() as { reasons: string[] }).reasons.join(" ")).toContain("out-of-scope.txt");

    git(repo, ["reset"]);
    writeFile(repo, "a.txt", "in scope\n");
    git(repo, ["add", "a.txt"]);
    const passed = gate(repo, ["guard", "run", "--json"]);
    expect(passed.code).toBe(0);
    expect((passed.json() as { ok: boolean }).ok).toBe(true);
  });

  it("end-to-end: an installed hook blocks `git commit` for out-of-scope staged files, and --no-verify / GATE_GUARD=0 bypass it", () => {
    const repo = makeRepo();
    startActiveRun(repo, ["a.txt"]);
    gate(repo, ["approve"]);
    gate(repo, ["next"]); // -> IMPLEMENT
    gate(repo, ["guard", "install"]);

    const envWithGate = withGateOnPath();
    writeFile(repo, "b.txt", "not declared\n");
    git(repo, ["add", "b.txt"]);

    const blocked = git(repo, ["commit", "-m", "should be blocked"], envWithGate);
    expect(blocked.code).not.toBe(0);

    const bypassed = git(repo, ["commit", "--no-verify", "-m", "bypassed"], envWithGate);
    expect(bypassed.code).toBe(0);

    writeFile(repo, "c.txt", "also not declared\n");
    git(repo, ["add", "c.txt"]);
    const envDisabled = { ...envWithGate, GATE_GUARD: "0" };
    const disabled = git(repo, ["commit", "-m", "guard disabled"], envDisabled);
    expect(disabled.code).toBe(0);
  });
});
