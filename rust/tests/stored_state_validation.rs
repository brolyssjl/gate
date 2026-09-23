//! Conformance tests for the 2026-09-22 security audit's findings 2 and 4
//! (see `audits/2026-09-22-security-audit-gate.md`): stored `run.json`
//! state (`baseRef`, and `id` whether read from `run.json` itself or from
//! `.gate/current.json`) must be validated before it can steer a `git`
//! invocation or a filesystem path outside `.gate/runs/`.
//!
//! Each test hand-edits a `run.json`/`current.json` a real `gate start`
//! just wrote, simulating a maliciously crafted (e.g. cloned/committed)
//! repo - exactly the threat model the audit tested against.

mod common;

use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{gate, git_out, make_repo, read_file, write_file};

/// A path guaranteed not to exist yet, outside any repo this test creates.
/// Used as the "attacker's target file" in the finding-2 repro - the test
/// asserts it is never created.
fn scratch_target(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "gate-stored-state-validation-{label}-{}-{nanos}.txt",
        process::id()
    ))
}

/// Replace a top-level `"field": "old"` string value in a pretty-printed
/// JSON document with `"field": "new"`, without a JSON writer - test-only
/// (the malicious values used here never contain a literal `"`).
fn replace_json_string_field(text: &str, field: &str, new_value: &str) -> String {
    let needle = format!("\"{field}\": \"");
    let start = text
        .find(&needle)
        .unwrap_or_else(|| panic!("field {field:?} not found in:\n{text}"));
    let value_start = start + needle.len();
    let end = text[value_start..]
        .find('"')
        .map(|i| i + value_start)
        .unwrap_or_else(|| panic!("unterminated {field:?} value in:\n{text}"));
    format!("{}{}{}", &text[..value_start], new_value, &text[end..])
}

fn run_json_rel(run_id: &str) -> String {
    format!(".gate/runs/{run_id}/run.json")
}

fn current_run_id(repo: &Path) -> String {
    gate(repo, &["status", "--json"])
        .json()
        .str("id")
        .expect("gate status --json must carry an id")
        .to_string()
}

/// Finding 2 (CRITICAL): a `run.json` `baseRef` starting with `-` is placed
/// in `git diff` argv before `--`, so e.g. `--output=<path>` makes git
/// write the diff to an attacker-chosen file. `gate check` must refuse
/// instead of ever invoking git with it.
#[test]
fn finding_2_option_like_base_ref_is_rejected_and_never_reaches_git() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["start", "finding 2 demo", "--profile", "bugfix"]);
    let run_id = current_run_id(&repo);

    let target = scratch_target("finding2");
    assert!(!target.exists(), "precondition: target must not pre-exist");

    let run_json_path = run_json_rel(&run_id);
    let original = read_file(&repo, &run_json_path);
    let poisoned = replace_json_string_field(
        &original,
        "baseRef",
        &format!("--output={}", target.display()),
    );
    write_file(&repo, &run_json_path, &poisoned);

    let res = gate(&repo, &["check"]);
    assert_ne!(res.code, 0, "gate check must refuse a hostile baseRef");
    assert!(
        res.stderr.contains("baseRef"),
        "error must name the offending field; stderr was: {}",
        res.stderr
    );
    assert!(
        !target.exists(),
        "the attacker's --output target must never be created: {}",
        target.display()
    );

    // `gate status` goes through the same `read_run`, but a `read_run`
    // failure is `status.rs`'s pre-existing "dangling mapping" self-heal
    // path (it clears the pointer and reports no active run rather than
    // erroring) - the security property that matters here is unchanged:
    // it must never create the attacker's target.
    let status_res = gate(&repo, &["status"]);
    assert_eq!(status_res.code, 0);
    assert!(!target.exists());
}

/// Finding 4 (HIGH), the `run.json`-own-`id`-field shape: an absolute path
/// in the `id` field must be rejected by `read_run` before `write_run` (or
/// anything else keyed on `run.id`) can turn it into a path outside
/// `.gate/runs/`.
#[test]
fn finding_4_absolute_id_field_in_run_json_is_rejected_and_creates_nothing_outside_gate_runs() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(
        &repo,
        &["start", "finding 4 absolute demo", "--profile", "docs"],
    );
    let run_id = current_run_id(&repo);

    let escape_dir = scratch_target("finding4-abs").with_extension("");
    assert!(
        !escape_dir.exists(),
        "precondition: escape dir must not pre-exist"
    );

    let run_json_path = run_json_rel(&run_id);
    let original = read_file(&repo, &run_json_path);
    let poisoned = replace_json_string_field(&original, "id", &escape_dir.display().to_string());
    write_file(&repo, &run_json_path, &poisoned);

    let res = gate(&repo, &["check"]);
    assert_ne!(res.code, 0, "gate check must refuse a hostile id field");
    assert!(
        res.stderr.contains("id"),
        "error must name the offending field; stderr was: {}",
        res.stderr
    );
    assert!(
        !escape_dir.exists(),
        "nothing may be created outside .gate/runs/: {}",
        escape_dir.display()
    );

    // `status.rs`'s pre-existing "dangling mapping" self-heal treats this
    // `read_run` failure the same as a missing run folder: it self-heals
    // rather than erroring. The property that matters is unaffected: no
    // escape.
    let status_res = gate(&repo, &["status"]);
    assert_eq!(status_res.code, 0);
    assert!(!escape_dir.exists());
}

/// Finding 4 (HIGH), the path-traversal shape of the same bug: `"id":
/// "../escape"` must be rejected the same way as an absolute path.
#[test]
fn finding_4_path_traversal_id_field_in_run_json_is_rejected() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(
        &repo,
        &["start", "finding 4 traversal demo", "--profile", "docs"],
    );
    let run_id = current_run_id(&repo);

    let run_json_path = run_json_rel(&run_id);
    let original = read_file(&repo, &run_json_path);
    let poisoned = replace_json_string_field(&original, "id", "../escape");
    write_file(&repo, &run_json_path, &poisoned);

    // A sibling of `.gate/runs/` (i.e. `.gate/escape`) is what `../escape`
    // resolves to when joined onto `.gate/runs/<run_id>/../escape` -
    // outside `.gate/runs/` itself, which is the confinement that matters.
    let escape_path = repo.join(".gate").join("escape");
    assert!(!escape_path.exists());

    let res = gate(&repo, &["check"]);
    assert_ne!(res.code, 0, "gate check must refuse a traversal id field");
    assert!(
        res.stderr.contains("id"),
        "error must name the offending field; stderr was: {}",
        res.stderr
    );
    assert!(!escape_path.exists());
}

/// Finding 4 (HIGH), the `.gate/current.json` shape: the pointer file
/// itself (not `run.json`'s own `id` field) can be hand-edited to map a
/// branch to a `..`-traversal id. `cli/context.rs`'s `require_active_run`
/// (used by every phase command, including `gate check`) and `gate guard
/// run` must both reject it before it ever reaches `read_run`/`run_paths`.
/// Unvalidated, `run_paths(root, "../escape")` resolves to `.gate/escape`,
/// a real sibling of `.gate/runs/` this test plants a fully valid,
/// unrelated run.json at, breaking `.gate/runs/`'s confinement by silently
/// adopting it as the active run (and, on `gate check`, writing back into
/// it) instead of refusing the malformed pointer.
#[test]
fn finding_4_malicious_id_in_current_json_pointer_is_rejected() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);

    // A fully valid run.json, planted directly at `.gate/escape/run.json`
    // - a sibling of `.gate/runs/`, never itself a run gate manages.
    let escape_run_json = r#"{
        "schema": 4,
        "id": "escape",
        "title": "not a real run",
        "profile": "docs",
        "phase": "PLAN",
        "status": "active",
        "createdAt": "2026-01-01T00:00:00.000Z",
        "updatedAt": "2026-01-01T00:00:00.000Z",
        "branch": null,
        "baseRef": null,
        "sessionId": null,
        "history": [],
        "overrides": [],
        "artifacts": {}
    }"#;
    write_file(&repo, ".gate/escape/run.json", escape_run_json);
    let before = read_file(&repo, ".gate/escape/run.json");

    // Point the current branch's `current.json` mapping at it via `..`
    // traversal, instead of any real id under `.gate/runs/`.
    let branch = git_out(&repo, &["branch", "--show-current"]);
    let current_path = ".gate/current.json";
    write_file(
        &repo,
        current_path,
        &format!("{{\"schema\": 1, \"branches\": {{\"{branch}\": \"../escape\"}}}}\n"),
    );

    let res = gate(&repo, &["check"]);
    assert_ne!(
        res.code, 0,
        "gate check must refuse a `..`-traversal current.json pointer, not silently adopt \
         .gate/escape/run.json as the active run"
    );
    assert!(
        res.stderr.contains("invalid run id"),
        "error must name the invalid id; stderr was: {}",
        res.stderr
    );
    assert_eq!(
        read_file(&repo, ".gate/escape/run.json"),
        before,
        ".gate/escape/run.json must never be read or written through the confused pointer"
    );

    let guard_res = gate(&repo, &["guard", "run"]);
    assert_ne!(guard_res.code, 0, "gate guard run must also refuse it");
    assert_eq!(read_file(&repo, ".gate/escape/run.json"), before);
}
