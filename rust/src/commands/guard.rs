//! Port of `src/commands/guard.ts`.
//!
//! `gate guard` - an opt-in `.git/hooks/pre-commit` hook (Milestone 4).
//! Never installed by `init` or any other command - only the explicit
//! `gate guard install` writes it, and only `gate guard uninstall` removes
//! it. Advisory by design: the hook runs only cheap deterministic checks
//! (never a build/test command), and is always escapable (`git commit
//! --no-verify`, `GATE_GUARD=0`).

use std::fs;
use std::path::Path;

use crate::artifacts::plan::parse_plan_file;
use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::core::current::{
    read_current_run_id, resolve_branch_key, BranchKeyResolution, NO_GIT_BRANCH_KEY,
};
use crate::core::fsx::write_file_atomic;
use crate::core::git::{git_hooks_dir, staged_files};
use crate::core::glob::matches_any;
use crate::core::json::Value;
use crate::core::paths::{run_paths, validate_run_id};
use crate::core::run::read_run;
use crate::core::state_machine::Phase;

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    // `parse_args` treats a leading non-flag token as the command name;
    // without a placeholder, `guard`'s own subcommand (install/uninstall/
    // run) would be swallowed into `args.command` instead of landing in
    // `args.positionals` (see `report::run`'s identical workaround, and
    // `start.rs`'s `run` for the pattern this mirrors).
    let mut full = vec!["guard".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = require_root()?;
    match args.positionals.first().map(String::as_str) {
        Some("install") => guard_install(&root, &args),
        Some("uninstall") => guard_uninstall(&root, &args),
        Some("run") => guard_run(&root, &args),
        _ => Err(UserError::usage(
            "gate guard needs a subcommand: install | uninstall | run",
        )),
    }
}

const MARKER: &str = "# gate-guard: v1";

fn hook_script(chained: bool) -> String {
    let mut lines: Vec<String> = vec![
        "#!/bin/sh".to_string(),
        MARKER.to_string(),
        "# Installed by `gate guard install` - advisory only, never load-bearing.".to_string(),
        "# Skip once: git commit --no-verify. Disable just the gate check: GATE_GUARD=0"
            .to_string(),
        "# (a chained pre-existing hook, if any, still runs either way).".to_string(),
        "if [ \"$GATE_GUARD\" != \"0\" ]; then".to_string(),
        "  if command -v gate >/dev/null 2>&1; then".to_string(),
        "    gate guard run".to_string(),
        "    status=$?".to_string(),
        "    if [ $status -ne 0 ]; then".to_string(),
        "      exit $status".to_string(),
        "    fi".to_string(),
        "  else".to_string(),
        "    echo \"gate guard: \\`gate\\` not found on PATH - skipping (advisory only)\" >&2"
            .to_string(),
        "  fi".to_string(),
        "fi".to_string(),
    ];
    if chained {
        lines.push("exec \"$(dirname \"$0\")/pre-commit.gate-backup\" \"$@\"".to_string());
    }
    lines.push("exit 0".to_string());
    lines.push(String::new());
    lines.join("\n")
}

fn set_executable(path: &Path) -> Result<(), UserError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)
        .map_err(|e| UserError::new(e.to_string()))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).map_err(|e| UserError::new(e.to_string()))
}

fn guard_install(root: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    let Some(hooks_dir) = git_hooks_dir(root) else {
        return Err(UserError::new(
            "not a git repository (or no git hooks directory) - gate guard needs git",
        ));
    };
    fs::create_dir_all(&hooks_dir).map_err(|e| UserError::new(e.to_string()))?;
    let hook_path = hooks_dir.join("pre-commit");
    let backup_path = hooks_dir.join("pre-commit.gate-backup");

    if hook_path.exists() {
        let existing = fs::read_to_string(&hook_path).map_err(|e| UserError::new(e.to_string()))?;
        if existing.contains(MARKER) {
            let mut data = Value::object();
            data.insert("installed", true);
            data.insert("alreadyInstalled", true);
            data.insert("chained", false);
            return emit("gate guard is already installed.", &data, &args.flags);
        }
        if backup_path.exists() {
            return Err(UserError::new(format!(
                "refusing to overwrite - a backup already exists at {}; resolve it manually (a previous install may have failed partway through)",
                backup_path.display()
            )));
        }
        fs::rename(&hook_path, &backup_path).map_err(|e| UserError::new(e.to_string()))?;
        write_file_atomic(&hook_path, &hook_script(true))
            .map_err(|e| UserError::new(e.to_string()))?;
        set_executable(&hook_path)?;
        let mut data = Value::object();
        data.insert("installed", true);
        data.insert("alreadyInstalled", false);
        data.insert("chained", true);
        return emit(
            &format!(
                "Installed .git/hooks/pre-commit\nChained the existing hook (backed up to {}).\nAdvisory only: bypass with `git commit --no-verify` or `GATE_GUARD=0`.",
                backup_path.display()
            ),
            &data,
            &args.flags,
        );
    }

    write_file_atomic(&hook_path, &hook_script(false))
        .map_err(|e| UserError::new(e.to_string()))?;
    set_executable(&hook_path)?;
    let mut data = Value::object();
    data.insert("installed", true);
    data.insert("alreadyInstalled", false);
    data.insert("chained", false);
    emit(
        "Installed .git/hooks/pre-commit\nAdvisory only: bypass with `git commit --no-verify` or `GATE_GUARD=0`.",
        &data,
        &args.flags,
    )
}

fn guard_uninstall(root: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    let Some(hooks_dir) = git_hooks_dir(root) else {
        return Err(UserError::new(
            "not a git repository (or no git hooks directory)",
        ));
    };
    let hook_path = hooks_dir.join("pre-commit");
    let backup_path = hooks_dir.join("pre-commit.gate-backup");

    if !hook_path.exists() {
        let mut data = Value::object();
        data.insert("uninstalled", false);
        data.insert("restored", false);
        return emit(
            "Nothing to uninstall - no .git/hooks/pre-commit.",
            &data,
            &args.flags,
        );
    }
    let existing = fs::read_to_string(&hook_path).map_err(|e| UserError::new(e.to_string()))?;
    if !existing.contains(MARKER) {
        return Err(UserError::new(
            ".git/hooks/pre-commit wasn't installed by `gate guard` - remove it manually if you want it gone",
        ));
    }
    if backup_path.exists() {
        fs::rename(&backup_path, &hook_path).map_err(|e| UserError::new(e.to_string()))?;
        set_executable(&hook_path)?;
        let mut data = Value::object();
        data.insert("uninstalled", true);
        data.insert("restored", true);
        return emit(
            "Restored the pre-existing pre-commit hook.",
            &data,
            &args.flags,
        );
    }
    fs::remove_file(&hook_path).map_err(|e| UserError::new(e.to_string()))?;
    let mut data = Value::object();
    data.insert("uninstalled", true);
    data.insert("restored", false);
    emit("Removed .git/hooks/pre-commit.", &data, &args.flags)
}

pub struct GuardResult {
    pub ok: bool,
    pub reasons: Vec<String>,
}

/// The hook's actual checks - cheap and deterministic only, never a
/// build/test command: an active run exists for the branch, staged files
/// stay within the plan's declared scope, and the run isn't still in PLAN.
/// Exported so `gate guard run` and its tests share the exact same logic
/// the installed hook executes.
pub fn run_guard_checks(root: &Path) -> Result<GuardResult, UserError> {
    let resolved = resolve_branch_key(root);
    let key = match resolved {
        BranchKeyResolution::Detached => {
            return Ok(GuardResult {
                ok: true,
                reasons: vec![
                    "HEAD is detached - gate guard has no branch to check against (advisory pass)"
                        .to_string(),
                ],
            });
        }
        BranchKeyResolution::Key(key) => key,
    };

    let id = match read_current_run_id(root, &key)? {
        Some(id) => id,
        None => {
            let where_ = if key == NO_GIT_BRANCH_KEY {
                String::new()
            } else {
                format!(" on branch \"{key}\"")
            };
            return Ok(GuardResult {
                ok: false,
                reasons: vec![format!(
                    "no active run{where_} - start one with `gate start \"<title>\"` or bypass with --no-verify"
                )],
            });
        }
    };

    // Gate report finding 4: `id` came from `.gate/current.json`, not free
    // text already validated elsewhere - reject an absolute/`..` id before
    // it reaches `read_run`/`run_paths`.
    if let Err(e) = validate_run_id(&id) {
        return Ok(GuardResult {
            ok: false,
            reasons: vec![format!(
                "active run id \"{id}\" is invalid: {}",
                e.message()
            )],
        });
    }

    let run = match read_run(root, &id) {
        Ok(r) => r,
        Err(_) => {
            return Ok(GuardResult {
                ok: false,
                reasons: vec![format!("active run \"{id}\" has an unreadable run.json")],
            });
        }
    };

    let mut reasons = Vec::new();
    let mut ok = true;

    if run.phase == Phase::Plan {
        reasons.push(format!(
            "run \"{id}\" is still in PLAN - nothing should be committed until IMPLEMENT starts"
        ));
        ok = false;
    }

    let staged = staged_files(root);
    let parsed = parse_plan_file(&run_paths(root, &id).plan);
    match parsed.plan {
        None => reasons.push("plan.md not parseable - scope check skipped".to_string()),
        Some(plan) => {
            let undeclared: Vec<String> = staged
                .into_iter()
                .filter(|f| !matches_any(f, &plan.files))
                .collect();
            if !undeclared.is_empty() {
                reasons.push(format!(
                    "staged files outside the declared plan scope: {}",
                    undeclared.join(", ")
                ));
                ok = false;
            }
        }
    }

    Ok(GuardResult { ok, reasons })
}

fn guard_run(root: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    let res = run_guard_checks(root)?;
    let human = if res.ok {
        if !res.reasons.is_empty() {
            format!("gate guard: ok ({})", res.reasons.join("; "))
        } else {
            "gate guard: ok".to_string()
        }
    } else {
        let mut lines = vec!["gate guard: blocked".to_string()];
        lines.extend(res.reasons.iter().map(|r| format!("  - {r}")));
        lines.push(String::new());
        lines.push("Bypass: `git commit --no-verify`, or `GATE_GUARD=0 git commit`.".to_string());
        lines.join("\n")
    };
    let ok = res.ok;
    let mut data = Value::object();
    data.insert("ok", ok);
    data.insert("reasons", res.reasons);
    emit(&human, &data, &args.flags)?;
    std::process::exit(if ok { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn tmp_repo(name: &str) -> std::path::PathBuf {
        let dir = crate::core::testutil::unique_temp_dir(&format!("guard-rs-{name}"));
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@test.co"]);
        git(&["config", "user.name", "test"]);
        git(&["config", "commit.gpgsign", "false"]);
        fs::write(dir.join("README.md"), "seed\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "init"]);
        dir
    }

    #[test]
    fn hook_script_carries_the_marker_and_the_escaped_backtick_fallback_message() {
        let script = hook_script(false);
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains(MARKER));
        assert!(script.contains("gate guard: \\`gate\\` not found on PATH"));
        assert!(!script.contains("pre-commit.gate-backup"));
        assert!(script.ends_with("exit 0\n"));
    }

    #[test]
    fn hook_script_chains_the_backup_when_requested() {
        let script = hook_script(true);
        assert!(script.contains("exec \"$(dirname \"$0\")/pre-commit.gate-backup\" \"$@\""));
    }

    #[test]
    fn run_guard_checks_blocks_with_no_active_run() {
        let root = tmp_repo("no-run");
        fs::create_dir_all(root.join(".gate")).unwrap();
        let res = run_guard_checks(&root).unwrap();
        assert!(!res.ok);
        assert!(res.reasons.join(" ").contains("no active run"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn run_guard_checks_passes_on_a_detached_head_advisory() {
        let root = tmp_repo("detached");
        let sha = String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&root)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        Command::new("git")
            .args(["checkout", "-q", sha.trim()])
            .current_dir(&root)
            .output()
            .unwrap();
        let res = run_guard_checks(&root).unwrap();
        assert!(res.ok);
        assert!(res.reasons.join(" ").contains("detached"));
        fs::remove_dir_all(&root).unwrap();
    }
}
