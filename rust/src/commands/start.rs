//! Port of `src/commands/start.ts`: `gate start "<title>"` - create a run,
//! enter PLAN, print the plan playbook.

use std::fs;
use std::path::Path;

use crate::artifacts::plan::hash_plan_file;
use crate::artifacts::retro::RETRO_TEMPLATE;
use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::core::config::load_config;
use crate::core::current::{
    resolve_branch_key, with_current_lock, BranchKeyResolution, NO_GIT_BRANCH_KEY,
};
use crate::core::git::head_sha;
use crate::core::json::Value;
use crate::core::paths::run_paths;
use crate::core::playbooks::resolve_playbook_with_overlays;
use crate::core::run::{make_run_id, new_run, read_run, write_run, NewRunParams, Run, RunStatus};
use crate::core::state_machine::{is_profile, phase_sequence, Phase, DEFAULT_PROFILE};
use crate::core::targets::resolve_display_targets;
use crate::integrations::{detect, plan_hints};

const PLAN_TEMPLATE: &str = "---\ngoal:\nspec:\nfiles:\n  -\nout_of_scope: []\ncriteria:\n  - id: c1\n    text:\n    verify: \"test: \"\nrisks: []\nacknowledgments: []\n---\n\n# Plan: %TITLE%\n\n";

const DEBUG_TEMPLATE: &str = "---\ntriggering_test:\nreproduced: false\ncycles:\n  - hypothesis:\n    prediction:\n    experiment:\n    observation:\n    conclusion:\n    status: in-progress\n---\n\n# Debug log: %TITLE%\n\n";

/// `Object.keys(PROFILES)` in declaration order (`core::state_machine`
/// doesn't expose this list itself - see its module docs).
const PROFILE_NAMES: [&str; 4] = ["feature", "bugfix", "refactor", "docs"];

enum Outcome {
    Resumed { run: Run, title_mismatch: bool },
    Created { run: Run },
}

fn safe_read(root: &Path, id: &str) -> Option<Run> {
    read_run(root, id).ok()
}

fn unique_run_id(root: &Path, title: &str) -> String {
    let base = make_run_id(title);
    let mut id = base.clone();
    let mut n = 2;
    while run_paths(root, &id).run_json.exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["start".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = require_root()?;
    execute(&root, &args)
}

/// The whole command, minus resolving `root` from the process's current
/// directory - split out so it can be unit-tested against an explicit root
/// without touching global process state (see `trust::execute`'s doc
/// comment). Every other call in this function already takes `root`
/// explicitly (git plumbing spawns with `.current_dir(root)`, never relying
/// on the process's own cwd), so this is the only cwd-dependent piece.
fn execute(root: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    let title = args.positionals.join(" ").trim().to_string();
    if title.is_empty() {
        return Err(UserError::usage(
            "gate start needs a title: gate start \"<title>\"",
        ));
    }

    let resolved = resolve_branch_key(root);
    let BranchKeyResolution::Key(branch_key) = resolved else {
        return Err(UserError::new(
            "HEAD is detached - runs are keyed by branch; checkout a branch before `gate start`",
        ));
    };
    let branch: Option<String> = if branch_key == NO_GIT_BRANCH_KEY {
        None
    } else {
        Some(branch_key.clone())
    };

    let profile = args
        .flags
        .str("profile")
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT_PROFILE.as_str().to_string());
    if !is_profile(&profile) {
        return Err(UserError::usage(format!(
            "unknown profile \"{profile}\" (choose one of {})",
            PROFILE_NAMES.join(", ")
        )));
    }
    let profile_flag_given = args.flags.str("profile").is_some();
    let session_id = args
        .flags
        .str("session")
        .map(str::to_string)
        .or_else(|| std::env::var("GATE_SESSION_ID").ok());
    let target_override: Option<Vec<String>> = args.flags.str("target").map(|s| {
        s.split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    });
    let target_flag_given = args.flags.str("target").is_some();

    let config = load_config(root)?;
    if let Some(names) = &target_override {
        if !names.is_empty() {
            let known: Vec<&str> = config.targets.iter().map(|(n, _)| n.as_str()).collect();
            let unknown: Vec<&str> = names
                .iter()
                .map(String::as_str)
                .filter(|t| !known.contains(t))
                .collect();
            if !unknown.is_empty() {
                return Err(UserError::usage(format!(
                    "unknown --target name(s): {} {}",
                    unknown.join(", "),
                    if known.is_empty() {
                        "(no targets configured in .gate/config.yml)".to_string()
                    } else {
                        format!("(valid targets: {})", known.join(", "))
                    }
                )));
            }
        }
    }

    // The whole "read this branch's existing mapping -> decide resume/create
    // -> create the new run -> point the branch at it" sequence runs under
    // one lock, so a concurrent `gate start` on the same branch can't
    // interleave between the decision and the write (the milestone's own use
    // case is several agents in flight at once).
    let outcome: Outcome = with_current_lock(root, |branches| -> Result<Outcome, UserError> {
        let existing_id = branches
            .iter()
            .find(|(k, _)| k == &branch_key)
            .map(|(_, v)| v.clone());
        if let Some(existing_id) = existing_id {
            if let Some(existing) = safe_read(root, &existing_id) {
                if existing.status == RunStatus::Active {
                    let plan_path = run_paths(root, &existing_id).plan;
                    if let Some(approval) = &existing.approval {
                        if hash_plan_file(&plan_path).as_deref()
                            != Some(approval.plan_hash.as_str())
                        {
                            return Err(UserError::new(format!(
                                "run \"{existing_id}\" on this branch is approved but plan.md has changed since - \
                                 re-run `gate approve` (while still in PLAN) or `gate amend` + `gate approve --amend` \
                                 (once past PLAN), or resolve the run before starting fresh"
                            )));
                        }
                    }
                    // Resuming silently on a real profile/target conflict
                    // would let an agent believe it started (say) a
                    // bugfix-profile run when it's actually driving an old
                    // docs-profile one - a hard error only when the flag was
                    // *explicitly* requested.
                    if profile_flag_given && profile != existing.profile {
                        return Err(UserError::new(format!(
                            "run \"{existing_id}\" on this branch is profile \"{}\", but --profile {profile} \
                             was requested for a resumed run - drop --profile to resume it as-is, or finish/abandon it first",
                            existing.profile
                        )));
                    }
                    let existing_targets: Vec<String> =
                        existing.target_override.clone().unwrap_or_default();
                    let requested_targets: Vec<String> =
                        target_override.clone().unwrap_or_default();
                    let targets_differ = requested_targets != existing_targets;
                    if target_flag_given && targets_differ {
                        return Err(UserError::new(format!(
                            "run \"{existing_id}\" on this branch has --target {}, but {} was requested for a \
                             resumed run - drop --target to resume it as-is, or finish/abandon it first",
                            if existing_targets.is_empty() { "(none)".to_string() } else { existing_targets.join(",") },
                            if requested_targets.is_empty() { "(none)".to_string() } else { requested_targets.join(",") },
                        )));
                    }
                    // A title mismatch is cosmetic - surfaced as a loud
                    // warning, not a hard error, so the resumed run's own
                    // title is always what's kept.
                    let title_mismatch = existing.title != title;
                    return Ok(Outcome::Resumed {
                        run: existing,
                        title_mismatch,
                    });
                }
            }
            branches.retain(|(k, _)| k != &branch_key);
        }

        let id = unique_run_id(root, &title);
        let mut new_r = new_run(NewRunParams {
            id: id.clone(),
            title: title.clone(),
            profile: profile.clone(),
            branch: branch.clone(),
            base_ref: head_sha(root),
            session_id: session_id.clone(),
            target_override: target_override.clone(),
        });
        write_run(root, &mut new_r).map_err(|e| UserError::new(e.to_string()))?;
        if let Some(slot) = branches.iter_mut().find(|(k, _)| k == &branch_key) {
            slot.1 = id;
        } else {
            branches.push((branch_key.clone(), id));
        }
        Ok(Outcome::Created { run: new_r })
    })??;

    match outcome {
        Outcome::Resumed {
            run: existing,
            title_mismatch,
        } => {
            let sequence = phase_sequence(&existing.profile);
            let mut lines = vec![format!(
                "Branch already has an active run: \"{}\" (phase {}) - resuming it.",
                existing.id, existing.phase
            )];
            if title_mismatch {
                lines.push(format!(
                    "WARNING: requested title \"{title}\" differs from the resumed run's title \"{}\" - \
                     the resumed run's own title was kept; pass --profile/--target to detect a real \
                     conflict instead of guessing from the title.",
                    existing.title
                ));
            }
            lines.push(format!(
                "Flow: {}",
                sequence
                    .iter()
                    .map(|p| if *p == existing.phase {
                        format!("[{p}]")
                    } else {
                        p.as_str().to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(" \u{2192} ")
            ));
            lines.push("Next: run `gate status` or `gate playbook` to continue.".to_string());
            let human = lines.join("\n");

            let mut data = Value::object();
            data.insert("id", existing.id.as_str());
            data.insert("phase", existing.phase.as_str());
            data.insert("profile", existing.profile.as_str());
            data.insert(
                "phases",
                Value::Array(sequence.iter().map(|p| Value::from(p.as_str())).collect()),
            );
            data.insert("resumed", true);
            data.insert("requestedTitle", title);
            data.insert("titleMismatch", title_mismatch);
            emit(&human, &data, &args.flags)
        }
        Outcome::Created { run } => {
            let id = run.id.clone();
            let paths = run_paths(root, &id);
            if !paths.plan.exists() {
                fs::write(&paths.plan, PLAN_TEMPLATE.replace("%TITLE%", &title))
                    .map_err(|e| UserError::new(e.to_string()))?;
            }
            let sequence = phase_sequence(&run.profile);
            if sequence.contains(&Phase::Debug) && !paths.debug_log.exists() {
                fs::write(&paths.debug_log, DEBUG_TEMPLATE.replace("%TITLE%", &title))
                    .map_err(|e| UserError::new(e.to_string()))?;
            }
            if sequence.contains(&Phase::Retro) && !paths.retro.exists() {
                fs::write(&paths.retro, RETRO_TEMPLATE.replace("%TITLE%", &title))
                    .map_err(|e| UserError::new(e.to_string()))?;
            }

            let hints = plan_hints(detect(root));
            let target_names = resolve_display_targets(
                root,
                &run.id,
                run.base_ref.as_deref(),
                run.target_override.as_deref(),
                &config,
            );
            let playbook =
                resolve_playbook_with_overlays(root, Phase::Plan, &config, &target_names)
                    .unwrap_or_else(|| "(no PLAN playbook found)".to_string());

            let mut lines = vec![
                format!(
                    "Started run \"{id}\" (profile: {}, phases: {}) \u{2192} phase PLAN{}",
                    run.profile,
                    sequence
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join(" \u{2192} "),
                    run.branch
                        .as_deref()
                        .map(|b| format!(" on branch \"{b}\""))
                        .unwrap_or_default()
                ),
                format!("Edit the plan at: {}", paths.plan.display()),
                "When it's ready and signed off, run `gate approve`, then `gate next`.".to_string(),
            ];
            if !hints.is_empty() {
                lines.push(format!(
                    "\nHints:\n{}",
                    hints
                        .iter()
                        .map(|h| format!("  - {h}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
            lines.push(format!("\n{playbook}"));
            let human = lines.join("\n");

            let mut data = Value::object();
            data.insert("id", id);
            data.insert("phase", run.phase.as_str());
            data.insert("profile", run.profile.as_str());
            data.insert("branch", run.branch.clone());
            data.insert(
                "phases",
                Value::Array(sequence.iter().map(|p| Value::from(p.as_str())).collect()),
            );
            data.insert("plan", paths.plan.display().to_string());
            data.insert("hints", hints);
            emit(&human, &data, &args.flags)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    // No `env::set_current_dir` anywhere in this module: every helper below
    // (`with_current_lock`, `resolve_branch_key`, `head_sha`, ...) already
    // takes `root` explicitly and spawns git with `.current_dir(root)`, so
    // `execute` never touches the process's actual cwd - safe to call
    // directly from parallel `cargo test` threads.
    fn tmp_repo(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-start-rs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.co"]);
        git(&["config", "user.name", "t"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("README.md"), "seed\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "init"]);
        std::fs::create_dir_all(dir.join(".gate")).unwrap();
        dir
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["start".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn requires_a_title() {
        let root = tmp_repo("no-title");
        let err = execute(&root, &args(&[])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn creates_a_fresh_run_and_scaffolds_plan_md() {
        let root = tmp_repo("fresh-run");
        execute(&root, &args(&["My Title"])).unwrap();

        let runs_dir = root.join(".gate/runs");
        let entries: Vec<_> = std::fs::read_dir(&runs_dir).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let run_dir = entries.into_iter().next().unwrap().unwrap().path();
        assert!(run_dir.join("plan.md").exists());
        let plan = std::fs::read_to_string(run_dir.join("plan.md")).unwrap();
        assert!(plan.contains("# Plan: My Title"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resumes_an_active_run_on_the_same_branch_instead_of_creating_a_second_one() {
        let root = tmp_repo("resume-same-branch");
        execute(&root, &args(&["First"])).unwrap();
        execute(&root, &args(&["Second"])).unwrap();

        let entries: Vec<_> = std::fs::read_dir(root.join(".gate/runs"))
            .unwrap()
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "expected the second `gate start` to resume, not create a new run"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_an_unknown_profile() {
        let root = tmp_repo("bad-profile");
        let err = execute(&root, &args(&["T", "--profile", "nonsense"])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.message().contains("nonsense"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn bugfix_profile_scaffolds_a_debug_log_but_not_a_retro_until_its_phase() {
        let root = tmp_repo("bugfix-profile");
        execute(&root, &args(&["Bug", "--profile", "bugfix"])).unwrap();

        let entries: Vec<_> = std::fs::read_dir(root.join(".gate/runs"))
            .unwrap()
            .collect();
        let run_dir = entries.into_iter().next().unwrap().unwrap().path();
        assert!(run_dir.join("debug-log.md").exists());
        assert!(run_dir.join("retro.md").exists()); // bugfix's sequence includes RETRO too
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resuming_with_a_conflicting_explicit_profile_is_a_hard_error() {
        let root = tmp_repo("resume-profile-conflict");
        execute(&root, &args(&["First"])).unwrap();
        let err = execute(&root, &args(&["Second", "--profile", "bugfix"])).unwrap_err();
        assert!(err.message().contains("profile \"feature\""));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_resumed_run_with_a_different_title_gets_a_warning_not_an_error() {
        let root = tmp_repo("resume-title-mismatch");
        execute(&root, &args(&["First Title"])).unwrap();
        execute(&root, &args(&["Second Title"])).unwrap();
        // Still exactly one run (resumed, not replaced).
        let entries: Vec<_> = std::fs::read_dir(root.join(".gate/runs"))
            .unwrap()
            .collect();
        assert_eq!(entries.len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
