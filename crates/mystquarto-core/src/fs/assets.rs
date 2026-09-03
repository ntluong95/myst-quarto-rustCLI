//! Copies non-Markdown, non-config files from an input tree to an output
//! tree — the images, `.bib` files, notebooks, and other static content a
//! `quarto render`/`myst build` needs alongside the converted `.qmd`/`.md`
//! files.
//!
//! Three policies, each stated once here rather than scattered across call
//! sites:
//!
//! 1. **Never dereference a symlink.** Every entry is classified with
//!    [`std::fs::symlink_metadata`] (never [`std::fs::metadata`], which
//!    follows symlinks) before it is touched. A symlink — to a file or a
//!    directory — is never copied and never descended into; it is recorded
//!    in the returned [`AssetCopyReport::skipped_symlinks`] instead. This is
//!    the fix for the reproduced hazard: `shutil.copy2` in the Python
//!    implementation follows symlinks, so a symlink planted in an input
//!    tree pointing at a secrets file outside it gets that file's *content*
//!    copied into the output tree as an ordinary file. Skipping (rather
//!    than recreating the symlink at the destination) is the safer default
//!    — see this phase's report for why recreation was not chosen.
//! 2. **Exclude the effective output root from the walk.** Every directory
//!    is checked with
//!    [`crate::fs::path_guard::effective_output_excluded_from_walk`] before
//!    it is descended into. This is the actual D16 fix: an output directory
//!    nested inside the input directory is never walked, so its contents
//!    are never copied into themselves one level deeper.
//! 3. **Refresh-on-change policy.** A destination that already exists is
//!    left alone only if its mtime is *exactly* equal to the source's (the
//!    cheap, common no-op case — nothing has changed since the last run).
//!    Any other case — the mtimes differ, or either file's mtime cannot be
//!    read on this platform — falls back to a full content-hash comparison
//!    before deciding whether to skip. This replaces the Python
//!    implementation's `if not os.path.exists(dst): copy` check, which
//!    never refreshes a destination that already exists no matter how the
//!    source has since changed.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hasher;
use std::io;
use std::path::{Path, PathBuf};

use crate::fs::path_guard::effective_output_excluded_from_walk;

/// Directories never walked into when copying assets — matches
/// [`crate::fs::path_guard`]'s D16 exclusion plus the same fixed
/// directory-name skip-set `discover.rs` uses for file discovery (see that
/// module's `DISCOVERY_SKIP_DIRS` — kept as a single source of truth would
/// require a shared crate-level constant, but this list is deliberately
/// duplicated rather than imported across the core/binary crate boundary so
/// each crate's skip-list is visible next to the walk it governs; both are
/// ported from the same Python `skip_dirs` literal, see `convert.py`
/// `discover_files`/`_copy_assets`).
pub const ASSET_SKIP_DIRS: &[&str] = &[
    "_build",
    ".git",
    ".hg",
    "__pycache__",
    "node_modules",
    ".venv",
    "venv",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    "_site",
    ".quarto",
];

/// What happened during one [`copy_assets`] call, broken down so a caller
/// can report skipped symlinks as a diagnostic without treating them as a
/// hard error.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AssetCopyReport {
    /// Destination paths that were written (new file, or refreshed because
    /// the source changed).
    pub copied: Vec<PathBuf>,
    /// Destination paths that already matched the source and were left
    /// alone.
    pub unchanged: Vec<PathBuf>,
    /// Source paths that were symlinks and were therefore skipped entirely
    /// — never copied, never dereferenced.
    pub skipped_symlinks: Vec<PathBuf>,
}

/// Error from [`copy_assets`]. Each variant names the path being operated
/// on so a caller can report exactly which asset failed.
#[derive(Debug, thiserror::Error)]
pub enum AssetCopyError {
    #[error("failed to read directory {path}: {source}")]
    ReadDir { path: PathBuf, source: io::Error },
    #[error("failed to stat {path}: {source}")]
    Stat { path: PathBuf, source: io::Error },
    #[error("failed to create directory {path}: {source}")]
    CreateDir { path: PathBuf, source: io::Error },
    #[error("failed to copy {src} to {dst}: {source}")]
    Copy {
        src: PathBuf,
        dst: PathBuf,
        source: io::Error,
    },
}

/// Copies every non-Markdown, non-config file under `input_root` to the
/// matching relative path under `output_root`, applying the three policies
/// documented on this module. `content_extensions` (e.g. `["md"]` or
/// `["qmd"]`, without the leading dot) and `config_names` (e.g.
/// `["myst.yml"]`) are excluded from copying — they are converted, not
/// copied, by a different code path.
///
/// `output_root` need not exist yet; directories are created as needed for
/// files actually copied. It is compared against each directory in the walk
/// via [`effective_output_excluded_from_walk`], so it does not need to be
/// canonicalized by the caller first — but if it already exists and the
/// caller has a canonical form handy, passing that avoids any ambiguity
/// from `..`-bearing or symlinked intermediate components.
pub fn copy_assets(
    input_root: &Path,
    output_root: &Path,
    content_extensions: &[&str],
    config_names: &[&str],
) -> Result<AssetCopyReport, AssetCopyError> {
    let mut report = AssetCopyReport::default();
    walk_and_copy(
        input_root,
        input_root,
        output_root,
        content_extensions,
        config_names,
        &mut report,
    )?;
    Ok(report)
}

fn walk_and_copy(
    dir: &Path,
    input_root: &Path,
    output_root: &Path,
    content_extensions: &[&str],
    config_names: &[&str],
    report: &mut AssetCopyReport,
) -> Result<(), AssetCopyError> {
    let entries = fs::read_dir(dir).map_err(|source| AssetCopyError::ReadDir {
        path: dir.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| AssetCopyError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|source| AssetCopyError::Stat {
            path: path.clone(),
            source,
        })?;

        if meta.file_type().is_symlink() {
            // Never dereferenced, whether it points at a file or a
            // directory — policy 1.
            report.skipped_symlinks.push(path);
            continue;
        }

        if meta.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if ASSET_SKIP_DIRS.contains(&name) {
                continue;
            }
            if effective_output_excluded_from_walk(output_root, &path) {
                continue; // policy 2 — the D16 fix
            }
            walk_and_copy(
                &path,
                input_root,
                output_root,
                content_extensions,
                config_names,
                report,
            )?;
            continue;
        }

        // A regular file.
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if content_extensions.contains(&ext) || config_names.contains(&name) {
            continue;
        }

        let rel = path.strip_prefix(input_root).unwrap_or(&path);
        let dst = output_root.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|source| AssetCopyError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        if needs_refresh(&path, &dst)? {
            fs::copy(&path, &dst).map_err(|source| AssetCopyError::Copy {
                src: path.clone(),
                dst: dst.clone(),
                source,
            })?;
            report.copied.push(dst);
        } else {
            report.unchanged.push(dst);
        }
    }

    Ok(())
}

/// Policy 3, precisely: `true` means "copy `src` over `dst`".
fn needs_refresh(src: &Path, dst: &Path) -> Result<bool, AssetCopyError> {
    let dst_meta = match fs::metadata(dst) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(source) => {
            return Err(AssetCopyError::Stat {
                path: dst.to_path_buf(),
                source,
            })
        }
    };
    let src_meta = fs::metadata(src).map_err(|source| AssetCopyError::Stat {
        path: src.to_path_buf(),
        source,
    })?;

    if let (Ok(s), Ok(d)) = (src_meta.modified(), dst_meta.modified()) {
        if s == d {
            return Ok(false);
        }
    }

    content_differs(src, dst)
}

fn content_differs(a: &Path, b: &Path) -> Result<bool, AssetCopyError> {
    Ok(hash_file(a)? != hash_file(b)?)
}

fn hash_file(path: &Path) -> Result<u64, AssetCopyError> {
    let bytes = fs::read(path).map_err(|source| AssetCopyError::Stat {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = DefaultHasher::new();
    hasher.write(&bytes);
    Ok(hasher.finish())
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
        let dir = std::env::temp_dir().join(format!("mystquarto-assets-test-{label}-{nanos}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_secret_outside_input_root_is_never_dereferenced() {
        let tmp = tempdir("symlink-secret");
        let input_root = tmp.join("input");
        let output_root = tmp.join("output");
        let outside = tmp.join("outside");
        fs::create_dir_all(&input_root).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let secret_path = outside.join("secret.txt");
        fs::write(&secret_path, "TOP SECRET").unwrap();
        std::os::unix::fs::symlink(&secret_path, input_root.join("link.txt")).unwrap();

        let report = copy_assets(&input_root, &output_root, &["md", "qmd"], &["myst.yml"])
            .expect("copy_assets should not error on an out-of-root symlink, only skip it");

        assert_eq!(report.skipped_symlinks, vec![input_root.join("link.txt")]);
        assert!(report.copied.is_empty());

        let would_be_dst = output_root.join("link.txt");
        assert!(
            !would_be_dst.exists(),
            "a symlink must never be materialized in the output tree, even as a broken link"
        );
        // Extra-paranoid check per the phase spec: fail loudly if secret
        // content ever made it into the output tree by any path.
        if let Ok(contents) = fs::read_to_string(&would_be_dst) {
            assert_ne!(
                contents, "TOP SECRET",
                "secret content was copied into the output tree"
            );
        }

        cleanup(&tmp);
    }

    #[test]
    fn changed_source_content_refreshes_the_destination() {
        let tmp = tempdir("refresh");
        let input_root = tmp.join("input");
        let output_root = tmp.join("output");
        fs::create_dir_all(&input_root).unwrap();

        fs::write(input_root.join("data.csv"), "a,b\n1,2\n").unwrap();
        copy_assets(&input_root, &output_root, &["md"], &["myst.yml"]).unwrap();
        assert_eq!(
            fs::read_to_string(output_root.join("data.csv")).unwrap(),
            "a,b\n1,2\n"
        );

        // Force a distinct mtime (some filesystems have 1s granularity) so
        // the cheap mtime-equality skip cannot mask the content change.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(input_root.join("data.csv"), "a,b\n3,4\n").unwrap();

        let report = copy_assets(&input_root, &output_root, &["md"], &["myst.yml"]).unwrap();
        assert_eq!(report.copied, vec![output_root.join("data.csv")]);
        assert_eq!(
            fs::read_to_string(output_root.join("data.csv")).unwrap(),
            "a,b\n3,4\n",
            "changed source content must refresh the destination, not leave it stale"
        );

        cleanup(&tmp);
    }

    #[test]
    fn unchanged_source_is_not_recopied() {
        let tmp = tempdir("unchanged");
        let input_root = tmp.join("input");
        let output_root = tmp.join("output");
        fs::create_dir_all(&input_root).unwrap();
        fs::write(input_root.join("banner.png"), b"fake-png-bytes").unwrap();

        copy_assets(&input_root, &output_root, &["md"], &["myst.yml"]).unwrap();
        let report = copy_assets(&input_root, &output_root, &["md"], &["myst.yml"]).unwrap();

        assert!(report.copied.is_empty());
        assert_eq!(report.unchanged, vec![output_root.join("banner.png")]);

        cleanup(&tmp);
    }

    #[test]
    fn output_directory_nested_inside_input_is_excluded_from_the_walk() {
        // The D16 fixture's shape: an output dir already sitting inside the
        // input tree, pre-populated as if a prior run had written it.
        let tmp = tempdir("d16");
        let input_root = tmp.join("input");
        let output_root = input_root.join("docs-quarto");
        fs::create_dir_all(&output_root).unwrap();
        fs::write(input_root.join("keep.txt"), "keep me").unwrap();
        fs::write(output_root.join("banner.png"), b"already-converted-output").unwrap();

        let report = copy_assets(&input_root, &output_root, &["md", "qmd"], &["myst.yml"])
            .expect("copying assets must not error even with output nested in input");

        assert_eq!(report.copied, vec![output_root.join("keep.txt")]);
        assert!(
            !output_root.join("docs-quarto").exists(),
            "the output dir must never be re-copied into itself"
        );
        assert!(!output_root.join("docs-quarto").join("banner.png").exists());

        cleanup(&tmp);
    }

    #[test]
    fn markdown_and_config_files_are_never_copied_as_assets() {
        let tmp = tempdir("skip-md-config");
        let input_root = tmp.join("input");
        let output_root = tmp.join("output");
        fs::create_dir_all(&input_root).unwrap();
        fs::write(input_root.join("article.md"), "# Hi\n").unwrap();
        fs::write(input_root.join("myst.yml"), "project: {}\n").unwrap();
        fs::write(input_root.join("helper.py"), "print(1)\n").unwrap();

        let report = copy_assets(&input_root, &output_root, &["md"], &["myst.yml"]).unwrap();

        assert!(!output_root.join("article.md").exists());
        assert!(!output_root.join("myst.yml").exists());
        assert!(output_root.join("helper.py").exists());
        assert_eq!(report.copied, vec![output_root.join("helper.py")]);

        cleanup(&tmp);
    }
}
