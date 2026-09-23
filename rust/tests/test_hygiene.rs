//! Source-lint guards for the crate's own unit tests, added for the
//! 2026-09-22 security audit finding 11 (test hygiene). A post-port suite
//! with no TS counterpart, like the others CONFORMANCE_MAP.md leaves out.
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

/// Files under `rust/src` allowed to call `std::env::temp_dir()`. Only the
/// shared test helper today: production code has no use for the OS temp
/// dir (`fsx` writes its temp files next to their target on purpose). A
/// future production caller goes on this list deliberately, with a reason.
const TEMP_DIR_ALLOWED: &[&str] = &["core/testutil.rs"];

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

/// `text` with every comment (`//` and `/* */`) and every string literal's
/// contents (plain, escaped, raw `r#"..."#`, byte) blanked to spaces, so a
/// needle only matches real code. Newlines are kept, so line numbers in the
/// result still point at the source line. Char literals (`'"'`, `'\''`)
/// are stepped over so a quote inside one cannot open a phantom string.
fn code_only(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, c: u8| out.push(if c == b'\n' { b'\n' } else { b' ' });
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            out.extend_from_slice(b"  ");
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                blank(&mut out, b[i]);
                i += 1;
            }
            if i < b.len() {
                out.extend_from_slice(b"  ");
                i += 2;
            }
        } else if c == b'"' {
            // Raw string? Count the `#`s just before the quote, then an `r`.
            let mut hashes = 0;
            while i > hashes && b[i - hashes - 1] == b'#' {
                hashes += 1;
            }
            let raw = i > hashes && b[i - hashes - 1] == b'r';
            out.push(b'"');
            i += 1;
            loop {
                if i >= b.len() {
                    break;
                }
                if raw {
                    if b[i] == b'"' && b[i + 1..].starts_with(&vec![b'#'; hashes]) {
                        out.push(b'"');
                        out.extend(std::iter::repeat_n(b'#', hashes));
                        i += 1 + hashes;
                        break;
                    }
                } else if b[i] == b'\\' {
                    blank(&mut out, b[i]);
                    i += 1;
                    if i < b.len() {
                        blank(&mut out, b[i]);
                        i += 1;
                    }
                    continue;
                } else if b[i] == b'"' {
                    out.push(b'"');
                    i += 1;
                    break;
                }
                blank(&mut out, b[i]);
                i += 1;
            }
        } else if c == b'\'' {
            if b.get(i + 1) == Some(&b'\\') {
                // Escaped char literal: copy through the closing quote.
                let end = b[i + 2..]
                    .iter()
                    .position(|&x| x == b'\'')
                    .map_or(b.len(), |p| i + 3 + p);
                out.extend_from_slice(&b[i..end]);
                i = end;
            } else if b.get(i + 2) == Some(&b'\'') {
                out.extend_from_slice(&b[i..i + 3]);
                i += 3;
            } else {
                out.push(c); // a lifetime
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).expect("only ASCII bytes were substituted")
}

/// `(relative path, line number, line)` for every code line in `rust/src`
/// containing `needle`, with comments and string contents blanked so prose
/// about a rule never trips the rule.
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
        let code = code_only(&text);
        for ((i, code_line), src_line) in code.lines().enumerate().zip(text.lines()) {
            if code_line.contains(needle) {
                hits.push((rel.clone(), i + 1, src_line.trim().to_string()));
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

/// The lint's own eyes: a `//` inside a string must not hide a real call on
/// the same line, and a call spelled inside a comment or a string must not
/// count. Each case is one line so the fixture reads like the rule.
#[test]
fn lint_sees_through_comments_and_string_literals() {
    let hit = |line: &str| code_only(line).contains("set_var(");
    assert!(hit(
        r#"let doc = "see https://example.com"; std::env::set_var("A", "b");"#
    ));
    assert!(hit(r#"let q = '"'; env::set_var("A", "b");"#));
    assert!(hit(r#"let q = '\''; env::set_var("A", "b");"#));
    assert!(hit("let s = r#\"x\"#; env::set_var(\"A\", \"b\");"));
    assert!(!hit(r#"// env::set_var("A", "b");"#));
    assert!(!hit(r#"/* env::set_var("A", "b"); */"#));
    assert!(!hit(r#"let s = "env::set_var(";"#));
    assert!(!hit(r#"let s = "escaped \" env::set_var(";"#));
    assert!(!hit("let s = r#\"env::set_var(\"#;"));
    assert!(!hit(r#"let s = b"env::set_var(";"#));
    // Line numbers survive blanking, including inside block comments.
    assert_eq!(code_only("a\n/* x\ny */\nb").lines().count(), 4);
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
/// unguessable name and refuses to adopt a pre-existing entry. The ban is
/// crate-wide (see `TEMP_DIR_ALLOWED`), so a production caller cannot
/// creep in unnoticed either.
#[test]
fn src_builds_test_temp_dirs_only_through_the_shared_helper() {
    let hits: Vec<_> = code_lines_containing("temp_dir()")
        .into_iter()
        .filter(|(f, _, _)| !TEMP_DIR_ALLOWED.contains(&f.as_str()))
        .collect();
    assert!(
        hits.is_empty(),
        "std::env::temp_dir() outside TEMP_DIR_ALLOWED under rust/src - in a unit test use \
         crate::core::testutil::unique_temp_dir; production code has no sanctioned use today, \
         allowlist it in rust/tests/test_hygiene.rs with a reason if that changes:\n{}",
        render(&hits)
    );
}
