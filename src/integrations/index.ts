import { existsSync } from "node:fs";
import { join } from "node:path";

/**
 * Integrations are **advisory, never load-bearing** (kickoff principle 5):
 * detection only changes hint lines and config, and no gate check depends on an
 * external tool being installed. Detection is pure filesystem presence, so
 * `init --refresh` re-detects idempotently.
 */
export interface Detection {
  agnosgram: boolean;
  sdd: false | "openspec" | "spec-kit" | "bmad";
}

export function detect(root: string): Detection {
  return {
    agnosgram: existsSync(join(root, ".agnosgram")),
    sdd: detectSdd(root),
  };
}

/** Directory name for each SDD framework `detectSdd` recognizes, in detection order. */
const SDD_DIRS: Array<[Detection["sdd"] & string, string]> = [
  ["openspec", "openspec"],
  ["spec-kit", ".specify"],
  ["bmad", "_bmad"],
  ["bmad", ".bmad"],
];

function detectSdd(root: string): Detection["sdd"] {
  for (const [kind, dir] of SDD_DIRS) {
    if (existsSync(join(root, dir))) return kind;
  }
  return false;
}

/**
 * The repo-relative SDD directory Gate detected, or null if none. Used by the
 * PLAN gate to check a cited `spec:` path lives under it. Distinct from
 * `detect().sdd` (which names the *framework*) because `bmad` has two possible
 * directory names; this resolves to the one actually present.
 */
export function sddDir(root: string): string | null {
  for (const [, dir] of SDD_DIRS) {
    if (existsSync(join(root, dir))) return dir;
  }
  return null;
}

/** Hint lines appended to the PLAN playbook output when integrations are present. */
export function planHints(det: Detection): string[] {
  const hints: string[] = [];
  if (det.agnosgram) {
    hints.push("Agnosgram detected: read `.agnosgram/lessons/pitfalls.md` before planning.");
  }
  if (det.sdd) {
    hints.push(`SDD (${det.sdd}) detected: cite the spec path in plan.md instead of restating it.`);
  }
  return hints;
}
