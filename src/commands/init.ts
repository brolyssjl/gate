import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { gatePaths } from "../core/paths.js";
import { bundledPlaybooksDir } from "../core/playbooks.js";
import { EMBEDDED_PLAYBOOKS } from "../core/embeddedPlaybooks.js";
import { detect } from "../integrations/index.js";
import { inferCommands } from "./inferStack.js";
import { emit, type ParsedArgs } from "./shared.js";

/**
 * `gate init` — scaffold `.gate/`, infer build/test/lint commands from the
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
    "",
    "Review .gate/config.yml, then run `gate trust` to approve its commands.",
    "Next: gate start \"<title>\"",
  ].join("\n");

  emit(human, { root, initialized: true, refreshed: refresh, detected }, args.flags);
}

function readBundledPlaybookFiles(): Array<[string, string]> {
  try {
    const bundled = bundledPlaybooksDir();
    return readdirSync(bundled)
      .filter((f) => f.endsWith(".md"))
      .map((f) => [f, readFileSync(join(bundled, f), "utf8")] as [string, string]);
  } catch {
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
    "",
  );
  return lines.join("\n");
}
