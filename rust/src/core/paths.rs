//! Port of `src/core/paths.ts`: resolving a Gate project root and the
//! well-known paths inside it.

use std::path::{Path, PathBuf};

/// Resolve the root of a Gate project by walking up from `start` until a
/// `.gate/` directory is found. Returns `None` if none exists (not yet
/// init'd). TS defaults `start` to `process.cwd()`; callers here pass
/// `std::env::current_dir()` explicitly (Rust has no default-parameter
/// sugar). Mirrors `path.resolve`: makes the path absolute without
/// resolving symlinks - it does not canonicalize.
pub fn find_gate_root(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(start)
    };
    loop {
        if dir.join(".gate").exists() {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
    }
}

/// Paths inside a resolved Gate project root.
#[derive(Debug, Clone)]
pub struct GatePaths {
    pub root: PathBuf,
    pub gate: PathBuf,
    pub config: PathBuf,
    pub playbooks: PathBuf,
    pub runs: PathBuf,
    /// Per-branch map of branch key -> active run id.
    pub current: PathBuf,
    /// Milestone 1-3 single-run pointer; migrated on first read.
    pub legacy_current: PathBuf,
    /// `gate prune`: summaries of pruned runs.
    pub archive: PathBuf,
    /// Hash of the root `.gitignore` as `gate init` last left it.
    pub gitignore_state: PathBuf,
}

pub fn gate_paths(root: &Path) -> GatePaths {
    let gate = root.join(".gate");
    GatePaths {
        root: root.to_path_buf(),
        config: gate.join("config.yml"),
        playbooks: gate.join("playbooks"),
        runs: gate.join("runs"),
        current: gate.join("current.json"),
        legacy_current: gate.join("current"),
        archive: gate.join("archive"),
        gitignore_state: gate.join("gitignore.json"),
        gate,
    }
}

/// Where `gate prune` archives a run's summary once its folder is removed.
pub fn archive_path(root: &Path, run_id: &str) -> PathBuf {
    gate_paths(root).archive.join(format!("{run_id}.json"))
}

/// Paths inside a single run folder.
#[derive(Debug, Clone)]
pub struct RunPaths {
    pub dir: PathBuf,
    pub run_json: PathBuf,
    pub plan: PathBuf,
    /// Snapshot of plan.md as of the last approval/amendment - `gate amend`
    /// diffs the current plan against this.
    pub plan_approved: PathBuf,
    pub worklog: PathBuf,
    pub test_report: PathBuf,
    pub debug_log: PathBuf,
    pub review_packet: PathBuf,
    pub review: PathBuf,
    pub retro: PathBuf,
}

pub fn run_paths(root: &Path, run_id: &str) -> RunPaths {
    let dir = root.join(".gate").join("runs").join(run_id);
    RunPaths {
        run_json: dir.join("run.json"),
        plan: dir.join("plan.md"),
        plan_approved: dir.join("plan.approved.md"),
        worklog: dir.join("worklog.md"),
        test_report: dir.join("test-report.json"),
        debug_log: dir.join("debug-log.md"),
        review_packet: dir.join("review-packet.md"),
        review: dir.join("review.md"),
        retro: dir.join("retro.md"),
        dir,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-paths-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn find_gate_root_returns_none_when_no_gate_dir_exists_up_to_the_fs_root() {
        let dir = tmp_dir("none");
        assert_eq!(find_gate_root(&dir), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_gate_root_finds_a_gate_dir_at_start() {
        let dir = tmp_dir("at-start");
        fs::create_dir_all(dir.join(".gate")).unwrap();
        assert_eq!(find_gate_root(&dir), Some(dir.clone()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_gate_root_walks_up_from_a_nested_directory() {
        let dir = tmp_dir("nested");
        fs::create_dir_all(dir.join(".gate")).unwrap();
        let nested = dir.join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(find_gate_root(&nested), Some(dir.clone()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gate_paths_joins_expected_subpaths() {
        let root = Path::new("/tmp/proj");
        let paths = gate_paths(root);
        assert_eq!(paths.gate, root.join(".gate"));
        assert_eq!(paths.config, root.join(".gate/config.yml"));
        assert_eq!(paths.playbooks, root.join(".gate/playbooks"));
        assert_eq!(paths.runs, root.join(".gate/runs"));
        assert_eq!(paths.current, root.join(".gate/current.json"));
        assert_eq!(paths.legacy_current, root.join(".gate/current"));
        assert_eq!(paths.archive, root.join(".gate/archive"));
        assert_eq!(paths.gitignore_state, root.join(".gate/gitignore.json"));
    }

    #[test]
    fn run_paths_joins_expected_subpaths() {
        let root = Path::new("/tmp/proj");
        let paths = run_paths(root, "r1");
        assert_eq!(paths.dir, root.join(".gate/runs/r1"));
        assert_eq!(paths.run_json, root.join(".gate/runs/r1/run.json"));
        assert_eq!(paths.plan, root.join(".gate/runs/r1/plan.md"));
        assert_eq!(
            paths.plan_approved,
            root.join(".gate/runs/r1/plan.approved.md")
        );
        assert_eq!(paths.worklog, root.join(".gate/runs/r1/worklog.md"));
        assert_eq!(
            paths.test_report,
            root.join(".gate/runs/r1/test-report.json")
        );
        assert_eq!(paths.debug_log, root.join(".gate/runs/r1/debug-log.md"));
        assert_eq!(
            paths.review_packet,
            root.join(".gate/runs/r1/review-packet.md")
        );
        assert_eq!(paths.review, root.join(".gate/runs/r1/review.md"));
        assert_eq!(paths.retro, root.join(".gate/runs/r1/retro.md"));
    }

    #[test]
    fn archive_path_joins_the_run_id_json_file() {
        let root = Path::new("/tmp/proj");
        assert_eq!(archive_path(root, "r1"), root.join(".gate/archive/r1.json"));
    }
}
