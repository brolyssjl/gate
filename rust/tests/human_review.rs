//! Port of `test/humanReview.test.ts` ("gate review --human").
//! See `CONFORMANCE_MAP.md` for the TS case -> Rust test fn mapping.

mod common;

use common::{gate, gate_opts, make_repo, write_file, GateOpts, GateOutput};
use std::path::Path;

/// This suite always calls `gate` with a raw stdin string (or none) - a
/// thin wrapper around the shared harness's `gate_opts`, mirroring the TS
/// file's local `gate(cwd, args, input?)` adapter.
fn gate_input(cwd: &Path, args: &[&str], input: &str) -> GateOutput {
    gate_opts(
        cwd,
        args,
        GateOpts {
            input: Some(input),
            ..Default::default()
        },
    )
}

/// Walk a fresh repo from init to an active REVIEW-phase run. Returns the
/// run id.
fn setup_review_run(repo: &Path) -> String {
    write_file(repo, "run-tests.sh", "#!/bin/sh\nexit 0\n");
    std::process::Command::new("chmod")
        .args(["+x", "run-tests.sh"])
        .current_dir(repo)
        .status()
        .unwrap();
    gate(repo, &["init", "--no-adapt"]);
    let config_path = repo.join(".gate").join("config.yml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    std::fs::write(
        &config_path,
        config.replace("# test: \"<command>\"", "test: \"./run-tests.sh\""),
    )
    .unwrap();
    gate(repo, &["trust"]);

    let started = gate(
        repo,
        &[
            "start",
            "human review test",
            "--profile",
            "feature",
            "--json",
        ],
    );
    let json = started.json();
    let run_id = json.str("id").unwrap().to_string();
    let plan_path = json.str("plan").unwrap().to_string();
    write_file(
        repo,
        &plan_path,
        &[
            "---",
            "goal: exercise human review",
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
    gate(repo, &["approve", "--by", "implementer"]);
    gate(repo, &["next"]); // -> IMPLEMENT
    write_file(repo, "a.txt", "hi\n");
    gate(repo, &["next"]); // -> TEST
    gate(repo, &["next"]); // -> REVIEW
    run_id
}

/// Writes a review.md with a pre-existing `f2` finding (`f1` deleted by
/// hand), for the finding-id-collision regression cases below.
fn seed_finding_f2(review_path: &Path) {
    std::fs::write(
        review_path,
        [
            "---",
            "reviewer:",
            "findings:",
            "  - id: f2",
            "    severity: minor",
            "    status: resolved",
            "    note: pre-existing - f1 was deleted by hand",
            "---",
            "",
            "# Review",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
}

#[test]
fn records_a_waived_blocker_with_a_rationale_satisfying_the_same_review_gate_as_an_agent_review() {
    let repo = make_repo(&[]);
    setup_review_run(&repo);

    let input = [
        "y",
        "",
        "blocker",
        "off by one in the loop",
        "waived",
        "acceptable for this run",
        "n",
        "tester",
    ]
    .join("\n")
        + "\n";
    let res = gate_input(&repo, &["review", "--human"], &input);
    assert_eq!(res.code, 0);

    let check = gate(&repo, &["check", "--json"]);
    assert_eq!(check.code, 0);
    assert_eq!(check.json().bool_at("ok"), Some(true));
}

#[test]
fn blocks_the_review_gate_on_an_open_blocker_finding_same_as_an_agent_recorded_one() {
    let repo = make_repo(&[]);
    setup_review_run(&repo);

    let input = [
        "y",
        "",
        "blocker",
        "this is unresolved",
        "open",
        "n",
        "tester",
    ]
    .join("\n")
        + "\n";
    let res = gate_input(&repo, &["review", "--human"], &input);
    assert_eq!(res.code, 0);

    let check = gate(&repo, &["check", "--json"]);
    assert_eq!(check.code, 1);
    let data = check.json();
    assert_eq!(data.bool_at("ok"), Some(false));
    let checks = data.get("checks").unwrap().as_array().unwrap();
    assert!(checks
        .iter()
        .any(|c| c.str("name") == Some("review.findings") && c.bool_at("ok") == Some(false)));
}

#[test]
fn records_multiple_findings_and_requires_a_reviewer_name_before_writing_review_md() {
    let repo = make_repo(&[]);
    setup_review_run(&repo);

    let input = [
        "y",
        "f-typo",
        "nit",
        "typo in a comment",
        "resolved",
        "y",
        "f-perf",
        "minor",
        "could be faster",
        "open",
        "n",
        "reviewer-two",
    ]
    .join("\n")
        + "\n";
    let res = gate_input(&repo, &["review", "--human"], &input);
    assert_eq!(res.code, 0);

    let check = gate(&repo, &["check", "--json"]);
    assert_eq!(check.code, 0); // minor/nit never block
}

#[test]
fn reuses_an_existing_packet_unless_fresh_is_passed_matching_the_non_human_review_command() {
    let repo = make_repo(&[]);
    setup_review_run(&repo);

    let first = gate_input(
        &repo,
        &["review", "--human"],
        &(["n", "tester"].join("\n") + "\n"),
    );
    assert!(first.stdout.contains("freshly generated"));

    let second = gate_input(
        &repo,
        &["review", "--human"],
        &(["n", "tester"].join("\n") + "\n"),
    );
    assert!(second.stdout.contains("reusing an existing packet"));
}

#[test]
fn errors_clearly_instead_of_hanging_when_input_ends_before_the_review_is_recorded() {
    let repo = make_repo(&[]);
    setup_review_run(&repo);

    let res = gate_input(&repo, &["review", "--human"], "n\n"); // stops right before the reviewer-name prompt
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains("input ended"));
}

#[test]
fn defaults_to_a_non_colliding_finding_id_when_an_earlier_one_was_deleted_by_hand_only_f2_remains()
{
    let repo = make_repo(&[]);
    let id = setup_review_run(&repo);
    let review_path = repo.join(".gate").join("runs").join(&id).join("review.md");
    seed_finding_f2(&review_path);

    // Accept the default id (empty input) for one new finding. The old bug:
    // the default was existing.length + 1 = "f2" again (length 1, not the
    // actual id present), producing a duplicate id that made review.md
    // permanently unparseable on every subsequent --human run.
    let input = ["y", "", "nit", "trivial", "resolved", "n", "tester"].join("\n") + "\n";
    let res = gate_input(&repo, &["review", "--human"], &input);
    assert_eq!(res.code, 0);

    let content = std::fs::read_to_string(&review_path).unwrap();
    assert_eq!(content.matches("id: f2").count(), 1);

    let check = gate(&repo, &["check", "--json"]);
    assert_eq!(check.code, 0);

    // Wedged in the old code: this second --human run would have thrown
    // "existing review.md is invalid" the moment it tried to parse the
    // duplicate-id file the first run just wrote.
    let again = gate_input(
        &repo,
        &["review", "--human"],
        &(["n", "tester"].join("\n") + "\n"),
    );
    assert_eq!(again.code, 0);
    assert!(!again.stderr.contains("invalid"));
}

#[test]
fn re_prompts_when_the_reviewer_explicitly_types_a_finding_id_thats_already_used() {
    let repo = make_repo(&[]);
    let id = setup_review_run(&repo);
    let review_path = repo.join(".gate").join("runs").join(&id).join("review.md");
    seed_finding_f2(&review_path);

    let input = ["y", "f2", "f9", "minor", "typo", "resolved", "n", "tester"].join("\n") + "\n";
    let res = gate_input(&repo, &["review", "--human"], &input);
    assert_eq!(res.code, 0);
    assert!(res.stdout.contains("already used"));

    let content = std::fs::read_to_string(&review_path).unwrap();
    assert!(content.contains("id: f9"));
    assert_eq!(content.matches("id: f2").count(), 1);
    assert_eq!(gate(&repo, &["check", "--json"]).code, 0);
}
