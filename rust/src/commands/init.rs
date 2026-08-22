//! Port of `src/commands/init.ts`: `gate init` - scaffold `.gate/`, infer
//! build/test/lint commands from the stack, detect advisory integrations,
//! and copy default playbooks. Idempotent: `--refresh` re-detects
//! integrations and rewrites the managed hint block without clobbering user
//! edits to config or playbooks.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::output::{emit, UserError};
use crate::commands::infer_stack::{infer_commands, infer_scope_ignore};
use crate::core::embedded_playbooks::EMBEDDED_PLAYBOOKS;
use crate::core::gitignore_state::record_gitignore_state;
use crate::core::json::{stringify_compact, Value};
use crate::core::paths::gate_paths;
use crate::core::playbooks::bundled_playbooks_dir;
use crate::integrations::{detect, Detection};

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["init".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = std::env::current_dir().map_err(|e| UserError::new(e.to_string()))?;
    execute(&root, &args)
}

/// The whole command, minus resolving `root` from the process's current
/// directory - split out so it can be unit-tested against an explicit root
/// without touching global process state (see `trust::execute`'s doc comment
/// for why).
fn execute(root: &Path, args: &ParsedArgs) -> Result<(), UserError> {
    let paths = gate_paths(root);
    let refresh = args.flags.is_true("refresh");
    let existed = paths.gate.exists();

    fs::create_dir_all(&paths.playbooks).map_err(|e| UserError::new(e.to_string()))?;
    fs::create_dir_all(&paths.runs).map_err(|e| UserError::new(e.to_string()))?;

    let gitignore_updated = ensure_gitignore(root)?;

    // Copy default playbooks that the user hasn't already customized. Prefer
    // the real directory (dev-from-source, npm install); a single-file
    // binary has none, so fall back to the copy compiled in at build time.
    for (file, content) in read_bundled_playbook_files() {
        let dest = paths.playbooks.join(&file);
        if !dest.exists() {
            fs::write(&dest, &content).map_err(|e| UserError::new(e.to_string()))?;
        }
    }

    let det = detect(root);

    // `refresh` re-detects integrations for the human-readable summary below,
    // but never rewrites an existing config.yml - only a config that doesn't
    // exist yet gets written, avoiding clobbering hand edits.
    if !paths.config.exists() {
        fs::write(&paths.config, build_config(root, det))
            .map_err(|e| UserError::new(e.to_string()))?;
    }

    let mut detected: Vec<String> = Vec::new();
    if det.agnosgram {
        detected.push("agnosgram".to_string());
    }
    if let Some(sdd) = det.sdd {
        detected.push(format!("sdd:{}", sdd.as_str()));
    }

    let mut lines: Vec<String> = vec![if existed && !refresh {
        ".gate/ already initialized (playbooks topped up).".to_string()
    } else {
        "Initialized .gate/".to_string()
    }];
    lines.push(format!("  config:    {}", paths.config.display()));
    lines.push(format!("  playbooks: {}", paths.playbooks.display()));
    lines.push(format!("  runs:      {}", paths.runs.display()));
    lines.push(if detected.is_empty() {
        "  detected:  none".to_string()
    } else {
        format!("  detected:  {}", detected.join(", "))
    });
    if gitignore_updated {
        lines.push(
            "  gitignore: added .gate/ run-state entries (config.yml and trust.json stay tracked)"
                .to_string(),
        );
    }
    lines.push(String::new());
    lines.push(
        "Review .gate/config.yml, then run `gate trust` to approve its commands.".to_string(),
    );
    lines.push("Next: gate start \"<title>\"".to_string());
    let human = lines.join("\n");

    let mut data = Value::object();
    data.insert("root", root.display().to_string());
    data.insert("initialized", true);
    data.insert("refreshed", refresh);
    data.insert("detected", detected);
    data.insert("gitignoreUpdated", gitignore_updated);
    emit(&human, &data, &args.flags)
}

/// `.gate/runs/`, the legacy single-run pointer, `current.json`, and
/// `archive/` are ephemeral run state - never `config.yml` or `trust.json`,
/// which stay tracked so CI inherits the command pin (see README). Idempotent
/// and non-destructive: only appends entries genuinely missing from an
/// existing `.gitignore`, on both a fresh `init` and every `--refresh`, so
/// upgrading an older `.gate/` project actually gets the entries the docs
/// have always claimed instead of leaving it prose-only.
const GITIGNORE_MARKER: &str =
    "# Gate's own ephemeral run folders (config + playbooks stay tracked)";
const GITIGNORE_ENTRIES: [&str; 4] = [
    ".gate/runs/",
    ".gate/current",
    ".gate/current.json",
    ".gate/archive/",
];

fn ensure_gitignore(root: &Path) -> Result<bool, UserError> {
    let path = root.join(".gitignore");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let lines: HashSet<&str> = existing.split('\n').map(|l| l.trim()).collect();
    let missing: Vec<&str> = GITIGNORE_ENTRIES
        .iter()
        .copied()
        .filter(|e| !lines.contains(e))
        .collect();
    if missing.is_empty() {
        return Ok(false);
    }

    let prefix = if existing.is_empty() || existing.ends_with('\n') {
        existing.clone()
    } else {
        format!("{existing}\n")
    };
    let mut block_lines: Vec<&str> = vec![GITIGNORE_MARKER];
    block_lines.extend(missing);
    let block = format!(
        "{}{}\n",
        if !prefix.is_empty() { "\n" } else { "" },
        block_lines.join("\n")
    );
    let content = format!("{prefix}{block}");
    fs::write(&path, &content).map_err(|e| UserError::new(e.to_string()))?;
    // Record exactly what was written so the scope check can tell this write
    // apart from any later edit (review finding: a blanket .gitignore
    // exemption is a scope-check bypass) - see core/gitignore_state.rs.
    record_gitignore_state(root, &content).map_err(|e| UserError::new(e.to_string()))?;
    Ok(true)
}

/// Prefer the real `playbooks/` directory on disk; fall back to the copy
/// compiled into the binary at build time (`core::embedded_playbooks`).
/// `bundled_playbooks_dir()` returns the plain "not found" error for the
/// *expected* case - a single-file binary with no sibling `playbooks/`
/// directory to walk to - and that's the only failure this should swallow
/// silently. Any other failure (e.g. the directory exists but a file in it
/// can't be read) is surfaced on stderr instead of going unmentioned; init
/// still completes on the embedded fallback (playbooks aren't load-bearing
/// for `.gate/` to exist).
fn read_bundled_playbook_files() -> Vec<(String, String)> {
    match try_read_bundled_playbook_files() {
        Ok(files) => files,
        Err(e) => {
            let expected = e == "bundled playbooks directory not found";
            if !expected {
                eprintln!(
                    "gate: warning: could not read bundled playbooks ({e}) - using the built-in copy"
                );
            }
            EMBEDDED_PLAYBOOKS
                .iter()
                .map(|(phase, content)| (format!("{phase}.md"), (*content).to_string()))
                .collect()
        }
    }
}

fn try_read_bundled_playbook_files() -> Result<Vec<(String, String)>, String> {
    let bundled = bundled_playbooks_dir()?;
    let names: Vec<String> = fs::read_dir(&bundled)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".md"))
        .collect();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let content = fs::read_to_string(bundled.join(&name)).map_err(|e| e.to_string())?;
        out.push((name, content));
    }
    Ok(out)
}

fn quote(s: &str) -> String {
    stringify_compact(&Value::String(s.to_string()))
}

fn build_config(root: &Path, det: Detection) -> String {
    let cmds = infer_commands(root);
    let mut lines = vec![
        "# Gate configuration. Commands are the same trust class as npm scripts.".to_string(),
        "commands:".to_string(),
    ];
    for (key, value) in [
        ("build", &cmds.build),
        ("test", &cmds.test),
        ("lint", &cmds.lint),
        ("coverage", &cmds.coverage),
    ] {
        match value {
            Some(v) => lines.push(format!("  {key}: {}", quote(v))),
            None => lines.push(format!("  # {key}: \"<command>\"")),
        }
    }
    lines.push("thresholds:".to_string());
    lines.push("  # diff_coverage: 80   # uncomment once a coverage command is set".to_string());
    lines.push("phases:".to_string());
    lines.push("  plan: required".to_string());
    lines.push("  implement: required".to_string());
    lines.push("  test: required".to_string());
    lines.push("integrations:".to_string());
    lines.push(format!(
        "  agnosgram: {}",
        if det.agnosgram { "auto" } else { "off" }
    ));
    lines.push(format!(
        "  sdd: {}",
        if det.sdd.is_some() { "auto" } else { "off" }
    ));

    let scope_ignore = infer_scope_ignore(root);
    lines.push(
        "# Noise the scope check ignores rather than flags as undeclared - part of the trust hash."
            .to_string(),
    );
    lines.push("scope_ignore:".to_string());
    if scope_ignore.is_empty() {
        lines.push("  # - <glob>".to_string());
    } else {
        for g in &scope_ignore {
            lines.push(format!("  - {}", quote(g)));
        }
    }
    lines.push(String::new());

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::json;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-init-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["init".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn creates_gate_scaffold_and_config_in_a_fresh_repo() {
        let root = tmp_dir("fresh");
        execute(&root, &args(&[])).unwrap();

        assert!(root.join(".gate/config.yml").exists());
        assert!(root.join(".gate/playbooks/plan.md").exists());
        assert!(root.join(".gate/runs").is_dir());
        let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(gitignore.contains(".gate/runs/"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ensure_gitignore_is_idempotent_and_non_destructive() {
        let root = tmp_dir("gitignore-idempotent");
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        assert!(ensure_gitignore(&root).unwrap());
        let first = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(first.contains("node_modules/"));
        assert!(first.contains(".gate/runs/"));
        assert!(!ensure_gitignore(&root).unwrap());
        let second = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(first, second);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn does_not_rewrite_an_existing_config_on_refresh() {
        let root = tmp_dir("refresh-preserves-config");
        execute(&root, &args(&[])).unwrap();
        fs::write(
            root.join(".gate/config.yml"),
            "# hand-edited\ncommands:\n  test: my-custom-test\n",
        )
        .unwrap();
        execute(&root, &args(&["--refresh"])).unwrap();

        let config = fs::read_to_string(root.join(".gate/config.yml")).unwrap();
        assert!(config.contains("my-custom-test"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn build_config_infers_node_commands_and_scope_ignore() {
        let root = tmp_dir("build-config-node");
        fs::write(
            root.join("package.json"),
            r#"{"scripts": {"test": "vitest"}}"#,
        )
        .unwrap();
        let config = build_config(
            &root,
            Detection {
                agnosgram: false,
                sdd: None,
            },
        );
        assert!(config.contains("test: \"npm test\""));
        assert!(config.contains("# build: \"<command>\""));
        assert!(config.contains("node_modules/**"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn quote_matches_json_stringify_escaping() {
        assert_eq!(
            quote("npm test"),
            json::stringify_compact(&Value::String("npm test".to_string()))
        );
    }
}
