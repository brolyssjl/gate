//! Port of `src/core/trust.ts`: trust-on-first-use for what gate *executes*
//! from config.yml. Gate itself runs the `commands:` block - the same trust
//! class as npm scripts - so the IMPLEMENT/TEST gates refuse to run them
//! until a human has run `gate trust`. `scope_ignore` and playbook
//! overrides/overlays ride in the same hash even though gate
//! doesn't execute them directly: they're what gate *tells the agent* to
//! execute, or what the agent's own diff is allowed to touch without
//! challenge - an attacker who can edit either gets the same effective
//! control as one who can edit `commands:`.
//!
//! Security audit 2026-09-22, finding 1 (CRITICAL): the trust record used
//! to live at `.gate/trust.json`, tracked in the repo. That made trust a
//! property of the *repo*, not of the human who reviewed it - an attacker
//! could run `gate trust` in their own copy, commit the resulting
//! `trust.json` alongside a hostile `commands:` block, and the victim's
//! very first `gate check` on a fresh clone would run it, no local
//! `gate trust` ever required.
//!
//! Trust is now a property of (this machine, this checkout), never of the
//! repo: the record lives outside the tree, under a machine-local config
//! directory (`resolve_config_dir`/`env_config_dir` below), keyed by a hash
//! of the checkout's canonicalized root path. `gate trust` writes only
//! there; `is_commands_trusted` reads only from there. A `.gate/trust.json`
//! that happens to exist in a repo (a leftover from before this change, or
//! something an attacker still commits) has no bearing on the decision -
//! `gate doctor` flags it (`trust.legacy-file`) as a stale/misleading file
//! to delete, nothing more.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::config::{commands_block_hash_source, commands_block_hash_source_legacy};
use crate::core::fsx::write_file_atomic;
use crate::core::json::{self, Value};
use crate::core::run::now_iso;
use crate::core::sha256::{hex_digest, sha256_prefixed};

/// The trust-hash schema/coverage version this build writes. Version 1 (the
/// implicit default for any trust record with no `coverageVersion` field)
/// is the pre-SEC-01 shape: commands + targets + scope_ignore, no playbook
/// coverage. Version 2 adds `.gate/playbooks/*.md` overrides and target
/// `playbooks:` overlays to the hash. Bump this again the next time gate
/// widens what the trust hash covers.
pub const CURRENT_COVERAGE_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct TrustRecord {
    pub commands_hash: String,
    pub trusted_at: String,
    pub trusted_by: Option<String>,
    /// Trust-hash schema version this record was written under. Absent in
    /// the stored JSON (any record from before this field existed) reads
    /// back as `1`, the pre-SEC-01 shape.
    pub coverage_version: u32,
    /// The canonicalized project root this record was trusted for, for
    /// human inspection (e.g. `gate doctor`, or someone auditing the
    /// contents of their config directory) - not itself part of the trust
    /// decision. The record's on-disk *location* (a hash of this same
    /// path) is what `is_commands_trusted` actually keys on; this field is
    /// just so the file is legible without reversing that hash.
    pub root: String,
}

/// Why a stored trust record's hash doesn't match the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchReason {
    /// The stored hash was computed under an older, narrower coverage
    /// version, and recomputing under THAT version's shape still matches -
    /// gate's trust coverage grew, but nothing the user previously trusted
    /// actually changed.
    CoverageExpanded,
    /// The hash differs even accounting for coverage growth: a real edit to
    /// `commands:`, `scope_ignore:`, or a covered playbook - or outright
    /// tampering.
    Changed,
}

fn non_empty(v: Option<&str>) -> Option<&str> {
    v.filter(|s| !s.is_empty())
}

/// Resolve the machine-local directory gate stores trust records (and
/// nothing else, today) under: `GATE_CONFIG_DIR` first, else
/// `$XDG_CONFIG_HOME/gate`, else `$HOME/.config/gate`. `None` only when
/// none of the three resolve to anything - callers fail closed (untrusted,
/// or a write error) rather than guess a location. A pure function of its
/// inputs so the resolution order is unit-testable without ever touching
/// `std::env` (see `env_config_dir`, the only caller that does).
pub fn resolve_config_dir(
    gate_config_dir: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(dir) = non_empty(gate_config_dir) {
        return Some(PathBuf::from(dir));
    }
    if let Some(xdg) = non_empty(xdg_config_home) {
        return Some(PathBuf::from(xdg).join("gate"));
    }
    let home = non_empty(home)?;
    Some(PathBuf::from(home).join(".config").join("gate"))
}

/// `resolve_config_dir` against the real process environment. The only
/// function in this module that reads `std::env` directly - every other
/// function here takes the config directory as a parameter, so tests never
/// need to mutate process-global environment to control it.
pub fn env_config_dir() -> Option<PathBuf> {
    resolve_config_dir(
        std::env::var("GATE_CONFIG_DIR").ok().as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Where trust records live under a resolved config directory.
pub fn trust_store_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("trust")
}

/// Canonicalize `root` for both the record's on-disk key and its `root`
/// field - falls back to `root` unchanged if canonicalization fails (e.g.
/// the path doesn't exist), so a trust decision never hard-fails on that
/// alone.
fn canonical_root_string(root: &Path) -> String {
    root.canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// The on-disk path of `root`'s trust record under `config_dir`: a hash of
/// the canonicalized root path, never the path itself (keeps the store flat
/// and filename-safe regardless of what the checkout path looks like).
fn record_path(config_dir: &Path, root: &Path) -> PathBuf {
    let key = hex_digest(canonical_root_string(root).as_bytes());
    trust_store_dir(config_dir).join(format!("{key}.json"))
}

pub(crate) fn config_dir_unresolved_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "cannot resolve gate's config directory - set GATE_CONFIG_DIR, XDG_CONFIG_HOME, or HOME",
    )
}

pub fn current_commands_hash(root: &Path) -> String {
    sha256_prefixed(commands_block_hash_source(root).as_bytes())
}

/// Hash of the config under the coverage-version-1 (pre-SEC-01) shape - see
/// `commands_block_hash_source_legacy`.
pub fn legacy_commands_hash(root: &Path) -> String {
    sha256_prefixed(commands_block_hash_source_legacy(root).as_bytes())
}

/// Diagnose a hash mismatch for a stored `record` against the current
/// config. Only meaningful once the caller already knows `record.commands_hash
/// != current_commands_hash(root)` - a record whose stored coverage version
/// is already current can't be explained by coverage growth, so it always
/// reads as `Changed`.
pub fn diagnose_mismatch(root: &Path, record: &TrustRecord) -> MismatchReason {
    if record.coverage_version < CURRENT_COVERAGE_VERSION
        && record.commands_hash == legacy_commands_hash(root)
    {
        MismatchReason::CoverageExpanded
    } else {
        MismatchReason::Changed
    }
}

/// True when the commands block is empty (nothing executes, e.g. `{}`) AND
/// `scope_ignore` is empty. Compared against the 2-key shape - matching
/// `commands_block_hash_source`'s own omission of an empty `scope_ignore` -
/// so a non-empty `scope_ignore` alone still forces a real `gate trust`.
pub fn has_no_commands(root: &Path) -> bool {
    commands_block_hash_source(root) == "{\"commands\":{},\"targets\":{}}"
}

/// Read `root`'s trust record out of `config_dir`'s store.
pub(crate) fn read_trust_in(config_dir: &Path, root: &Path) -> Option<TrustRecord> {
    let path = record_path(config_dir, root);
    let text = fs::read_to_string(&path).ok()?;
    let parsed = json::parse(&text).ok()?;
    Some(TrustRecord {
        commands_hash: parsed.get("commandsHash")?.as_str()?.to_string(),
        trusted_at: parsed.get("trustedAt")?.as_str()?.to_string(),
        trusted_by: parsed
            .get("trustedBy")
            .and_then(|v| v.as_str())
            .map(String::from),
        coverage_version: parsed
            .get("coverageVersion")
            .and_then(|v| v.as_i64())
            .map(|n| n as u32)
            .unwrap_or(1),
        root: parsed
            .get("root")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| canonical_root_string(root)),
    })
}

/// Write `root`'s trust record into `config_dir`'s store.
pub(crate) fn write_trust_in(
    config_dir: &Path,
    root: &Path,
    by: Option<&str>,
) -> io::Result<TrustRecord> {
    let record = TrustRecord {
        commands_hash: current_commands_hash(root),
        trusted_at: now_iso(),
        trusted_by: by.map(String::from),
        coverage_version: CURRENT_COVERAGE_VERSION,
        root: canonical_root_string(root),
    };
    let mut obj = Value::object();
    obj.insert("commandsHash", record.commands_hash.as_str());
    obj.insert("trustedAt", record.trusted_at.as_str());
    obj.insert("trustedBy", record.trusted_by.clone());
    obj.insert("coverageVersion", record.coverage_version as i64);
    obj.insert("root", record.root.as_str());
    let text = json::stringify_pretty(&obj) + "\n";
    write_file_atomic(&record_path(config_dir, root), &text)?;
    Ok(record)
}

pub(crate) fn is_commands_trusted_in(config_dir: &Path, root: &Path) -> bool {
    match read_trust_in(config_dir, root) {
        Some(record) => record.commands_hash == current_commands_hash(root),
        None => false,
    }
}

/// Read the trust record for `root` from the machine-local store resolved
/// from the real environment (`env_config_dir`). Never reads a repo-tracked
/// `.gate/trust.json` - see the module doc.
pub fn read_trust(root: &Path) -> Option<TrustRecord> {
    read_trust_in(&env_config_dir()?, root)
}

/// Write the trust record for `root` into the machine-local store resolved
/// from the real environment. Never writes `.gate/trust.json` in the repo.
pub fn write_trust(root: &Path, by: Option<&str>) -> io::Result<TrustRecord> {
    let config_dir = env_config_dir().ok_or_else(config_dir_unresolved_error)?;
    write_trust_in(&config_dir, root, by)
}

/// Whether the current commands block is trusted on this machine, for this
/// checkout: trivially true when there are no commands to run, otherwise
/// the machine-local store must hold a record whose hash matches the
/// current one. A `.gate/trust.json` inside the repo - however it got
/// there - has no bearing on this decision (finding 1).
pub fn is_commands_trusted(root: &Path) -> bool {
    if has_no_commands(root) {
        return true;
    }
    match env_config_dir() {
        Some(dir) => is_commands_trusted_in(&dir, root),
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

    /// A config directory, distinct from any repo root, standing in for
    /// `env_config_dir()`'s result - every test below passes this
    /// explicitly to the `_in` functions rather than setting
    /// `GATE_CONFIG_DIR`/`HOME` on the process (never `set_var` in tests).
    fn tmp_config_dir(name: &str) -> PathBuf {
        tmp_dir(&format!("{name}-config"))
    }

    fn write_config(root: &Path, content: &str) {
        stdfs::create_dir_all(root.join(".gate")).unwrap();
        stdfs::write(root.join(".gate/config.yml"), content).unwrap();
    }

    // ---- resolve_config_dir: directory resolution order --------------

    #[test]
    fn resolve_config_dir_prefers_gate_config_dir_over_everything() {
        assert_eq!(
            resolve_config_dir(Some("/explicit"), Some("/xdg"), Some("/home")),
            Some(PathBuf::from("/explicit"))
        );
    }

    #[test]
    fn resolve_config_dir_falls_back_to_xdg_config_home_slash_gate() {
        assert_eq!(
            resolve_config_dir(None, Some("/xdg"), Some("/home")),
            Some(PathBuf::from("/xdg/gate"))
        );
    }

    #[test]
    fn resolve_config_dir_falls_back_to_home_dot_config_gate() {
        assert_eq!(
            resolve_config_dir(None, None, Some("/home/alice")),
            Some(PathBuf::from("/home/alice/.config/gate"))
        );
    }

    #[test]
    fn resolve_config_dir_is_none_when_nothing_resolves() {
        assert_eq!(resolve_config_dir(None, None, None), None);
    }

    #[test]
    fn resolve_config_dir_treats_empty_env_values_as_unset() {
        // `GATE_CONFIG_DIR=""` or `XDG_CONFIG_HOME=""` in the real
        // environment must not resolve to a nonsensical empty path.
        assert_eq!(
            resolve_config_dir(Some(""), Some(""), Some("/home/alice")),
            Some(PathBuf::from("/home/alice/.config/gate"))
        );
        assert_eq!(resolve_config_dir(Some(""), Some(""), Some("")), None);
    }

    // ---- trivially-trusted short-circuit (no config dir touched) ------

    #[test]
    fn treats_an_empty_commands_block_as_trusted() {
        let root = tmp_dir("empty-commands");
        write_config(&root, "commands: {}\n");
        assert!(has_no_commands(&root));
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
        // No record has ever been written for this fresh root under any
        // config dir, so this reads as untrusted regardless of which real
        // directory `env_config_dir()` resolves to - no file is written on
        // this path, so it's safe to exercise the real env resolution here.
        assert!(!is_commands_trusted(&root));
        stdfs::remove_dir_all(&root).unwrap();
    }

    // ---- local-store read/write, keyed by config_dir + root -----------

    #[test]
    fn is_untrusted_before_trusted_after_write_trust() {
        let root = tmp_dir("untrusted-then-trusted");
        let config_dir = tmp_config_dir("untrusted-then-trusted");
        write_config(&root, "commands:\n  test: echo hi\n");
        assert!(!is_commands_trusted_in(&config_dir, &root));
        write_trust_in(&config_dir, &root, Some("tester")).unwrap();
        assert!(is_commands_trusted_in(&config_dir, &root));
        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
    }

    #[test]
    fn write_trust_never_touches_gate_trust_json_in_the_repo() {
        // Finding 1: the local store is the only place a trust record is
        // ever written - a repo-tracked `.gate/trust.json` (present or
        // not) must be untouched by `gate trust`.
        let root = tmp_dir("no-repo-write");
        let config_dir = tmp_config_dir("no-repo-write");
        write_config(&root, "commands:\n  test: echo hi\n");
        stdfs::write(root.join(".gate/trust.json"), "not touched\n").unwrap();

        write_trust_in(&config_dir, &root, None).unwrap();

        assert_eq!(
            stdfs::read_to_string(root.join(".gate/trust.json")).unwrap(),
            "not touched\n"
        );
        assert!(trust_store_dir(&config_dir).is_dir());
        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
    }

    #[test]
    fn a_committed_trust_json_in_the_repo_does_not_grant_trust() {
        // Finding 1, at the unit level: even a `.gate/trust.json` whose
        // hash matches the current commands block exactly is never
        // consulted - only the local store, keyed by config_dir + root,
        // decides.
        let root = tmp_dir("repo-trust-json-ignored");
        let config_dir = tmp_config_dir("repo-trust-json-ignored");
        write_config(&root, "commands:\n  test: echo hi\n");
        let hash = current_commands_hash(&root);
        stdfs::write(
            root.join(".gate/trust.json"),
            format!(
                "{{\n  \"commandsHash\": \"{hash}\",\n  \"trustedAt\": \"2026-01-01T00:00:00.000Z\",\n  \"trustedBy\": \"attacker\",\n  \"coverageVersion\": {CURRENT_COVERAGE_VERSION}\n}}\n"
            ),
        )
        .unwrap();

        assert!(!is_commands_trusted_in(&config_dir, &root));
        assert!(read_trust_in(&config_dir, &root).is_none());
        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
    }

    #[test]
    fn invalidates_trust_when_the_commands_change() {
        let root = tmp_dir("commands-change");
        let config_dir = tmp_config_dir("commands-change");
        write_config(&root, "commands:\n  test: echo one\n");
        write_trust_in(&config_dir, &root, None).unwrap();
        assert!(is_commands_trusted_in(&config_dir, &root));
        write_config(&root, "commands:\n  test: echo two\n");
        assert!(!is_commands_trusted_in(&config_dir, &root));
        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
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
        let config_dir = tmp_config_dir("scope-ignore-change");
        write_config(
            &root,
            "commands:\n  test: echo hi\nscope_ignore:\n  - node_modules/**\n",
        );
        write_trust_in(&config_dir, &root, None).unwrap();
        assert!(is_commands_trusted_in(&config_dir, &root));
        write_config(
            &root,
            "commands:\n  test: echo hi\nscope_ignore:\n  - node_modules/**\n  - coverage/**\n",
        );
        assert!(!is_commands_trusted_in(&config_dir, &root));
        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
    }

    #[test]
    fn f4_an_empty_scope_ignore_does_not_invalidate_a_pre_existing_trust_hash() {
        let root = tmp_dir("f4-empty-scope-ignore");
        write_config(&root, "commands:\n  test: \"echo hi\"\n");
        let config_dir = tmp_config_dir("f4-empty-scope-ignore");
        write_trust_in(&config_dir, &root, None).unwrap();
        assert!(is_commands_trusted_in(&config_dir, &root));
        let pre_upgrade_hash = current_commands_hash(&root);

        write_config(&root, "commands:\n  test: \"echo hi\"\nscope_ignore: []\n");
        assert_eq!(current_commands_hash(&root), pre_upgrade_hash);
        assert!(is_commands_trusted_in(&config_dir, &root));
        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
    }

    #[test]
    fn round_trips_an_old_format_trust_record_with_no_coverage_version_or_root_field() {
        let root = tmp_dir("old-format-round-trip");
        let config_dir = tmp_config_dir("old-format-round-trip");
        write_config(&root, "commands:\n  test: echo hi\n");
        let path = record_path(&config_dir, &root);
        stdfs::create_dir_all(path.parent().unwrap()).unwrap();
        stdfs::write(
            &path,
            "{\n  \"commandsHash\": \"sha256:abc\",\n  \"trustedAt\": \"2026-01-01T00:00:00.000Z\",\n  \"trustedBy\": null\n}\n",
        )
        .unwrap();

        let record = read_trust_in(&config_dir, &root).unwrap();
        assert_eq!(record.commands_hash, "sha256:abc");
        assert_eq!(record.coverage_version, 1);
        assert_eq!(record.root, canonical_root_string(&root));
        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
    }

    #[test]
    fn write_trust_stamps_the_current_coverage_version_and_canonical_root() {
        let root = tmp_dir("write-stamps-coverage-version");
        let config_dir = tmp_config_dir("write-stamps-coverage-version");
        write_config(&root, "commands:\n  test: echo hi\n");
        let record = write_trust_in(&config_dir, &root, None).unwrap();
        assert_eq!(record.coverage_version, CURRENT_COVERAGE_VERSION);
        assert_eq!(record.root, canonical_root_string(&root));
        let reread = read_trust_in(&config_dir, &root).unwrap();
        assert_eq!(reread.coverage_version, CURRENT_COVERAGE_VERSION);
        assert_eq!(reread.root, canonical_root_string(&root));
        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
    }

    #[test]
    fn two_different_roots_get_independent_records_under_the_same_config_dir() {
        let config_dir = tmp_config_dir("two-roots");
        let root_a = tmp_dir("two-roots-a");
        let root_b = tmp_dir("two-roots-b");
        write_config(&root_a, "commands:\n  test: echo a\n");
        write_config(&root_b, "commands:\n  test: echo b\n");

        write_trust_in(&config_dir, &root_a, None).unwrap();
        assert!(is_commands_trusted_in(&config_dir, &root_a));
        assert!(!is_commands_trusted_in(&config_dir, &root_b));

        write_trust_in(&config_dir, &root_b, None).unwrap();
        assert!(is_commands_trusted_in(&config_dir, &root_a));
        assert!(is_commands_trusted_in(&config_dir, &root_b));

        let count = stdfs::read_dir(trust_store_dir(&config_dir))
            .unwrap()
            .count();
        assert_eq!(count, 2);
        stdfs::remove_dir_all(&root_a).unwrap();
        stdfs::remove_dir_all(&root_b).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
    }

    // ---- diagnose_mismatch: pure over a TrustRecord, no config dir -----

    #[test]
    fn diagnoses_a_coverage_expansion_when_the_legacy_hash_still_matches() {
        let root = tmp_dir("diagnose-coverage-expanded");
        write_config(&root, "commands:\n  test: echo hi\n");
        // A version-1 record, hashed under the legacy (pre-SEC-01) shape,
        // for a config that has since grown playbook coverage (which the
        // current, version-2 hash source would fold in) but whose commands
        // themselves never changed.
        let legacy_hash = legacy_commands_hash(&root);
        stdfs::create_dir_all(root.join(".gate/playbooks")).unwrap();
        stdfs::write(root.join(".gate/playbooks/plan.md"), "# custom\n").unwrap();
        let record = TrustRecord {
            commands_hash: legacy_hash,
            trusted_at: now_iso(),
            trusted_by: None,
            coverage_version: 1,
            root: canonical_root_string(&root),
        };
        assert_ne!(record.commands_hash, current_commands_hash(&root));
        assert_eq!(
            diagnose_mismatch(&root, &record),
            MismatchReason::CoverageExpanded
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn diagnoses_a_real_change_when_the_legacy_hash_also_no_longer_matches() {
        let root = tmp_dir("diagnose-real-change");
        write_config(&root, "commands:\n  test: echo one\n");
        let legacy_hash = legacy_commands_hash(&root);
        write_config(&root, "commands:\n  test: echo two\n");
        let record = TrustRecord {
            commands_hash: legacy_hash,
            trusted_at: now_iso(),
            trusted_by: None,
            coverage_version: 1,
            root: canonical_root_string(&root),
        };
        assert_eq!(diagnose_mismatch(&root, &record), MismatchReason::Changed);
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn diagnoses_a_real_change_when_the_stored_record_is_already_current_version() {
        let root = tmp_dir("diagnose-current-version-mismatch");
        let config_dir = tmp_config_dir("diagnose-current-version-mismatch");
        write_config(&root, "commands:\n  test: echo one\n");
        write_trust_in(&config_dir, &root, None).unwrap();
        write_config(&root, "commands:\n  test: echo two\n");
        let record = read_trust_in(&config_dir, &root).unwrap();
        assert_eq!(record.coverage_version, CURRENT_COVERAGE_VERSION);
        assert_eq!(diagnose_mismatch(&root, &record), MismatchReason::Changed);
        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&config_dir).unwrap();
    }
}
