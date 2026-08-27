//! `.gate/playbooks.lock`: version-stamped provenance for each playbook copy
//! materialized under `.gate/playbooks/` - which bundled content (by hash)
//! and which `gate` version wrote it, so `gate doctor`/`gate update` can
//! tell a pristine-but-outdated copy from a user edit without polluting the
//! playbook content itself with a header comment. Same "record on write,
//! read back for later diagnosis" shape as `core::gitignore_state`, just
//! keyed per file since playbooks are gained/refreshed independently rather
//! than as one all-or-nothing write.
//!
//! A project that predates this manifest (or a hand-deleted one) simply has
//! no entry for a given file - `core::doctor` reports that as unknown
//! provenance rather than guessing.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::fsx::write_file_atomic;
use crate::core::json::{self, Value};
use crate::core::paths::gate_paths;
use crate::core::sha256::sha256_prefixed;
use crate::core::version::read_version;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Hash of the bundled content this copy was materialized from.
    pub source_hash: String,
    /// `gate` version that materialized it.
    pub gate_version: String,
}

fn manifest_path(root: &Path) -> PathBuf {
    gate_paths(root).gate.join("playbooks.lock")
}

pub fn content_hash(content: &str) -> String {
    sha256_prefixed(content.as_bytes())
}

/// Every file the manifest currently has provenance for. Missing, empty, or
/// unparseable manifest all read back as "no entries" - the pre-manifest
/// legacy case `core::doctor` is meant to handle gracefully, not an error.
pub fn read_manifest(root: &Path) -> BTreeMap<String, ManifestEntry> {
    let Ok(text) = fs::read_to_string(manifest_path(root)) else {
        return BTreeMap::new();
    };
    let Ok(parsed) = json::parse(&text) else {
        return BTreeMap::new();
    };
    let Some(entries) = parsed.get("files").and_then(|v| v.as_object()) else {
        return BTreeMap::new();
    };
    entries
        .iter()
        .filter_map(|(name, v)| {
            let source_hash = v.get("sourceHash")?.as_str()?.to_string();
            let gate_version = v.get("gateVersion")?.as_str()?.to_string();
            Some((
                name.clone(),
                ManifestEntry {
                    source_hash,
                    gate_version,
                },
            ))
        })
        .collect()
}

fn write_manifest(root: &Path, entries: &BTreeMap<String, ManifestEntry>) -> io::Result<()> {
    let mut files = Value::object();
    for (name, entry) in entries {
        let mut obj = Value::object();
        obj.insert("sourceHash", entry.source_hash.as_str());
        obj.insert("gateVersion", entry.gate_version.as_str());
        files.insert(name.as_str(), obj);
    }
    let mut root_obj = Value::object();
    root_obj.insert("files", files);
    let text = json::stringify_pretty(&root_obj) + "\n";
    write_file_atomic(&manifest_path(root), &text)
}

/// Record (or replace) one file's provenance after materializing it from
/// `source_content` at the current gate version. Read-modify-write against
/// the whole manifest, same as every other single-record `.gate/*.json`
/// state file in this crate (`core::trust`, `core::gitignore_state`) - gate
/// is a single-process CLI, so there is no concurrent-writer race to guard
/// against here.
pub fn record_entry(root: &Path, file: &str, source_content: &str) -> io::Result<()> {
    let mut entries = read_manifest(root);
    entries.insert(
        file.to_string(),
        ManifestEntry {
            source_hash: content_hash(source_content),
            gate_version: read_version().to_string(),
        },
    );
    write_manifest(root, &entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gate-playbook-manifest-rs-{name}-{}",
            std::process::id()
        ));
        let _ = stdfs::remove_dir_all(&dir);
        stdfs::create_dir_all(root_gate(&dir)).unwrap();
        dir
    }

    fn root_gate(dir: &Path) -> PathBuf {
        dir.join(".gate")
    }

    #[test]
    fn reads_back_an_empty_manifest_when_none_has_ever_been_written() {
        let root = tmp_dir("no-manifest");
        assert!(read_manifest(&root).is_empty());
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn records_and_reads_back_one_entry() {
        let root = tmp_dir("record-one");
        record_entry(&root, "plan.md", "# PLAN playbook\n").unwrap();
        let entries = read_manifest(&root);
        let entry = entries.get("plan.md").unwrap();
        assert_eq!(entry.source_hash, content_hash("# PLAN playbook\n"));
        assert_eq!(entry.gate_version, read_version());
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn recording_a_second_file_preserves_the_first() {
        let root = tmp_dir("record-two");
        record_entry(&root, "plan.md", "plan v1\n").unwrap();
        record_entry(&root, "test.md", "test v1\n").unwrap();
        let entries = read_manifest(&root);
        assert_eq!(entries.len(), 2);
        assert!(entries.contains_key("plan.md"));
        assert!(entries.contains_key("test.md"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn re_recording_the_same_file_replaces_its_entry() {
        let root = tmp_dir("re-record");
        record_entry(&root, "plan.md", "v1\n").unwrap();
        record_entry(&root, "plan.md", "v2\n").unwrap();
        let entries = read_manifest(&root);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries["plan.md"].source_hash, content_hash("v2\n"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_unparseable_manifest_reads_back_as_empty_rather_than_erroring() {
        let root = tmp_dir("corrupt-manifest");
        stdfs::write(manifest_path(&root), "not json").unwrap();
        assert!(read_manifest(&root).is_empty());
        stdfs::remove_dir_all(&root).unwrap();
    }
}
