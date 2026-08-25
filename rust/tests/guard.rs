//! Port of `test/guard.test.ts` ("gate guard": opt-in pre-commit hook).
//! See `CONFORMANCE_MAP.md` for the TS case -> Rust test fn mapping.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{gate, gate_shell_command, git_run, make_repo, make_temp_dir, read_file, write_file};

/// A `gate` shim on PATH so the installed hook (which looks up `gate` via
/// PATH) resolves in tests - honors `$GATE_BIN` (conformance mode) the same
/// way `gate_shell_command` does, so the hook itself invokes the binary
/// under test.
fn gate_shim_path() -> PathBuf {
    let bin_dir = make_temp_dir("gate-bin");
    write_file(
        &bin_dir,
        "gate",
        &format!("#!/bin/sh\nexec {} \"$@\"\n", gate_shell_command()),
    );
    let shim = bin_dir.join("gate");
    let mut perms = std::fs::metadata(&shim).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&shim, perms).unwrap();
    bin_dir
}

/// A `PATH` value with the `gate` shim prepended, for `git_run`'s env.
fn path_with_gate_on_it() -> String {
    let bin_dir = gate_shim_path();
    format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

/// Start a run, write a plan declaring `files`. Returns the run id. Unlike
/// the TS helper, uses the default profile (no `--profile` flag) - the
/// guard checks under test (active run / phase / declared scope) don't
/// depend on which profile a run walks.
fn start_active_run(repo: &Path, files: &[&str]) -> String {
    gate(repo, &["init", "--no-adapt"]);
    gate(repo, &["trust"]);
    let started = gate(repo, &["start", "guarded work", "--json"]);
    let json = started.json();
    let run_id = json.str("id").unwrap().to_string();
    let plan_path = json.str("plan").unwrap().to_string();
    let mut plan = String::from("---\ngoal: guarded work\nfiles:\n");
    for f in files {
        plan.push_str(&format!("  - {f}\n"));
    }
    plan.push_str("criteria:\n  - id: c1\n    text: it works\n    verify: \"manual\"\n---\n");
    write_file(repo, &plan_path, &plan);
    run_id
}

fn hook_path(repo: &Path) -> PathBuf {
    repo.join(".git").join("hooks").join("pre-commit")
}

fn backup_path(repo: &Path) -> PathBuf {
    repo.join(".git")
        .join("hooks")
        .join("pre-commit.gate-backup")
}

#[test]
fn install_writes_an_executable_pre_commit_hook_carrying_the_gate_guard_marker() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    let res = gate(&repo, &["guard", "install", "--json"]);
    assert_eq!(res.code, 0);
    assert_eq!(res.json().bool_at("installed"), Some(true));

    let hook = hook_path(&repo);
    assert!(hook.exists());
    assert!(read_file(&repo, ".git/hooks/pre-commit").contains("gate-guard: v1"));
    let mode = std::fs::metadata(&hook).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0); // executable
}

#[test]
fn install_is_idempotent_a_second_install_reports_already_installed_instead_of_re_wrapping() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["guard", "install"]);
    let again = gate(&repo, &["guard", "install", "--json"]);
    assert_eq!(again.json().bool_at("alreadyInstalled"), Some(true));
}

#[test]
fn install_backs_up_and_chains_a_pre_existing_foreign_hook() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    let hook = hook_path(&repo);
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho original-hook-ran\nexit 0\n").unwrap();
    let mut perms = std::fs::metadata(&hook).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&hook, perms).unwrap();

    let res = gate(&repo, &["guard", "install", "--json"]);
    assert_eq!(res.json().bool_at("chained"), Some(true));

    let backup = backup_path(&repo);
    assert!(backup.exists());
    assert!(read_file(&repo, ".git/hooks/pre-commit.gate-backup").contains("original-hook-ran"));
    assert!(read_file(&repo, ".git/hooks/pre-commit").contains("pre-commit.gate-backup"));
}

#[test]
fn uninstall_restores_a_backed_up_foreign_hook() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    let hook = hook_path(&repo);
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho original-hook-ran\nexit 0\n").unwrap();
    gate(&repo, &["guard", "install"]);

    let res = gate(&repo, &["guard", "uninstall", "--json"]);
    assert_eq!(res.json().bool_at("restored"), Some(true));
    assert!(read_file(&repo, ".git/hooks/pre-commit").contains("original-hook-ran"));
    assert!(!backup_path(&repo).exists());
}

#[test]
fn uninstall_removes_a_hook_it_installed_with_nothing_to_restore() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["guard", "install"]);
    let hook = hook_path(&repo);
    assert!(hook.exists());

    gate(&repo, &["guard", "uninstall"]);
    assert!(!hook.exists());
}

#[test]
fn uninstall_refuses_to_touch_a_pre_commit_hook_gate_didnt_install() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    let hook = hook_path(&repo);
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho someone-elses-hook\nexit 0\n").unwrap();

    let res = gate(&repo, &["guard", "uninstall"]);
    assert_ne!(res.code, 0);
    assert!(res.stderr.contains("wasn't installed by"));
    assert!(read_file(&repo, ".git/hooks/pre-commit").contains("someone-elses-hook"));
}

#[test]
fn guard_run_blocks_with_no_active_run_on_the_branch() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    let res = gate(&repo, &["guard", "run", "--json"]);
    assert_eq!(res.code, 1);
    let data = res.json();
    assert_eq!(data.bool_at("ok"), Some(false));
    let reasons: Vec<&str> = data
        .get("reasons")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(reasons.join(" ").contains("no active run"));
}

#[test]
fn guard_run_informs_rather_than_misleadingly_blocks_on_an_unparseable_missing_plan_md() {
    // e.g. after `gate skip PLAN`.
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["trust"]);
    gate(&repo, &["start", "skip planning", "--profile", "docs"]); // plan.md left as the untouched scaffold
    assert_eq!(
        gate(&repo, &["skip", "PLAN", "--reason", "trivial change"]).code,
        0
    ); // -> IMPLEMENT

    write_file(&repo, "whatever.txt", "anything\n");
    git_run(&repo, &["add", "whatever.txt"], &[]);

    let res = gate(&repo, &["guard", "run", "--json"]);
    assert_eq!(res.code, 0);
    let data = res.json();
    assert_eq!(data.bool_at("ok"), Some(true));
    let reasons_joined = data
        .get("reasons")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(reasons_joined.contains("not parseable"));
    // The old behavior: declared = [] flagged every staged file as "outside
    // the declared plan scope" - a misleading hard block on an advisory hook.
    assert!(!reasons_joined.contains("outside the declared plan scope"));
}

#[test]
fn guard_run_blocks_while_still_in_plan() {
    let repo = make_repo(&[]);
    start_active_run(&repo, &["a.txt"]);
    let res = gate(&repo, &["guard", "run", "--json"]);
    assert_eq!(res.code, 1);
    let reasons_joined = res
        .json()
        .get("reasons")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(reasons_joined.contains("still in PLAN"));
}

#[test]
fn guard_run_blocks_staged_files_outside_the_declared_plan_scope_passes_for_in_scope_files() {
    let repo = make_repo(&[]);
    start_active_run(&repo, &["a.txt"]);
    gate(&repo, &["approve"]);
    gate(&repo, &["next"]); // -> IMPLEMENT

    write_file(&repo, "out-of-scope.txt", "nope\n");
    git_run(&repo, &["add", "out-of-scope.txt"], &[]);
    let blocked = gate(&repo, &["guard", "run", "--json"]);
    assert_eq!(blocked.code, 1);
    let blocked_reasons = blocked
        .json()
        .get("reasons")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(blocked_reasons.contains("out-of-scope.txt"));

    git_run(&repo, &["reset"], &[]);
    write_file(&repo, "a.txt", "in scope\n");
    git_run(&repo, &["add", "a.txt"], &[]);
    let passed = gate(&repo, &["guard", "run", "--json"]);
    assert_eq!(passed.code, 0);
    assert_eq!(passed.json().bool_at("ok"), Some(true));
}

#[test]
fn end_to_end_installed_hook_blocks_git_commit_for_out_of_scope_staged_files_and_no_verify_gate_guard_0_bypass_it(
) {
    let repo = make_repo(&[]);
    start_active_run(&repo, &["a.txt"]);
    gate(&repo, &["approve"]);
    gate(&repo, &["next"]); // -> IMPLEMENT
    gate(&repo, &["guard", "install"]);

    let path_value = path_with_gate_on_it();
    write_file(&repo, "b.txt", "not declared\n");
    git_run(&repo, &["add", "b.txt"], &[("PATH", path_value.as_str())]);

    let blocked = git_run(
        &repo,
        &["commit", "-m", "should be blocked"],
        &[("PATH", path_value.as_str())],
    );
    assert_ne!(blocked.code, 0);

    let bypassed = git_run(
        &repo,
        &["commit", "--no-verify", "-m", "bypassed"],
        &[("PATH", path_value.as_str())],
    );
    assert_eq!(bypassed.code, 0);

    write_file(&repo, "c.txt", "also not declared\n");
    git_run(&repo, &["add", "c.txt"], &[("PATH", path_value.as_str())]);
    let disabled = git_run(
        &repo,
        &["commit", "-m", "guard disabled"],
        &[("PATH", path_value.as_str()), ("GATE_GUARD", "0")],
    );
    assert_eq!(disabled.code, 0);
}

#[test]
fn gate_guard_0_disables_only_the_gate_check_a_chained_pre_existing_hook_still_runs() {
    let repo = make_repo(&[]);
    start_active_run(&repo, &["a.txt"]);
    gate(&repo, &["approve"]);
    gate(&repo, &["next"]); // -> IMPLEMENT

    let hook = hook_path(&repo);
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    let marker = repo.join("chained-hook-ran");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch \"{}\"\nexit 0\n", marker.display()),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&hook).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&hook, perms).unwrap();
    gate(&repo, &["guard", "install"]);
    let reinstall = gate(&repo, &["guard", "install", "--json"]);
    let reinstall_json = reinstall.json();
    assert_eq!(reinstall_json.bool_at("chained"), Some(false));
    assert_eq!(reinstall_json.bool_at("alreadyInstalled"), Some(true));

    // Out of scope - gate guard's own check would block this (proven by the
    // "should be blocked" case above); GATE_GUARD=0 must skip only that
    // check, not the commit's chained hook.
    write_file(&repo, "b.txt", "not declared in the plan\n");
    let path_value = path_with_gate_on_it();
    git_run(&repo, &["add", "b.txt"], &[("PATH", path_value.as_str())]);
    let res = git_run(
        &repo,
        &["commit", "-m", "guard disabled, hook still chained"],
        &[("PATH", path_value.as_str()), ("GATE_GUARD", "0")],
    );

    assert_eq!(res.code, 0);
    assert!(marker.exists());
}
