//! Port of `src/gates/planDrift.ts` (`planDriftCheck`). Ported in wave 3
//! (see `docs/rust-port.md`).
//!
//! Approved-plan drift (Milestone 5): once a plan is approved, every later
//! gate re-checks plan.md's content hash against the hash `gate approve`
//! recorded, restoring void-on-edit for the whole run - previously this was
//! only enforced up to PLAN itself (`gates::plan`'s own `approval_check`),
//! so a post-PLAN edit to plan.md (including scope widening) was accepted
//! with no re-approval and no mechanism to record one. `gate amend` shows
//! the diff and records intent; `gate approve --amend` re-approves the
//! delta.
//!
//! Returns `None` when there's nothing to drift from - never approved
//! (PLAN's own gate owns that message), or the approved content still
//! matches - so this stays silent (no check line at all) for the
//! overwhelming majority of runs where the plan was never touched again
//! after approval.

use crate::artifacts::plan::hash_plan_file;
use crate::core::paths::run_paths;
use crate::gates::types::{fail, Check, GateContext};

pub fn plan_drift_check(ctx: &GateContext, name: &str) -> Option<Check> {
    let approval = ctx.run.approval.as_ref()?;
    let current_hash = hash_plan_file(&run_paths(&ctx.root, &ctx.run.id).plan);
    if current_hash.as_deref() == Some(approval.plan_hash.as_str()) {
        return None;
    }
    Some(fail(
        name,
        "plan.md changed since approval - run `gate amend` to review the diff, then `gate approve --amend` to re-approve",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::GateConfig;
    use crate::core::run::{new_run, now_iso, Approval, NewRunParams};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = crate::core::testutil::unique_temp_dir(&format!("plandrift-rs-{name}"));
        dir
    }

    fn write_plan(root: &Path, run_id: &str, content: &str) {
        let dir = root.join(".gate/runs").join(run_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plan.md"), content).unwrap();
    }

    fn ctx_with(root: PathBuf, approval: Option<Approval>) -> GateContext {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.approval = approval;
        GateContext {
            root,
            run,
            config: GateConfig::default(),
        }
    }

    #[test]
    fn is_none_when_never_approved() {
        let root = tmp_dir("never-approved");
        write_plan(&root, "r1", "plan v1");
        let ctx = ctx_with(root.clone(), None);
        assert!(plan_drift_check(&ctx, "x.plan-drift").is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn is_none_when_unchanged_since_approval() {
        let root = tmp_dir("unchanged");
        write_plan(&root, "r1", "plan v1");
        let hash = crate::artifacts::plan::hash_plan("plan v1");
        let ctx = ctx_with(
            root.clone(),
            Some(Approval {
                by: None,
                at: now_iso(),
                reason: None,
                plan_hash: hash,
            }),
        );
        assert!(plan_drift_check(&ctx, "x.plan-drift").is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_once_the_plan_changes_after_approval() {
        let root = tmp_dir("changed");
        write_plan(&root, "r1", "plan v1");
        let hash = crate::artifacts::plan::hash_plan("plan v1");
        let ctx = ctx_with(
            root.clone(),
            Some(Approval {
                by: None,
                at: now_iso(),
                reason: None,
                plan_hash: hash,
            }),
        );
        write_plan(&root, "r1", "plan v2 - scope widened");
        let check = plan_drift_check(&ctx, "implement.plan-drift").unwrap();
        assert!(!check.ok);
        assert_eq!(check.name, "implement.plan-drift");
        fs::remove_dir_all(&root).unwrap();
    }
}
