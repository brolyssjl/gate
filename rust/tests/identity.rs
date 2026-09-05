//! Conformance tests for identity fallback and `identity.require_identity`
//! (issue #29, "Identity hardening: trust/approve/override accept
//! identity-less sign-offs"). Not a TS port - no TS counterpart exists; see
//! `core::identity` and the README's "Identity fallback" section.
//!
//! `make_repo` seeds a repo-local `git config user.name "test"` (see
//! `common::make_repo`), which the git-fallback cases below rely on. Cases
//! that need NO identity to resolve at all isolate `$HOME` and (git >=
//! 2.32) `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` for the `gate` subprocess,
//! so a real `~/.gitconfig` on the machine running the suite can't leak a
//! name in and make the test non-hermetic.

mod common;

use common::{gate, gate_opts, git_run, make_repo, make_temp_dir, write_file, GateOpts};

/// Unset the repo-local `user.name` `make_repo` seeds, so a test that needs
/// "no identity resolves at all" isn't defeated by the local git config
/// that's always consulted regardless of `$HOME`.
fn unset_local_git_user_name(repo: &std::path::Path) {
    let out = git_run(repo, &["config", "--unset", "user.name"], &[]);
    assert_eq!(out.code, 0, "failed to unset user.name: {}", out.stderr);
}

const MIN_PLAN: &str = "---\ngoal: g\nfiles:\n  - a.ts\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan\n";

fn run_id(repo: &std::path::Path) -> String {
    gate(repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string()
}

fn start_approvable_run(repo: &std::path::Path) -> String {
    gate(repo, &["init", "--no-adapt"]);
    gate(repo, &["trust"]);
    gate(repo, &["start", "identity demo"]);
    let id = run_id(repo);
    write_file(repo, &format!(".gate/runs/{id}/plan.md"), MIN_PLAN);
    id
}

fn read_run_json(repo: &std::path::Path, id: &str) -> common::json::Value {
    common::json::parse(&common::read_file(
        repo,
        &format!(".gate/runs/{id}/run.json"),
    ))
    .unwrap()
}

// ---- git user.name fallback, one per command --------------------------

#[test]
fn trust_records_by_from_git_user_name_when_flag_and_env_are_absent() {
    let repo = make_repo(&[]); // local `git config user.name` = "test"
    gate(&repo, &["init", "--no-adapt"]);
    let trusted = gate(&repo, &["trust", "--json"]);
    assert_eq!(trusted.code, 0);
    assert_eq!(trusted.json().str("trustedBy"), Some("test"));
}

#[test]
fn approve_records_by_from_git_user_name_when_flag_and_env_are_absent() {
    let repo = make_repo(&[]);
    start_approvable_run(&repo);
    let approved = gate(&repo, &["approve", "--json"]);
    assert_eq!(approved.code, 0);
    assert_eq!(approved.json().str("by"), Some("test"));
}

#[test]
fn streak_reset_records_by_from_git_user_name_when_flag_and_env_are_absent() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["start", "identity demo"]);
    let id = run_id(&repo);
    assert_eq!(gate(&repo, &["check"]).code, 1); // build one failure to reset

    let reset = gate(&repo, &["streak", "reset", "--reason", "reviewed"]);
    assert_eq!(reset.code, 0);

    let run = read_run_json(&repo, &id);
    let overrides = run.get("overrides").unwrap().as_array().unwrap();
    assert_eq!(
        overrides.last().unwrap().get("by").unwrap().as_str(),
        Some("test")
    );
}

// ---- precedence: --by > GATE_SESSION_ID > git user.name ---------------

#[test]
fn by_flag_wins_over_gate_session_id_and_git_user_name() {
    let repo = make_repo(&[]); // git user.name = "test"
    gate(&repo, &["init", "--no-adapt"]);
    let trusted = gate_opts(
        &repo,
        &["trust", "--by", "alice", "--json"],
        GateOpts {
            env: &[("GATE_SESSION_ID", "session-bob")],
            input: None,
        },
    );
    assert_eq!(trusted.code, 0);
    assert_eq!(trusted.json().str("trustedBy"), Some("alice"));
}

#[test]
fn gate_session_id_wins_over_git_user_name_when_by_flag_is_absent() {
    let repo = make_repo(&[]); // git user.name = "test"
    gate(&repo, &["init", "--no-adapt"]);
    let trusted = gate_opts(
        &repo,
        &["trust", "--json"],
        GateOpts {
            env: &[("GATE_SESSION_ID", "session-bob")],
            input: None,
        },
    );
    assert_eq!(trusted.code, 0);
    assert_eq!(trusted.json().str("trustedBy"), Some("session-bob"));
}

// ---- identity.require_identity ------------------------------------------

#[test]
fn require_identity_fails_trust_when_no_identity_resolves_at_all() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    write_file(
        &repo,
        ".gate/config.yml",
        "identity:\n  require_identity: true\ncommands: {}\n",
    );
    unset_local_git_user_name(&repo);
    let home = make_temp_dir("gate-identity-home");
    let home_str = home.to_str().unwrap().to_string();

    let refused = gate_opts(
        &repo,
        &["trust"],
        GateOpts {
            env: &[
                ("HOME", &home_str),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ],
            input: None,
        },
    );
    assert_eq!(refused.code, 1);
    assert!(refused.stderr.contains("identity.require_identity"));
    assert!(!std::path::Path::new(&repo)
        .join(".gate/trust.json")
        .exists());
}

#[test]
fn require_identity_fails_approve_when_no_identity_resolves_at_all() {
    let repo = make_repo(&[]);
    let id = start_approvable_run(&repo);
    write_file(
        &repo,
        ".gate/config.yml",
        "identity:\n  require_identity: true\ncommands: {}\n",
    );
    unset_local_git_user_name(&repo);
    let home = make_temp_dir("gate-identity-home");
    let home_str = home.to_str().unwrap().to_string();

    let refused = gate_opts(
        &repo,
        &["approve"],
        GateOpts {
            env: &[
                ("HOME", &home_str),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ],
            input: None,
        },
    );
    assert_eq!(refused.code, 1);
    assert!(refused.stderr.contains("identity.require_identity"));

    let run = read_run_json(&repo, &id);
    assert!(run.get("approval").map(|v| v.is_null()).unwrap_or(true));
}

#[test]
fn require_identity_fails_streak_reset_when_no_identity_resolves_at_all() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["start", "identity demo"]);
    for _ in 0..3 {
        gate(&repo, &["check"]);
    }
    assert_eq!(gate(&repo, &["check"]).code, 3); // blocked, ready for `streak reset`
    write_file(
        &repo,
        ".gate/config.yml",
        "identity:\n  require_identity: true\ncommands: {}\n",
    );
    unset_local_git_user_name(&repo);
    let home = make_temp_dir("gate-identity-home");
    let home_str = home.to_str().unwrap().to_string();

    let refused = gate_opts(
        &repo,
        &["streak", "reset", "--reason", "reviewed"],
        GateOpts {
            env: &[
                ("HOME", &home_str),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ],
            input: None,
        },
    );
    assert_eq!(refused.code, 1);
    assert!(refused.stderr.contains("identity.require_identity"));
    // Still blocked - the streak was never actually cleared.
    assert_eq!(gate(&repo, &["check"]).code, 3);
}

#[test]
fn require_identity_off_by_default_so_no_identity_still_succeeds() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]); // no identity: config, default false
    unset_local_git_user_name(&repo);
    let home = make_temp_dir("gate-identity-home");
    let home_str = home.to_str().unwrap().to_string();

    let trusted = gate_opts(
        &repo,
        &["trust", "--json"],
        GateOpts {
            env: &[
                ("HOME", &home_str),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ],
            input: None,
        },
    );
    assert_eq!(trusted.code, 0);
    assert!(trusted.json().get("trustedBy").unwrap().is_null());
}

#[test]
fn require_identity_still_passes_when_git_user_name_alone_resolves() {
    let repo = make_repo(&[]); // git user.name = "test", no isolation here
    gate(&repo, &["init", "--no-adapt"]);
    write_file(
        &repo,
        ".gate/config.yml",
        "identity:\n  require_identity: true\ncommands: {}\n",
    );
    let trusted = gate(&repo, &["trust", "--json"]);
    assert_eq!(trusted.code, 0);
    assert_eq!(trusted.json().str("trustedBy"), Some("test"));
}

// ---- issue #33: skip/review/start/amend join the fallback chain -------

#[test]
fn skip_records_by_from_git_user_name_when_flag_and_env_are_absent() {
    let repo = make_repo(&[]); // local `git config user.name` = "test"
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["start", "identity demo"]);
    let id = run_id(&repo);

    let skipped = gate(&repo, &["skip", "PLAN", "--reason", "trivial"]);
    assert_eq!(skipped.code, 0);

    let run = read_run_json(&repo, &id);
    let overrides = run.get("overrides").unwrap().as_array().unwrap();
    assert_eq!(overrides[0].get("by").unwrap().as_str(), Some("test"));
}

#[test]
fn skip_fails_when_require_identity_and_nothing_resolves() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["start", "identity demo"]);
    let id = run_id(&repo);
    write_file(
        &repo,
        ".gate/config.yml",
        "identity:\n  require_identity: true\ncommands: {}\n",
    );
    unset_local_git_user_name(&repo);
    let home = make_temp_dir("gate-identity-home");
    let home_str = home.to_str().unwrap().to_string();

    let refused = gate_opts(
        &repo,
        &["skip", "PLAN", "--reason", "trivial"],
        GateOpts {
            env: &[
                ("HOME", &home_str),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ],
            input: None,
        },
    );
    assert_eq!(refused.code, 1);
    assert!(refused.stderr.contains("identity.require_identity"));

    // The skip never landed: no override recorded, phase unchanged.
    let run = read_run_json(&repo, &id);
    assert!(run.get("overrides").unwrap().as_array().unwrap().is_empty());
    assert_eq!(run.get("phase").unwrap().as_str(), Some("PLAN"));
}

#[test]
fn amend_records_by_from_git_user_name_when_flag_and_env_are_absent() {
    let repo = make_repo(&[]);
    let id = start_approvable_run(&repo);
    assert_eq!(gate(&repo, &["approve"]).code, 0);
    write_file(
        &repo,
        &format!(".gate/runs/{id}/plan.md"),
        &format!("{MIN_PLAN}\nextra detail\n"),
    );

    let amended = gate(&repo, &["amend"]);
    assert_eq!(amended.code, 0);

    let run = read_run_json(&repo, &id);
    let amendment = run.get("amendment").unwrap();
    assert_eq!(amendment.get("by").unwrap().as_str(), Some("test"));
}

#[test]
fn review_records_requested_by_from_git_user_name_when_flag_and_env_are_absent() {
    let repo = make_repo(&[]);
    write_file(&repo, "run-tests.sh", "#!/bin/sh\nexit 0\n");
    let _ = std::process::Command::new("chmod")
        .args(["+x", "run-tests.sh"])
        .current_dir(&repo)
        .status();
    gate(&repo, &["init", "--no-adapt"]);
    let config_path = repo.join(".gate").join("config.yml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    std::fs::write(
        &config_path,
        config.replace("# test: \"<command>\"", "test: \"./run-tests.sh\""),
    )
    .unwrap();
    gate(&repo, &["trust"]);
    gate(&repo, &["start", "identity demo"]);
    let id = run_id(&repo);
    // MIN_PLAN plus run-tests.sh, which sits untracked in the working
    // tree and must be declared or the IMPLEMENT scope check rejects it.
    write_file(
        &repo,
        &format!(".gate/runs/{id}/plan.md"),
        "---\ngoal: g\nfiles:\n  - a.ts\n  - run-tests.sh\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan\n",
    );
    gate(&repo, &["approve"]);
    gate(&repo, &["next"]); // -> IMPLEMENT
    write_file(&repo, "a.ts", "export {};\n");
    gate(&repo, &["next"]); // -> TEST
    gate(&repo, &["next"]); // -> REVIEW

    let reviewed = gate(&repo, &["review"]);
    assert_eq!(reviewed.code, 0, "stderr: {}", reviewed.stderr);

    let run = read_run_json(&repo, &id);
    let review = run.get("review").unwrap();
    assert_eq!(review.get("requestedBy").unwrap().as_str(), Some("test"));
}

// ---- issue #33: start records startedBy, sessionId stays strict -------

#[test]
fn start_records_started_by_but_not_session_id_from_git_user_name() {
    let repo = make_repo(&[]); // git user.name = "test"
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["start", "identity demo"]);
    let id = run_id(&repo);

    // startedBy takes the fallback chain; sessionId must NOT - it feeds
    // the REVIEW gate's reviewer-independence check, which would flip
    // from advisory to blocking in every solo repo if the git-name
    // fallback leaked into it.
    let run = read_run_json(&repo, &id);
    assert_eq!(run.get("startedBy").unwrap().as_str(), Some("test"));
    assert!(run.get("sessionId").unwrap().is_null());
}

#[test]
fn started_by_is_omitted_when_absent_and_optional_on_read() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    unset_local_git_user_name(&repo);
    let home = make_temp_dir("gate-identity-home");
    let home_str = home.to_str().unwrap().to_string();

    let started = gate_opts(
        &repo,
        &["start", "identity demo"],
        GateOpts {
            env: &[
                ("HOME", &home_str),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_SYSTEM", "/dev/null"),
            ],
            input: None,
        },
    );
    assert_eq!(started.code, 0, "stderr: {}", started.stderr);
    let id = run_id(&repo);

    // Additive key: absent entirely (not null) when nothing resolved, so
    // pre-#33 run.json files and this one share one shape.
    let text = common::read_file(&repo, &format!(".gate/runs/{id}/run.json"));
    assert!(!text.contains("startedBy"));

    // And the read path treats it as optional: status works fine.
    assert_eq!(gate(&repo, &["status"]).code, 0);
}
