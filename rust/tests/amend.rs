//! Port of `test/amend.test.ts` ("approved-plan drift + gate amend").
//! See `CONFORMANCE_MAP.md` for the TS case -> Rust test fn mapping.

mod common;

use common::{gate, make_repo, read_file, write_file};

fn plan_with_files(files: &[&str]) -> String {
    let mut out = String::from("---\ngoal: exercise drift\nfiles:\n");
    for f in files {
        out.push_str(&format!("  - {f}\n"));
    }
    out.push_str("criteria:\n  - id: c1\n    text: it works\n    verify: \"manual\"\n---\n");
    out
}

/// Start a run, write and approve a plan declaring `files`. Returns the run
/// id and the plan's path relative to the repo root.
fn setup_approved_run(repo: &std::path::Path, files: &[&str]) -> (String, String) {
    gate(repo, &["init", "--no-adapt"]);
    let started = gate(repo, &["start", "drift demo", "--json"]);
    let json = started.json();
    let run_id = json.str("id").unwrap().to_string();
    let plan_path = json.str("plan").unwrap().to_string();
    write_file(repo, &plan_path, &plan_with_files(files));
    assert_eq!(gate(repo, &["approve", "--by", "human"]).code, 0);
    (run_id, plan_path)
}

#[test]
fn post_plan_scope_widening_fails_closed_until_amended() {
    let repo = make_repo(&[]);
    let (_run_id, plan_path) = setup_approved_run(&repo, &["a.txt"]);
    assert_eq!(gate(&repo, &["next"]).code, 0); // PLAN -> IMPLEMENT

    write_file(&repo, &plan_path, &plan_with_files(&["a.txt", "b.txt"])); // scope widened, no re-approval

    let check = gate(&repo, &["check", "--json"]);
    assert_eq!(check.code, 1);
    let data = check.json();
    let checks = data.get("checks").unwrap().as_array().unwrap();
    assert!(checks
        .iter()
        .any(|c| c.str("name") == Some("implement.plan-drift") && c.bool_at("ok") == Some(false)));
}

#[test]
fn amend_refuses_without_prior_approval_and_refuses_when_nothing_drifted() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    let started = gate(&repo, &["start", "no approval yet", "--json"]);
    let plan_path = started.json().str("plan").unwrap().to_string();
    write_file(&repo, &plan_path, &plan_with_files(&["a.txt"]));

    let before_approval = gate(&repo, &["amend"]);
    assert_ne!(before_approval.code, 0);
    assert!(before_approval.stderr.contains("never approved"));

    assert_eq!(gate(&repo, &["approve"]).code, 0);
    let no_drift = gate(&repo, &["amend"]);
    assert_ne!(no_drift.code, 0);
    assert!(no_drift.stderr.contains("nothing to amend"));
}

#[test]
fn approve_amend_refuses_without_gate_amend_having_recorded_intent_first() {
    let repo = make_repo(&[]);
    let (_run_id, plan_path) = setup_approved_run(&repo, &["a.txt"]);
    write_file(&repo, &plan_path, &plan_with_files(&["a.txt", "b.txt"]));

    let res = gate(&repo, &["approve", "--amend"]);
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains("gate amend"));
}

/// `gate amend`'s diff must show clean `a/plan.approved.md` / `b/plan.md`
/// unified-diff headers, not `git diff --no-index`'s mangled form for
/// absolute paths (the leading `/` silently dropped, so
/// `a//private/tmp/.../plan.approved.md` prints as
/// `a/tmp/.../plan.approved.md` - a header that looks like a real relative
/// path but isn't).
#[test]
fn amend_diff_headers_use_clean_relative_plan_paths_not_a_mangled_absolute_one() {
    let repo = make_repo(&[]);
    let (_run_id, plan_path) = setup_approved_run(&repo, &["a.txt"]);
    write_file(&repo, &plan_path, &plan_with_files(&["a.txt", "b.txt"]));

    let amend = gate(&repo, &["amend", "--json"]);
    assert_eq!(amend.code, 0);
    let diff = amend.json().str("diff").unwrap().to_string();
    assert!(diff.contains("a/plan.approved.md"));
    assert!(diff.contains("b/plan.md"));
    assert!(!diff.contains(repo.to_str().unwrap()));
}

#[test]
fn amend_then_approve_amend_goes_green() {
    let repo = make_repo(&[]);
    let (run_id, plan_path) = setup_approved_run(&repo, &["a.txt"]);
    assert_eq!(gate(&repo, &["next"]).code, 0); // PLAN -> IMPLEMENT

    write_file(&repo, &plan_path, &plan_with_files(&["a.txt", "b.txt"]));
    assert_eq!(gate(&repo, &["check"]).code, 1);

    let amend = gate(&repo, &["amend", "--json"]);
    assert_eq!(amend.code, 0);
    let amend_data = amend.json();
    assert_eq!(amend_data.bool_at("amended"), Some(true));
    assert!(amend_data.str("diff").unwrap().contains("b.txt"));

    let approve_amend = gate(&repo, &["approve", "--amend", "--by", "human"]);
    assert_eq!(approve_amend.code, 0);

    let after_amend = gate(&repo, &["check", "--json"]).json();
    let checks = after_amend.get("checks").unwrap().as_array().unwrap();
    assert!(!checks
        .iter()
        .any(|c| c.str("name") == Some("implement.plan-drift")));

    // Continue the flow with the newly widened scope - it goes all the way green.
    write_file(&repo, "a.txt", "hi\n");
    write_file(&repo, "b.txt", "hi\n");
    assert_eq!(gate(&repo, &["next"]).code, 0); // IMPLEMENT -> TEST

    let run_json = read_file(&repo, &format!(".gate/runs/{run_id}/run.json"));
    let run = common::json::parse(&run_json).unwrap();
    assert!(run.get("amendment").is_none());
}

#[test]
fn f2_regression_doctored_approved_plan_snapshot_never_trusted() {
    // Attack this review finding described: `plan.approved.md` is a plain
    // file on disk with no integrity check of its own - if something
    // rewrites it to match the *current* (drifted) plan.md exactly, a naive
    // diff against it shows "no changes", and a human re-approving on that
    // diff approves a scope change they never actually saw.
    let repo = make_repo(&[]);
    let (run_id, plan_path) = setup_approved_run(&repo, &["a.txt"]);
    assert_eq!(gate(&repo, &["next"]).code, 0); // PLAN -> IMPLEMENT

    let drifted_plan = plan_with_files(&["a.txt", "b.txt", "smuggled-secret-scope.txt"]);
    write_file(&repo, &plan_path, &drifted_plan);

    // Doctor the snapshot to match the drifted plan byte-for-byte, so an
    // untrusted diff would show nothing changed at all.
    let approved_snapshot_path = format!(".gate/runs/{run_id}/plan.approved.md");
    write_file(&repo, &approved_snapshot_path, &drifted_plan);

    let amend = gate(&repo, &["amend", "--json"]);
    assert_eq!(amend.code, 0);
    let amend_data = amend.json();
    assert_eq!(amend_data.bool_at("trustedSnapshot"), Some(false));
    // Never an empty/"no changes" diff against the doctored snapshot - the
    // full current plan is shown instead, so the smuggled scope is visible.
    assert!(amend_data
        .str("diff")
        .unwrap()
        .contains("smuggled-secret-scope.txt"));
    let warning = amend_data.str("warning").unwrap();
    assert!(warning.contains("WARNING"));
    assert!(warning
        .to_lowercase()
        .contains("does not match the recorded approval hash"));

    // The human-readable form surfaces the same warning, not just --json.
    let human_form = gate(&repo, &["amend"]);
    assert!(human_form.stdout.contains("WARNING"));
}

#[test]
fn editing_plan_again_after_amend_invalidates_that_amendment() {
    let repo = make_repo(&[]);
    let (_run_id, plan_path) = setup_approved_run(&repo, &["a.txt"]);
    assert_eq!(gate(&repo, &["next"]).code, 0);

    write_file(&repo, &plan_path, &plan_with_files(&["a.txt", "b.txt"]));
    assert_eq!(gate(&repo, &["amend"]).code, 0);

    write_file(
        &repo,
        &plan_path,
        &plan_with_files(&["a.txt", "b.txt", "c.txt"]),
    );
    let res = gate(&repo, &["approve", "--amend"]);
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains("gate amend"));
}
