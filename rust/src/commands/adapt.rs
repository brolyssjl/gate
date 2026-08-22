//! Port of `src/commands/adapt.ts`: `gate adapt [adapter...]` - write (or
//! refresh) agent config pointer blocks.

use std::fs;
use std::path::Path;

use crate::adapters::{adapter_keys, build_pointer_body, get_adapter, Adapter};
use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::core::json::Value;
use crate::core::markers::upsert_managed_block;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdaptAction {
    Created,
    Updated,
    Unchanged,
}

impl AdaptAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdaptAction::Created => "created",
            AdaptAction::Updated => "updated",
            AdaptAction::Unchanged => "unchanged",
        }
    }
}

pub struct AdaptResult {
    pub adapter: &'static str,
    pub path: &'static str,
    pub action: AdaptAction,
}

/// Inject or refresh one adapter's managed block. Idempotent: a second call
/// with nothing changed reports "unchanged" and does not touch the file.
pub fn apply_adapter(root: &Path, adapter: &'static Adapter) -> Result<AdaptResult, UserError> {
    let target = root.join(adapter.target_path);
    let body = build_pointer_body();

    let existed_before = target.exists();
    let existing = if existed_before {
        fs::read_to_string(&target).map_err(|e| UserError::new(e.to_string()))?
    } else if adapter.dedicated_file {
        adapter.preamble.unwrap_or("").to_string()
    } else {
        String::new()
    };

    let next = upsert_managed_block(&existing, &body);

    if existed_before && next == existing {
        return Ok(AdaptResult {
            adapter: adapter.key,
            path: adapter.target_path,
            action: AdaptAction::Unchanged,
        });
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| UserError::new(e.to_string()))?;
    }
    fs::write(&target, &next).map_err(|e| UserError::new(e.to_string()))?;
    Ok(AdaptResult {
        adapter: adapter.key,
        path: adapter.target_path,
        action: if existed_before {
            AdaptAction::Updated
        } else {
            AdaptAction::Created
        },
    })
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    let mut full = vec!["adapt".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let root = require_root()?;
    execute(&root, &args)
}

/// The whole command, minus resolving `root` from the process's current
/// directory - split out so it can be unit-tested against an explicit root
/// without touching global process state (see `trust::execute`'s doc comment
/// for why: `cargo test` shares one process across the whole crate's tests).
fn execute(root: &std::path::Path, args: &ParsedArgs) -> Result<(), UserError> {
    let requested = &args.positionals;

    let unknown: Vec<&str> = requested
        .iter()
        .map(String::as_str)
        .filter(|t| get_adapter(t).is_none())
        .collect();
    if !unknown.is_empty() {
        return Err(UserError::usage(format!(
            "unknown adapter(s): {} (known: {})",
            unknown.join(", "),
            adapter_keys().join(", ")
        )));
    }

    let targets: Vec<&'static Adapter> = if requested.is_empty() {
        adapter_keys()
            .into_iter()
            .map(|k| get_adapter(k).expect("adapter_keys() entries always resolve"))
            .collect()
    } else {
        requested
            .iter()
            .map(|k| get_adapter(k).expect("validated above"))
            .collect()
    };

    let mut results = Vec::with_capacity(targets.len());
    for adapter in targets {
        results.push(apply_adapter(root, adapter)?);
    }

    let human = results
        .iter()
        .map(|r| {
            format!(
                "  {:<9} {}  ({})",
                r.action.as_str(),
                r.path,
                get_adapter(r.adapter).expect("known adapter key").name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut data = Value::object();
    let mut adapters_json = Value::array();
    for r in &results {
        let mut entry = Value::object();
        entry.insert("adapter", r.adapter);
        entry.insert("path", r.path);
        entry.insert("action", r.action.as_str());
        adapters_json.push(entry);
    }
    data.insert("adapters", adapters_json);
    emit(&human, &data, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-adapt-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".gate")).unwrap();
        dir
    }

    fn args(items: &[&str]) -> ParsedArgs {
        let mut full = vec!["adapt".to_string()];
        full.extend(items.iter().map(|s| s.to_string()));
        parse_args(&full)
    }

    #[test]
    fn creates_a_shared_file_with_a_managed_block() {
        let root = tmp_dir("create-shared");
        let adapter = get_adapter("claude").unwrap();
        let result = apply_adapter(&root, adapter).unwrap();
        assert_eq!(result.action, AdaptAction::Created);
        let content = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert!(content.contains("Gate quality flow"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn creates_a_dedicated_file_with_its_preamble() {
        let root = tmp_dir("create-dedicated");
        let adapter = get_adapter("cursor").unwrap();
        apply_adapter(&root, adapter).unwrap();
        let content = fs::read_to_string(root.join(".cursor/rules/gate.mdc")).unwrap();
        assert!(content.starts_with("---\ndescription: Gate quality-flow protocol"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn second_call_with_nothing_changed_reports_unchanged_and_does_not_touch_the_file() {
        let root = tmp_dir("idempotent");
        let adapter = get_adapter("claude").unwrap();
        apply_adapter(&root, adapter).unwrap();
        let result = apply_adapter(&root, adapter).unwrap();
        assert_eq!(result.action, AdaptAction::Unchanged);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_an_unknown_adapter_name() {
        let root = tmp_dir("unknown-adapter");
        let err = execute(&root, &args(&["nope"])).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.message().contains("nope"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn writes_every_known_adapter_when_none_are_named() {
        let root = tmp_dir("all-adapters");
        execute(&root, &args(&[])).unwrap();
        for key in adapter_keys() {
            let adapter = get_adapter(key).unwrap();
            assert!(root.join(adapter.target_path).exists(), "{key} missing");
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
