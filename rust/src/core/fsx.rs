//! Port of `src/core/fsx.ts`: crash-safe atomic file writes, plus
//! containment helpers so a write under the project root can never be
//! redirected outside it by a symlink (report finding 3).

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

/// Validate that `rel` (a path meant to be joined onto `root`) is safe to
/// write to, and return the joined target.
///
/// Rejects:
/// - an absolute `rel` (it would ignore `root` entirely),
/// - any `..` component (lexical escape),
/// - a target whose deepest *existing* ancestor, once canonicalized,
///   resolves outside the canonicalized `root` (a symlinked directory
///   partway down the path - the leaf itself need not exist yet),
/// - a target that already exists as anything other than a regular file
///   (a symlink, most importantly - `symlink_metadata` never follows it).
///
/// Callers still get a normal `fs::write`/`write_file_atomic` afterward;
/// this only decides whether the target is safe to write, it does not
/// perform the write itself.
pub fn confined_write_target(root: &Path, rel: &Path) -> io::Result<PathBuf> {
    if rel.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to write to an absolute path outside the project root: {}",
                rel.display()
            ),
        ));
    }
    if rel.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to write outside the project root (path contains '..'): {}",
                rel.display()
            ),
        ));
    }

    let root_canon = fs::canonicalize(root)?;
    let target = root.join(rel);

    // Walk up from the target to the deepest ancestor that actually exists
    // (the leaf, and any number of not-yet-created parent directories, are
    // fine - what matters is that whatever directory *does* exist doesn't
    // resolve, via a symlink, outside root).
    let mut ancestor = target.clone();
    loop {
        if ancestor.exists() {
            break;
        }
        match ancestor.parent() {
            Some(p) => ancestor = p.to_path_buf(),
            None => break,
        }
    }
    let ancestor_canon = fs::canonicalize(&ancestor)?;
    if !ancestor_canon.starts_with(&root_canon) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "refusing to write outside the project root: {} resolves through {} to {}, which is not under {}",
                target.display(),
                ancestor.display(),
                ancestor_canon.display(),
                root_canon.display()
            ),
        ));
    }

    if let Ok(meta) = fs::symlink_metadata(&target) {
        if !meta.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "refusing to write through a symlink (or other non-regular-file) at {}",
                    target.display()
                ),
            ));
        }
    }

    Ok(target)
}

/// A handful of bytes of entropy for a unique temp-file suffix. Prefers
/// `/dev/urandom` (unavailable on non-Unix, or if sandboxed away); falls
/// back to the nanosecond clock, which is unique enough for one process's
/// sequence of writes and costs nothing extra when urandom is fine anyway
/// (both are mixed in together).
fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut rand_hex = String::new();
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let mut buf = [0u8; 8];
        if f.read_exact(&mut buf).is_ok() {
            for b in buf {
                rand_hex.push_str(&format!("{b:02x}"));
            }
        }
    }
    format!("{:x}{rand_hex}", nanos)
}

/// Write a file atomically: write to a freshly-created, uniquely-named
/// sibling temp file, then rename over the target. A crash mid-write leaves
/// the old content (or a stray `.tmp`), never a truncated file.
///
/// The temp file is opened with `create_new(true)` (`O_CREAT|O_EXCL`), which
/// fails rather than following any existing entry at that path - including a
/// symlink, predictable name or not (report finding 3(b): a committed
/// `<path>.tmp -> /outside/file` sibling used to be opened and written
/// through by a plain `fs::write`, then renamed *itself* over the real
/// target). The name is also unique per call (pid + a random/time suffix)
/// so nothing about it is guessable in advance.
pub fn write_file_atomic(path: &Path, content: &str) -> io::Result<()> {
    write_file_atomic_mode(path, content, None)
}

/// Same as [`write_file_atomic`], additionally setting the file's Unix mode
/// (e.g. `0o600` for a file that may contain sensitive diff content - report
/// finding 10) before any content is written to it, rather than leaving it
/// at whatever the ambient umask produces.
pub fn write_file_atomic_mode(path: &Path, content: &str, mode: Option<u32>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Refuse to write through a symlink at the final destination itself.
    // `rename` would just replace the symlink's directory entry rather than
    // writing through it, so this isn't the same bug as the `.tmp`-sibling
    // one above - but silently swapping a tracked symlink for a plain file
    // is not what any caller wants, so fail loudly instead.
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "refusing to write through a symlink (or other non-regular-file) at {}",
                    path.display()
                ),
            ));
        }
    }

    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.{}.tmp", std::process::id(), unique_suffix()));
    let tmp_path = PathBuf::from(tmp);

    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gate-fsx-rs-{name}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_the_file_and_leaves_no_tmp_file_behind() {
        let dir = tmp_dir("basic");
        let target = dir.join("a").join("b").join("file.txt");
        write_file_atomic(&target, "hello").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "hello");
        let leftovers: Vec<_> = fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left tmp files behind: {leftovers:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overwrites_existing_content_in_full() {
        let dir = tmp_dir("overwrite");
        let target = dir.join("file.txt");
        write_file_atomic(&target, "first").unwrap();
        write_file_atomic(&target, "second, and longer than first").unwrap();
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "second, and longer than first"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creates_missing_parent_directories() {
        let dir = tmp_dir("mkdirp");
        let target = dir.join("nested").join("deep").join("file.txt");
        assert!(!target.parent().unwrap().exists());
        write_file_atomic(&target, "x").unwrap();
        assert!(target.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Report finding 3(b): a committed `<path>.tmp`-shaped sibling used to
    /// be a predictable target; now the temp name is unique per call and
    /// created with `O_EXCL`, so a pre-existing symlink at *any* `.tmp`
    /// sibling name is simply not the one this call creates or opens.
    #[test]
    fn write_file_atomic_never_opens_a_preexisting_tmp_sibling() {
        let dir = tmp_dir("tmp-sibling");
        let outside = dir.join("outside.txt");
        fs::write(&outside, "untouched").unwrap();
        let target = dir.join("trust.json");
        let legacy_tmp = dir.join("trust.json.tmp");
        std::os::unix::fs::symlink(&outside, &legacy_tmp).unwrap();

        write_file_atomic(&target, "new content").unwrap();

        assert_eq!(fs::read_to_string(&outside).unwrap(), "untouched");
        assert_eq!(fs::read_to_string(&target).unwrap(), "new content");
        // The old predictable sibling is untouched - it's not our tmp file.
        assert!(fs::symlink_metadata(&legacy_tmp).unwrap().is_symlink());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_file_atomic_refuses_to_write_through_a_symlinked_target() {
        let dir = tmp_dir("symlinked-target");
        let outside = dir.join("outside.txt");
        fs::write(&outside, "untouched").unwrap();
        let target = dir.join("plan.approved.md");
        std::os::unix::fs::symlink(&outside, &target).unwrap();

        let err = write_file_atomic(&target, "attacker content").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&outside).unwrap(), "untouched");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_file_atomic_mode_sets_the_requested_mode() {
        let dir = tmp_dir("mode");
        let target = dir.join("review-packet.md");
        write_file_atomic_mode(&target, "content", Some(0o600)).unwrap();
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn confined_write_target_rejects_absolute_paths() {
        let dir = tmp_dir("confine-abs");
        let err = confined_write_target(&dir, Path::new("/etc/passwd")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn confined_write_target_rejects_dot_dot_components() {
        let dir = tmp_dir("confine-dotdot");
        let err = confined_write_target(&dir, Path::new("../outside.txt")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let err2 = confined_write_target(&dir, Path::new("sub/../../outside.txt")).unwrap_err();
        assert_eq!(err2.kind(), io::ErrorKind::InvalidInput);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn confined_write_target_rejects_a_symlinked_ancestor_directory() {
        let base = tmp_dir("confine-symlinked-parent");
        let root = base.join("root");
        fs::create_dir_all(&root).unwrap();
        let outside = base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        // `root/.gate/runs` is a symlink pointing entirely outside root.
        fs::create_dir_all(root.join(".gate")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".gate").join("runs")).unwrap();

        let err = confined_write_target(&root, Path::new(".gate/runs/x/run.json")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn confined_write_target_rejects_a_symlinked_target() {
        let dir = tmp_dir("confine-symlinked-target");
        let outside = dir.join("outside.txt");
        fs::write(&outside, "untouched").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("plan.approved.md")).unwrap();

        let err = confined_write_target(&dir, Path::new("plan.approved.md")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn confined_write_target_accepts_a_normal_nested_path_with_missing_parents() {
        let dir = tmp_dir("confine-ok");
        let target = confined_write_target(&dir, Path::new(".gate/runs/x/run.json")).unwrap();
        assert_eq!(target, dir.join(".gate/runs/x/run.json"));
        assert!(!target.exists());
        write_file_atomic(&target, "{}").unwrap();
        assert!(target.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
