//! Port of `src/commands/adapt.ts`: `gate adapt [adapter...]` - write (or
//! refresh) agent config pointer blocks.

use std::fs;
use std::path::Path;

use crate::adapters::{adapter_keys, get_adapter, resolve_pointer_body, Adapter};
use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::require_root;
use crate::cli::output::{emit, UserError};
use crate::core::fsx::{confined_write_target, write_file_atomic};
use crate::core::json::Value;
use crate::core::markers::upsert_managed_block;

/// Resolve the real path `gate adapt` should write for this adapter's
/// target (report finding 3(c)). A plain, non-symlink target goes through
/// `confined_write_target` like any other write under `root`. A target that
/// is *itself* a symlink is only followed if it resolves inside `root` -
/// e.g. a repo that keeps `CLAUDE.md` as a symlink to `docs/CLAUDE.md` -
/// anything resolving outside (`CLAUDE.md -> ~/.claude/CLAUDE.md`, the
/// scope-escape the report reproduced) is refused, naming the path.
fn resolve_adapter_target(
    root: &Path,
    adapter: &'static Adapter,
) -> Result<std::path::PathBuf, UserError> {
    let rel = Path::new(adapter.target_path);
    let target = root.join(rel);
    if let Ok(meta) = fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            let root_canon = fs::canonicalize(root).map_err(|e| UserError::new(e.to_string()))?;
            let resolved = fs::canonicalize(&target).map_err(|e| {
                UserError::new(format!(
                    "refusing to write adapter target through a broken symlink at {}: {e}",
                    target.display()
                ))
            })?;
            if !resolved.starts_with(&root_canon) {
                return Err(UserError::new(format!(
                    "refusing to write adapter target through a symlink that escapes the project root: {} resolves to {}",
                    target.display(),
                    resolved.display()
                )));
            }
            return Ok(resolved);
        }
    }
    confined_write_target(root, rel).map_err(|e| UserError::new(e.to_string()))
}

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
/// `body` is the caller's job to compute (`adapters::resolve_pointer_body`) -
/// shared across a whole `gate adapt`/`init`/`update` run so every target
/// gets byte-identical content without recomputing SDD detection per file.
pub fn apply_adapter(
    root: &Path,
    adapter: &'static Adapter,
    body: &str,
) -> Result<AdaptResult, UserError> {
    let target = resolve_adapter_target(root, adapter)?;

    let existed_before = target.exists();
    let existing = if existed_before {
        fs::read_to_string(&target).map_err(|e| UserError::new(e.to_string()))?
    } else if adapter.dedicated_file {
        adapter.preamble.unwrap_or("").to_string()
    } else {
        String::new()
    };

    let next = upsert_managed_block(&existing, body);

    if existed_before && next == existing {
        return Ok(AdaptResult {
            adapter: adapter.key,
            path: adapter.target_path,
            action: AdaptAction::Unchanged,
        });
    }

    write_file_atomic(&target, &next).map_err(|e| UserError::new(e.to_string()))?;
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

    let body = resolve_pointer_body(root);
    let mut results = Vec::with_capacity(targets.len());
    for adapter in targets {
        results.push(apply_adapter(root, adapter, &body)?);
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
        let dir = crate::core::testutil::unique_temp_dir(&format!("adapt-rs-{name}"));
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
        let body = resolve_pointer_body(&root);
        let result = apply_adapter(&root, adapter, &body).unwrap();
        assert_eq!(result.action, AdaptAction::Created);
        let content = fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert!(content.contains("Gate quality flow"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn creates_a_dedicated_file_with_its_preamble() {
        let root = tmp_dir("create-dedicated");
        let adapter = get_adapter("cursor").unwrap();
        let body = resolve_pointer_body(&root);
        apply_adapter(&root, adapter, &body).unwrap();
        let content = fs::read_to_string(root.join(".cursor/rules/gate.mdc")).unwrap();
        assert!(content.starts_with("---\ndescription: Gate quality-flow protocol"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn second_call_with_nothing_changed_reports_unchanged_and_does_not_touch_the_file() {
        let root = tmp_dir("idempotent");
        let adapter = get_adapter("claude").unwrap();
        let body = resolve_pointer_body(&root);
        apply_adapter(&root, adapter, &body).unwrap();
        let result = apply_adapter(&root, adapter, &body).unwrap();
        assert_eq!(result.action, AdaptAction::Unchanged);
        fs::remove_dir_all(&root).unwrap();
    }

    /// Report finding 3(c): `CLAUDE.md` (or any adapter target) as a
    /// committed symlink pointing outside the project must not be followed.
    #[test]
    fn refuses_to_write_an_adapter_target_that_symlinks_outside_root() {
        let root = tmp_dir("symlink-escape");
        let outside_dir = crate::core::testutil::unique_temp_dir("adapt-rs-outside");
        let outside_file = outside_dir.join("CLAUDE.md");
        fs::write(&outside_file, "not gate's business").unwrap();
        std::os::unix::fs::symlink(&outside_file, root.join("CLAUDE.md")).unwrap();

        let adapter = get_adapter("claude").unwrap();
        let body = resolve_pointer_body(&root);
        let err = apply_adapter(&root, adapter, &body)
            .map(|_| ())
            .unwrap_err();
        assert!(
            err.message().contains("CLAUDE.md")
                || err.message().contains(&root.display().to_string()),
            "error should name the offending path: {}",
            err.message()
        );
        assert_eq!(
            fs::read_to_string(&outside_file).unwrap(),
            "not gate's business"
        );
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&outside_dir).unwrap();
    }

    /// A symlink that resolves *inside* the project root is a legitimate
    /// repo layout (e.g. `CLAUDE.md -> docs/CLAUDE.md`) and should still be
    /// followed.
    #[test]
    fn follows_an_adapter_target_that_symlinks_inside_root() {
        let root = tmp_dir("symlink-inside");
        fs::create_dir_all(root.join("docs")).unwrap();
        let real = root.join("docs").join("CLAUDE.md");
        fs::write(&real, "existing content\n").unwrap();
        std::os::unix::fs::symlink(&real, root.join("CLAUDE.md")).unwrap();

        let adapter = get_adapter("claude").unwrap();
        let body = resolve_pointer_body(&root);
        let result = apply_adapter(&root, adapter, &body).unwrap();
        assert_eq!(result.action, AdaptAction::Updated);
        let content = fs::read_to_string(&real).unwrap();
        assert!(content.contains("existing content"));
        assert!(content.contains("Gate quality flow"));
        // The symlink itself is untouched, still pointing at the real file.
        assert!(fs::symlink_metadata(root.join("CLAUDE.md"))
            .unwrap()
            .file_type()
            .is_symlink());
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
