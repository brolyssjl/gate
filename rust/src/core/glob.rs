//! Port of `src/core/glob.ts`: tiny glob matcher for repo-relative POSIX
//! paths. Supports `*` (any run except `/`), `**` (any run including `/`),
//! and `?` (a single character except `/`). A bare directory or file path
//! also matches everything beneath it, so `src/foo` matches `src/foo/bar.ts`
//! and `src/foo.ts` matches itself.
//!
//! TS builds a `RegExp` and reuses JS's engine; std Rust has no regex crate
//! (std-only policy), so this is a small hand-rolled glob matcher producing
//! the same accept/reject decisions instead of an intermediate `Regex`
//! object. `matches_any` is the only function other modules call.

/// Match `path` against a single already-anchored glob pattern.
fn glob_matches(glob: &str, path: &str) -> bool {
    let g: Vec<char> = glob.chars().collect();
    let p: Vec<char> = path.chars().collect();
    matches_from(&g, 0, &p, 0)
}

/// Recursive matcher: `gi`/`pi` are current positions into the glob/path
/// character vectors. `**` needs backtracking (it may consume zero or more
/// path characters including `/`), so this isn't a simple linear scan.
fn matches_from(g: &[char], gi: usize, p: &[char], pi: usize) -> bool {
    let mut gi = gi;
    let mut pi = pi;
    loop {
        if gi == g.len() {
            return pi == p.len();
        }
        match g[gi] {
            '*' if g.get(gi + 1) == Some(&'*') => {
                // `**` - consume the following `/` in the glob if present
                // (it's absorbed into the `**` span), then try every
                // possible split of the remaining path.
                let mut next_gi = gi + 2;
                if g.get(next_gi) == Some(&'/') {
                    next_gi += 1;
                }
                for split in pi..=p.len() {
                    if matches_from(g, next_gi, p, split) {
                        return true;
                    }
                }
                return false;
            }
            '*' => {
                let next_gi = gi + 1;
                for split in pi..=p.len() {
                    if p[pi..split].contains(&'/') {
                        break;
                    }
                    if matches_from(g, next_gi, p, split) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if pi >= p.len() || p[pi] == '/' {
                    return false;
                }
                gi += 1;
                pi += 1;
            }
            c => {
                if pi >= p.len() || p[pi] != c {
                    return false;
                }
                gi += 1;
                pi += 1;
            }
        }
    }
}

/// Trim trailing slashes, mirroring `glob.replace(/\/+$/, "")`.
fn trim_trailing_slashes(s: &str) -> &str {
    s.trim_end_matches('/')
}

pub fn matches_any(path: &str, globs: &[String]) -> bool {
    for g in globs {
        let norm = trim_trailing_slashes(g);
        if glob_matches(norm, path) {
            return true;
        }
        // Directory prefix: `src/foo` covers `src/foo/**`.
        if !norm.contains('*') && (path == norm || path.starts_with(&format!("{norm}/"))) {
            return true;
        }
        if glob_matches(&format!("{norm}/**"), path) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn star_matches_within_a_single_path_segment() {
        assert!(matches_any("src/a.ts", &globs(&["src/*.ts"])));
        assert!(!matches_any("src/sub/a.ts", &globs(&["src/*.ts"])));
    }

    #[test]
    fn double_star_matches_across_directories() {
        assert!(matches_any("src/a/b/c.ts", &globs(&["src/**"])));
        assert!(matches_any("src/a/b/c.ts", &globs(&["src/**/*.ts"])));
        assert!(matches_any("src/c.ts", &globs(&["src/**/*.ts"])));
    }

    #[test]
    fn question_mark_matches_a_single_non_slash_character() {
        assert!(matches_any("a.ts", &globs(&["?.ts"])));
        assert!(!matches_any("ab.ts", &globs(&["?.ts"])));
        assert!(!matches_any("a/ts", &globs(&["?.ts"])));
    }

    #[test]
    fn a_bare_directory_glob_covers_everything_beneath_it() {
        assert!(matches_any("src/foo/bar.ts", &globs(&["src/foo"])));
        assert!(matches_any("src/foo", &globs(&["src/foo"])));
        assert!(!matches_any("src/foobar.ts", &globs(&["src/foo"])));
    }

    #[test]
    fn a_bare_file_glob_matches_itself_exactly() {
        assert!(matches_any("src/foo.ts", &globs(&["src/foo.ts"])));
        assert!(!matches_any("src/foo.ts.bak", &globs(&["src/foo.ts"])));
    }

    #[test]
    fn trailing_slashes_in_the_glob_are_ignored() {
        assert!(matches_any("src/foo/bar.ts", &globs(&["src/foo/"])));
    }

    #[test]
    fn matches_the_first_glob_that_applies_out_of_several() {
        assert!(matches_any(
            "apps/api/x.py",
            &globs(&["apps/web/**", "apps/api/**"])
        ));
        assert!(!matches_any(
            "README.md",
            &globs(&["apps/web/**", "apps/api/**"])
        ));
    }

    #[test]
    fn no_globs_never_matches() {
        assert!(!matches_any("anything", &globs(&[])));
    }
}
