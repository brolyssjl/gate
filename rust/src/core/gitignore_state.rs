//! Port of `src/core/gitignoreState.ts`: records what `gate init` last left
//! the repo-root `.gitignore` containing, so the scope check can tell
//! "gate's own bookkeeping write, untouched since" apart from "someone
//! edited .gitignore after that". Only an exact content
//! match to the hash recorded right after `gate init` wrote the file counts
//! as bookkeeping; any other edit is a normal touched file.

use std::fs;
use std::io;
use std::path::Path;

use crate::core::fsx::write_file_atomic;
use crate::core::json::{self, Value};
use crate::core::paths::gate_paths;
use crate::core::run::now_iso;
use crate::core::sha256::sha256_prefixed;

fn content_hash(content: &str) -> String {
    sha256_prefixed(content.as_bytes())
}

/// Record the hash of `.gitignore`'s full content immediately after `gate
/// init` writes it.
pub fn record_gitignore_state(root: &Path, content: &str) -> io::Result<()> {
    let mut record = Value::object();
    record.insert("hash", content_hash(content));
    record.insert("writtenAt", now_iso());
    let text = json::stringify_pretty(&record) + "\n";
    write_file_atomic(&gate_paths(root).gitignore_state, &text)
}

/// True only when `.gitignore` exists and its current content byte-for-byte
/// matches what `gate init` recorded - i.e. nothing has touched it since.
/// False when no record exists, the file was deleted, or its content has
/// changed at all - fails closed.
pub fn is_gitignore_unchanged_since_init(root: &Path) -> bool {
    let state_path = gate_paths(root).gitignore_state;
    let Ok(text) = fs::read_to_string(&state_path) else {
        return false;
    };
    let Ok(record) = json::parse(&text) else {
        return false;
    };
    let Some(hash) = record.get("hash").and_then(|v| v.as_str()) else {
        return false;
    };
    let gitignore_path = root.join(".gitignore");
    let Ok(content) = fs::read_to_string(&gitignore_path) else {
        return false;
    };
    content_hash(&content) == hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        crate::core::testutil::unique_temp_dir(&format!("gitignorestate-rs-{name}"))
    }

    #[test]
    fn is_not_exempt_before_gate_init_ever_records_a_write() {
        let root = tmp_dir("no-record");
        stdfs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        assert!(!is_gitignore_unchanged_since_init(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn is_exempt_only_while_the_file_matches_exactly_what_was_recorded() {
        let root = tmp_dir("exempt-until-edit");
        let content = "node_modules/\n# gate\n.gate/runs/\n";
        stdfs::write(root.join(".gitignore"), content).unwrap();
        record_gitignore_state(&root, content).unwrap();
        assert!(is_gitignore_unchanged_since_init(&root));

        // Any edit at all - even appending something that looks harmless -
        // revokes the exemption; it does not try to distinguish "safe" edits.
        stdfs::write(root.join(".gitignore"), format!("{content}payload/\n")).unwrap();
        assert!(!is_gitignore_unchanged_since_init(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn is_not_exempt_when_the_file_was_deleted_after_being_recorded() {
        let root = tmp_dir("deleted-after-record");
        let content = ".gate/runs/\n";
        stdfs::write(root.join(".gitignore"), content).unwrap();
        record_gitignore_state(&root, content).unwrap();
        assert!(is_gitignore_unchanged_since_init(&root));

        stdfs::write(root.join(".gitignore"), "something else entirely\n").unwrap();
        assert!(!is_gitignore_unchanged_since_init(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }
}
