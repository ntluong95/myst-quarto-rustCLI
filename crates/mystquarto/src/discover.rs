//! File discovery: which files a CLI invocation touches. Ported from
//! `discover_files` (`src/mystquarto/convert.py:48-99`), plus the D16 fix
//! the Python implementation lacks: excluding the effective output
//! directory from the walk via
//! `mystquarto_core::fs::path_guard::effective_output_excluded_from_walk`.
//!
//! **Placement decision:** this lives in the `mystquarto` binary crate, not
//! `mystquarto-core`, because it answers "what files does *this CLI
//! invocation* touch" — a directory root, a conversion direction, and an
//! excluded output path are CLI-level concerns tied to one run, not a
//! reusable library primitive the way `path_guard`/`assets`/`atomic` are
//! (those are useful to Phase 4/5/6 readers and writers directly; a flat
//! `Vec<PathBuf>` of "files this invocation should process" is not).
//! `assets.rs` in core does its own directory walk for the same D16 reason,
//! but its job (copy non-content files) is a library-level primitive
//! regardless of who invokes it — this module's job (decide the CLI's file
//! list, with content/config classification by conversion direction) is
//! not. If a later phase needs discovery from non-CLI contexts, revisit —
//! see this phase's final report for the explicit call-out.

use std::path::{Path, PathBuf};

use mystquarto_core::fs::path_guard::effective_output_excluded_from_walk;
use walkdir::WalkDir;

/// Directories `discover_files` never descends into. Ported from
/// `convert.py`'s `skip_dirs` (both its copies — `discover_files` and
/// `_copy_assets` maintained separate but identical sets; this port keeps
/// exactly one, mirrored in `mystquarto_core::fs::assets::ASSET_SKIP_DIRS`
/// for the asset-walk side). `.pytest_cache`/`__pycache__` are not
/// meaningful to a Rust tool's own runs, but real-world input trees being
/// *converted* may contain them (e.g. a mixed Python+docs repo), so they
/// stay in the list.
pub const DISCOVERY_SKIP_DIRS: &[&str] = &[
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

/// Conversion direction. Defined here (not in `mystquarto-core`) because
/// discovery — "which extension/config-file name am I looking for" — is
/// this module's own reason to need it; `crate::orchestrate` reuses this
/// same type rather than defining a second one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    MystToQuarto,
    QuartoToMyst,
}

impl Direction {
    /// The content-file extension (without the leading dot) this direction
    /// reads from.
    #[must_use]
    pub fn source_extension(self) -> &'static str {
        match self {
            Direction::MystToQuarto => "md",
            Direction::QuartoToMyst => "qmd",
        }
    }

    /// The content-file extension this direction writes to.
    #[must_use]
    pub fn target_extension(self) -> &'static str {
        match self {
            Direction::MystToQuarto => "qmd",
            Direction::QuartoToMyst => "md",
        }
    }

    /// The config file name this direction reads from.
    #[must_use]
    pub fn source_config_name(self) -> &'static str {
        match self {
            Direction::MystToQuarto => "myst.yml",
            Direction::QuartoToMyst => "_quarto.yml",
        }
    }

    /// The config file name this direction writes to.
    #[must_use]
    pub fn target_config_name(self) -> &'static str {
        match self {
            Direction::MystToQuarto => "_quarto.yml",
            Direction::QuartoToMyst => "myst.yml",
        }
    }
}

/// Finds convertible files under `directory`: `.md`/`myst.yml` for
/// `MystToQuarto`, `.qmd`/`_quarto.yml` for `QuartoToMyst`.
///
/// `directory` must already be canonicalized by the caller (this function
/// does no canonicalization of its own — see `path_guard`'s contract that
/// canonicalization happens once, at the top of a run). Pass
/// `effective_output_root` (also caller-canonicalized, or best-effort
/// resolved — see `path_guard::canonicalize_best_effort`) whenever an
/// output directory might be nested inside `directory`; pass `None` only
/// when that concern genuinely does not apply.
///
/// Returns a sorted list of absolute file paths — sorted for determinism,
/// matching `convert.py`'s own `sorted(files)` (`os.walk`'s directory order
/// is OS-dependent; `walkdir`'s is platform-stable, but sorting keeps
/// behavior identical either way, and is cheap at the file counts this tool
/// handles).
///
/// Symlinks are never followed: `walkdir::WalkDir` does not follow symlinks
/// unless `.follow_links(true)` is called, which this function never does.
/// A symlink entry (to a file or a directory) is therefore never descended
/// into and never reported as a discovered file — see
/// `tests/cli.rs`'s `discover_does_not_follow_symlinks` for a test proving
/// this.
pub fn discover_files(
    directory: &Path,
    direction: Direction,
    effective_output_root: Option<&Path>,
) -> Vec<PathBuf> {
    let target_ext = direction.source_extension();
    let config_name = direction.source_config_name();

    let mut files = Vec::new();
    let walker = WalkDir::new(directory).into_iter().filter_entry(|entry| {
        if !entry.file_type().is_dir() {
            return true; // filtering directories only; files are handled below
        }
        if entry.depth() == 0 {
            return true; // never prune the root itself
        }
        let name = entry.file_name().to_str().unwrap_or("");
        if DISCOVERY_SKIP_DIRS.contains(&name) {
            return false;
        }
        if let Some(out_root) = effective_output_root {
            if effective_output_excluded_from_walk(out_root, entry.path()) {
                return false; // the D16 fix
            }
        }
        true
    });

    for entry in walker {
        let Ok(entry) = entry else {
            continue; // permission errors etc. on one entry must not abort discovery
        };
        if !entry.file_type().is_file() {
            continue; // directories (already handled) and symlinks (never followed)
        }
        let path = entry.path();
        let file_name = entry.file_name().to_str().unwrap_or("");
        if file_name == config_name {
            files.push(path.to_path_buf());
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some(target_ext) {
            files.push(path.to_path_buf());
        }
    }

    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("mystquarto-discover-test-{label}-{nanos}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    /// Phase 1 did not freeze a `discover_files`-specific expected-file-list
    /// fixture under `tests/corpus/` (confirmed by grep — see this phase's
    /// final report), so this is a synthetic substitute built in-test: a
    /// mix of `.md`/`.qmd`/config/asset files, plus a nested "prior output"
    /// directory that must be excluded (the D16 concern).
    #[test]
    fn discovers_myst_files_and_excludes_nested_prior_output() {
        let tmp = tempdir("synthetic-tree");
        fs::write(tmp.join("myst.yml"), "project: {}\n").unwrap();
        fs::write(tmp.join("intro.md"), "# Intro\n").unwrap();
        fs::create_dir_all(tmp.join("chapters")).unwrap();
        fs::write(tmp.join("chapters/methods.md"), "# Methods\n").unwrap();
        fs::write(tmp.join("helper.py"), "print(1)\n").unwrap();
        fs::write(tmp.join("stray.qmd"), "# Not this direction\n").unwrap();

        // A "prior output" dir nested inside the input tree, already
        // containing converted-looking output — the D16 shape.
        let output_root = tmp.join("docs-quarto");
        fs::create_dir_all(&output_root).unwrap();
        fs::write(output_root.join("intro.qmd"), "# Intro\n").unwrap();
        fs::write(output_root.join("_quarto.yml"), "project: {}\n").unwrap();

        let found = discover_files(&tmp, Direction::MystToQuarto, Some(&output_root));
        let names: Vec<String> = found
            .iter()
            .map(|p| p.strip_prefix(&tmp).unwrap().to_string_lossy().into_owned())
            .collect();

        assert!(names.contains(&"myst.yml".to_string()));
        assert!(names.contains(&"intro.md".to_string()));
        assert!(names.contains(&"chapters/methods.md".to_string()));
        assert!(!names.contains(&"helper.py".to_string()));
        assert!(!names.contains(&"stray.qmd".to_string()));
        assert!(
            names.iter().all(|n| !n.starts_with("docs-quarto")),
            "the nested output dir must be excluded from discovery entirely, found: {names:?}"
        );

        cleanup(&tmp);
    }

    #[test]
    fn discovers_quarto_files() {
        let tmp = tempdir("quarto-files");
        fs::write(tmp.join("_quarto.yml"), "project: {}\n").unwrap();
        fs::write(tmp.join("intro.qmd"), "# Intro\n").unwrap();
        fs::write(tmp.join("methods.qmd"), "# Methods\n").unwrap();
        fs::write(tmp.join("helper.py"), "print(1)\n").unwrap();

        let found = discover_files(&tmp, Direction::QuartoToMyst, None);
        let names: Vec<String> = found
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();

        assert!(names.contains(&"intro.qmd".to_string()));
        assert!(names.contains(&"methods.qmd".to_string()));
        assert!(names.contains(&"_quarto.yml".to_string()));
        assert!(!names.contains(&"helper.py".to_string()));

        cleanup(&tmp);
    }

    #[test]
    fn no_config_file_is_fine() {
        let tmp = tempdir("no-config");
        fs::write(tmp.join("doc.md"), "# Hello\n").unwrap();

        let found = discover_files(&tmp, Direction::MystToQuarto, None);
        let names: Vec<String> = found
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();

        assert!(names.contains(&"doc.md".to_string()));
        assert!(!names.contains(&"myst.yml".to_string()));

        cleanup(&tmp);
    }

    #[test]
    fn empty_directory_returns_empty_list() {
        let tmp = tempdir("empty");
        let found = discover_files(&tmp, Direction::MystToQuarto, None);
        assert_eq!(found, Vec::<PathBuf>::new());
        cleanup(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directory_is_never_followed() {
        let tmp = tempdir("symlink-dir");
        let input_root = tmp.join("input");
        let outside = tmp.join("outside");
        fs::create_dir_all(&input_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.md"), "# secret\n").unwrap();

        std::os::unix::fs::symlink(&outside, input_root.join("linked")).unwrap();

        let found = discover_files(&input_root, Direction::MystToQuarto, None);
        assert!(
            found.is_empty(),
            "a symlinked directory must never be descended into, found: {found:?}"
        );

        cleanup(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_file_is_never_reported() {
        let tmp = tempdir("symlink-file");
        let input_root = tmp.join("input");
        let outside = tmp.join("outside");
        fs::create_dir_all(&input_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("real.md"), "# real\n").unwrap();

        std::os::unix::fs::symlink(outside.join("real.md"), input_root.join("link.md")).unwrap();

        let found = discover_files(&input_root, Direction::MystToQuarto, None);
        assert!(
            found.is_empty(),
            "a symlinked file must never be reported as discovered, found: {found:?}"
        );

        cleanup(&tmp);
    }
}
