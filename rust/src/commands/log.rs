//! Port of `src/commands/log.ts`: `gate log <file>` - register an artifact
//! against the current phase.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_active_run;
use crate::cli::output::{emit, UserError};
use crate::core::json::Value;
use crate::core::paths::run_paths;
use crate::core::run::{now_iso, write_run, ArtifactEntry, Run};

/// Minimal port of Node's `path.resolve(base, target)`: joins `target` onto
/// `base` when relative (an absolute `target` wins outright), then
/// lexically collapses `.`/`..` segments - no symlink resolution, no
/// filesystem access, matching Node's own purely-textual behavior.
fn resolve_path(base: &Path, target: &str) -> PathBuf {
    let target_path = Path::new(target);
    let joined: PathBuf = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        base.join(target_path)
    };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Heuristic for "this file is probably someone trying to hand-register test
/// evidence" - broader than the literal `test-report.json` name Gate itself
/// writes, so `gate log jest-results.json` (or `results.json`, `junit.xml`,
/// ...) also gets pointed at the real mechanism instead of registering
/// silently and leaving the author to discover the gate ignores it.
fn looks_like_test_report(name: &str) -> bool {
    let lower = name.to_lowercase();
    let ext_ok = lower.ends_with(".json") || lower.ends_with(".xml");
    let keyword_ok = ["test", "report", "result", "junit"]
        .iter()
        .any(|k| lower.contains(k));
    ext_ok && keyword_ok
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["log".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let ctx = require_active_run(Some(&args))?;
    let cwd = std::env::current_dir().map_err(|e| UserError::new(e.to_string()))?;
    execute(&ctx.root, ctx.run, &cwd, &args)
}

/// The whole command, minus resolving `root`/`run` (via `require_active_run`)
/// and `cwd` from the process - split out so it can be unit-tested against
/// explicit values without touching global process state (see
/// `trust::execute`'s doc comment for why).
fn execute(root: &Path, mut run: Run, cwd: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    let Some(arg) = args.positionals.first() else {
        return Err(UserError::usage("gate log needs a file: gate log <file>"));
    };
    let src = resolve_path(cwd, arg);
    if !src.exists() {
        return Err(UserError::new(format!("file not found: {arg}")));
    }

    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dest = resolve_path(&run_paths(root, &run.id).dir, &name);
    if src != dest {
        fs::copy(&src, &dest).map_err(|e| UserError::new(e.to_string()))?;
    }

    let phase = run.phase;
    if let Some(slot) = run.artifacts.iter_mut().find(|(k, _)| k == &name) {
        slot.1 = ArtifactEntry {
            phase,
            at: now_iso(),
        };
    } else {
        run.artifacts.push((
            name.clone(),
            ArtifactEntry {
                phase,
                at: now_iso(),
            },
        ));
    }
    write_run(root, &mut run).map_err(|e| UserError::new(e.to_string()))?;

    let note = if looks_like_test_report(&name) {
        " (heads up: this alone won't count as TEST evidence - the gate only trusts the test command's \
         own stdout, or a file it writes to $GATE_TEST_REPORT during the run; see `gate playbook TEST` \
         for the real mechanism)"
    } else {
        ""
    };
    let human = format!("Registered artifact \"{name}\" against {phase}{note}");

    let mut data = Value::object();
    data.insert("artifact", name);
    data.insert("phase", phase.as_str());
    emit(&human, &data, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::run::{new_run, NewRunParams};
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = crate::core::testutil::unique_temp_dir(&format!("log-rs-{name}"));
        dir
    }

    fn setup_run(root: &Path) -> Run {
        fs::create_dir_all(root.join(".gate")).unwrap();
        let mut r = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        write_run(root, &mut r).unwrap();
        r
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["log".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn registers_an_artifact_and_copies_it_into_the_run_folder() {
        let root = tmp_dir("basic");
        let run = setup_run(&root);
        let id = run.id.clone();
        fs::write(root.join("notes.md"), "hi").unwrap();

        execute(&root, run, &root, &args(&["notes.md"])).unwrap();

        assert!(root.join(".gate/runs").join(&id).join("notes.md").exists());
        let saved = crate::core::run::read_run(&root, &id).unwrap();
        assert!(saved.artifacts.iter().any(|(k, _)| k == "notes.md"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn errors_when_the_file_does_not_exist() {
        let root = tmp_dir("missing-file");
        let run = setup_run(&root);
        let err = execute(&root, run, &root, &args(&["nope.md"])).unwrap_err();
        assert!(err.message().contains("file not found"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn requires_a_file_argument() {
        let root = tmp_dir("no-arg");
        let run = setup_run(&root);
        let err = execute(&root, run, &root, &args(&[])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn flags_a_test_report_looking_filename_with_a_heads_up_note() {
        assert!(looks_like_test_report("test-report.json"));
        assert!(looks_like_test_report("junit.xml"));
        assert!(looks_like_test_report("RESULTS.JSON"));
        assert!(!looks_like_test_report("notes.md"));
    }

    #[test]
    fn resolve_path_collapses_parent_dir_segments() {
        let base = Path::new("/a/b/c");
        assert_eq!(resolve_path(base, "../x"), Path::new("/a/b/x"));
        assert_eq!(resolve_path(base, "/abs/y"), Path::new("/abs/y"));
        assert_eq!(resolve_path(base, "./z"), Path::new("/a/b/c/z"));
    }
}
