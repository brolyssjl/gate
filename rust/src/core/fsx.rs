//! Port of `src/core/fsx.ts`: crash-safe atomic file writes.

use std::fs;
use std::io;
use std::path::Path;

/// Write a file atomically: write to a sibling temp file, then rename over
/// the target. A crash mid-write leaves the old content (or a stray `.tmp`),
/// never a truncated file. Gate is a single-process CLI, so a fixed temp
/// name per target is safe (mirrors the TS implementation exactly - no
/// randomized suffix).
pub fn write_file_atomic(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp_path = Path::new(&tmp);
    fs::write(tmp_path, content)?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-fsx-rs-{name}-{}", std::process::id()));
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
        let mut tmp = target.as_os_str().to_owned();
        tmp.push(".tmp");
        assert!(!Path::new(&tmp).exists());
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
}
