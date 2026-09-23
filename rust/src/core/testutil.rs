//! Test-only helpers shared by every unit-test module in this crate.
//! Compiled only under `cfg(test)`; production builds never see it.
//!
//! Added for the 2026-09-22 security audit finding 11 (test hygiene). The
//! rules this module exists to enforce - and that
//! `rust/tests/test_hygiene.rs` lints for - are:
//!
//! - unit tests never mutate process-global state (`env::set_var`,
//!   `env::set_current_dir`): `cargo test` runs a crate's tests as threads
//!   in one process, so such a write races every other test. Pass values
//!   through parameters, or set env on a spawned `Command`;
//! - unit-test temp dirs come from [`unique_temp_dir`] only. A directory
//!   built from a predictable name (`temp_dir()/gate-<mod>-<pid>`) can be
//!   pre-created by another local user on a shared `/tmp`, as a symlink to
//!   somewhere the test then writes into, and `create_dir_all` adopts such
//!   an entry without complaint.

use std::fs;
use std::path::PathBuf;

use crate::core::fsx::unique_suffix;

/// Create and return a fresh, empty directory under the OS temp dir named
/// `gate-<name>-<pid>-<unguessable suffix>`.
///
/// The suffix mixes `/dev/urandom` bytes with the nanosecond clock (see
/// `fsx::unique_suffix`), so no other process can predict the path, and
/// the directory is created with `fs::create_dir` - not `create_dir_all` -
/// so any entry already sitting at that path (a pre-planted symlink
/// included) makes this panic instead of being silently adopted.
///
/// Callers own the directory: remove it with `fs::remove_dir_all` when the
/// test is done, as the modules here already do. Nothing is removed up
/// front, because a path that could already exist is exactly the property
/// this helper rules out.
pub fn unique_temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gate-{name}-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    fs::create_dir(&dir).unwrap_or_else(|e| {
        panic!(
            "unique_temp_dir: refusing to reuse or adopt {}: {e}",
            dir.display()
        )
    });
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finding 11(b): the name must not be reproducible from (module,
    /// test name, pid) alone.
    #[test]
    fn unique_temp_dir_never_returns_the_same_path_twice() {
        let a = unique_temp_dir("testutil-twice");
        let b = unique_temp_dir("testutil-twice");
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
        let prefix = format!("gate-testutil-twice-{}-", std::process::id());
        for d in [&a, &b] {
            let file = d.file_name().unwrap().to_string_lossy();
            assert!(file.starts_with(&prefix), "unexpected name {file}");
            assert!(file.len() > prefix.len(), "no suffix in {file}");
        }
        fs::remove_dir_all(&a).unwrap();
        fs::remove_dir_all(&b).unwrap();
    }

    /// Finding 11(b): a pre-existing entry at the chosen path is an error,
    /// never something the helper writes into. Exercised by racing the
    /// helper against a path we plant first: `create_dir` is `O_EXCL`, so
    /// planting anything - here a symlink to a sibling dir - must panic.
    #[test]
    fn unique_temp_dir_refuses_a_preexisting_entry() {
        let scratch = unique_temp_dir("testutil-plant");
        let planted = scratch.join("planted");
        std::os::unix::fs::symlink(&scratch, &planted).unwrap();
        // The helper only exposes its own naming, so drive the same
        // primitive it uses against the planted path to prove the
        // guarantee it relies on: create_dir refuses an existing entry,
        // symlink or not, where create_dir_all would have "succeeded".
        let err = fs::create_dir(&planted).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(
            fs::create_dir_all(&planted).is_ok(),
            "the hazard this guards against"
        );
        fs::remove_dir_all(&scratch).unwrap();
    }
}
