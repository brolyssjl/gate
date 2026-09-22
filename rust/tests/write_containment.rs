//! Conformance tests for report finding 3 (HIGH - symlinks in the repo are
//! followed on write; arbitrary content lands outside the project) and
//! finding 10's file-mode note. No TS counterpart - added post-port
//! (see `CONFORMANCE_MAP.md`'s note on `identity.rs`/`streak.rs` for the
//! same pattern: security regressions found after the TS suite was
//! retired).
//!
//! Each write-path test reproduces the report's exact recipe: a committed
//! symlink pointing at a file in a temp dir *outside* the repo, then the
//! command that used to write through it. Asserts the outside file is
//! never touched and gate fails with a clear error.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{gate, make_repo, make_temp_dir, write_file};

const VALID_PLAN: &str = "---\ngoal: g\nfiles:\n  - a.txt\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan\n";

/// Report finding 3(a): `rust/src/commands/approve.rs` used to `fs::write`
/// the plan's content straight to `plan.approved.md`. A committed symlink
/// there (`.gate/runs/<id>/plan.approved.md -> /outside/marker`) landed the
/// plan's content - and, moving the rename target, effectively the whole
/// snapshot mechanism - on a file entirely outside the project.
#[test]
fn approve_refuses_symlinked_plan_approved() {
    let repo = make_repo(&[]);
    let outside_dir = make_temp_dir("write-containment-outside-a");
    let outside_file = outside_dir.join("marker.txt");
    write_file(&outside_dir, "marker.txt", "untouched\n");

    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["trust"]);
    let started = gate(&repo, &["start", "approve symlink test", "--json"]);
    assert_eq!(started.code, 0, "start failed: {}", started.stderr);
    let json = started.json();
    let run_id = json.str("id").unwrap().to_string();
    let plan_path = json.str("plan").unwrap().to_string();
    write_file(&repo, &plan_path, VALID_PLAN);

    let plan_approved = repo
        .join(".gate")
        .join("runs")
        .join(&run_id)
        .join("plan.approved.md");
    std::os::unix::fs::symlink(&outside_file, &plan_approved).unwrap();

    let result = gate(&repo, &["approve", "--by", "tester"]);
    assert_ne!(
        result.code, 0,
        "gate approve should refuse to write through the symlink, got: stdout={} stderr={}",
        result.stdout, result.stderr
    );

    assert_eq!(
        std::fs::read_to_string(&outside_file).unwrap(),
        "untouched\n",
        "the outside file must never be written to"
    );
    // The symlink itself is untouched too (not swapped for a real file).
    assert!(std::fs::symlink_metadata(&plan_approved)
        .unwrap()
        .file_type()
        .is_symlink());
}

/// Report finding 3(b): `fsx::write_file_atomic` used to write its content
/// to a fixed, predictable `<path>.tmp` sibling with a plain `fs::write`
/// (no `O_EXCL`), then rename it over the target. A committed
/// `.gate/trust.json.tmp -> /outside/marker` symlink made `gate trust`
/// write through it and then move the symlink itself over `trust.json`.
#[test]
fn write_file_atomic_refuses_tmp_symlink_sibling_via_gate_trust() {
    let repo = make_repo(&[]);
    let outside_dir = make_temp_dir("write-containment-outside-b");
    let outside_file = outside_dir.join("marker.txt");
    write_file(&outside_dir, "marker.txt", "untouched\n");

    gate(&repo, &["init", "--no-adapt"]);

    let legacy_tmp_sibling = repo.join(".gate").join("trust.json.tmp");
    std::os::unix::fs::symlink(&outside_file, &legacy_tmp_sibling).unwrap();

    let result = gate(&repo, &["trust"]);

    assert_eq!(
        std::fs::read_to_string(&outside_file).unwrap(),
        "untouched\n",
        "the outside file must never be written to, even though `gate trust` itself \
         may still succeed by writing its own uniquely-named temp file"
    );
    // The legacy predictable sibling was never touched either - it isn't
    // the (unique, random-suffixed) temp file this write actually uses.
    assert!(
        std::fs::symlink_metadata(&legacy_tmp_sibling)
            .unwrap()
            .file_type()
            .is_symlink(),
        "gate trust: stdout={} stderr={}",
        result.stdout,
        result.stderr
    );
    // And trust.json itself, if written, is a real file (not the symlink
    // moved over it).
    let trust_json = repo.join(".gate").join("trust.json");
    if trust_json.exists() {
        assert!(std::fs::symlink_metadata(&trust_json)
            .unwrap()
            .file_type()
            .is_file());
    }
}

/// Report finding 3(c): `rust/src/commands/adapt.rs` used to
/// `root.join(adapter.target_path)` and `fs::write` straight to it. A
/// committed `CLAUDE.md -> /outside/marker` symlink made `gate init`/
/// `gate adapt` write gate's managed block into a file entirely outside
/// the project.
#[test]
fn adapt_refuses_symlinked_target_outside_root() {
    let repo = make_repo(&[]);
    let outside_dir = make_temp_dir("write-containment-outside-c");
    let outside_file = outside_dir.join("marker.txt");
    write_file(&outside_dir, "marker.txt", "untouched\n");

    gate(&repo, &["init", "--no-adapt"]);
    std::os::unix::fs::symlink(&outside_file, repo.join("CLAUDE.md")).unwrap();

    let result = gate(&repo, &["adapt", "claude"]);
    assert_ne!(
        result.code, 0,
        "gate adapt should refuse to write through the symlink, got: stdout={} stderr={}",
        result.stdout, result.stderr
    );
    assert!(
        result.stderr.contains("CLAUDE.md") || result.stderr.contains(&repo.display().to_string()),
        "error should name the offending path: {}",
        result.stderr
    );

    assert_eq!(
        std::fs::read_to_string(&outside_file).unwrap(),
        "untouched\n",
        "the outside file must never be written to"
    );
}

/// Report finding 10 (LOW): `review-packet.md` can carry the full repo
/// diff and used to be written with the ambient umask. It must be 0600.
#[test]
fn review_packet_is_written_with_mode_0o600() {
    let repo = make_repo(&[]);
    write_file(&repo, "run-tests.sh", "#!/bin/sh\nexit 0\n");
    std::process::Command::new("chmod")
        .args(["+x", "run-tests.sh"])
        .current_dir(&repo)
        .status()
        .unwrap();
    gate(&repo, &["init", "--no-adapt"]);
    let config_path: &Path = &repo.join(".gate").join("config.yml");
    let config = std::fs::read_to_string(config_path).unwrap();
    std::fs::write(
        config_path,
        config.replace("# test: \"<command>\"", "test: \"./run-tests.sh\""),
    )
    .unwrap();
    gate(&repo, &["trust"]);

    let started = gate(
        &repo,
        &[
            "start",
            "packet mode test",
            "--profile",
            "feature",
            "--json",
        ],
    );
    assert_eq!(started.code, 0, "start failed: {}", started.stderr);
    let json = started.json();
    let run_id = json.str("id").unwrap().to_string();
    let plan_path = json.str("plan").unwrap().to_string();
    write_file(
        &repo,
        &plan_path,
        &[
            "---",
            "goal: exercise the review packet",
            "files:",
            "  - a.txt",
            "  - run-tests.sh",
            "criteria:",
            "  - id: c1",
            "    text: it works",
            "    verify: \"manual\"",
            "---",
            "",
        ]
        .join("\n"),
    );
    gate(&repo, &["approve", "--by", "implementer"]);
    gate(&repo, &["next"]); // -> IMPLEMENT
    write_file(&repo, "a.txt", "hi\n");
    gate(&repo, &["next"]); // -> TEST
    gate(&repo, &["next"]); // -> REVIEW
    let review = gate(&repo, &["review", "--fresh"]);
    assert_eq!(review.code, 0, "gate review failed: {}", review.stderr);

    let packet_path = repo
        .join(".gate")
        .join("runs")
        .join(&run_id)
        .join("review-packet.md");
    let mode = std::fs::metadata(&packet_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "review-packet.md should be 0600, was {mode:o}");
}
