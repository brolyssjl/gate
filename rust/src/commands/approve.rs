//! Port of `src/commands/approve.ts`: `gate approve` - record PLAN sign-off
//! in run.json, bound to the plan's content hash.

use std::fs;
use std::path::Path;

use crate::artifacts::plan::{hash_plan_file, parse_plan_file};
use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_active_run;
use crate::cli::output::{emit, UserError};
use crate::core::config::load_config;
use crate::core::fsx::{confined_write_target, write_file_atomic};
use crate::core::identity;
use crate::core::json::Value;
use crate::core::paths::run_paths;
use crate::core::run::{now_iso, write_run, Approval, Run};
use crate::core::state_machine::{next_phase, Phase};

/// Report finding 3(a): this used to be a plain `fs::write` of the plan's
/// content (attacker-authored) to `plan.approved.md` - a committed symlink
/// there landed that content anywhere on disk. Route it through the same
/// containment + atomic-write path as every other write under `root`.
fn snapshot_approved_plan(
    root: &std::path::Path,
    run_id: &str,
    plan_path: &std::path::Path,
) -> Result<(), UserError> {
    let content = fs::read_to_string(plan_path).map_err(|e| UserError::new(e.to_string()))?;
    let plan_approved = run_paths(root, run_id).plan_approved;
    let rel = plan_approved.strip_prefix(root).unwrap_or(&plan_approved);
    let target = confined_write_target(root, rel).map_err(|e| UserError::new(e.to_string()))?;
    write_file_atomic(&target, &content).map_err(|e| UserError::new(e.to_string()))
}

fn approval_to_data(a: &Approval) -> Value {
    let mut v = Value::object();
    v.insert("by", a.by.clone());
    v.insert("at", a.at.as_str());
    v.insert("reason", a.reason.clone());
    v.insert("planHash", a.plan_hash.as_str());
    v
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["approve".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let ctx = require_active_run(Some(&args))?;
    execute(&ctx.root, ctx.run, &args)
}

/// The whole command, minus resolving `root`/`run` via `require_active_run` -
/// split out so it can be unit-tested against explicit values without
/// touching global process state (see `trust::execute`'s doc comment for
/// why).
fn execute(root: &Path, mut run: Run, args: &ParsedArgs) -> Result<(), UserError> {
    let amend = args.flags.is_true("amend");
    if !amend && run.phase != Phase::Plan {
        return Err(UserError::new(format!(
            "nothing to approve - run is in {}, not PLAN",
            run.phase
        )));
    }

    let plan_path = run_paths(root, &run.id).plan;
    let parsed = parse_plan_file(&plan_path);
    if parsed.plan.is_none() {
        return Err(UserError::new(format!(
            "cannot approve an invalid plan: {}",
            parsed.errors.join("; ")
        )));
    }
    let Some(plan_hash) = hash_plan_file(&plan_path) else {
        return Err(UserError::new("plan.md not found"));
    };

    let config = load_config(root)?;
    let by = identity::resolve(args, root);
    identity::require_if_configured(&by, &config)?;
    let reason = args.flags.str("reason").map(str::to_string);

    if amend {
        if run.approval.is_none() {
            return Err(UserError::new(
                "nothing to amend - plan was never approved; run `gate approve` first",
            ));
        }
        let Some(amendment) = &run.amendment else {
            return Err(UserError::new(
                "no amendment recorded - run `gate amend` first to review the diff and record intent",
            ));
        };
        if amendment.plan_hash != plan_hash {
            return Err(UserError::new(
                "plan.md changed again since `gate amend` - re-run `gate amend` on the current plan",
            ));
        }
        let approval = Approval {
            by: by.clone(),
            at: now_iso(),
            reason,
            plan_hash,
        };
        run.approval = Some(approval.clone());
        run.amendment = None;
        snapshot_approved_plan(root, &run.id, &plan_path)?;
        write_run(root, &mut run).map_err(|e| UserError::new(e.to_string()))?;

        let human =
            format!(
            "Amendment approved{}. Re-run the current gate (`gate check`/`gate next`) to continue.",
            by.as_deref().map(|b| format!(" by {b}")).unwrap_or_default()
        );
        let mut data = approval_to_data(&approval);
        // `{ approved: true, amended: true, ...run.approval }`: approved and
        // amended lead, then the approval's own fields in their declared
        // order - rebuild the object in that exact key order.
        let mut ordered = Value::object();
        ordered.insert("approved", true);
        ordered.insert("amended", true);
        if let Value::Object(entries) = &mut data {
            for (k, v) in entries.drain(..) {
                ordered.insert(k, v);
            }
        }
        emit(&human, &ordered, &args.flags)?;
        return Ok(());
    }

    let approval = Approval {
        by: by.clone(),
        at: now_iso(),
        reason,
        plan_hash,
    };
    run.approval = Some(approval.clone());
    snapshot_approved_plan(root, &run.id, &plan_path)?;
    write_run(root, &mut run).map_err(|e| UserError::new(e.to_string()))?;

    // The phase after PLAN varies by profile (bugfix skips straight to
    // DEBUG) - derive it from the run's own configured sequence rather than
    // hardcoding the feature-profile default, which prints an actively
    // wrong phase name on any other profile.
    let next = next_phase(Phase::Plan, &run.profile).unwrap_or(Phase::Done);
    let human = format!(
        "Plan approved{}. Run `gate next` to enter {next}.",
        by.as_deref()
            .map(|b| format!(" by {b}"))
            .unwrap_or_default()
    );
    let mut ordered = Value::object();
    ordered.insert("approved", true);
    if let Value::Object(entries) = approval_to_data(&approval) {
        for (k, v) in entries {
            ordered.insert(k, v);
        }
    }
    emit(&human, &ordered, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::run::{new_run, NewRunParams};
    use std::fs;

    const VALID_PLAN: &str = "---\ngoal: g\nfiles:\n  - a.ts\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan\n";

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = crate::core::testutil::unique_temp_dir(&format!("approve-rs-{name}"));
        dir
    }

    fn setup_run(root: &Path, plan: &str) -> Run {
        setup_run_with_profile(root, plan, "feature")
    }

    fn setup_run_with_profile(root: &Path, plan: &str, profile: &str) -> Run {
        fs::create_dir_all(root.join(".gate")).unwrap();
        let mut r = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: profile.to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        write_run(root, &mut r).unwrap();
        fs::write(run_paths(root, &r.id).plan, plan).unwrap();
        r
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["approve".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn approves_a_valid_plan_in_plan_phase() {
        let root = tmp_dir("approve-ok");
        let run = setup_run(&root, VALID_PLAN);
        let id = run.id.clone();
        execute(&root, run, &args(&[])).unwrap();

        let saved = crate::core::run::read_run(&root, &id).unwrap();
        assert!(saved.approval.is_some());
        assert!(run_paths(&root, &id).plan_approved.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_an_invalid_plan() {
        let root = tmp_dir("approve-invalid");
        let run = setup_run(&root, "not frontmatter");
        let err = execute(&root, run, &args(&[])).unwrap_err();
        assert!(err.message().contains("cannot approve an invalid plan"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn amend_without_a_prior_approval_is_an_error() {
        let root = tmp_dir("amend-no-approval");
        let run = setup_run(&root, VALID_PLAN);
        let err = execute(&root, run, &args(&["--amend"])).unwrap_err();
        assert!(err.message().contains("nothing to amend"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn records_by_from_the_by_flag() {
        let root = tmp_dir("approve-by");
        let run = setup_run(&root, VALID_PLAN);
        let id = run.id.clone();
        execute(&root, run, &args(&["--by", "alice"])).unwrap();
        let saved = crate::core::run::read_run(&root, &id).unwrap();
        assert_eq!(saved.approval.unwrap().by.as_deref(), Some("alice"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_approving_outside_plan_phase() {
        let root = tmp_dir("wrong-phase");
        let mut run = setup_run(&root, VALID_PLAN);
        run.phase = Phase::Implement;
        let err = execute(&root, run, &args(&[])).unwrap_err();
        assert!(err.message().contains("nothing to approve"));
        fs::remove_dir_all(&root).unwrap();
    }
}
