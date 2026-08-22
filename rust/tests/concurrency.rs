//! Port of `test/concurrency.test.ts` ("branch-keyed run concurrency").
//! See `CONFORMANCE_MAP.md` for the TS case -> Rust test fn mapping.

mod common;

use common::json::Value;
use common::{gate, gate_spawn, git_out, make_repo, make_temp_dir, read_file, wait, write_file};
use std::path::Path;

fn package_json_files() -> Vec<(&'static str, &'static str)> {
    vec![(
        "package.json",
        "{\"name\":\"fx\",\"scripts\":{\"test\":\"node -e 0\"}}",
    )]
}

/// Raw schema-2 `run.json` body, matching the literal the TS suite's local
/// `seedLegacyRun` helper writes (pre-Milestone-4 shape: no `branch` field).
fn legacy_run_json(id: &str, status: &str) -> String {
    format!(
        "{{\"schema\":2,\"id\":\"{id}\",\"title\":\"{id}\",\"profile\":\"docs\",\"phase\":\"PLAN\",\
\"status\":\"{status}\",\"createdAt\":\"2026-01-01T00:00:00.000Z\",\
\"updatedAt\":\"2026-01-01T00:00:00.000Z\",\"baseRef\":null,\"sessionId\":null,\
\"history\":[{{\"phase\":\"PLAN\",\"event\":\"entered\",\"at\":\"2026-01-01T00:00:00.000Z\"}}],\
\"overrides\":[],\"artifacts\":{{}}}}"
    )
}

/// Write a minimal, valid, active schema-2 run.json directly (pre-Milestone-4).
/// Port of `concurrency.test.ts`'s local `seedLegacyRun`.
fn seed_legacy_run(repo: &Path, id: &str) {
    write_file(
        repo,
        &format!(".gate/runs/{id}/run.json"),
        &legacy_run_json(id, "active"),
    );
}

fn path_exists(repo: &Path, rel: &str) -> bool {
    repo.join(rel).exists()
}

#[test]
fn keys_runs_by_branch_independent_runs_on_separate_branches_status_shows_the_current_one_plus_others_in_flight(
) {
    let repo = make_repo(&package_json_files());
    gate(&repo, &["init"]);
    gate(&repo, &["trust"]);
    let main_branch = git_out(&repo, &["branch", "--show-current"]);

    let started = gate(
        &repo,
        &["start", "run on main", "--profile", "docs", "--json"],
    );
    assert_eq!(started.code, 0);
    let main_run = started.json();
    let main_run_id = main_run.str("id").unwrap().to_string();
    assert_eq!(main_run.str("branch"), Some(main_branch.as_str()));

    git_out(&repo, &["checkout", "-b", "other-branch"]);
    let started_other = gate(
        &repo,
        &[
            "start",
            "run on other branch",
            "--profile",
            "docs",
            "--json",
        ],
    );
    assert_eq!(started_other.code, 0);
    let other_run = started_other.json();
    let other_run_id = other_run.str("id").unwrap().to_string();
    assert_eq!(other_run.str("branch"), Some("other-branch"));
    assert_ne!(other_run_id, main_run_id);

    let status_other = gate(&repo, &["status", "--json"]).json();
    assert_eq!(status_other.str("id"), Some(other_run_id.as_str()));
    assert_eq!(status_other.str("branch"), Some("other-branch"));
    let others = status_other.get("others").unwrap().as_array().unwrap();
    assert_eq!(others.len(), 1);
    assert_eq!(others[0].str("branch"), Some(main_branch.as_str()));
    assert_eq!(others[0].str("id"), Some(main_run_id.as_str()));
    assert_eq!(others[0].str("phase"), Some("PLAN"));

    git_out(&repo, &["checkout", &main_branch]);
    let status_main = gate(&repo, &["status", "--json"]).json();
    assert_eq!(status_main.str("id"), Some(main_run_id.as_str()));
    assert_eq!(status_main.str("branch"), Some(main_branch.as_str()));
}

#[test]
fn gate_start_resumes_a_branchs_existing_active_run_instead_of_erroring() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    let first = gate(
        &repo,
        &["start", "first title", "--profile", "docs", "--json"],
    )
    .json();
    let first_id = first.str("id").unwrap().to_string();

    let second = gate(
        &repo,
        &["start", "a different title", "--profile", "docs", "--json"],
    );
    assert_eq!(second.code, 0);
    let resumed = second.json();
    assert_eq!(resumed.bool_at("resumed"), Some(true));
    assert_eq!(resumed.str("id"), Some(first_id.as_str()));
}

#[test]
fn gate_start_refuses_to_resume_on_an_explicit_profile_conflict_instead_of_silently_discarding_it()
{
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    gate(&repo, &["start", "first title", "--profile", "docs"]);

    let res = gate(&repo, &["start", "second title", "--profile", "bugfix"]);
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains("docs"));
    assert!(res.stderr.contains("bugfix"));
}

#[test]
fn gate_start_refuses_to_resume_on_an_explicit_target_conflict_instead_of_silently_discarding_it() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    let config_path = ".gate/config.yml";
    let existing = read_file(&repo, config_path);
    write_file(
        &repo,
        config_path,
        &format!("{existing}\ntargets:\n  api:\n    match: [\"apps/api/**\"]\n  web:\n    match: [\"apps/web/**\"]\n"),
    );

    gate(
        &repo,
        &[
            "start",
            "first title",
            "--profile",
            "docs",
            "--target",
            "api",
        ],
    );
    let res = gate(
        &repo,
        &[
            "start",
            "second title",
            "--profile",
            "docs",
            "--target",
            "web",
        ],
    );
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains("api"));
    assert!(res.stderr.contains("web"));
}

#[test]
fn gate_start_warns_instead_of_silently_discarding_a_title_mismatch_when_resuming_title_alone_never_blocks(
) {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    let first = gate(
        &repo,
        &["start", "original title", "--profile", "docs", "--json"],
    )
    .json();
    let first_id = first.str("id").unwrap().to_string();

    let json_res = gate(
        &repo,
        &[
            "start",
            "a totally different title",
            "--profile",
            "docs",
            "--json",
        ],
    );
    assert_eq!(json_res.code, 0);
    let data = json_res.json();
    assert_eq!(data.bool_at("resumed"), Some(true));
    assert_eq!(data.bool_at("titleMismatch"), Some(true));
    assert_eq!(
        data.str("requestedTitle"),
        Some("a totally different title")
    );
    assert_eq!(data.str("id"), Some(first_id.as_str()));

    let human_res = gate(&repo, &["start", "yet another title", "--profile", "docs"]);
    assert!(human_res.stdout.contains("WARNING"));
}

#[test]
fn gate_start_refuses_to_resume_when_the_plan_changed_since_approval() {
    let repo = make_repo(&package_json_files());
    gate(&repo, &["init"]);
    gate(&repo, &["trust"]);
    let started = gate(&repo, &["start", "needs a plan", "--json"]).json();
    let plan_path = started.str("plan").unwrap().to_string();

    let plan_content = [
        "---",
        "goal: ship it",
        "files:",
        "  - a.txt",
        "criteria:",
        "  - id: c1",
        "    text: it works",
        "    verify: \"manual\"",
        "---",
        "",
    ]
    .join("\n");
    write_file(&repo, &plan_path, &plan_content);
    assert_eq!(gate(&repo, &["approve", "--by", "tester"]).code, 0);

    // Edit the plan post-approval without re-approving.
    write_file(&repo, &plan_path, &format!("{plan_content}\nextra line\n"));

    let retry = gate(&repo, &["start", "needs a plan again"]);
    assert_ne!(retry.code, 0);
    assert!(retry.stderr.contains("changed since"));
}

#[test]
fn refuses_to_start_on_a_detached_head_no_branch_to_key_the_run_by() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    let sha = git_out(&repo, &["rev-parse", "HEAD"]);
    git_out(&repo, &["checkout", &sha]);

    let res = gate(&repo, &["start", "detached attempt"]);
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains("detached"));
}

#[test]
fn detached_head_gate_status_reports_it_without_throwing_phase_commands_require_run() {
    let repo = make_repo(&package_json_files());
    gate(&repo, &["init"]);
    gate(&repo, &["trust"]);
    let started = gate(&repo, &["start", "run before detaching", "--json"]).json();
    let run_id = started.str("id").unwrap().to_string();

    let sha = git_out(&repo, &["rev-parse", "HEAD"]);
    git_out(&repo, &["checkout", &sha]);

    let status = gate(&repo, &["status", "--json"]).json();
    assert_eq!(status.bool_at("active"), Some(false));
    assert_eq!(status.bool_at("detached"), Some(true));

    let without_run = gate(&repo, &["check"]);
    assert_ne!(without_run.code, 0);
    assert!(without_run.stderr.contains("detached"));

    let with_run = gate(&repo, &["check", "--run", &run_id]);
    assert!(!with_run.stderr.contains("detached"));
}

#[test]
fn detached_head_gate_report_with_no_run_id_refuses_instead_of_silently_describing_another_branchs_run(
) {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    gate(
        &repo,
        &["start", "run before detaching", "--profile", "docs"],
    );
    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();

    git_out(&repo, &["checkout", "-b", "other-branch"]);
    gate(
        &repo,
        &[
            "start",
            "an unrelated run on another branch",
            "--profile",
            "docs",
        ],
    );

    let sha = git_out(&repo, &["rev-parse", "HEAD"]);
    git_out(&repo, &["checkout", &sha]);

    let without_id = gate(&repo, &["report"]);
    assert_ne!(without_id.code, 0);
    assert!(without_id.stderr.contains("detached"));

    // Explicit run id still works on a detached HEAD.
    let with_id = gate(&repo, &["report", &run_id, "--json"]).json();
    assert_eq!(with_id.str("id"), Some(run_id.as_str()));
}

#[test]
fn run_refuses_a_non_active_done_abandoned_run_instead_of_letting_phase_gates_pass_vacuously() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    // schema 2, status: active by default - override to "done" below.
    write_file(
        &repo,
        ".gate/runs/finished-run/run.json",
        &legacy_run_json("finished-run", "done"),
    );

    let res = gate(&repo, &["check", "--run", "finished-run"]);
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains("not active"));
}

#[test]
fn run_refuses_a_run_whose_recorded_branch_doesnt_match_the_checked_out_branch_would_otherwise_diff_against_the_wrong_tree(
) {
    let repo = make_repo(&package_json_files());
    gate(&repo, &["init"]);
    gate(&repo, &["trust"]);
    let main_branch = git_out(&repo, &["branch", "--show-current"]);
    let started = gate(
        &repo,
        &["start", "run on main", "--profile", "docs", "--json"],
    )
    .json();
    let run_id = started.str("id").unwrap().to_string();

    git_out(&repo, &["checkout", "-b", "other-branch"]);
    let res = gate(&repo, &["check", "--run", &run_id]);
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains(&main_branch));
    assert!(res.stderr.contains("other-branch"));

    // Checking back out to the run's own branch, --run works again.
    git_out(&repo, &["checkout", &main_branch]);
    let ok = gate(&repo, &["check", "--run", &run_id]);
    assert!(!ok.stderr.contains("checked out"));
}

#[test]
fn migrates_the_legacy_single_run_gate_current_pointer_into_per_branch_current_json() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    seed_legacy_run(&repo, "legacy-run");
    let branch = git_out(&repo, &["branch", "--show-current"]);
    write_file(&repo, ".gate/current", "legacy-run\n");

    let status = gate(&repo, &["status", "--json"]).json();
    assert_eq!(status.bool_at("active"), Some(true));
    assert_eq!(status.str("id"), Some("legacy-run"));

    assert!(!path_exists(&repo, ".gate/current"));
    let migrated = common::json::parse(&read_file(&repo, ".gate/current.json")).unwrap();
    let branches = migrated.get("branches").unwrap();
    assert_eq!(branches.str(&branch), Some("legacy-run"));
}

#[test]
fn finishing_a_migrated_legacy_run_backfills_its_branch_and_clears_its_current_json_mapping_on_done(
) {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    seed_legacy_run(&repo, "legacy-run"); // schema 2, profile docs, phase PLAN
    let branch = git_out(&repo, &["branch", "--show-current"]);
    write_file(&repo, ".gate/current", "legacy-run\n");

    // Trigger the migration.
    assert_eq!(
        gate(&repo, &["status", "--json"]).json().str("id"),
        Some("legacy-run")
    );

    // Backfilled: the migrated run now records the branch it was filed under
    // (schema-2->3 migration alone leaves `branch: null` - it has no way to
    // infer it - so without this the terminal-advance cleanup below would
    // have nothing correct to trust from run.branch).
    let migrated_run =
        common::json::parse(&read_file(&repo, ".gate/runs/legacy-run/run.json")).unwrap();
    assert_eq!(migrated_run.str("branch"), Some(branch.as_str()));

    // Walk it to DONE (docs profile: PLAN -> IMPLEMENT -> DONE).
    assert_eq!(gate(&repo, &["skip", "PLAN", "--reason", "test"]).code, 0);
    assert_eq!(
        gate(&repo, &["skip", "IMPLEMENT", "--reason", "test"]).code,
        0
    );
    assert_eq!(
        gate(&repo, &["report", "legacy-run", "--json"])
            .json()
            .str("status"),
        Some("done")
    );

    // The mapping must be cleared on the mechanism level (scanning for the
    // run id), not by trusting run.branch - status must not keep resolving
    // the finished run for this branch.
    let final_status = gate(&repo, &["status", "--json"]).json();
    assert_eq!(final_status.bool_at("active"), Some(false));
    let state = common::json::parse(&read_file(&repo, ".gate/current.json")).unwrap();
    let branches = state.get("branches").unwrap();
    assert!(branches.get(&branch).is_none());
}

#[test]
fn resolves_the_branch_name_on_an_unborn_branch_git_init_zero_commits_yet_distinct_from_detached_head(
) {
    // Not make_repo() - that helper always commits. A fresh `git init` repo
    // has no commits, so `git rev-parse --abbrev-ref HEAD` fails exactly as
    // it does on a real detached HEAD; this must still resolve the branch
    // name.
    let repo = make_temp_dir("gate-unborn");
    git_out(&repo, &["init", "-q"]);
    gate(&repo, &["init"]);

    let started = gate(&repo, &["start", "first run ever", "--json"]);
    assert_eq!(started.code, 0);
    let data = started.json();
    assert!(!data.get("branch").map(Value::is_null).unwrap_or(true));

    let status = gate(&repo, &["status", "--json"]).json();
    let detached = status
        .get("detached")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    assert!(!detached);
    assert_eq!(status.bool_at("active"), Some(true));
}

#[test]
fn works_with_no_git_repo_at_all_a_single_implicit_key_no_branch_ambiguity() {
    // Not make_repo() - a plain directory with no `git init`, exercising the
    // NO_GIT_BRANCH_KEY path (distinct from detached HEAD, which *is* a repo).
    let repo = make_temp_dir("gate-no-git");
    gate(&repo, &["init"]);

    let started = gate(
        &repo,
        &["start", "no-git run", "--profile", "docs", "--json"],
    );
    assert_eq!(started.code, 0);
    let started_data = started.json();
    assert!(started_data
        .get("branch")
        .map(Value::is_null)
        .unwrap_or(false));
    let started_id = started_data.str("id").unwrap().to_string();

    let status = gate(&repo, &["status", "--json"]).json();
    assert_eq!(status.bool_at("active"), Some(true));
    assert_eq!(status.str("id"), Some(started_id.as_str()));
    assert!(status.get("branch").map(Value::is_null).unwrap_or(false));

    // A second `gate start` on the same (non-)branch resumes rather than erroring.
    let again = gate(&repo, &["start", "different title", "--json"]).json();
    assert_eq!(again.bool_at("resumed"), Some(true));
    assert_eq!(again.str("id"), Some(started_id.as_str()));
}

#[test]
fn fails_closed_on_a_corrupt_current_json_instead_of_silently_discarding_every_branchs_mapping() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    gate(&repo, &["start", "will be corrupted", "--profile", "docs"]);
    write_file(&repo, ".gate/current.json", "{ not: valid json");

    let status = gate(&repo, &["status"]);
    assert_ne!(status.code, 0);
    assert!(status.stderr.contains("corrupt"));
    assert!(status.stderr.contains("current.json"));
}

#[test]
/// With the whole read-decide-write sequence under one lock, every process
/// but the first to acquire it must see the first's write and resume -
/// never two winners, never a lost mapping.
fn serializes_concurrent_gate_start_on_the_same_branch_exactly_one_run_wins_current_json_never_corrupts(
) {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    let branch = git_out(&repo, &["branch", "--show-current"]);

    const N: usize = 8;
    let titles: Vec<String> = (0..N).map(|i| format!("concurrent run {i}")).collect();
    let mut children = Vec::with_capacity(N);
    for title in &titles {
        children.push(gate_spawn(
            &repo,
            &["start", title, "--profile", "docs", "--json"],
        ));
    }
    let results: Vec<_> = children.into_iter().map(wait).collect();
    assert!(results.iter().all(|r| r.code == 0));

    let parsed: Vec<Value> = results.iter().map(|r| r.json()).collect();
    let created: Vec<&Value> = parsed
        .iter()
        .filter(|p| p.bool_at("resumed") != Some(true))
        .collect();
    let resumed: Vec<&Value> = parsed
        .iter()
        .filter(|p| p.bool_at("resumed") == Some(true))
        .collect();
    assert_eq!(created.len(), 1);
    assert_eq!(resumed.len(), N - 1);
    let winning_id = created[0].str("id").unwrap().to_string();
    assert!(parsed
        .iter()
        .all(|p| p.str("id") == Some(winning_id.as_str())));

    let state = common::json::parse(&read_file(&repo, ".gate/current.json")).unwrap();
    let branches = state.get("branches").unwrap();
    assert_eq!(branches.str(&branch), Some(winning_id.as_str()));
    assert_eq!(branches.sorted_keys(), vec![branch.clone()]);
}

#[test]
fn gate_status_degrades_gracefully_and_self_heals_on_a_dangling_mapping_instead_of_crashing_with_run_not_found(
) {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    let started = gate(
        &repo,
        &["start", "will vanish", "--profile", "docs", "--json"],
    )
    .json();
    let run_id = started.str("id").unwrap().to_string();

    // Simulate the mapping outliving its run (e.g. the folder was pruned or
    // removed by hand) without going through `gate prune`.
    std::fs::remove_dir_all(repo.join(".gate/runs").join(&run_id)).unwrap();

    let status = gate(&repo, &["status", "--json"]);
    assert_eq!(status.code, 0);
    let data = status.json();
    assert_eq!(data.bool_at("active"), Some(false));
    assert_eq!(data.str("healed"), Some(run_id.as_str()));

    // Self-healed: the stale mapping is gone, not just papered over for one call.
    let state = common::json::parse(&read_file(&repo, ".gate/current.json")).unwrap();
    let branches = state.get("branches").unwrap().as_object().unwrap();
    assert!(!branches
        .iter()
        .any(|(_, v)| v.as_str() == Some(run_id.as_str())));

    let again = gate(&repo, &["status", "--json"]).json();
    assert_eq!(again.bool_at("active"), Some(false));
    assert!(again.get("healed").is_none());
}
