//! Port of `src/core/git.ts`: git plumbing gate builds its scope/diff/
//! staleness checks on. Every function spawns a real `git` (or, for
//! `diff_no_index`, works even outside a repo) via `std::process::Command`
//! with an argv array - never a shell (contrast `core/exec.rs`'s
//! `runCommand`, the one place gate goes through `sh -c`).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::gitignore_state::is_gitignore_unchanged_since_init;
use crate::core::sha256::sha256_prefixed;

struct GitResult {
    ok: bool,
    stdout: String,
}

fn git(root: &Path, args: &[&str]) -> GitResult {
    match Command::new("git").args(args).current_dir(root).output() {
        Ok(output) => GitResult {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        },
        Err(_) => GitResult {
            ok: false,
            stdout: String::new(),
        },
    }
}

/// True for paths Gate's own bookkeeping owns, never counted as "touched"
/// code by scope/coverage/evidence checks: `.gate/` run state always, and
/// the repo-root `.gitignore` ONLY when it's still byte-for-byte what `gate
/// init` last wrote. See `core/gitignore_state.rs`.
pub fn is_gate_bookkeeping(root: &Path, path: &str) -> bool {
    if path.starts_with(".gate/") {
        return true;
    }
    if path == ".gitignore" {
        return is_gitignore_unchanged_since_init(root);
    }
    false
}

/// Untracked (not-ignored) files, excluding Gate's own bookkeeping.
/// Repo-relative POSIX paths, as git reports them.
pub fn untracked_files(root: &Path) -> Vec<String> {
    git(root, &["ls-files", "--others", "--exclude-standard"])
        .stdout
        .split('\n')
        .map(|l| l.trim())
        .filter(|f| !f.is_empty() && !is_gate_bookkeeping(root, f))
        .map(String::from)
        .collect()
}

pub fn is_git_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"]).ok
}

/// Current branch name, or `None` when not a repo or on a detached HEAD.
/// `symbolic-ref` (not `rev-parse --abbrev-ref`) so this resolves correctly
/// on an *unborn* branch too.
pub fn current_branch(root: &Path) -> Option<String> {
    let res = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    if !res.ok {
        return None;
    }
    let branch = res.stdout.trim();
    if branch.is_empty() {
        None
    } else {
        Some(branch.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchState {
    Branch(String),
    Detached,
    None,
}

/// Tri-state read of the repo's branch identity, for branch-keyed run
/// concurrency (Milestone 4). A successful `symbolic-ref` alone proves both
/// "this is a git repo" and "on a branch"; `is_git_repo` only disambiguates
/// "not a repo" from "genuinely detached" on the rarer path where
/// `symbolic-ref` didn't resolve a branch.
pub fn branch_state(root: &Path) -> BranchState {
    let res = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let branch = res.stdout.trim();
    if res.ok && !branch.is_empty() {
        return BranchState::Branch(branch.to_string());
    }
    if is_git_repo(root) {
        BranchState::Detached
    } else {
        BranchState::None
    }
}

/// `git config user.name`, trimmed. `None` when git errors (no such key,
/// not a repo with no global fallback set, `git` missing) or the value is
/// empty - the weakest of the three identity sources `gate
/// trust`/`gate approve`/`gate streak reset` fall back through (see
/// `core::identity`), so an absent/blank name must read as "no identity"
/// rather than as an empty string sitting in `by`.
pub fn user_name(root: &Path) -> Option<String> {
    let res = git(root, &["config", "user.name"]);
    if !res.ok {
        return None;
    }
    let name = res.stdout.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// The repo's git hooks directory, respecting worktrees and a custom
/// `core.hooksPath` - not always a plain `.git/hooks`. `None` when not a
/// git repo.
pub fn git_hooks_dir(root: &Path) -> Option<PathBuf> {
    let res = git(root, &["rev-parse", "--git-path", "hooks"]);
    if !res.ok {
        return None;
    }
    let p = res.stdout.trim();
    if p.is_empty() {
        return None;
    }
    let path = Path::new(p);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    })
}

/// Staged (index) files, excluding Gate's own bookkeeping. `-z` (NUL-
/// separated, unquoted paths) rather than the default newline-separated
/// output: with `core.quotepath` (git's default), a non-ASCII filename
/// otherwise arrives C-quoted, which fails glob matching outright and even
/// defeats the bookkeeping exclusion below.
pub fn staged_files(root: &Path) -> Vec<String> {
    git(root, &["diff", "--cached", "--name-only", "-z", "--"])
        .stdout
        .split('\0')
        .filter(|f| !f.is_empty() && !is_gate_bookkeeping(root, f))
        .map(String::from)
        .collect()
}

/// Unified diff between two files on disk, independent of any git repo or
/// index. `a`/`b` are resolved relative to `dir` (the diff headers'
/// `a/`/`b/` prefixes come straight from whatever's passed here - an
/// absolute path makes git strip its leading `/` and use the remainder
/// verbatim, producing a header that *looks* like a normal relative path
/// but isn't, e.g. `a/private/tmp/x` reading as `a/tmp/x`; a short relative
/// path avoids that entirely). Exits non-zero when the files differ; the
/// diff is still on stdout.
pub fn diff_no_index(dir: &Path, a: &str, b: &str) -> String {
    Command::new("git")
        .args(["diff", "--no-color", "--no-index", "--", a, b])
        .current_dir(dir)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Current HEAD sha, or `None` if there are no commits yet / not a repo.
pub fn head_sha(root: &Path) -> Option<String> {
    let res = git(root, &["rev-parse", "HEAD"]);
    if res.ok {
        Some(res.stdout.trim().to_string())
    } else {
        None
    }
}

/// Files changed since `base_ref` (committed diff) plus uncommitted and
/// untracked changes in the working tree. Repo-relative POSIX paths,
/// deduped, in first-seen order (mirrors JS `Set` insertion order).
pub fn changed_files(root: &Path, base_ref: Option<&str>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut add = |stdout: &str| {
        for line in stdout.split('\n') {
            let f = line.trim();
            if !f.is_empty() && seen.insert(f.to_string()) {
                out.push(f.to_string());
            }
        }
    };
    if let Some(base) = base_ref {
        // `--end-of-options` (git >= 2.24) stops option parsing before
        // `base`, a stored `run.json` value - without it a value starting
        // with `-` (e.g. `--output=<path>`) is parsed as a git flag instead
        // of a revision (gate report finding 2). `run.rs`'s
        // `validate_base_ref` also rejects that charset outright; this is
        // defense in depth for every git invocation that takes a stored rev.
        add(&git(
            root,
            &["diff", "--name-only", "--end-of-options", base, "--"],
        )
        .stdout);
    }
    add(&git(root, &["diff", "--name-only", "--"]).stdout); // unstaged vs index
    add(&git(root, &["diff", "--name-only", "--cached", "--"]).stdout); // staged
    add(&git(root, &["ls-files", "--others", "--exclude-standard"]).stdout); // untracked
    out
}

/// Unified diff text since `base_ref` (committed + uncommitted +
/// untracked), for the review packet. `git diff` alone omits untracked
/// files, so each untracked file is appended as a synthesized new-file diff.
/// `.gate/` bookkeeping is excluded. `""` when not a repo or nothing to
/// show.
pub fn diff_text(root: &Path, base_ref: Option<&str>) -> String {
    if !is_git_repo(root) {
        return String::new();
    }
    let base = base_ref.unwrap_or("HEAD");
    // See `changed_files`'s comment on `--end-of-options` (gate report
    // finding 2): `base` is a stored `run.json` value here too.
    let mut parts = vec![
        git(
            root,
            &[
                "diff",
                "--no-color",
                "--end-of-options",
                base,
                "--",
                ".",
                ":(exclude).gate/**",
            ],
        )
        .stdout,
    ];
    for f in untracked_files(root) {
        // --no-index exits non-zero when the files differ; the diff is
        // still on stdout.
        parts.push(
            git(
                root,
                &["diff", "--no-color", "--no-index", "--", "/dev/null", &f],
            )
            .stdout,
        );
    }
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

/// A content fingerprint of the working tree: HEAD plus every uncommitted
/// and untracked change (excluding `.gate/`). `None` when not a git repo.
pub fn tree_fingerprint(root: &Path) -> Option<String> {
    if !is_git_repo(root) {
        return None;
    }
    let mut parts = vec![
        head_sha(root).unwrap_or_else(|| "(no-head)".to_string()),
        git(
            root,
            &[
                "diff",
                "--no-color",
                "HEAD",
                "--",
                ".",
                ":(exclude).gate/**",
            ],
        )
        .stdout,
    ];
    for f in untracked_files(root) {
        let h = git(root, &["hash-object", "--", &f]);
        let hash = if h.ok {
            h.stdout.trim().to_string()
        } else {
            "(unhashable)".to_string()
        };
        parts.push(format!("{f}\0{hash}"));
    }
    Some(sha256_prefixed(parts.join("\0").as_bytes()))
}

/// Parsed hunk header `@@ -a,b +c,d @@` -> (new-file start line, line
/// count). The count defaults to 1 when omitted (`@@ -0,0 +1 @@`).
fn parse_hunk_header(line: &str) -> Option<(i64, i64)> {
    let rest = line.strip_prefix("@@ -")?;
    let plus_idx = rest.find('+')?;
    let after_plus = &rest[plus_idx + 1..];
    let space_idx = after_plus.find(' ')?;
    let new_part = &after_plus[..space_idx];
    let (new_line_str, count_str) = match new_part.split_once(',') {
        Some((a, b)) => (a, Some(b)),
        None => (new_part, None),
    };
    let new_line: i64 = new_line_str.parse().ok()?;
    let count: i64 = match count_str {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    Some((new_line, count))
}

fn parse_unified_diff(diff: &str, out: &mut Vec<(String, HashSet<i64>)>) {
    let mut file: Option<String> = None;
    let mut new_line: i64 = 0;
    let mut remaining: i64 = 0;
    let find_mut = |out: &mut Vec<(String, HashSet<i64>)>, name: &str| -> usize {
        if let Some(i) = out.iter().position(|(k, _)| k == name) {
            i
        } else {
            out.push((name.to_string(), HashSet::new()));
            out.len() - 1
        }
    };
    for line in diff.split('\n') {
        if let Some(path) = line.strip_prefix("+++ ") {
            let path = path.trim();
            file = if path == "/dev/null" {
                None
            } else {
                Some(path.strip_prefix("b/").unwrap_or(path).to_string())
            };
            if let Some(f) = &file {
                find_mut(out, f);
            }
            continue;
        }
        if let Some((nl, count)) = parse_hunk_header(line) {
            new_line = nl;
            remaining = count;
            continue;
        }
        if let Some(f) = &file {
            if remaining > 0 && line.starts_with('+') && !line.starts_with("+++") {
                let idx = find_mut(out, f);
                out[idx].1.insert(new_line);
                new_line += 1;
                remaining -= 1;
            }
            // deleted lines (`-`, not `---`) don't consume a new-file line number.
        }
    }
}

fn count_lines(abs: &Path) -> i64 {
    let Ok(text) = fs::read_to_string(abs) else {
        return 0; // unreadable (e.g. binary garbage) - contributes no measurable lines
    };
    if text.is_empty() {
        return 0;
    }
    let parts = text.split('\n').count() as i64;
    if text.ends_with('\n') {
        parts - 1
    } else {
        parts
    }
}

/// Map of file -> set of added/modified line numbers (new-file line
/// numbers) since `base_ref`, including uncommitted work. Used for diff
/// coverage.
pub fn changed_lines(root: &Path, base_ref: Option<&str>) -> Vec<(String, HashSet<i64>)> {
    let mut result: Vec<(String, HashSet<i64>)> = Vec::new();
    let base = base_ref.unwrap_or("HEAD");
    // See `changed_files`'s comment on `--end-of-options` (gate report
    // finding 2): `base` is a stored `run.json` value here too.
    let committed = git(
        root,
        &[
            "diff",
            "--unified=0",
            "--no-color",
            "--end-of-options",
            base,
            "--",
        ],
    )
    .stdout;
    parse_unified_diff(&committed, &mut result);
    // Untracked files: every line counts as added. Enumerated explicitly -
    // an empty set must keep meaning "no added lines", never double as a
    // wholly-new-file sentinel.
    for f in untracked_files(root) {
        if result.iter().any(|(k, _)| *k == f) {
            continue;
        }
        let total = count_lines(&root.join(&f));
        let lines: HashSet<i64> = (1..=total).collect();
        result.push((f, lines));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command as StdCommand;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-git-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = StdCommand::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    /// A throwaway git repo with one committed file, mirroring `test/helpers.ts`'s `makeRepo`.
    fn make_repo(name: &str) -> PathBuf {
        let dir = tmp_dir(name);
        run_git(&dir, &["init", "-q"]);
        run_git(&dir, &["config", "user.email", "test@test.co"]);
        run_git(&dir, &["config", "user.name", "test"]);
        run_git(&dir, &["config", "commit.gpgsign", "false"]);
        write_file(&dir, "README.md", "seed\n");
        run_git(&dir, &["add", "-A"]);
        run_git(&dir, &["commit", "-qm", "init"]);
        dir
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let abs = root.join(rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, content).unwrap();
    }

    #[test]
    fn diff_text_includes_untracked_files_as_new_file_diffs() {
        let root = make_repo("difftext-untracked");
        write_file(&root, "brand-new.js", "console.log('new module')\n");
        let diff = diff_text(&root, head_sha(&root).as_deref());
        assert!(diff.contains("brand-new.js"));
        assert!(diff.contains("console.log('new module')"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn diff_text_excludes_gate_bookkeeping_tracked_or_untracked() {
        let root = make_repo("difftext-bookkeeping");
        write_file(&root, ".gate/runs/r1/run.json", "{}\n");
        write_file(&root, "code.js", "x\n");
        let diff = diff_text(&root, head_sha(&root).as_deref());
        assert!(diff.contains("code.js"));
        assert!(!diff.contains("run.json"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn changed_lines_enumerates_every_line_of_an_untracked_file() {
        let root = make_repo("changedlines-untracked");
        write_file(&root, "new.js", "a\nb\nc\n");
        let lines = changed_lines(&root, head_sha(&root).as_deref());
        let found = lines.iter().find(|(k, _)| k == "new.js").unwrap();
        let mut got: Vec<i64> = found.1.iter().copied().collect();
        got.sort();
        assert_eq!(got, vec![1, 2, 3]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn changed_lines_keeps_a_deletion_only_diff_as_an_empty_set() {
        let root = make_repo("changedlines-deletion");
        write_file(&root, "old.js", "a\nb\nc\n");
        run_git(&root, &["add", "-A"]);
        run_git(&root, &["commit", "-qm", "seed old.js"]);
        write_file(&root, "old.js", "a\n"); // delete two lines, add none
        let lines = changed_lines(&root, head_sha(&root).as_deref());
        let found = lines.iter().find(|(k, _)| k == "old.js");
        assert!(found.map(|(_, s)| s.is_empty()).unwrap_or(true));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tree_fingerprint_is_stable_while_the_tree_is_unchanged() {
        let root = make_repo("fingerprint-stable");
        assert_eq!(tree_fingerprint(&root), tree_fingerprint(&root));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tree_fingerprint_changes_on_edits_and_untracked_files_reverts_on_removal() {
        let root = make_repo("fingerprint-changes");
        write_file(&root, "a.js", "one\n");
        run_git(&root, &["add", "-A"]);
        run_git(&root, &["commit", "-qm", "seed a.js"]);
        let clean = tree_fingerprint(&root);
        write_file(&root, "a.js", "two\n");
        let edited = tree_fingerprint(&root);
        assert_ne!(edited, clean);
        write_file(&root, "extra.js", "three\n");
        let with_untracked = tree_fingerprint(&root);
        assert_ne!(with_untracked, edited);
        fs::remove_file(root.join("extra.js")).unwrap();
        assert_eq!(tree_fingerprint(&root), edited);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tree_fingerprint_ignores_gate_bookkeeping() {
        let root = make_repo("fingerprint-bookkeeping");
        let before = tree_fingerprint(&root);
        write_file(&root, ".gate/runs/r1/review.md", "reviewer notes\n");
        assert_eq!(tree_fingerprint(&root), before);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn staged_files_returns_exact_unquoted_filenames_for_non_ascii_paths() {
        let root = make_repo("staged-nonascii");
        let name = "café.ts";
        write_file(&root, name, "export const x = 1;\n");
        run_git(&root, &["add", name]);
        assert_eq!(staged_files(&root), vec![name.to_string()]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn staged_files_excludes_gate_bookkeeping_even_when_staged() {
        let root = make_repo("staged-bookkeeping");
        write_file(&root, ".gate/runs/r1/run.json", "{}\n");
        write_file(&root, "code.js", "x\n");
        run_git(&root, &["add", "-A"]);
        let files = staged_files(&root);
        assert!(files.contains(&"code.js".to_string()));
        assert!(!files.iter().any(|f| f.contains("run.json")));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn is_gate_bookkeeping_is_unconditional_for_dot_gate() {
        let root = make_repo("bookkeeping-dotgate");
        assert!(is_gate_bookkeeping(&root, ".gate/runs/x/run.json"));
        assert!(is_gate_bookkeeping(&root, ".gate/trust.json"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn branch_state_reports_the_checked_out_branch_on_a_normal_repo() {
        let root = make_repo("branch-state-normal");
        match branch_state(&root) {
            BranchState::Branch(_) => {}
            other => panic!("expected Branch, got {other:?}"),
        }
        assert!(is_git_repo(&root));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn branch_state_is_none_outside_any_git_repo() {
        let dir = tmp_dir("branch-state-none");
        assert_eq!(branch_state(&dir), BranchState::None);
        assert!(!is_git_repo(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn head_sha_is_none_before_the_first_commit() {
        let dir = tmp_dir("headsha-unborn");
        run_git(&dir, &["init", "-q"]);
        assert_eq!(head_sha(&dir), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn changed_files_dedupes_and_includes_untracked_entries() {
        let root = make_repo("changed-files-dedup");
        write_file(&root, "new.js", "x\n");
        let files = changed_files(&root, head_sha(&root).as_deref());
        assert_eq!(files.iter().filter(|f| *f == "new.js").count(), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn diff_no_index_shows_a_diff_between_two_files_outside_any_repo() {
        let dir = tmp_dir("diff-no-index");
        write_file(&dir, "a.txt", "one\n");
        write_file(&dir, "b.txt", "two\n");
        let diff = diff_no_index(&dir, "a.txt", "b.txt");
        assert!(diff.contains("-one"));
        assert!(diff.contains("+two"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn diff_no_index_headers_use_the_given_relative_paths_not_a_mangled_absolute_one() {
        let dir = tmp_dir("diff-no-index-headers");
        write_file(&dir, "plan.approved.md", "one\n");
        write_file(&dir, "plan.md", "two\n");
        let diff = diff_no_index(&dir, "plan.approved.md", "plan.md");
        assert!(diff.contains("a/plan.approved.md"));
        assert!(diff.contains("b/plan.md"));
        assert!(!diff.contains(dir.to_str().unwrap()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn user_name_reads_the_repo_local_git_config() {
        let root = make_repo("user-name-set");
        assert_eq!(user_name(&root).as_deref(), Some("test"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn user_name_trims_the_configured_value() {
        let root = make_repo("user-name-trim");
        run_git(&root, &["config", "user.name", "  Spacey Name  "]);
        assert_eq!(user_name(&root).as_deref(), Some("Spacey Name"));
        fs::remove_dir_all(&root).unwrap();
    }

    // The "unset, and no global fallback either" case (git config errors,
    // `user_name` returns `None`) is exercised end-to-end by the
    // conformance suite's identity-fallback tests (`rust/tests/cli_e2e.rs`),
    // which can isolate `$HOME` for a subprocess without touching this
    // process's own environment - unsafe to do from a `cargo test` unit
    // test, since `cargo test` runs the crate's tests in one process and
    // `std::env::set_var` is a data race against every other test running
    // concurrently in it.

    #[test]
    fn git_hooks_dir_resolves_to_an_absolute_path_under_dot_git() {
        let root = make_repo("hooks-dir");
        let dir = git_hooks_dir(&root).unwrap();
        assert!(dir.is_absolute());
        assert!(dir.ends_with("hooks"));
        fs::remove_dir_all(&root).unwrap();
    }
}
