import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { gatePaths } from "../core/paths.js";
import { bundledPlaybooksDir } from "../core/playbooks.js";
import { EMBEDDED_PLAYBOOKS } from "../core/embeddedPlaybooks.js";
import { detect } from "../integrations/index.js";
import { inferCommands, inferScopeIgnore } from "./inferStack.js";
import { emit, type ParsedArgs } from "./shared.js";

/**
 * `gate init` - scaffold `.gate/`, infer build/test/lint commands from the
 * stack, detect advisory integrations, and copy default playbooks. Idempotent:
 * `--refresh` re-detects integrations and rewrites the managed hint block
 * without clobbering user edits to config or playbooks.
 */
export function cmdInit(args: ParsedArgs): void {
  const root = process.cwd();
  const paths = gatePaths(root);
  const refresh = args.flags.refresh === true;
  const existed = existsSync(paths.gate);

  mkdirSync(paths.playbooks, { recursive: true });
  mkdirSync(paths.runs, { recursive: true });

  const gitignoreUpdated = ensureGitignore(root);

  // Copy default playbooks that the user hasn't already customized. Prefer
  // the real directory (dev-from-source, npm install); a single-file binary
  // has none, so fall back to the copy compiled in at build time.
  const bundledFiles = readBundledPlaybookFiles();
  for (const [file, content] of bundledFiles) {
    const dest = join(paths.playbooks, file);
    if (!existsSync(dest)) writeFileSync(dest, content);
  }

  const det = detect(root);

  if (!existsSync(paths.config) || refresh) {
    if (!existsSync(paths.config)) writeFileSync(paths.config, buildConfig(root, det));
    // On refresh we intentionally do not rewrite an existing config to avoid
    // clobbering hand edits; integration detection is advisory and re-read live.
  }

  const detected = [
    det.agnosgram ? "agnosgram" : null,
    det.sdd ? `sdd:${det.sdd}` : null,
  ].filter(Boolean);

  const human = [
    existed && !refresh ? ".gate/ already initialized (playbooks topped up)." : "Initialized .gate/",
    `  config:    ${paths.config}`,
    `  playbooks: ${paths.playbooks}`,
    `  runs:      ${paths.runs}`,
    detected.length ? `  detected:  ${detected.join(", ")}` : "  detected:  none",
    gitignoreUpdated ? "  gitignore: added .gate/ run-state entries (config.yml and trust.json stay tracked)" : null,
    "",
    "Review .gate/config.yml, then run `gate trust` to approve its commands.",
    "Next: gate start \"<title>\"",
  ]
    .filter((line): line is string => line !== null)
    .join("\n");

  emit(human, { root, initialized: true, refreshed: refresh, detected, gitignoreUpdated }, args.flags);
}

/**
 * `.gate/runs/`, the legacy single-run pointer, `current.json`, and
 * `archive/` are ephemeral run state - never `config.yml` or `trust.json`,
 * which stay tracked so CI inherits the command pin (see README). Idempotent
 * and non-destructive: only appends entries genuinely missing from an
 * existing `.gitignore`, on both a fresh `init` and every `--refresh`, so
 * upgrading an older `.gate/` project actually gets the entries the docs
 * have always claimed instead of leaving it prose-only.
 */
const GITIGNORE_MARKER = "# Gate's own ephemeral run folders (config + playbooks stay tracked)";
const GITIGNORE_ENTRIES = [".gate/runs/", ".gate/current", ".gate/current.json", ".gate/archive/"];

function ensureGitignore(root: string): boolean {
  const path = join(root, ".gitignore");
  const existing = existsSync(path) ? readFileSync(path, "utf8") : "";
  const lines = new Set(existing.split("\n").map((l) => l.trim()));
  const missing = GITIGNORE_ENTRIES.filter((e) => !lines.has(e));
  if (missing.length === 0) return false;

  const prefix = existing.length === 0 || existing.endsWith("\n") ? existing : existing + "\n";
  const block = (prefix.length > 0 ? "\n" : "") + [GITIGNORE_MARKER, ...missing].join("\n") + "\n";
  writeFileSync(path, prefix + block);
  return true;
}

/**
 * Prefer the real `playbooks/` directory on disk; fall back to the copy
 * compiled into the binary at build time (`scripts/embedPlaybooks.mjs`).
 * `bundledPlaybooksDir()` throws plain `Error("bundled playbooks directory
 * not found")` for the *expected* case - a single-file binary with no
 * sibling `playbooks/` directory to walk to - and that's the only failure
 * this should swallow silently. Any other failure (e.g. the directory exists
 * but a file in it can't be read - permissions, a symlink loop) is not "no
 * directory", it's something actually wrong; init still completes on the
 * embedded fallback (playbooks aren't load-bearing for `.gate/` to exist),
 * but an umpire that quietly wallpapers over an unexpected error is not
 * trustworthy - so it's surfaced on stderr instead of going unmentioned.
 */
function readBundledPlaybookFiles(): Array<[string, string]> {
  try {
    const bundled = bundledPlaybooksDir();
    return readdirSync(bundled)
      .filter((f) => f.endsWith(".md"))
      .map((f) => [f, readFileSync(join(bundled, f), "utf8")] as [string, string]);
  } catch (e) {
    const expected = e instanceof Error && e.message === "bundled playbooks directory not found";
    if (!expected) {
      process.stderr.write(
        `gate: warning: could not read bundled playbooks (${(e as Error).message}) - using the built-in copy\n`,
      );
    }
    return Object.entries(EMBEDDED_PLAYBOOKS).map(([phase, content]) => [`${phase}.md`, content]);
  }
}

function buildConfig(root: string, det: ReturnType<typeof detect>): string {
  const cmds = inferCommands(root);
  const lines = ["# Gate configuration. Commands are the same trust class as npm scripts.", "commands:"];
  for (const key of ["build", "test", "lint", "coverage"] as const) {
    if (cmds[key]) lines.push(`  ${key}: ${JSON.stringify(cmds[key])}`);
    else lines.push(`  # ${key}: "<command>"`);
  }
  lines.push(
    "thresholds:",
    "  # diff_coverage: 80   # uncomment once a coverage command is set",
    "phases:",
    "  plan: required",
    "  implement: required",
    "  test: required",
    "integrations:",
    `  agnosgram: ${det.agnosgram ? "auto" : "off"}`,
    `  sdd: ${det.sdd ? "auto" : "off"}`,
  );

  const scopeIgnore = inferScopeIgnore(root);
  lines.push("# Noise the scope check ignores rather than flags as undeclared - part of the trust hash.", "scope_ignore:");
  if (scopeIgnore.length === 0) lines.push("  # - <glob>");
  else for (const g of scopeIgnore) lines.push(`  - ${JSON.stringify(g)}`);
  lines.push("");

  return lines.join("\n");
}
