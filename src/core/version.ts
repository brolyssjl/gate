import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * The CLI version, read from the package's own package.json at runtime so it
 * never drifts from the published version. Walks up from this module to the
 * nearest package.json (the package root in both src and dist layouts).
 */
export function readVersion(): string {
  let dir = dirname(fileURLToPath(import.meta.url));
  for (;;) {
    const pkg = join(dir, "package.json");
    if (existsSync(pkg)) {
      try {
        return (JSON.parse(readFileSync(pkg, "utf8")) as { version?: string }).version ?? "0.0.0";
      } catch {
        return "0.0.0";
      }
    }
    const parent = dirname(dir);
    if (parent === dir) return "0.0.0";
    dir = parent;
  }
}
