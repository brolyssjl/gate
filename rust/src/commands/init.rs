//! Port of `src/commands/init.ts`: `gate init` - scaffold `.gate/`, infer
//! build/test/lint commands from the stack, detect advisory integrations,
//! copy default playbooks (each materialized copy gets a manifest entry -
//! `core::playbook_manifest`), and install the default adapter set
//! (`claude` + `agents`, `--no-adapt`/`--adapt <keys>` to change that).
//! Idempotent: `--refresh` re-detects integrations and rewrites the managed
//! hint block without clobbering user edits to config or playbooks; a
//! second run's adapter/playbook writes report `unchanged` rather than
//! rewriting anything already current.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::adapters::{adapter_keys, get_adapter, resolve_pointer_body, Adapter};
use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::output::{emit, UserError};
use crate::commands::adapt::{apply_adapter, AdaptResult};
use crate::commands::infer_stack::{infer_commands, infer_scope_ignore};
use crate::core::fsx::{confined_write_target, write_file_atomic};
use crate::core::gitignore_state::record_gitignore_state;
use crate::core::json::{stringify_compact, Value};
use crate::core::paths::gate_paths;
use crate::core::playbook_manifest::record_entry;
use crate::core::playbooks::current_bundled_playbooks;
use crate::integrations::{detect, Detection};

/// The default adapter set `gate init` installs when neither `--no-adapt`
/// nor `--adapt` is given - mirrors agnosgram's init ergonomics, scoped down
/// to the two shared-file adapters (every project has *a* CLAUDE.md or
/// AGENTS.md convention; the dedicated-file adapters are opt-in via
/// `--adapt` or a later `gate adapt <key>`).
const DEFAULT_INIT_ADAPTERS: [&str; 2] = ["claude", "agents"];

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
    // the real directory (running from a checkout of this repo); an
    // installed single-file binary has none, so fall back to the copy
    // compiled in at build time. Every copy materialized here (not
    // pre-existing) gets a manifest entry so `gate doctor`/`update` can tell
    // a pristine-but-outdated copy from a user edit later.
    for (file, content) in current_bundled_playbooks() {
        let dest = paths.playbooks.join(&file);
        if !dest.exists() {
            let rel = dest.strip_prefix(root).unwrap_or(&dest);
            let confined =
                confined_write_target(root, rel).map_err(|e| UserError::new(e.to_string()))?;
            write_file_atomic(&confined, &content).map_err(|e| UserError::new(e.to_string()))?;
            record_entry(root, &file, &content).map_err(|e| UserError::new(e.to_string()))?;
        }
    }

    let adapt_results = install_adapters(root, args)?;

    let det = detect(root);

    // `refresh` re-detects integrations for the human-readable summary below,
    // but never rewrites an existing config.yml - only a config that doesn't
    // exist yet gets written, avoiding clobbering hand edits.
    if !paths.config.exists() {
        let rel = paths.config.strip_prefix(root).unwrap_or(&paths.config);
        let confined =
            confined_write_target(root, rel).map_err(|e| UserError::new(e.to_string()))?;
        write_file_atomic(&confined, &build_config(root, det))
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
            "  gitignore: added .gate/ run-state entries (config.yml stays tracked)".to_string(),
        );
    }
    lines.push(if adapt_results.is_empty() {
        "  adapters:  none installed (--no-adapt)".to_string()
    } else {
        format!(
            "  adapters:  {}",
            adapt_results
                .iter()
                .map(|r| r.path)
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    lines.push(String::new());
    lines.push(
        "Review .gate/config.yml, then run `gate trust` to approve its commands.".to_string(),
    );
    lines.push(
        "Playbook overrides under .gate/playbooks/ are the per-project way to define phase"
            .to_string(),
    );
    lines.push(
        "details (what/how to test, etc.) - edit them directly; `gate doctor`/`gate update`"
            .to_string(),
    );
    lines.push("track drift against the bundled defaults.".to_string());
    lines.push("Next: gate start \"<title>\"".to_string());
    let human = lines.join("\n");

    let mut data = Value::object();
    data.insert("root", root.display().to_string());
    let mut adapters_json = Value::array();
    for r in &adapt_results {
        let mut entry = Value::object();
        entry.insert("adapter", r.adapter);
        entry.insert("path", r.path);
        entry.insert("action", r.action.as_str());
        adapters_json.push(entry);
    }
    data.insert("adapters", adapters_json);
    data.insert("initialized", true);
    data.insert("refreshed", refresh);
    data.insert("detected", detected);
    data.insert("gitignoreUpdated", gitignore_updated);
    emit(&human, &data, &args.flags)
}

/// `.gate/runs/`, the legacy single-run pointer, `current.json`, and
/// `archive/` are ephemeral run state - never `config.yml`, which stays
/// tracked so CI inherits the command pin (see README). Trust (`gate
/// trust`) is not repo state at all any more (security audit 2026-09-22,
/// finding 1): it lives in a machine-local store outside the tree
/// (`core::trust`), so there is no `trust.json` for this function to
/// exempt or for `gate init` to create. Idempotent and non-destructive:
/// only appends entries genuinely missing from an existing `.gitignore`, on
/// both a fresh `init` and every `--refresh`, so upgrading an older
/// `.gate/` project actually gets the entries the docs have always claimed
/// instead of leaving it prose-only.
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
    let rel = path.strip_prefix(root).unwrap_or(&path);
    let confined = confined_write_target(root, rel).map_err(|e| UserError::new(e.to_string()))?;
    write_file_atomic(&confined, &content).map_err(|e| UserError::new(e.to_string()))?;
    // Record exactly what was written so the scope check can tell this write
    // apart from any later edit (review finding: a blanket .gitignore
    // exemption is a scope-check bypass) - see core/gitignore_state.rs.
    record_gitignore_state(root, &content).map_err(|e| UserError::new(e.to_string()))?;
    Ok(true)
}

/// Which adapters to install and write their managed blocks - `--no-adapt`
/// wins outright (installs nothing); `--adapt <a,b>` names an explicit set;
/// otherwise `DEFAULT_INIT_ADAPTERS`. Mirrors agnosgram's init ergonomics.
/// Idempotent the same way `gate adapt` is: re-running `gate init` (e.g.
/// `--refresh`) reports `unchanged` for anything already current rather than
/// rewriting it.
fn install_adapters(root: &Path, args: &ParsedArgs) -> Result<Vec<AdaptResult>, UserError> {
    if args.flags.is_true("no-adapt") {
        return Ok(Vec::new());
    }
    let targets: Vec<&'static Adapter> = match args.flags.str("adapt") {
        Some(raw) => {
            let mut out = Vec::new();
            for token in raw.split(',') {
                let key = token.trim().to_lowercase();
                if key.is_empty() {
                    continue;
                }
                let Some(adapter) = get_adapter(&key) else {
                    return Err(UserError::usage(format!(
                        "unknown adapter \"{key}\" in --adapt (known: {})",
                        adapter_keys().join(", ")
                    )));
                };
                out.push(adapter);
            }
            out
        }
        None => DEFAULT_INIT_ADAPTERS
            .iter()
            .map(|key| get_adapter(key).expect("DEFAULT_INIT_ADAPTERS names known keys"))
            .collect(),
    };
    let body = resolve_pointer_body(root);
    targets
        .into_iter()
        .map(|adapter| apply_adapter(root, adapter, &body))
        .collect()
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
        crate::core::testutil::unique_temp_dir(&format!("init-rs-{name}"))
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
    fn installs_the_claude_and_agents_adapters_by_default() {
        let root = tmp_dir("default-adapters");
        execute(&root, &args(&[])).unwrap();

        assert!(root.join("CLAUDE.md").exists());
        assert!(fs::read_to_string(root.join("CLAUDE.md"))
            .unwrap()
            .contains("Gate quality flow"));
        assert!(root.join("AGENTS.md").exists());
        // Not installed by default.
        assert!(!root.join(".cursor/rules/gate.mdc").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn no_adapt_skips_every_adapter() {
        let root = tmp_dir("no-adapt");
        execute(&root, &args(&["--no-adapt"])).unwrap();

        assert!(!root.join("CLAUDE.md").exists());
        assert!(!root.join("AGENTS.md").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adapt_flag_installs_a_custom_set_instead_of_the_default() {
        let root = tmp_dir("adapt-custom");
        execute(&root, &args(&["--adapt", "cursor"])).unwrap();

        assert!(root.join(".cursor/rules/gate.mdc").exists());
        assert!(!root.join("CLAUDE.md").exists());
        assert!(!root.join("AGENTS.md").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn adapt_flag_rejects_an_unknown_adapter() {
        let root = tmp_dir("adapt-unknown");
        let err = execute(&root, &args(&["--adapt", "nope"])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn re_running_init_reports_adapters_unchanged_not_rewritten() {
        let root = tmp_dir("adapters-idempotent");
        execute(&root, &args(&[])).unwrap();
        let before = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        execute(&root, &args(&["--refresh"])).unwrap();
        let after = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert_eq!(before, after);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn writes_a_playbook_manifest_entry_for_every_copy_it_materializes() {
        let root = tmp_dir("playbook-manifest");
        execute(&root, &args(&[])).unwrap();

        assert!(root.join(".gate/playbooks.lock").exists());
        let manifest = crate::core::playbook_manifest::read_manifest(&root);
        for phase in ["plan", "debug", "implement", "test", "review", "retro"] {
            assert!(
                manifest.contains_key(&format!("{phase}.md")),
                "missing manifest entry for {phase}.md"
            );
        }
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
