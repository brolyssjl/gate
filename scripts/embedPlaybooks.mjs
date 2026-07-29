#!/usr/bin/env node
/**
 * Generates src/core/embeddedPlaybooks.ts from the bundled playbooks/*.md
 * files plus the current package.json version. Run as part of `npm run
 * build` so a single-file binary (Node SEA / `bun build --compile`) has the
 * playbooks compiled in, with no dependency on a `playbooks/` directory
 * sitting next to the executable. `core/playbooks.ts` still tries the real
 * on-disk directory first (the disk-walk fallback stays authoritative for
 * normal npm-install and dev-from-source use - a playbook edit is live
 * immediately, no rebuild needed) and only falls back to this embedded copy
 * when the directory can't be found.
 *
 * The generated file is committed (like other generated-but-source-tracked
 * files in this repo) so a clean checkout typechecks without running this
 * script first; re-run it whenever playbooks/*.md or the version changes.
 */
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const playbooksDir = join(root, "playbooks");
const outPath = join(root, "src", "core", "embeddedPlaybooks.ts");

const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

const files = readdirSync(playbooksDir)
  .filter((f) => f.endsWith(".md"))
  .sort();

const entries = files.map((f) => {
  const phase = f.replace(/\.md$/, "");
  const content = readFileSync(join(playbooksDir, f), "utf8");
  return `  ${JSON.stringify(phase)}: ${JSON.stringify(content)},`;
});

const out = `// GENERATED FILE - do not hand-edit. Regenerate with:
//   node scripts/embedPlaybooks.mjs
// Source: playbooks/*.md + package.json version, at build time.

/** Playbook content by lowercase phase name, compiled in for single-file binaries. */
export const EMBEDDED_PLAYBOOKS: Record<string, string> = {
${entries.join("\n")}
};

/** The package version this embed was generated from (for \`gate --version\` in a binary build). */
export const EMBEDDED_VERSION = ${JSON.stringify(pkg.version)};
`;

writeFileSync(outPath, out);
process.stdout.write(`Wrote ${files.length} playbook(s) to ${outPath}\n`);
