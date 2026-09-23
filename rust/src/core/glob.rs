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

/// Cap on how many `**` segments a single pattern may contain (2026-09-22
/// audit, finding 8). `matches_from`'s `**` arm tries every split of the
/// remaining path for each `**` it sees; nested `**`s compound that into
/// backtracking that can blow up exponentially, and plan-declared `files:`
/// globs come from attacker-controllable `plan.md`. Real patterns need at
/// most a couple of `**`s - this leaves headroom without leaving the door
/// open.
const MAX_DOUBLE_STAR: usize = 4;

fn count_double_star(glob: &str) -> usize {
    glob.matches("**").count()
}

/// `matches_any` is the only function other modules call, so the `**` cap
/// lives here rather than as a `Result`-returning API change that would
/// ripple into every caller: a pattern beyond the cap is refused (treated
/// as matching nothing) and noted on stderr rather than evaluated.
pub fn matches_any(path: &str, globs: &[String]) -> bool {
    for g in globs {
        let norm = trim_trailing_slashes(g);
        if count_double_star(norm) > MAX_DOUBLE_STAR {
            eprintln!(
                "gate: glob pattern rejected (more than {MAX_DOUBLE_STAR} \"**\" wildcards): {norm}"
            );
            continue;
        }
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

    // ---- finding 8: bounding ** backtracking --------------------------

    #[test]
    fn allows_double_star_occurrences_at_the_cap() {
        let at_cap = "a/**/b/**/c/**/d/**/e"; // 4 "**"s
        assert!(matches_any("a/x/b/x/c/x/d/x/e", &globs(&[at_cap])));
    }

    #[test]
    fn rejects_a_pattern_with_more_double_stars_than_the_cap() {
        let too_many = "a/**/b/**/c/**/d/**/e/**/f"; // 5 "**"s
        assert!(!matches_any("a/x/b/x/c/x/d/x/e/x/f", &globs(&[too_many])));
    }

    #[test]
    fn a_rejected_pattern_does_not_block_a_later_valid_one() {
        let too_many = "a/**/b/**/c/**/d/**/e/**/f"; // 5 "**"s
        assert!(matches_any("src/a.ts", &globs(&[too_many, "src/*.ts"])));
    }

    #[test]
    fn matches_from_finishes_on_a_pathologically_nested_double_star_pattern() {
        // Timing-insensitive (finding 8): this only asserts the call
        // returns. Without the cap, `matches_from`'s `**` backtracking on
        // a path with no trailing match is exponential in the number of
        // `**` segments.
        let pattern = "**/**/**/**/**/**/**/**/**/**/**/**/**/**/**/**/x";
        assert!(!matches_any(
            "a/b/c/d/e/f/g/h/i/j/k/l/m/n/o",
            &globs(&[pattern])
        ));
    }
}
