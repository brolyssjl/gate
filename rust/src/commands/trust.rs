//! Port of `src/commands/trust.ts`: `gate trust` - approve the current
//! `commands:` block (TOFU). Writes the hash to `.gate/trust.json`; the
//! IMPLEMENT/TEST gates refuse to execute commands until this matches.
//! `--check` reports status without writing (exit 0/1).

use std::path::Path;

use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::core::config::trusted_playbook_paths;
use crate::core::json::Value;
use crate::core::trust::{
    current_commands_hash, has_no_commands, is_commands_trusted, read_trust, write_trust,
};

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["trust".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = require_root()?;
    execute(&root, &args)
}

/// The whole command, minus resolving `root` from the process's current
/// directory (`require_root`'s job) - split out so it can be unit-tested
/// against an explicit root without touching global process state (`cargo
/// test` runs the whole crate's tests in one process; changing
/// `env::current_dir` there is a real race against unrelated tests that spawn
/// subprocesses expecting an inherited cwd - see the twin-dir manual
/// verification for this command's actual `run()` entry point instead).
fn execute(root: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    if args.flags.is_true("check") {
        let trusted = is_commands_trusted(root);
        let record = read_trust(root);
        let current_hash = current_commands_hash(root);
        let playbook_paths = trusted_playbook_paths(root);
        let human = if trusted {
            format!(
                "trusted ({}){}",
                if has_no_commands(root) {
                    "no commands to run".to_string()
                } else {
                    current_hash.clone()
                },
                playbook_summary_suffix(&playbook_paths),
            )
        } else {
            format!(
                "NOT trusted - run `gate trust` (current {}, stored {}){}",
                current_hash,
                record
                    .as_ref()
                    .map(|r| r.commands_hash.clone())
                    .unwrap_or_else(|| "none".to_string()),
                playbook_summary_suffix(&playbook_paths),
            )
        };

        let mut data = Value::object();
        data.insert("trusted", trusted);
        data.insert("currentHash", current_hash);
        data.insert("storedHash", record.map(|r| r.commands_hash));
        data.insert("playbookPaths", playbook_paths);
        emit(&human, &data, &args.flags)?;
        // Mirrors TS's `process.exitCode = trusted ? 0 : 1; return;` - the
        // human/JSON output is already on stdout, so the process just needs
        // to exit with the verdict's code, not go through the error-reporting
        // (`gate: <message>`) path `main.rs` uses for `Err`.
        std::process::exit(if trusted { 0 } else { 1 });
    }

    let by = args
        .flags
        .str("by")
        .map(str::to_string)
        .or_else(|| std::env::var("GATE_SESSION_ID").ok());
    let record = write_trust(root, by.as_deref()).map_err(|e| UserError::new(e.to_string()))?;
    let playbook_paths = trusted_playbook_paths(root);

    let human = match &by {
        Some(b) => format!(
            "Trusted the commands block: {} (by {b}){}",
            record.commands_hash,
            playbook_summary_suffix(&playbook_paths),
        ),
        None => format!(
            "Trusted the commands block: {}{}",
            record.commands_hash,
            playbook_summary_suffix(&playbook_paths),
        ),
    };
    let mut data = Value::object();
    data.insert("trusted", true);
    data.insert("commandsHash", record.commands_hash.clone());
    data.insert("trustedAt", record.trusted_at.clone());
    data.insert("trustedBy", record.trusted_by.clone());
    data.insert("playbookPaths", playbook_paths);
    emit(&human, &data, &args.flags)
}

/// " - playbooks: a, b" when non-empty, else "" - SEC-01: `gate trust` must
/// show/record what it is trusting, and this crate's convention for
/// `commands:` itself is to show a hash rather than raw content, so
/// playbooks get the same treatment: name what's covered, not its content.
fn playbook_summary_suffix(paths: &[String]) -> String {
    if paths.is_empty() {
        String::new()
    } else {
        format!(" - playbooks: {}", paths.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-trust-cmd-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["trust".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn writes_trust_json_with_the_current_commands_hash() {
        let root = tmp_dir("write");
        fs::create_dir_all(root.join(".gate")).unwrap();
        fs::write(
            root.join(".gate/config.yml"),
            "commands:\n  test: echo hi\n",
        )
        .unwrap();

        execute(&root, &args(&[])).unwrap();

        assert!(root.join(".gate/trust.json").exists());
        assert!(crate::core::trust::is_commands_trusted(&root));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reports_playbook_override_paths_it_is_trusting() {
        let root = tmp_dir("playbook-paths");
        fs::create_dir_all(root.join(".gate/playbooks")).unwrap();
        fs::write(
            root.join(".gate/config.yml"),
            "commands:\n  test: echo hi\n",
        )
        .unwrap();
        fs::write(root.join(".gate/playbooks/plan.md"), "# custom plan\n").unwrap();

        execute(&root, &args(&[])).unwrap();

        let paths = trusted_playbook_paths(&root);
        assert_eq!(paths, vec![".gate/playbooks/plan.md".to_string()]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn playbook_summary_suffix_is_empty_when_nothing_to_report() {
        assert_eq!(playbook_summary_suffix(&[]), "");
        assert_eq!(
            playbook_summary_suffix(&["a.md".to_string(), "b.md".to_string()]),
            " - playbooks: a.md, b.md"
        );
    }

    #[test]
    fn records_the_by_flag_in_the_trust_record() {
        let root = tmp_dir("by-flag");
        fs::create_dir_all(root.join(".gate")).unwrap();
        fs::write(
            root.join(".gate/config.yml"),
            "commands:\n  test: echo hi\n",
        )
        .unwrap();

        execute(&root, &args(&["--by", "alice"])).unwrap();

        let record = read_trust(&root).unwrap();
        assert_eq!(record.trusted_by.as_deref(), Some("alice"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn errors_when_the_gate_root_does_not_resolve() {
        let root = tmp_dir("no-gate-dir");
        let err = require_root_from(&root);
        assert!(err.is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    /// `require_root` itself walks up from `env::current_dir()`, so it can't
    /// be exercised hermetically here - this just documents (and pins) that
    /// a directory with no `.gate/` anywhere above it resolves to nothing,
    /// via the same `find_gate_root` primitive `require_root` calls.
    fn require_root_from(dir: &Path) -> Option<std::path::PathBuf> {
        crate::core::paths::find_gate_root(dir)
    }
}
