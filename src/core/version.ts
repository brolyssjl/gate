import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { EMBEDDED_VERSION } from "./embeddedPlaybooks.js";

/**
 * The CLI version, read from the package's own package.json at runtime so it
 * never drifts from the published version. Walks up from this module to the
 * nearest package.json (the package root in both src and dist layouts).
 * Falls back to the version embedded at build time when no package.json is
 * reachable — a single-file binary ships with no sibling package.json.
 */
export function readVersion(): string {
  let dir = dirname(fileURLToPath(import.meta.url));
  for (;;) {
    const pkg = join(dir, "package.json");
    if (existsSync(pkg)) {
      try {
        return (JSON.parse(readFileSync(pkg, "utf8")) as { version?: string }).version ?? EMBEDDED_VERSION;
      } catch {
        return EMBEDDED_VERSION;
      }
    }
    const parent = dirname(dir);
    if (parent === dir) return EMBEDDED_VERSION;
    dir = parent;
  }
}
