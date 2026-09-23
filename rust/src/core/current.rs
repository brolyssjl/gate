//! Port of `src/core/current.ts`: the active run is tracked per branch
//! (Milestone 4) - `.gate/current.json` maps a branch key to its active run
//! id, so concurrent work-in-progress on separate branches doesn't collide.
//! Milestone 1-3 tracked a single global run in a plain-text `.gate/current`
//! file; that file is migrated in place the first time it's read after
//! upgrade (`migrate_legacy`).
//!
//! Multiple `gate` processes can genuinely race to read-modify-write
//! `current.json` at the same time. Every mutation goes through
//! `with_current_lock`, an O_EXCL lockfile with retry, so "read, decide,
//! write" is one atomic-with-respect-to-other-processes step.

use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::cli::output::UserError;
use crate::core::fsx::write_file_atomic;
use crate::core::git::branch_state;
use crate::core::git::BranchState;
use crate::core::json::{self, Value};
use crate::core::paths::gate_paths;
use crate::core::run::{read_run, write_run};

/// Sentinel key for repos with no git branch concept at all (not a git
/// repo). Contains `~`, a character forbidden in real git ref names, so it
/// can never collide with an actual branch.
pub const NO_GIT_BRANCH_KEY: &str = "~no-git~";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchKeyResolution {
    Key(String),
    Detached,
}

/// Resolve the branch key `current.json` is keyed by for this working tree.
/// Detached HEAD is genuinely ambiguous - it resolves to `Detached` so
/// callers can require an explicit `--run` instead of guessing.
pub fn resolve_branch_key(root: &Path) -> BranchKeyResolution {
    match branch_state(root) {
        BranchState::Detached => BranchKeyResolution::Detached,
        BranchState::Branch(name) => BranchKeyResolution::Key(name),
        BranchState::None => BranchKeyResolution::Key(NO_GIT_BRANCH_KEY.to_string()),
    }
}

const LOCK_RETRY: Duration = Duration::from_millis(20);
const LOCK_TIMEOUT: Duration = Duration::from_millis(5000);

/// Acquire an exclusive lock on `current.json` via an `O_EXCL`-created
/// lockfile (atomic create-if-absent at the filesystem level), retrying
/// with a short blocking sleep until another process's lock is released or
/// the timeout is hit. Returns a release guard; dropping it releases the
/// lock exactly once.
struct LockGuard {
    path: std::path::PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Already gone - nothing left to release.
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_lock(root: &Path) -> Result<LockGuard, UserError> {
    let lock_path = {
        let mut p = gate_paths(root).current.into_os_string();
        p.push(".lock");
        std::path::PathBuf::from(p)
    };
    let deadline = Instant::now() + LOCK_TIMEOUT;
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => return Ok(LockGuard { path: lock_path }),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                if Instant::now() > deadline {
                    return Err(UserError::new(format!(
                        "timed out waiting for the lock on {} - remove it manually if you're sure no other gate process is running",
                        lock_path.display()
                    )));
                }
                std::thread::sleep(LOCK_RETRY);
            }
            Err(err) => return Err(UserError::new(err.to_string())),
        }
    }
}

/// `.gate/current.json`'s parsed shape: schema 1, a branch-key -> run-id map.
struct CurrentState {
    branches: Vec<(String, String)>,
}

fn read_state(root: &Path) -> Result<CurrentState, UserError> {
    migrate_legacy(root)?;
    let current_path = gate_paths(root).current;
    let text = match fs::read_to_string(&current_path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(CurrentState {
                branches: Vec::new(),
            })
        }
        Err(e) => return Err(UserError::new(e.to_string())),
    };
    // Fail closed: silently treating corrupt state as "no active run
    // anywhere" would let the next write quietly discard every branch's
    // mapping. A human needs to look at the file.
    let parsed = json::parse(&text).map_err(|e| {
        UserError::new(format!(
            "{} is corrupt ({e}) - fix or remove it by hand",
            current_path.display()
        ))
    })?;
    let branches = parsed
        .get("branches")
        .and_then(|v| v.as_object())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Ok(CurrentState { branches })
}

fn write_state(root: &Path, state: &CurrentState) -> io::Result<()> {
    let mut branches = Value::object();
    for (k, v) in &state.branches {
        branches.insert(k.as_str(), v.as_str());
    }
    let mut obj = Value::object();
    obj.insert("schema", 1i64);
    obj.insert("branches", branches);
    let text = json::stringify_pretty(&obj) + "\n";
    write_file_atomic(&gate_paths(root).current, &text)
}

/// One-time migration: the old single-run `.gate/current` file maps onto
/// whatever branch is checked out right now (best-effort), or the "no git
/// branch" sentinel when that can't be determined. The migrated run's own
/// `branch` field is backfilled to match when it's still null.
fn migrate_legacy(root: &Path) -> Result<(), UserError> {
    let paths = gate_paths(root);
    if paths.current.exists() || !paths.legacy_current.exists() {
        return Ok(());
    }
    let run_id = fs::read_to_string(&paths.legacy_current)
        .map_err(|e| UserError::new(e.to_string()))?
        .trim()
        .to_string();
    if !run_id.is_empty() {
        let key = match resolve_branch_key(root) {
            BranchKeyResolution::Key(k) => k,
            BranchKeyResolution::Detached => NO_GIT_BRANCH_KEY.to_string(),
        };
        write_state(
            root,
            &CurrentState {
                branches: vec![(key.clone(), run_id.clone())],
            },
        )
        .map_err(|e| UserError::new(e.to_string()))?;
        backfill_migrated_branch(root, &run_id, &key);
    }
    // force: a second gate invocation racing through this same one-time
    // migration can pass the exists() guard above right before the first
    // process's remove_file runs; ignore a "not found" race on removal.
    match fs::remove_file(&paths.legacy_current) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(UserError::new(e.to_string())),
    }
    Ok(())
}

fn backfill_migrated_branch(root: &Path, run_id: &str, key: &str) {
    // Run unreadable or missing - nothing to backfill; the current.json
    // mapping itself is still migrated regardless.
    let Ok(mut run) = read_run(root, run_id) else {
        return;
    };
    if run.branch.is_none() {
        run.branch = if key == NO_GIT_BRANCH_KEY {
            None
        } else {
            Some(key.to_string())
        };
        let _ = write_run(root, &mut run);
    }
}

pub fn read_current_run_id(root: &Path, key: &str) -> Result<Option<String>, UserError> {
    let state = read_state(root)?;
    Ok(state
        .branches
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone()))
}

/// The current branch's mapped run id, or `None` - including on a detached
/// HEAD (nothing "current" to resolve a key from).
pub fn current_run_id_or_null(root: &Path) -> Result<Option<String>, UserError> {
    match resolve_branch_key(root) {
        BranchKeyResolution::Key(key) => read_current_run_id(root, &key),
        BranchKeyResolution::Detached => Ok(None),
    }
}

/// Run `f` with exclusive access to `current.json`'s branch map: locks,
/// reads (fail-closed on corruption, migrating the legacy pointer first),
/// lets `f` mutate the map in place and return a result, then persists and
/// unlocks - all as one critical section. A returned error skips the write
/// entirely (no partial state persisted) but the lock is always released
/// (the `LockGuard`'s `Drop`).
pub fn with_current_lock<T>(
    root: &Path,
    f: impl FnOnce(&mut Vec<(String, String)>) -> T,
) -> Result<T, UserError> {
    let _guard = acquire_lock(root)?;
    let mut state = read_state(root)?;
    let result = f(&mut state.branches);
    write_state(root, &state).map_err(|e| UserError::new(e.to_string()))?;
    Ok(result)
}

pub fn set_current_run_id(root: &Path, key: &str, run_id: &str) -> Result<(), UserError> {
    with_current_lock(root, |branches| {
        if let Some(slot) = branches.iter_mut().find(|(k, _)| k == key) {
            slot.1 = run_id.to_string();
        } else {
            branches.push((key.to_string(), run_id.to_string()));
        }
    })
}

pub fn clear_current_run_id(root: &Path, key: &str) -> Result<(), UserError> {
    with_current_lock(root, |branches| {
        branches.retain(|(k, _)| k != key);
    })
}

/// Remove every mapping pointing at `run_id`, regardless of which branch
/// key it's filed under - used when a run reaches a terminal phase.
/// Deliberately does not trust `run.branch` to compute the one key to
/// clear: the mapping itself, not the run's memory of its own branch, is
/// the source of truth for where it's filed.
pub fn clear_run_everywhere(root: &Path, run_id: &str) -> Result<(), UserError> {
    with_current_lock(root, |branches| {
        branches.retain(|(_, id)| id != run_id);
    })
}

/// Every branch with an active-run mapping, for `gate status`'s "other
/// in-flight runs" list.
pub fn list_active_branches(root: &Path) -> Result<Vec<(String, String)>, UserError> {
    Ok(read_state(root)?.branches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;
    use std::process::Command as StdCommand;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = crate::core::testutil::unique_temp_dir(&format!("current-rs-{name}"));
        dir
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = StdCommand::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn make_repo(name: &str) -> std::path::PathBuf {
        let dir = tmp_dir(name);
        run_git(&dir, &["init", "-q"]);
        run_git(&dir, &["config", "user.email", "test@test.co"]);
        run_git(&dir, &["config", "user.name", "test"]);
        run_git(&dir, &["config", "commit.gpgsign", "false"]);
        stdfs::write(dir.join("README.md"), "seed\n").unwrap();
        run_git(&dir, &["add", "-A"]);
        run_git(&dir, &["commit", "-qm", "init"]);
        stdfs::create_dir_all(dir.join(".gate")).unwrap();
        dir
    }

    #[test]
    fn resolve_branch_key_returns_the_checked_out_branch_name() {
        let root = make_repo("resolve-key-normal");
        assert_eq!(
            resolve_branch_key(&root),
            BranchKeyResolution::Key("master".to_string())
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_branch_key_uses_the_sentinel_for_a_non_git_directory() {
        let dir = tmp_dir("resolve-key-no-git");
        assert_eq!(
            resolve_branch_key(&dir),
            BranchKeyResolution::Key(NO_GIT_BRANCH_KEY.to_string())
        );
        stdfs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_read_and_clear_a_run_id_round_trip() {
        let root = make_repo("set-read-clear");
        assert_eq!(read_current_run_id(&root, "master").unwrap(), None);
        set_current_run_id(&root, "master", "r1").unwrap();
        assert_eq!(
            read_current_run_id(&root, "master").unwrap(),
            Some("r1".to_string())
        );
        clear_current_run_id(&root, "master").unwrap();
        assert_eq!(read_current_run_id(&root, "master").unwrap(), None);
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tracks_separate_runs_per_branch_key() {
        let root = make_repo("per-branch");
        set_current_run_id(&root, "master", "r1").unwrap();
        set_current_run_id(&root, "feature-x", "r2").unwrap();
        assert_eq!(
            read_current_run_id(&root, "master").unwrap(),
            Some("r1".to_string())
        );
        assert_eq!(
            read_current_run_id(&root, "feature-x").unwrap(),
            Some("r2".to_string())
        );
        let mut branches = list_active_branches(&root).unwrap();
        branches.sort();
        assert_eq!(
            branches,
            vec![
                ("feature-x".to_string(), "r2".to_string()),
                ("master".to_string(), "r1".to_string())
            ]
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn clear_run_everywhere_removes_every_key_pointing_at_the_run_regardless_of_branch() {
        let root = make_repo("clear-everywhere");
        set_current_run_id(&root, "master", "r1").unwrap();
        set_current_run_id(&root, "feature-x", "r1").unwrap();
        set_current_run_id(&root, "feature-y", "r2").unwrap();
        clear_run_everywhere(&root, "r1").unwrap();
        let mut branches = list_active_branches(&root).unwrap();
        branches.sort();
        assert_eq!(branches, vec![("feature-y".to_string(), "r2".to_string())]);
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn read_state_fails_closed_on_a_corrupt_current_json() {
        let root = make_repo("corrupt-state");
        stdfs::write(root.join(".gate/current.json"), "{not json").unwrap();
        let err = read_current_run_id(&root, "master").unwrap_err();
        assert!(err.message().contains("corrupt"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn migrates_the_legacy_single_run_pointer_onto_the_checked_out_branch() {
        let root = make_repo("migrate-legacy");
        stdfs::write(root.join(".gate/current"), "legacy-run-1\n").unwrap();
        assert_eq!(
            read_current_run_id(&root, "master").unwrap(),
            Some("legacy-run-1".to_string())
        );
        assert!(!root.join(".gate/current").exists());
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn current_run_id_or_null_is_none_on_a_detached_head() {
        let root = make_repo("detached-head");
        // Detach HEAD at the initial commit.
        let sha = String::from_utf8(
            StdCommand::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&root)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        run_git(&root, &["checkout", "-q", sha.trim()]);
        set_current_run_id(&root, "master", "r1").unwrap();
        assert_eq!(current_run_id_or_null(&root).unwrap(), None);
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lock_file_is_cleaned_up_after_a_mutation() {
        let root = make_repo("lock-cleanup");
        set_current_run_id(&root, "master", "r1").unwrap();
        let mut lock_path = gate_paths(&root).current.into_os_string();
        lock_path.push(".lock");
        assert!(!Path::new(&lock_path).exists());
        stdfs::remove_dir_all(&root).unwrap();
    }
}
