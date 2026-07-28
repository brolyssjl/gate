#!/usr/bin/env node
/**
 * Build a single-file `gate` executable for CI / Node-less machines
 * (proposal §10, distribution tier 2). Mechanical prep only - not run as
 * part of `npm run build` or CI by default; the owner wires this into a
 * release job when ready to ship binaries.
 *
 * Two paths, in order of preference:
 *
 *   1. `bun build --compile` - zero extra dependencies (bun is a separate
 *      runtime, not an npm package), and Gate has no runtime deps besides
 *      `yaml`, so bun bundles everything. Used automatically when `bun` is
 *      on PATH.
 *   2. Node's built-in Single Executable Applications (SEA) - works with
 *      plain Node, but needs `postject` to inject the bundle into a copied
 *      node binary. Not added as a project dependency (mechanical-prep
 *      scope, no new deps); install it ad hoc (`npm install --no-save
 *      postject`) before using this path.
 *
 * Either way, `npm run build` must have already produced `dist/cli.js` with
 * playbooks embedded (`scripts/embedPlaybooks.mjs` runs as part of that).
 */
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const entry = join(root, "dist", "cli.js");
const outDir = join(root, "dist-bin");
const outFile = join(outDir, process.platform === "win32" ? "gate.exe" : "gate");

function hasBun() {
  const res = spawnSync("bun", ["--version"], { stdio: "ignore" });
  return res.status === 0;
}

function buildWithBun() {
  mkdirSync(outDir, { recursive: true });
  execFileSync("bun", ["build", "--compile", entry, "--outfile", outFile], { stdio: "inherit", cwd: root });
  process.stdout.write(`Built ${outFile} with bun build --compile\n`);
}

function buildWithNodeSea() {
  const seaConfigPath = join(root, "sea-config.json");
  const seaBlobPath = join(root, "dist", "sea-prep.blob");
  writeFileSync(
    seaConfigPath,
    JSON.stringify({ main: entry, output: seaBlobPath, disableExperimentalSEAWarning: true }, null, 2),
  );
  mkdirSync(outDir, { recursive: true });

  process.stdout.write("bun not found - falling back to Node SEA (requires `postject`).\n");
  execFileSync(process.execPath, ["--experimental-sea-config", seaConfigPath], { stdio: "inherit", cwd: root });

  const nodeBin = existsSync(outFile) ? outFile : process.execPath;
  execFileSync("node", ["-e", `require("fs").copyFileSync(${JSON.stringify(process.execPath)}, ${JSON.stringify(outFile)})`], {
    stdio: "inherit",
  });

  const postjectArgs = [
    "-y",
    "postject",
    outFile,
    "NODE_SEA_BLOB",
    seaBlobPath,
    "--sentinel-fuse",
    "NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2",
  ];
  if (process.platform === "darwin") postjectArgs.push("--macho-segment-name", "NODE_SEA");
  const res = spawnSync("npx", postjectArgs, { stdio: "inherit", cwd: root });
  if (res.status !== 0) {
    process.stderr.write(
      "postject failed - install it first: npm install --no-save postject\n" +
        "(kept as an ad-hoc dependency, not a project devDependency, per the minimal-deps rule)\n",
    );
    process.exitCode = 1;
    return;
  }
  process.stdout.write(`Built ${nodeBin === outFile ? outFile : "gate"} with Node SEA\n`);
}

if (!existsSync(entry)) {
  process.stderr.write(`${entry} not found - run \`npm run build\` first.\n`);
  process.exit(1);
}

if (hasBun()) {
  buildWithBun();
} else {
  buildWithNodeSea();
}
