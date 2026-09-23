//! Source-lint guards for the crate's own unit tests, added for the
//! 2026-09-22 security audit finding 11 (test hygiene). No TS counterpart -
//! see CONFORMANCE_MAP.md for why post-port suites are absent there.
//!
//! `cargo test` runs a crate's unit tests as threads in one process, so a
//! test that mutates process-global state (`env::set_var`,
//! `env::set_current_dir`) races every other test; under Rust 2024 `set_var`
//! is `unsafe` for exactly this reason. And a temp dir built from a
//! predictable name (`temp_dir()/gate-<mod>-<pid>`) can be pre-created by
//! another local user as a symlink on a shared `/tmp`, which
//! `create_dir_all` then silently adopts. Both rules live in
//! `rust/src/core/testutil.rs`; these tests keep every module honest.

use std::fs;
use std::path::{Path, PathBuf};

fn src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `(relative path, line number, line)` for every code line in `rust/src`
/// containing `needle`, with `//` comments stripped so prose about a rule
/// never trips the rule.
fn code_lines_containing(needle: &str) -> Vec<(String, usize, String)> {
    let root = src_root();
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    files.sort();
    let mut hits = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("read source file");
        let rel = file
            .strip_prefix(&root)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        for (i, line) in text.lines().enumerate() {
            let code = match line.find("//") {
                Some(idx) => &line[..idx],
                None => line,
            };
            if code.contains(needle) {
                hits.push((rel.clone(), i + 1, line.trim().to_string()));
            }
        }
    }
    hits
}

fn render(hits: &[(String, usize, String)]) -> String {
    hits.iter()
        .map(|(f, n, l)| format!("  {f}:{n}: {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Finding 11(a): nothing under `rust/src` may mutate the process
/// environment or cwd in-process. Pass values through parameters, or set
/// env on a spawned `Command`.
#[test]
fn src_has_no_in_process_env_mutation() {
    let mut hits = Vec::new();
    for needle in ["set_var(", "remove_var(", "set_current_dir("] {
        hits.extend(code_lines_containing(needle));
    }
    assert!(
        hits.is_empty(),
        "in-process env/cwd mutation under rust/src (race across cargo test threads):\n{}",
        render(&hits)
    );
}

/// Finding 11(b): unit-test temp dirs are created only through
/// `core::testutil::unique_temp_dir`, which gives every call an
/// unguessable name and refuses to adopt a pre-existing entry.
#[test]
fn src_builds_test_temp_dirs_only_through_the_shared_helper() {
    let hits: Vec<_> = code_lines_containing("temp_dir()")
        .into_iter()
        .filter(|(f, _, _)| f != "core/testutil.rs")
        .collect();
    assert!(
        hits.is_empty(),
        "ad hoc temp dir under rust/src (use crate::core::testutil::unique_temp_dir):\n{}",
        render(&hits)
    );
}
