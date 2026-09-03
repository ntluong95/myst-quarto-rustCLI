//! Atomic single-file writes: temp file in the destination's own directory,
//! then `rename` into place, so a reader of the final path always sees
//! either the complete old content or the complete new content — never a
//! partial write from a process that died mid-write.
//!
//! The temp file is created in `path`'s parent directory deliberately, not
//! a global temp directory like `/tmp`: `rename` is only atomic when source
//! and destination are on the same filesystem, and a global temp directory
//! is not guaranteed to share a filesystem with an arbitrary output path
//! (a different mount, a different volume on macOS, a container bind mount,
//! …). Same-directory placement is the only way to guarantee the rename is
//! actually atomic rather than silently degrading to a non-atomic
//! copy-then-delete.
//!
//! This module is only the single-file primitive. The "abort a multi-file
//! batch before deleting any source" contract that `--in-place` needs lives
//! in the CLI layer (`mystquarto`'s orchestration code), which calls
//! [`write_atomic`] once per file and decides what to do across files when
//! one of those calls fails.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Error from [`write_atomic`]. The two variants are deliberately distinct
/// so a caller can tell "nothing was written, no cleanup needed" apart from
/// "a temp file may be left behind — see [`AtomicWriteError::orphan_temp_path`]".
#[derive(Debug, thiserror::Error)]
pub enum AtomicWriteError {
    /// The temp file could not be created, written, or flushed. Nothing was
    /// left on disk — this variant's own handling already removes the
    /// partially-written temp file before returning.
    #[error("failed to write temp file {temp_path} for {target}: {source}")]
    WriteTemp {
        target: PathBuf,
        temp_path: PathBuf,
        source: io::Error,
    },
    /// The temp file was written successfully but `rename` into `target`
    /// failed. `target` is therefore untouched (old content, or absent, as
    /// before the call), but `temp_path` may still exist on disk — see
    /// [`AtomicWriteError::orphan_temp_path`].
    #[error("wrote temp file {temp_path} but failed to rename it to {target}: {source}")]
    Rename {
        target: PathBuf,
        temp_path: PathBuf,
        source: io::Error,
    },
}

impl AtomicWriteError {
    /// The orphaned temp file path a caller should attempt to remove, if
    /// this error left one behind. `None` for [`AtomicWriteError::WriteTemp`],
    /// which already cleaned up after itself.
    #[must_use]
    pub fn orphan_temp_path(&self) -> Option<&Path> {
        match self {
            AtomicWriteError::Rename { temp_path, .. } => Some(temp_path),
            AtomicWriteError::WriteTemp { .. } => None,
        }
    }
}

/// Writes `contents` to `path` atomically. `path`'s parent directory must
/// already exist (this function does not create it — callers that need
/// `create_dir_all` first should call it explicitly, since "did the
/// directory need creating" is meaningful for `--dry-run` accounting at the
/// call site).
///
/// # Errors
/// See [`AtomicWriteError`].
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), AtomicWriteError> {
    let temp_path = temp_path_for(path);

    let write_result: io::Result<()> = (|| {
        let mut file = File::create(&temp_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(source) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(AtomicWriteError::WriteTemp {
            target: path.to_path_buf(),
            temp_path,
            source,
        });
    }

    fs::rename(&temp_path, path).map_err(|source| AtomicWriteError::Rename {
        target: path.to_path_buf(),
        temp_path,
        source,
    })
}

fn temp_path_for(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("output");
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".{file_name}.{pid}.{nanos}.mqtmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("mystquarto-atomic-test-{label}-{nanos}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn write_then_read_round_trips() {
        let tmp = tempdir("roundtrip");
        let target = tmp.join("out.qmd");
        write_atomic(&target, b"hello atomic world").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"hello atomic world");
        cleanup(&tmp);
    }

    #[test]
    fn overwrite_replaces_content_fully() {
        let tmp = tempdir("overwrite");
        let target = tmp.join("out.qmd");
        write_atomic(&target, b"first version, quite long indeed").unwrap();
        write_atomic(&target, b"v2").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"v2");
        cleanup(&tmp);
    }

    #[test]
    fn no_leftover_temp_files_after_success() {
        let tmp = tempdir("no-leftovers");
        let target = tmp.join("out.qmd");
        write_atomic(&target, b"content").unwrap();

        let leftovers: Vec<_> = fs::read_dir(&tmp)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .filter(|n| n != "out.qmd")
            .collect();
        assert!(
            leftovers.is_empty(),
            "expected no temp files left behind, found {leftovers:?}"
        );
        cleanup(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn write_failure_leaves_no_partial_or_truncated_file_at_the_final_path() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir("write-failure");
        let readonly_dir = tmp.join("readonly");
        fs::create_dir_all(&readonly_dir).unwrap();
        let target = readonly_dir.join("out.qmd");

        // Make the directory read-only so File::create for the temp file
        // fails with a permissions error.
        fs::set_permissions(&readonly_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let result = write_atomic(&target, b"should never land");

        // Restore permissions before any assertion that might panic, so
        // cleanup always succeeds.
        fs::set_permissions(&readonly_dir, fs::Permissions::from_mode(0o755)).unwrap();

        let err = result.expect_err("write into a read-only directory must fail");
        assert!(matches!(err, AtomicWriteError::WriteTemp { .. }));
        assert!(
            !target.exists(),
            "no partial/truncated file may exist at the final path after a write failure"
        );
        assert!(err.orphan_temp_path().is_none());

        cleanup(&tmp);
    }
}
