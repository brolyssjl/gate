//! Conformance tests for the failure-streak cap (a loop-enforcement circuit
//! breaker). Not a TS port - no TS counterpart exists. See
//! `CONFORMANCE_MAP.md`.
//!
//! PLAN is used throughout as the phase under test: a freshly started run's
//! PLAN gate fails deterministically (never approved) with no build/lint/
//! test commands or `gate trust` involved, keeping these tests fast and
//! focused on the streak-cap behavior itself rather than gate mechanics
//! already covered elsewhere.

mod common;

use common::{gate, make_repo};

fn start_run(repo: &std::path::Path) {
    gate(repo, &["init"]);
    gate(repo, &["start", "streak demo"]);
}

#[test]
fn fails_three_times_then_blocks_then_skip_unblocks() {
    let repo = make_repo(&[]);
    start_run(&repo);

    for _ in 0..3 {
        assert_eq!(gate(&repo, &["check"]).code, 1);
    }

    let blocked = gate(&repo, &["check"]);
    assert_eq!(blocked.code, 3);
    assert!(blocked.stderr.contains("3 consecutive failures"));
    assert!(blocked.stderr.contains("gate skip"));
    assert!(blocked.stderr.contains("gate streak reset"));

    // `gate next` hits the identical cap, not just `gate check`.
    let blocked_next = gate(&repo, &["next"]);
    assert_eq!(blocked_next.code, 3);

    // Recovery path 1: an explicit, reasoned skip.
    let skip = gate(&repo, &["skip", "PLAN", "--reason", "moving on"]);
    assert_eq!(skip.code, 0);

    // The next phase starts with a clean streak.
    assert_eq!(
        gate(&repo, &["check", "--json"]).json().str("phase"),
        Some("IMPLEMENT")
    );
}

#[test]
fn streak_reset_unblocks_without_skipping_the_phase() {
    let repo = make_repo(&[]);
    start_run(&repo);

    for _ in 0..3 {
        assert_eq!(gate(&repo, &["check"]).code, 1);
    }
    assert_eq!(gate(&repo, &["check"]).code, 3);

    let reset = gate(
        &repo,
        &["streak", "reset", "--reason", "reviewed, retry warranted"],
    );
    assert_eq!(reset.code, 0);

    // Back to evaluating for real - still PLAN, still unapproved, so it
    // fails again rather than being silently treated as a pass.
    assert_eq!(gate(&repo, &["check"]).code, 1);
}

#[test]
fn a_blocked_phase_keeps_reporting_the_same_streak_not_growing_past_the_limit() {
    let repo = make_repo(&[]);
    start_run(&repo);

    for _ in 0..3 {
        gate(&repo, &["check"]);
    }
    for _ in 0..3 {
        let blocked = gate(&repo, &["check"]);
        assert_eq!(blocked.code, 3);
        assert!(blocked.stderr.contains("3 consecutive failures"));
    }
}

#[test]
fn streak_reset_refuses_when_there_is_nothing_to_reset() {
    let repo = make_repo(&[]);
    start_run(&repo);
    let res = gate(&repo, &["streak", "reset", "--reason", "x"]);
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains("no failure streak to reset"));
}

#[test]
fn streak_reset_requires_a_reason() {
    let repo = make_repo(&[]);
    start_run(&repo);
    gate(&repo, &["check"]);
    let res = gate(&repo, &["streak", "reset"]);
    assert_eq!(res.code, 2);
}

#[test]
fn a_passing_gate_never_blocks_no_matter_how_many_times_it_is_checked() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    let started = gate(&repo, &["start", "always green", "--json"]);
    let plan_path = started.json().str("plan").unwrap().to_string();
    common::write_file(
        &repo,
        &plan_path,
        "---\ngoal: g\nfiles:\n  - a.txt\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n",
    );
    assert_eq!(gate(&repo, &["approve", "--by", "human"]).code, 0);

    for _ in 0..5 {
        assert_eq!(gate(&repo, &["check"]).code, 0);
    }
}

#[test]
fn config_zero_disables_the_cap_entirely() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    common::write_file(
        &repo,
        ".gate/config.yml",
        "thresholds:\n  failure_streak_limit: 0\n",
    );
    gate(&repo, &["start", "no cap"]);

    for _ in 0..6 {
        assert_eq!(gate(&repo, &["check"]).code, 1);
    }
}

#[test]
fn config_supports_a_custom_limit() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    common::write_file(
        &repo,
        ".gate/config.yml",
        "thresholds:\n  failure_streak_limit: 2\n",
    );
    gate(&repo, &["start", "tighter cap"]);

    assert_eq!(gate(&repo, &["check"]).code, 1);
    assert_eq!(gate(&repo, &["check"]).code, 1);
    let blocked = gate(&repo, &["check"]);
    assert_eq!(blocked.code, 3);
    assert!(blocked.stderr.contains("2 consecutive failures"));
}

#[test]
fn streak_show_reports_the_current_state_as_json() {
    let repo = make_repo(&[]);
    start_run(&repo);
    gate(&repo, &["check"]);
    gate(&repo, &["check"]);

    let shown = gate(&repo, &["streak", "--json"]);
    assert_eq!(shown.code, 0);
    let data = shown.json();
    assert_eq!(data.str("phase"), Some("PLAN"));
    assert_eq!(
        data.get("streaks").unwrap().get("PLAN").unwrap().as_i64(),
        Some(2)
    );
    assert_eq!(data.get("limit").unwrap().as_i64(), Some(3));
}

/// Once a run reaches DONE, `advance` clears its branch's "current run"
/// pointer (`clear_run_everywhere`) - `gate streak` used to rely entirely on
/// that pointer, so it could show nothing at all for a run that had just
/// finished. It should instead report the finished run's final state.
#[test]
fn streak_show_reports_a_finished_runs_final_state_instead_of_nothing() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    let started = gate(
        &repo,
        &["start", "docs demo", "--profile", "docs", "--json"],
    );
    let plan_path = started.json().str("plan").unwrap().to_string();
    let run_id = started.json().str("id").unwrap().to_string();
    common::write_file(
        &repo,
        &plan_path,
        "---\ngoal: g\nfiles:\n  - a.txt\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n",
    );
    assert_eq!(gate(&repo, &["approve", "--by", "human"]).code, 0);
    assert_eq!(gate(&repo, &["next"]).code, 0); // PLAN -> IMPLEMENT

    common::write_file(&repo, "a.txt", "docs change\n");
    let advanced = gate(&repo, &["next", "--json"]);
    assert_eq!(advanced.code, 0);
    assert_eq!(
        advanced.json().get("done").and_then(|v| v.as_bool()),
        Some(true)
    );

    // No `--run`: resolved via the fallback (no active run left for the
    // branch once DONE cleared the pointer).
    let shown = gate(&repo, &["streak", "--json"]);
    assert_eq!(shown.code, 0);
    let data = shown.json();
    assert_eq!(data.str("phase"), Some("DONE"));
    assert_eq!(data.str("status"), Some("done"));
    assert_eq!(
        data.get("streaks").unwrap().get("PLAN").unwrap().as_i64(),
        Some(0)
    );
    let shown_human = gate(&repo, &["streak"]);
    assert!(shown_human.stdout.contains("status: done"));

    // Explicit `--run <finished-id>` also works, unlike `require_active_run`.
    let shown_explicit = gate(&repo, &["streak", "--run", &run_id, "--json"]);
    assert_eq!(shown_explicit.code, 0);
    assert_eq!(shown_explicit.json().str("status"), Some("done"));
}

#[test]
fn a_usage_error_never_counts_as_a_failure() {
    let repo = make_repo(&[]);
    start_run(&repo);

    // A bogus --run id is a usage/lookup error, not a gate evaluation.
    let bad = gate(&repo, &["check", "--run", "nope"]);
    assert_ne!(bad.code, 0);
    assert_ne!(bad.code, 3);

    let shown = gate(&repo, &["streak", "--json"]).json();
    assert_eq!(
        shown.get("streaks").unwrap().get("PLAN").unwrap().as_i64(),
        Some(0)
    );
}
