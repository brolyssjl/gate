//! Port of `src/commands/status.ts`: `gate status` - cheap metadata only.
//! On the agent hot path, so it never executes build/test commands.

use std::path::Path;

use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::core::config::load_config;
use crate::core::current::{
    clear_current_run_id, list_active_branches, read_current_run_id, resolve_branch_key,
    BranchKeyResolution, NO_GIT_BRANCH_KEY,
};
use crate::core::json::Value;
use crate::core::run::{read_run, Run};
use crate::core::state_machine::phase_sequence;
use crate::gates::has_gate;

struct OtherRun {
    branch: String,
    id: String,
    phase: crate::core::state_machine::Phase,
}

fn other_active_runs(root: &Path, exclude_key: Option<&str>) -> Result<Vec<OtherRun>, UserError> {
    let mut out = Vec::new();
    for (key, run_id) in list_active_branches(root)? {
        if Some(key.as_str()) == exclude_key {
            continue;
        }
        let Ok(run) = read_run(root, &run_id) else {
            continue; // unreadable run.json - leave it out, don't guess
        };
        out.push(OtherRun {
            branch: if key == NO_GIT_BRANCH_KEY {
                "(no git)".to_string()
            } else {
                key
            },
            id: run.id,
            phase: run.phase,
        });
    }
    Ok(out)
}

fn format_other(o: &OtherRun) -> String {
    format!("  {}: {} ({})", o.branch, o.id, o.phase)
}

fn others_to_json(others: &[OtherRun]) -> Value {
    let mut arr = Value::array();
    for o in others {
        let mut entry = Value::object();
        entry.insert("branch", o.branch.as_str());
        entry.insert("id", o.id.as_str());
        entry.insert("phase", o.phase.as_str());
        arr.push(entry);
    }
    arr
}

fn others_block(others: &[OtherRun]) -> String {
    if others.is_empty() {
        String::new()
    } else {
        format!(
            "\nOther runs in flight:\n{}",
            others
                .iter()
                .map(format_other)
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["status".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = require_root()?;
    execute(&root, &args)
}

/// The whole command, minus resolving `root` from the process's current
/// directory - split out so it can be unit-tested against an explicit root
/// without touching global process state (see `trust::execute`'s doc comment
/// for why).
fn execute(root: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    load_config(root)?; // validate config is loadable; surfaces parse errors early

    let resolved = resolve_branch_key(root);
    let exclude_key = match &resolved {
        BranchKeyResolution::Key(k) => Some(k.as_str()),
        BranchKeyResolution::Detached => None,
    };
    let others = other_active_runs(root, exclude_key)?;

    let BranchKeyResolution::Key(key) = resolved else {
        let block = others_block(&others);
        let human = format!(
            "HEAD is detached - no branch to resolve a current run from; pass --run <id> to a phase command.\n{}",
            if others.is_empty() { "No runs in flight.".to_string() } else { block }
        );
        let mut data = Value::object();
        data.insert("active", false);
        data.insert("detached", true);
        data.insert("others", others_to_json(&others));
        return emit(&human, &data, &args.flags);
    };

    let id = read_current_run_id(root, &key)?;
    let Some(id) = id else {
        let mut lines = vec!["No active run. Start one with: gate start \"<title>\"".to_string()];
        if !others.is_empty() {
            lines.push(others_block(&others));
        }
        let human = lines.join("\n");
        let mut data = Value::object();
        data.insert("active", false);
        data.insert("others", others_to_json(&others));
        return emit(&human, &data, &args.flags);
    };

    let run: Run = match read_run(root, &id) {
        Ok(r) => r,
        Err(_) => {
            // Dangling mapping - self-heal instead of crashing with an
            // uncaught "run not found".
            clear_current_run_id(root, &key)?;
            let mut lines = vec![format!(
                "No active run (a stale pointer to \"{id}\" was cleared - its run folder is gone)."
            )];
            if !others.is_empty() {
                lines.push(others_block(&others));
            }
            let human = lines.join("\n");
            let mut data = Value::object();
            data.insert("active", false);
            data.insert("healed", id);
            data.insert("others", others_to_json(&others));
            return emit(&human, &data, &args.flags);
        }
    };

    let artifacts: Vec<String> = run.artifacts.iter().map(|(k, _)| k.clone()).collect();
    let phases = phase_sequence(&run.profile);
    let next_action = if has_gate(run.phase) {
        format!("run `gate check` to see if the {} gate passes", run.phase)
    } else {
        format!("{} is terminal", run.phase)
    };

    let flow = phases
        .iter()
        .map(|p| {
            if *p == run.phase {
                format!("[{p}]")
            } else {
                p.as_str().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" \u{2192} ");

    let mut lines = vec![
        format!("Run:      {}", run.id),
        format!("Title:    {}", run.title),
        format!(
            "Branch:   {}",
            run.branch.as_deref().unwrap_or("(no branch)")
        ),
        format!(
            "Phase:    {}  (profile {}, status {})",
            run.phase,
            run.profile,
            run.status.as_str()
        ),
        format!("Flow:     {flow}"),
        format!(
            "Base:     {}",
            run.base_ref.as_deref().unwrap_or("(no git base)")
        ),
        if artifacts.is_empty() {
            "Artifacts: none".to_string()
        } else {
            format!("Artifacts: {}", artifacts.join(", "))
        },
        format!("Next:     {next_action}"),
    ];
    if !others.is_empty() {
        lines.push(others_block(&others));
    }
    let human = lines.join("\n");

    let mut data = Value::object();
    data.insert("active", true);
    data.insert("id", run.id.as_str());
    data.insert("title", run.title.as_str());
    data.insert("branch", run.branch.clone());
    data.insert("phase", run.phase.as_str());
    data.insert("profile", run.profile.as_str());
    data.insert(
        "phases",
        Value::Array(phases.iter().map(|p| Value::from(p.as_str())).collect()),
    );
    data.insert("status", run.status.as_str());
    data.insert("baseRef", run.base_ref.clone());
    data.insert("sessionId", run.session_id.clone());
    data.insert("artifacts", artifacts);
    data.insert("nextAction", next_action);
    data.insert("others", others_to_json(&others));
    emit(&human, &data, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::run::{new_run, write_run, NewRunParams};
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-status-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".gate")).unwrap();
        dir
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["status".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn reports_no_active_run_in_a_fresh_gate_project() {
        let root = tmp_dir("no-run");
        execute(&root, &args(&[])).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reports_an_active_run_for_the_current_branch() {
        let root = tmp_dir("active-run");
        let mut r = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "My Run".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        write_run(&root, &mut r).unwrap();
        fs::write(
            root.join(".gate/current.json"),
            r#"{"schema":1,"branches":{"~no-git~":"r1"}}"#,
        )
        .unwrap();
        execute(&root, &args(&["--json"])).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn self_heals_a_stale_current_pointer_to_a_removed_run() {
        let root = tmp_dir("stale-pointer");
        fs::write(
            root.join(".gate/current.json"),
            r#"{"schema":1,"branches":{"~no-git~":"ghost"}}"#,
        )
        .unwrap();
        execute(&root, &args(&[])).unwrap();
        let current = fs::read_to_string(root.join(".gate/current.json")).unwrap();
        assert!(!current.contains("ghost"));
        fs::remove_dir_all(&root).unwrap();
    }
}
