//! Port of `src/cli/context.ts`: resolving the `.gate/` root and the run a
//! phase-scoped command operates on.

use std::env;
use std::path::PathBuf;

use crate::cli::args::ParsedArgs;
use crate::cli::output::UserError;
use crate::core::config::{load_config, GateConfig};
use crate::core::current::{
    read_current_run_id, resolve_branch_key, BranchKeyResolution, NO_GIT_BRANCH_KEY,
};
use crate::core::paths::{find_gate_root, validate_run_id};
use crate::core::run::{read_run, Run, RunStatus};

/// Port of `requireRoot`.
pub fn require_root() -> Result<PathBuf, UserError> {
    let cwd = env::current_dir().map_err(|e| UserError::new(e.to_string()))?;
    find_gate_root(&cwd).ok_or_else(|| UserError::new("no .gate/ found - run `gate init` first"))
}

/// Port of `ActiveContext`.
pub struct ActiveContext {
    pub root: PathBuf,
    pub run: Run,
    pub config: GateConfig,
}

/// Port of `requireActiveRun`: `--run <id>` always wins (works even in
/// detached HEAD - its stated use case); otherwise the active run for the
/// current branch. A detached HEAD has no branch to resolve "current" from,
/// so it refuses with a pointer to `--run` rather than guessing. A `--run`
/// pointing at a non-active run, or at a run recorded on a branch other than
/// the checked-out one, is refused (see the TS doc comment for why `--run`
/// is not a bare bypass).
pub fn require_active_run(args: Option<&ParsedArgs>) -> Result<ActiveContext, UserError> {
    let root = require_root()?;
    let config = load_config(&root)?;

    let explicit = args.and_then(|a| a.flags.str("run")).map(str::to_string);
    if let Some(explicit) = explicit {
        validate_run_id(&explicit)?;
        let run = read_run(&root, &explicit)?;
        if run.status != RunStatus::Active {
            return Err(UserError::new(format!(
                "run \"{}\" is {}, not active - --run only selects an in-flight run",
                explicit,
                run.status.as_str()
            )));
        }
        let resolved = resolve_branch_key(&root);
        if let (Some(branch), BranchKeyResolution::Key(key)) = (&run.branch, &resolved) {
            if key != branch {
                return Err(UserError::new(format!(
                    "run \"{explicit}\" was started on branch \"{branch}\", but \"{key}\" is checked out - checkout \"{branch}\" first (or detach HEAD) to act on this run"
                )));
            }
        }
        return Ok(ActiveContext { root, run, config });
    }

    let resolved = resolve_branch_key(&root);
    let key = match resolved {
        BranchKeyResolution::Detached => {
            return Err(UserError::new(
                "HEAD is detached - no branch to resolve the active run from; pass --run <id> (see `gate status` for runs in flight)",
            ));
        }
        BranchKeyResolution::Key(key) => key,
    };
    let id = read_current_run_id(&root, &key)?;
    let Some(id) = id else {
        return Err(UserError::new(if key == NO_GIT_BRANCH_KEY {
            "no active run - start one with `gate start \"<title>\"`".to_string()
        } else {
            format!("no active run on branch \"{key}\" - start one with `gate start \"<title>\"`")
        }));
    };
    // Gate report finding 4: `id` came from `.gate/current.json`, not free
    // text a caller already validated - reject an absolute/`..` id before
    // it reaches `read_run`/`run_paths`.
    validate_run_id(&id)?;
    let run = read_run(&root, &id)?;
    Ok(ActiveContext { root, run, config })
}
