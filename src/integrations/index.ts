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

function detectSdd(root: string): Detection["sdd"] {
  if (existsSync(join(root, "openspec"))) return "openspec";
  if (existsSync(join(root, ".specify"))) return "spec-kit";
  if (existsSync(join(root, "_bmad")) || existsSync(join(root, ".bmad"))) return "bmad";
  return false;
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
