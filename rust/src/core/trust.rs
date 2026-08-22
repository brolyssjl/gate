//! Port of `src/core/trust.ts`: trust-on-first-use for what gate *executes*
//! from config.yml. Gate itself runs the `commands:` block - the same trust
//! class as npm scripts - so the IMPLEMENT/TEST gates refuse to run them
//! until a human has run `gate trust`. `scope_ignore` and playbook
//! overrides/overlays (SEC-01) ride in the same hash even though gate
//! doesn't execute them directly: they're what gate *tells the agent* to
//! execute, or what the agent's own diff is allowed to touch without
//! challenge - an attacker who can edit either gets the same effective
//! control as one who can edit `commands:`. The approved hash lives in
//! `.gate/trust.json`, which is tracked: changing any of these and
//! re-trusting land in the same diff, giving PR review the checkpoint.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::config::commands_block_hash_source;
use crate::core::fsx::write_file_atomic;
use crate::core::json::{self, Value};
use crate::core::paths::gate_paths;
use crate::core::run::now_iso;
use crate::core::sha256::sha256_prefixed;

#[derive(Debug, Clone, PartialEq)]
pub struct TrustRecord {
    pub commands_hash: String,
    pub trusted_at: String,
    pub trusted_by: Option<String>,
}

fn trust_path(root: &Path) -> PathBuf {
    gate_paths(root).gate.join("trust.json")
}

pub fn current_commands_hash(root: &Path) -> String {
    sha256_prefixed(commands_block_hash_source(root).as_bytes())
}

/// True when the commands block is empty (nothing executes, e.g. `{}`) AND
/// `scope_ignore` is empty. Compared against the 2-key shape - matching
/// `commands_block_hash_source`'s own omission of an empty `scope_ignore` -
/// so a non-empty `scope_ignore` alone still forces a real `gate trust`.
pub fn has_no_commands(root: &Path) -> bool {
    commands_block_hash_source(root) == "{\"commands\":{},\"targets\":{}}"
}

pub fn read_trust(root: &Path) -> Option<TrustRecord> {
    let path = trust_path(root);
    let text = fs::read_to_string(&path).ok()?;
    let parsed = json::parse(&text).ok()?;
    Some(TrustRecord {
        commands_hash: parsed.get("commandsHash")?.as_str()?.to_string(),
        trusted_at: parsed.get("trustedAt")?.as_str()?.to_string(),
        trusted_by: parsed
            .get("trustedBy")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

pub fn write_trust(root: &Path, by: Option<&str>) -> io::Result<TrustRecord> {
    let record = TrustRecord {
        commands_hash: current_commands_hash(root),
        trusted_at: now_iso(),
        trusted_by: by.map(String::from),
    };
    let mut obj = Value::object();
    obj.insert("commandsHash", record.commands_hash.as_str());
    obj.insert("trustedAt", record.trusted_at.as_str());
    obj.insert("trustedBy", record.trusted_by.clone());
    let text = json::stringify_pretty(&obj) + "\n";
    write_file_atomic(&trust_path(root), &text)?;
    Ok(record)
}

/// Whether the current commands block is trusted: trivially true when there
/// are no commands to run, otherwise the stored hash must match the current
/// one.
pub fn is_commands_trusted(root: &Path) -> bool {
    if has_no_commands(root) {
        return true;
    }
    match read_trust(root) {
        Some(record) => record.commands_hash == current_commands_hash(root),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-trust-rs-{name}-{}", std::process::id()));
        let _ = stdfs::remove_dir_all(&dir);
        stdfs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_config(root: &Path, content: &str) {
        stdfs::create_dir_all(root.join(".gate")).unwrap();
        stdfs::write(root.join(".gate/config.yml"), content).unwrap();
    }

    #[test]
    fn treats_an_empty_commands_block_as_trusted() {
        let root = tmp_dir("empty-commands");
        write_config(&root, "commands: {}\n");
        assert!(has_no_commands(&root));
        assert!(is_commands_trusted(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn is_untrusted_before_trusted_after_write_trust() {
        let root = tmp_dir("untrusted-then-trusted");
        write_config(&root, "commands:\n  test: echo hi\n");
        assert!(!is_commands_trusted(&root));
        write_trust(&root, Some("tester")).unwrap();
        assert!(is_commands_trusted(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn invalidates_trust_when_the_commands_change() {
        let root = tmp_dir("commands-change");
        write_config(&root, "commands:\n  test: echo one\n");
        write_trust(&root, None).unwrap();
        assert!(is_commands_trusted(&root));
        write_config(&root, "commands:\n  test: echo two\n");
        assert!(!is_commands_trusted(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn produces_a_stable_hash_for_the_same_commands_block() {
        let root = tmp_dir("stable-hash");
        write_config(&root, "commands:\n  test: echo hi\n");
        assert_eq!(current_commands_hash(&root), current_commands_hash(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn invalidates_trust_when_scope_ignore_changes_even_with_commands_untouched() {
        let root = tmp_dir("scope-ignore-change");
        write_config(
            &root,
            "commands:\n  test: echo hi\nscope_ignore:\n  - node_modules/**\n",
        );
        write_trust(&root, None).unwrap();
        assert!(is_commands_trusted(&root));
        write_config(
            &root,
            "commands:\n  test: echo hi\nscope_ignore:\n  - node_modules/**\n  - coverage/**\n",
        );
        assert!(!is_commands_trusted(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn f4_an_empty_scope_ignore_does_not_invalidate_a_pre_existing_trust_hash() {
        let root = tmp_dir("f4-empty-scope-ignore");
        write_config(&root, "commands:\n  test: \"echo hi\"\n");
        write_trust(&root, None).unwrap();
        assert!(is_commands_trusted(&root));
        let pre_upgrade_hash = current_commands_hash(&root);

        write_config(&root, "commands:\n  test: \"echo hi\"\nscope_ignore: []\n");
        assert_eq!(current_commands_hash(&root), pre_upgrade_hash);
        assert!(is_commands_trusted(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn f4_a_populated_scope_ignore_still_forces_a_real_re_trust() {
        let root = tmp_dir("f4-populated-scope-ignore");
        write_config(&root, "commands: {}\nscope_ignore: []\n");
        assert!(has_no_commands(&root));
        assert!(is_commands_trusted(&root)); // nothing to trust yet

        write_config(&root, "commands: {}\nscope_ignore:\n  - node_modules/**\n");
        assert!(!has_no_commands(&root));
        assert!(!is_commands_trusted(&root)); // scope_ignore alone is not trivially trusted
        stdfs::remove_dir_all(&root).unwrap();
    }
}
