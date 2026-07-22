/**
 * Tiny glob matcher for repo-relative POSIX paths. Supports:
 *   `*`  — any run of characters except `/`
 *   `**` — any run of characters including `/` (spanning directories)
 *   `?`  — a single character except `/`
 * A bare directory or file path also matches everything beneath it, so
 * `src/foo` matches `src/foo/bar.ts` and `src/foo.ts` matches itself.
 */
export function globToRegExp(glob: string): RegExp {
  let re = "";
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i]!;
    if (c === "*") {
      if (glob[i + 1] === "*") {
        re += ".*";
        i++;
        if (glob[i + 1] === "/") i++; // consume the slash after **
      } else {
        re += "[^/]*";
      }
    } else if (c === "?") {
      re += "[^/]";
    } else if ("\\^$.|+()[]{}".includes(c)) {
      re += "\\" + c;
    } else {
      re += c;
    }
  }
  return new RegExp("^" + re + "$");
}

export function matchesAny(path: string, globs: string[]): boolean {
  for (const g of globs) {
    const norm = g.replace(/\/+$/, "");
    if (globToRegExp(norm).test(path)) return true;
    // Directory prefix: `src/foo` covers `src/foo/**`.
    if (!norm.includes("*") && (path === norm || path.startsWith(norm + "/"))) return true;
    if (globToRegExp(norm + "/**").test(path)) return true;
  }
  return false;
}
