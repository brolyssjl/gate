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
use std::io;
use std::path::{Path, PathBuf};

use crate::core::fsx::unique_suffix;

/// Create and return a fresh, empty directory under the OS temp dir named
/// `gate-<name>-<pid>-<unguessable suffix>`.
///
/// The suffix mixes `/dev/urandom` bytes with the nanosecond clock (see
/// `fsx::unique_suffix`), so no other process can predict the path, and
/// the directory is created exclusively (see [`unique_temp_dir_in`]) so any
/// entry already sitting at that path - a pre-planted symlink included -
/// makes this panic instead of being silently adopted.
///
/// Callers own the directory: remove it with `fs::remove_dir_all` when the
/// test is done, as the modules here already do. Nothing is removed up
/// front, because a path that could already exist is exactly the property
/// this helper rules out.
pub fn unique_temp_dir(name: &str) -> PathBuf {
    let base = std::env::temp_dir();
    unique_temp_dir_in(&base, name, &unique_suffix()).unwrap_or_else(|e| {
        panic!(
            "unique_temp_dir: could not create a fresh dir for {name} under {}: {e}",
            base.display()
        )
    })
}

/// The seam behind [`unique_temp_dir`]: `base/gate-<name>-<pid>-<suffix>`,
/// created with `fs::create_dir` - never `create_dir_all`, which would
/// succeed on (and then write through) an entry someone planted at that
/// path first. Split out with `base` and `suffix` as parameters so the
/// test below can plant an entry at the exact path and prove the refusal
/// against this function, not against `std`.
fn unique_temp_dir_in(base: &Path, name: &str, suffix: &str) -> io::Result<PathBuf> {
    let dir = base.join(format!("gate-{name}-{}-{suffix}", std::process::id()));
    fs::create_dir(&dir)?;
    Ok(dir)
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
    /// never something the helper writes into. Uses the seam with a fixed
    /// suffix inside a scratch dir of our own, plants a symlink at exactly
    /// the path the helper will pick (pointing at a directory that must
    /// stay untouched), and asserts the refusal. Flipping the helper to
    /// `create_dir_all` makes this fail: that call returns Ok on an
    /// existing symlink-to-directory.
    #[test]
    fn unique_temp_dir_refuses_a_preexisting_entry() {
        let scratch = unique_temp_dir("testutil-plant");
        let victim = scratch.join("victim");
        fs::create_dir(&victim).unwrap();
        let planted = scratch.join(format!("gate-planted-{}-fixed", std::process::id()));
        std::os::unix::fs::symlink(&victim, &planted).unwrap();

        let err = unique_temp_dir_in(&scratch, "planted", "fixed").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read_dir(&victim).unwrap().count(),
            0,
            "victim was written into"
        );

        // The same call on an unplanted path is the normal, working case.
        let fresh = unique_temp_dir_in(&scratch, "fresh", "fixed").unwrap();
        assert!(fresh.is_dir());
        assert_eq!(
            fresh,
            scratch.join(format!("gate-fresh-{}-fixed", std::process::id()))
        );

        fs::remove_dir_all(&scratch).unwrap();
    }
}
