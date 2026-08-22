//! Port of `src/commands/amend.ts`.
//!
//! `gate amend` - show the diff between the current plan.md and the last
//! approved snapshot, and record intent to re-approve it (Milestone 5).
//! Does not itself re-approve - that's `gate approve --amend` - so a delta
//! re-approval always happens after a human has actually seen the diff,
//! the same discipline `gate approve` already applies to the initial
//! sign-off.

use std::fs;
use std::path::Path;

use crate::artifacts::plan::{hash_plan_file, parse_plan_file};
use crate::cli::args::parse_args;
use crate::cli::context::require_active_run;
use crate::cli::output::{emit, UserError};
use crate::core::git::diff_no_index;
use crate::core::json::Value;
use crate::core::paths::run_paths;
use crate::core::run::{now_iso, write_run, Amendment};

struct DiffResult {
    diff: String,
    trusted_snapshot: bool,
    warning: Option<String>,
}

/// The diff `gate amend` shows, only ever computed from a snapshot proven
/// to be the plan `gate approve` actually recorded (review finding F2):
/// trusting `plan.approved.md` on its face would let a doctored snapshot
/// produce an empty or misleading diff. The snapshot is trustworthy only
/// when its own hash matches `approval.planHash`; anything else (missing,
/// unreadable, or hash-mismatched) falls back to showing the full current
/// plan with a loud warning, never a diff computed against content that
/// can't be trusted.
fn render_diff(approved_path: &Path, plan_path: &Path, approved_hash: &str) -> DiffResult {
    if !approved_path.exists() {
        let current = fs::read_to_string(plan_path).unwrap_or_default();
        return DiffResult {
            diff: format!(
                "(no approved snapshot on disk - showing the full current plan)\n\n{current}"
            ),
            trusted_snapshot: false,
            warning: None,
        };
    }
    let snapshot_hash = hash_plan_file(approved_path);
    if snapshot_hash.as_deref() != Some(approved_hash) {
        let current = fs::read_to_string(plan_path).unwrap_or_default();
        return DiffResult {
            diff: current,
            trusted_snapshot: false,
            warning: Some(
                "WARNING: the approved-plan snapshot on disk does not match the recorded approval hash \
                 (edited or corrupted since `gate approve` wrote it) - showing the full current plan \
                 instead of an untrustworthy diff. Review it in full before re-approving."
                    .to_string(),
            ),
        };
    }
    DiffResult {
        diff: diff_no_index(
            &approved_path.to_string_lossy(),
            &plan_path.to_string_lossy(),
        ),
        trusted_snapshot: true,
        warning: None,
    }
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    // See `check::run`'s comment: prepend a placeholder command token so
    // `parse_args` never misreads a leading flag/positional as the command.
    let mut full = vec!["amend".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let ctx = require_active_run(Some(&args))?;
    let mut run = ctx.run;
    let Some(approval) = run.approval.clone() else {
        return Err(UserError::new(
            "plan was never approved - nothing to amend; run `gate approve` first",
        ));
    };

    let paths = run_paths(&ctx.root, &run.id);
    let parsed = parse_plan_file(&paths.plan);
    if parsed.plan.is_none() {
        return Err(UserError::new(format!(
            "cannot amend an invalid plan: {}",
            parsed.errors.join("; ")
        )));
    }
    let Some(current_hash) = hash_plan_file(&paths.plan) else {
        return Err(UserError::new("plan.md not found"));
    };
    if current_hash == approval.plan_hash {
        return Err(UserError::new(
            "plan.md matches the approved version - nothing to amend",
        ));
    }

    let DiffResult {
        diff,
        trusted_snapshot,
        warning,
    } = render_diff(&paths.plan_approved, &paths.plan, &approval.plan_hash);

    let by = args
        .flags
        .str("by")
        .map(String::from)
        .or_else(|| std::env::var("GATE_SESSION_ID").ok());

    run.amendment = Some(Amendment {
        plan_hash: current_hash,
        at: now_iso(),
        by: by.clone(),
    });
    write_run(&ctx.root, &mut run).map_err(|e| UserError::new(e.to_string()))?;

    let by_suffix = by
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| format!(" by {s}"))
        .unwrap_or_default();
    let diff_trimmed = diff.trim();
    let human = [
        format!("Amendment recorded{by_suffix}."),
        warning
            .clone()
            .unwrap_or_else(|| "Diff vs the approved plan:".to_string()),
        String::new(),
        if diff_trimmed.is_empty() {
            "(no textual diff produced)".to_string()
        } else {
            diff_trimmed.to_string()
        },
        String::new(),
        "Run `gate approve --amend` to re-approve this delta.".to_string(),
    ]
    .join("\n");

    let amendment = run.amendment.as_ref().unwrap();
    let mut data = Value::object();
    data.insert("amended", true);
    data.insert("diff", diff.as_str());
    data.insert("trustedSnapshot", trusted_snapshot);
    data.insert("warning", warning.clone());
    data.insert("planHash", amendment.plan_hash.as_str());
    data.insert("at", amendment.at.as_str());
    data.insert("by", amendment.by.clone());

    emit(&human, &data, &args.flags)
}
