//! CLI integration tests: black-box invocations of the three built
//! binaries (`mystquarto`, `myst2quarto`, `quarto2myst`) via `assert_cmd`.
//!
//! This file is also where every Phase 1 bucket-B `tests/test_cli.py` case
//! lands, per the phase spec. `tests/corpus/classification.md`'s
//! `test_cli.py` rows list 43 bucket-B tests (plus one bucket-C row,
//! `test_report_empty`, excluded — not this phase's to port). Every one of
//! the 43 gets either a real port below, or an `#[ignore = "..."]`'d stub
//! with a reason, grouped into `mod` blocks mirroring the Python test
//! classes so the mapping is traceable class-by-class. As of Phase 6, every
//! group but one is fully real — only `warning_collector` (8 tests) is
//! still ignored, since it exercises Python's `WarningCollector`, which has
//! no Rust equivalent yet (Phase 7's diagnostics system, not any earlier
//! phase's scope).
//!
//! Beyond the Python port, this file also proves (via the built binaries,
//! black-box) the `--dry-run` zero-bytes guarantee across flag
//! combinations with a recursive tree snapshot, and the D16 exclusion
//! end-to-end. The more granular `--in-place`/`--force`/config-conflict
//! contract tests live as direct-call unit tests next to
//! `crate::orchestrate::execute` itself (see that module's `tests`
//! submodule) — spawning a subprocess for every one of those cases would
//! not add coverage `execute`'s own tests don't already have, and direct
//! calls can assert on `RunReport` instead of only filesystem side effects.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use assert_cmd::Command;
use predicates::prelude::*;

// ---------------------------------------------------------------------
// Test fixtures — ported from tests/conftest.py's `myst_project` /
// `quarto_project` fixtures, as plain helper functions (no pytest-style
// fixture injection in Rust).
// ---------------------------------------------------------------------

fn tempdir(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("mystquarto-cli-test-{label}-{nanos}-{n}"));
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// Ports `conftest.py::myst_project`.
fn myst_project(dir: &Path) {
    fs::write(
        dir.join("myst.yml"),
        "project:\n  title: Test Project\n  authors:\n    - name: Test Author\n  toc:\n    - file: intro\n    - file: methods\nsite:\n  template: book-theme\n",
    )
    .unwrap();
    fs::write(
        dir.join("intro.md"),
        "---\ntitle: Introduction\n---\n\n# Introduction\n\nThis is a MyST doc with {cite}`smith2020`.\n\n```{code-cell} python\nx = 1\n```\n",
    )
    .unwrap();
    fs::write(
        dir.join("methods.md"),
        "---\ntitle: Methods\n---\n\n# Methods\n\nSee {eq}`energy` for details.\n\n```{math}\n:label: eq-energy\n\nE = mc^2\n```\n",
    )
    .unwrap();
    fs::write(dir.join("helper.py"), "# Python helper\nprint('hello')\n").unwrap();
    fs::create_dir_all(dir.join("chapters")).unwrap();
    fs::write(
        dir.join("chapters").join("chapter1.md"),
        "# Chapter 1\n\nContent of chapter 1.\n",
    )
    .unwrap();
}

/// Ports `conftest.py::quarto_project`.
fn quarto_project(dir: &Path) {
    fs::write(
        dir.join("_quarto.yml"),
        "project:\n  type: book\nbook:\n  title: Test Project\n  author:\n    - name: Test Author\n  chapters:\n    - intro.qmd\n    - methods.qmd\n",
    )
    .unwrap();
    fs::write(
        dir.join("intro.qmd"),
        "---\ntitle: Introduction\n---\n\n# Introduction\n\nThis is a Quarto doc with [@smith2020].\n\n```{python}\nx = 1\n```\n",
    )
    .unwrap();
    fs::write(
        dir.join("methods.qmd"),
        "---\ntitle: Methods\n---\n\n# Methods\n\nSee @eq-energy for details.\n\n$$\nE = mc^2\n$$ {#eq-energy}\n",
    )
    .unwrap();
    fs::write(dir.join("helper.py"), "# Python helper\nprint('hello')\n").unwrap();
}

/// Recursive (relative path, content) snapshot of a directory tree, sorted
/// for a stable comparison — the "tree hash" the phase spec asks
/// `--dry-run` be verified against. Kept as the actual snapshot rather than
/// a single hash integer so a failed assertion shows exactly what changed.
fn tree_snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    if root.exists() {
        walk(root, root, &mut out);
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn myst2quarto_cmd() -> Command {
    Command::cargo_bin("myst2quarto").unwrap()
}

fn quarto2myst_cmd() -> Command {
    Command::cargo_bin("quarto2myst").unwrap()
}

fn mystquarto_cmd() -> Command {
    Command::cargo_bin("mystquarto").unwrap()
}

// =======================================================================
// Ports tests/test_cli.py::TestWarningCollector (8 bucket-B tests), against
// this phase's real replacement: `mystquarto_core::diagnostics::Diagnostic`
// plus `orchestrate::print_summary`'s human renderer and `--strict`
// promotion — there is no separate `WarningCollector` type in the Rust
// port (see `crate::orchestrate::RunReport::warnings`), so these test the
// end-to-end CLI behavior the Python tests were checking instead of a
// class-level API that has no Rust analogue.
// =======================================================================
mod warning_collector {
    use super::*;

    #[test]
    fn test_warning_with_file_and_line_prints_in_the_diagnostic_line() {
        // A book with a `part:`-grouped chapter set is exactly the
        // MQ0403 lossy-narrowing path (config/quarto_to_myst.rs) — real,
        // deterministic, and file-scoped.
        let tmp = tempdir("warning-file-line");
        fs::write(
            tmp.join("_quarto.yml"),
            "project:\n  type: book\nbook:\n  chapters:\n    - index.qmd\n    - part: \"P\"\n      chapters:\n        - a.qmd\n",
        )
        .unwrap();
        fs::write(tmp.join("index.qmd"), "# Index\n").unwrap();
        fs::write(tmp.join("a.qmd"), "# A\n").unwrap();
        let output_dir = tempdir("warning-file-line-out");

        let assert = quarto2myst_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(
            stderr.contains("_quarto.yml") && stderr.contains("[MQ0403]"),
            "expected a file-scoped MQ0403 diagnostic line, got:\n{stderr}"
        );

        cleanup(&tmp);
        cleanup(&output_dir);
    }

    #[test]
    fn test_strict_mode_warnings_become_errors() {
        // `_quarto.yml` `categories` with more than one entry (MQ0405,
        // Severity::Warning — only the first maps back to myst.yml's
        // single-valued `subject`): plain conversion succeeds, `--strict`
        // on the identical input fails — proving `--strict` actually
        // promotes a real Warning-class diagnostic to a non-zero exit,
        // which the old bool-only flag never did.
        let tmp = tempdir("strict-promotes");
        fs::write(
            tmp.join("_quarto.yml"),
            "title: X\ncategories:\n  - A\n  - B\n",
        )
        .unwrap();
        fs::write(tmp.join("index.qmd"), "# Index\n").unwrap();

        let out1 = tempdir("strict-promotes-out1");
        quarto2myst_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&out1)
            .assert()
            .success();

        let out2 = tempdir("strict-promotes-out2");
        quarto2myst_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&out2)
            .arg("--strict")
            .assert()
            .failure()
            .code(1);

        cleanup(&tmp);
        cleanup(&out1);
        cleanup(&out2);
    }

    #[test]
    fn test_strict_all_also_promotes_expected_lossy() {
        // The `part:`-grouping case (MQ0403, Severity::LossyExpected):
        // bare `--strict` does not fail on it (LossyExpected is opt-in
        // only), but `--strict=all` does.
        let tmp = tempdir("strict-all-promotes");
        fs::write(
            tmp.join("_quarto.yml"),
            "project:\n  type: book\nbook:\n  chapters:\n    - index.qmd\n    - part: \"P\"\n      chapters:\n        - a.qmd\n",
        )
        .unwrap();
        fs::write(tmp.join("index.qmd"), "# Index\n").unwrap();
        fs::write(tmp.join("a.qmd"), "# A\n").unwrap();

        let out1 = tempdir("strict-all-promotes-out1");
        quarto2myst_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&out1)
            .arg("--strict")
            .assert()
            .success();

        let out2 = tempdir("strict-all-promotes-out2");
        quarto2myst_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&out2)
            .arg("--strict=all")
            .assert()
            .failure()
            .code(1);

        cleanup(&tmp);
        cleanup(&out1);
        cleanup(&out2);
    }

    #[test]
    fn test_has_errors_exit_code_reflects_a_real_file_failure_regardless_of_strict() {
        // Severity::Error-class failures (an unreadable file) already
        // always fail a run via `FileStatus::Failed` — `--strict` must not
        // be required to observe that, and must not change it either.
        let tmp = tempdir("has-errors");
        fs::write(tmp.join("myst.yml"), "project:\n  title: T\n").unwrap();
        fs::write(tmp.join("broken.md"), [0xFF, 0xFE, 0x00, 0x01]).unwrap();
        let output_dir = tempdir("has-errors-out");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .failure()
            .code(1);

        cleanup(&tmp);
        cleanup(&output_dir);
    }

    #[test]
    fn test_report_format_prints_explicit_zero_counts_on_a_clean_run() {
        // "A visible `0 warnings` is the signal that silence was earned"
        // (reference §11) — the summary line's counts must always print,
        // including zeros, not just when something is actually wrong.
        let tmp = tempdir("report-format-clean");
        fs::write(tmp.join("doc.md"), "# Hello\n").unwrap();
        let output_dir = tempdir("report-format-clean-out");

        let assert = myst2quarto_cmd()
            .arg(tmp.join("doc.md"))
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(
            stderr.contains("0 error(s), 0 warning(s), 0 expected-lossy"),
            "got:\n{stderr}"
        );

        cleanup(&tmp);
        cleanup(&output_dir);
    }

    #[test]
    fn test_suppress_toml_baselines_a_code() {
        // The same MQ0403 case as above, but with `.mystquarto/suppress.toml`
        // (in the *input* tree) naming that code — it must vanish from both
        // the printed diagnostics and the counted total, and `--strict`
        // must no longer fail on it.
        let tmp = tempdir("suppress-toml");
        fs::write(
            tmp.join("_quarto.yml"),
            "project:\n  type: book\nbook:\n  chapters:\n    - index.qmd\n    - part: \"P\"\n      chapters:\n        - a.qmd\n",
        )
        .unwrap();
        fs::write(tmp.join("index.qmd"), "# Index\n").unwrap();
        fs::write(tmp.join("a.qmd"), "# A\n").unwrap();
        fs::create_dir_all(tmp.join(".mystquarto")).unwrap();
        fs::write(
            tmp.join(".mystquarto").join("suppress.toml"),
            "codes = [\"MQ0403\"]\n",
        )
        .unwrap();
        let output_dir = tempdir("suppress-toml-out");

        let assert = quarto2myst_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--strict")
            .assert()
            .success();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(!stderr.contains("MQ0403"), "got:\n{stderr}");
        assert!(
            stderr.contains("0 error(s), 0 warning(s), 0 expected-lossy"),
            "got:\n{stderr}"
        );

        cleanup(&tmp);
        cleanup(&output_dir);
    }
}

// =======================================================================
// Ports tests/test_cli.py::TestDiscoverFiles (4 bucket-B tests, all real).
// =======================================================================
mod discover_files {
    use super::*;
    use mystquarto::discover::{discover_files, Direction};

    #[test]
    fn test_discover_myst_files() {
        let tmp = tempdir("discover-myst");
        myst_project(&tmp);

        let files = discover_files(&tmp, Direction::MystToQuarto, None);
        let names: Vec<String> = files
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();

        assert!(names.contains(&"intro.md".to_string()));
        assert!(names.contains(&"methods.md".to_string()));
        assert!(names.contains(&"myst.yml".to_string()));
        assert!(!names.contains(&"helper.py".to_string()));

        cleanup(&tmp);
    }

    #[test]
    fn test_discover_quarto_files() {
        let tmp = tempdir("discover-quarto");
        quarto_project(&tmp);

        let files = discover_files(&tmp, Direction::QuartoToMyst, None);
        let names: Vec<String> = files
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
    fn test_discover_myst_no_config() {
        let tmp = tempdir("discover-no-config");
        fs::write(tmp.join("doc.md"), "# Hello\n").unwrap();

        let files = discover_files(&tmp, Direction::MystToQuarto, None);
        let names: Vec<String> = files
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();

        assert!(names.contains(&"doc.md".to_string()));
        assert!(!names.contains(&"myst.yml".to_string()));

        cleanup(&tmp);
    }

    #[test]
    fn test_discover_empty_directory() {
        let tmp = tempdir("discover-empty");
        let files = discover_files(&tmp, Direction::MystToQuarto, None);
        assert_eq!(files, Vec::<PathBuf>::new());
        cleanup(&tmp);
    }
}

// =======================================================================
// Ports tests/test_cli.py::TestConvertFile (5 bucket-B tests: 2 real, 3
// ignored). This class calls Python's `convert_file` directly; our port
// exercises the equivalent behavior through the CLI binaries, since
// `run_conversion` (the direct equivalent) is a stub this phase.
// =======================================================================
mod convert_file {
    use super::*;

    #[test]
    fn test_convert_single_file_myst_to_quarto() {
        let tmp = tempdir("convert-file-m2q");
        fs::write(
            tmp.join("doc.md"),
            "# Hello\n\nSee {cite}`smith2020` for details.\n",
        )
        .unwrap();
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(tmp.join("doc.md"))
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        let output_file = output_dir.join("doc.qmd");
        assert!(output_file.exists());
        let content = fs::read_to_string(&output_file).unwrap();
        assert!(content.contains("[@smith2020]"));

        cleanup(&tmp);
    }

    #[test]
    fn test_convert_single_file_quarto_to_myst() {
        let tmp = tempdir("convert-file-q2m");
        fs::write(
            tmp.join("doc.qmd"),
            "# Hello\n\nSee [@smith2020] for details.\n",
        )
        .unwrap();
        let output_dir = tmp.join("output");

        quarto2myst_cmd()
            .arg(tmp.join("doc.qmd"))
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        let output_file = output_dir.join("doc.md");
        assert!(output_file.exists());
        let content = fs::read_to_string(&output_file).unwrap();
        // Modern MyST v1 only (accepted decision): `[@smith2020]` is already
        // correct MyST syntax and passes through unchanged — it does *not*
        // become the legacy `{cite}`smith2020`` role. This intentionally
        // diverges from the Python original's assertion, which expected the
        // legacy form; see `docs/dialect-comparison.md` §10 and the plan's
        // "modern MyST dialect" accepted decision.
        assert!(content.contains("[@smith2020]"));
        assert!(!content.contains("{cite}"));

        cleanup(&tmp);
    }

    #[test]
    fn test_convert_file_dry_run() {
        let tmp = tempdir("convert-file-dry-run");
        fs::write(tmp.join("doc.md"), "# Hello\n").unwrap();
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(tmp.join("doc.md"))
            .arg("-o")
            .arg(&output_dir)
            .arg("--dry-run")
            .assert()
            .success();

        assert!(!output_dir.join("doc.qmd").exists());

        cleanup(&tmp);
    }

    /// Ports `test_cli.py::TestConvertFile::test_convert_file_with_frontmatter`.
    #[test]
    fn test_convert_file_with_frontmatter() {
        let tmp = tempdir("convert-file-frontmatter");
        fs::write(
            tmp.join("doc.md"),
            "---\ntitle: My Doc\nkernelspec:\n  name: python3\n---\n\n# Content\n",
        )
        .unwrap();
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(tmp.join("doc.md"))
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        let content = fs::read_to_string(output_dir.join("doc.qmd")).unwrap();
        assert!(content.contains("jupyter:"), "got:\n{content}");
        assert!(!content.contains("kernelspec"), "got:\n{content}");

        cleanup(&tmp);
    }

    #[test]
    fn test_convert_file_nonexistent() {
        let tmp = tempdir("convert-file-nonexistent");

        myst2quarto_cmd()
            .arg(tmp.join("nonexistent.md"))
            .assert()
            .failure();

        cleanup(&tmp);
    }
}

// =======================================================================
// Ports tests/test_cli.py::TestConvertDirectory (12 bucket-B tests: 6
// real, 6 ignored). Real ones assert directory creation, output
// path/extension computation, or asset copying (all implemented this
// phase); ignored ones need real converted content or a real config file
// (Phase 4/5/6).
// =======================================================================
mod convert_directory {
    use super::*;

    #[test]
    fn test_convert_directory_creates_output() {
        let tmp = tempdir("convert-dir-creates-output");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        // The Python test's assertion is purely structural (directory
        // exists; >=2 filtered results end in .qmd) — it never inspects
        // file *content*, so it is exercisable even with `run_conversion`
        // stubbed: path/extension computation happens regardless of
        // whether the stub succeeds.
        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .output()
            .unwrap();

        assert!(output_dir.is_dir());

        cleanup(&tmp);
    }

    /// Ports `test_cli.py::TestConvertDirectory::test_config_file_conversion`.
    /// `myst_project`'s `myst.yml` has `site.template: book-theme`, so §8.1
    /// infers `project.type: book` and places content under a `book:` block.
    #[test]
    fn test_config_file_conversion() {
        let tmp = tempdir("config-file-conversion");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        let quarto_config = output_dir.join("_quarto.yml");
        assert!(quarto_config.exists());
        let content = fs::read_to_string(&quarto_config).unwrap();
        assert!(content.contains("book"), "got:\n{content}");

        cleanup(&tmp);
    }

    /// Ports `test_cli.py::TestConvertDirectory::test_quarto_config_file_conversion`.
    #[test]
    fn test_quarto_config_file_conversion() {
        let tmp = tempdir("quarto-config-file-conversion");
        quarto_project(&tmp);
        let output_dir = tmp.join("output");

        quarto2myst_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        let myst_config = output_dir.join("myst.yml");
        assert!(myst_config.exists());
        let content = fs::read_to_string(&myst_config).unwrap();
        assert!(content.contains("project"), "got:\n{content}");

        cleanup(&tmp);
    }

    #[test]
    fn test_non_markdown_copied_as_assets() {
        let tmp = tempdir("non-markdown-assets");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .output()
            .unwrap();

        assert!(
            output_dir.join("helper.py").exists(),
            "non-Markdown assets must be copied to the output even though \
             content-file conversion is stubbed this phase"
        );

        cleanup(&tmp);
    }

    #[test]
    fn test_file_extension_renaming_myst_to_quarto() {
        let tmp = tempdir("ext-rename-myst-to-quarto");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .output()
            .unwrap();

        // Assets (non-md/config) are the only files actually written this
        // phase, but discovery + path computation still ran; assert no
        // stray .md file was ever created directly under the output root
        // (an extension-swap bug would produce one).
        let stray_md: Vec<_> = fs::read_dir(&output_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
            .collect();
        assert!(
            stray_md.is_empty(),
            "found stray .md file(s) in output: {stray_md:?}"
        );

        cleanup(&tmp);
    }

    #[test]
    fn test_file_extension_renaming_quarto_to_myst() {
        let tmp = tempdir("ext-rename-quarto-to-myst");
        quarto_project(&tmp);
        let output_dir = tmp.join("output");

        quarto2myst_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .output()
            .unwrap();

        let stray_qmd: Vec<_> = fs::read_dir(&output_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("qmd"))
            .collect();
        assert!(
            stray_qmd.is_empty(),
            "found stray .qmd file(s) in output: {stray_qmd:?}"
        );

        cleanup(&tmp);
    }

    #[test]
    fn test_in_place_modifies_source() {
        let tmp = tempdir("convert-dir-in-place");
        myst_project(&tmp);
        // Requires a clean VCS state or --force (crate::orchestrate's
        // in-place contract, absent from the Python original since that
        // gate is this port's own addition — RT-06).
        std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&tmp)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&tmp)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&tmp)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&tmp)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-q", "-m", "initial"])
            .current_dir(&tmp)
            .output()
            .unwrap();

        // --no-config: isolates this test to the --in-place content-file
        // behavior under test (config conversion is covered separately by
        // test_config_file_conversion / test_config_only).
        myst2quarto_cmd()
            .arg(&tmp)
            .arg("--in-place")
            .arg("--no-config")
            .assert()
            .success();

        let converted = tmp.join("intro.qmd");
        assert!(converted.exists());
        let content = fs::read_to_string(&converted).unwrap();
        assert!(content.contains("[@smith2020]"));

        cleanup(&tmp);
    }

    /// Ports `test_cli.py::TestConvertDirectory::test_config_only`.
    #[test]
    fn test_config_only() {
        let tmp = tempdir("config-only");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--config-only")
            .assert()
            .success();

        assert!(output_dir.join("_quarto.yml").exists());
        assert!(!output_dir.join("intro.qmd").exists());
        assert!(!output_dir.join("methods.qmd").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_no_config() {
        let tmp = tempdir("convert-dir-no-config");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--no-config")
            .assert()
            .success();

        assert!(!output_dir.join("_quarto.yml").exists());
        assert!(output_dir.join("intro.qmd").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_dry_run_no_writes() {
        let tmp = tempdir("convert-dir-dry-run-no-writes");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--dry-run")
            .output()
            .unwrap();

        if output_dir.exists() {
            assert!(!output_dir.join("intro.qmd").exists());
            assert!(!output_dir.join("methods.qmd").exists());
        }

        cleanup(&tmp);
    }

    #[test]
    fn test_default_output_dir() {
        let tmp = tempdir("default-output-dir");
        myst_project(&tmp);

        myst2quarto_cmd().arg(&tmp).output().unwrap();

        let expected_dir = {
            let mut os = tmp.as_os_str().to_os_string();
            os.push("-quarto");
            PathBuf::from(os)
        };
        assert!(
            expected_dir.is_dir(),
            "expected default output dir {} to be created",
            expected_dir.display()
        );

        cleanup(&tmp);
        cleanup(&expected_dir);
    }

    #[test]
    fn test_single_file_path() {
        let tmp = tempdir("convert-dir-single-file-path");
        fs::write(tmp.join("doc.md"), "# Hello\n\nSee {cite}`ref1`.\n").unwrap();
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(tmp.join("doc.md"))
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        assert!(output_dir.join("doc.qmd").exists());

        cleanup(&tmp);
    }
}

// =======================================================================
// Ports tests/test_cli.py::TestMyst2QuartoCLI (3 bucket-B tests: 1 real, 2
// ignored).
// =======================================================================
mod myst2quarto_cli {
    use super::*;

    #[test]
    fn test_myst2quarto_single_file() {
        let tmp = tempdir("myst2quarto-single-file");
        fs::write(tmp.join("doc.md"), "# Hello\n\nSee {cite}`ref1`.\n").unwrap();
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(tmp.join("doc.md"))
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        assert!(output_dir.join("doc.qmd").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_myst2quarto_directory() {
        let tmp = tempdir("myst2quarto-directory");
        myst_project(&tmp);
        let mut os = tmp.as_os_str().to_os_string();
        os.push("-out");
        let output_dir = PathBuf::from(os);

        // --no-config: see `test_in_place_modifies_source`'s comment.
        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--no-config")
            .assert()
            .success();

        assert!(output_dir.is_dir());
        assert!(output_dir.join("intro.qmd").exists());

        cleanup(&tmp);
        cleanup(&output_dir);
    }

    #[test]
    fn test_nonexistent_path() {
        let tmp = tempdir("nonexistent-path");

        myst2quarto_cmd().arg(tmp.join("nope")).assert().failure();

        cleanup(&tmp);
    }
}

// =======================================================================
// Ports tests/test_cli.py::TestQuarto2MystCLI (2 bucket-B tests, both
// ignored — both need real converted content).
// =======================================================================
mod quarto2myst_cli {
    use super::*;

    #[test]
    fn test_quarto2myst_single_file() {
        let tmp = tempdir("quarto2myst-single-file");
        fs::write(tmp.join("doc.qmd"), "# Hello\n\nSee [@ref1].\n").unwrap();
        let output_dir = tmp.join("output");

        quarto2myst_cmd()
            .arg(tmp.join("doc.qmd"))
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        assert!(output_dir.join("doc.md").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_quarto2myst_directory() {
        let tmp = tempdir("quarto2myst-directory");
        quarto_project(&tmp);
        let mut os = tmp.as_os_str().to_os_string();
        os.push("-out");
        let output_dir = PathBuf::from(os);

        // --no-config: see `test_in_place_modifies_source`'s comment.
        quarto2myst_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--no-config")
            .assert()
            .success();

        assert!(output_dir.is_dir());
        assert!(output_dir.join("intro.md").exists());

        cleanup(&tmp);
        cleanup(&output_dir);
    }
}

// =======================================================================
// Ports tests/test_cli.py::TestCLIOptions (6 bucket-B tests: 1 real, 5
// ignored).
// =======================================================================
mod cli_options {
    use super::*;

    #[test]
    fn test_output_option() {
        let tmp = tempdir("cli-options-output");
        myst_project(&tmp);
        let output_dir = tmp.join("custom_output");

        // --no-config: see `test_in_place_modifies_source`'s comment.
        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--no-config")
            .assert()
            .success();

        assert!(output_dir.exists());
        assert!(output_dir.join("intro.qmd").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_in_place_option() {
        let tmp = tempdir("cli-options-in-place");
        myst_project(&tmp);
        std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&tmp)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&tmp)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&tmp)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&tmp)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-q", "-m", "initial"])
            .current_dir(&tmp)
            .output()
            .unwrap();

        // --no-config: isolates this test to the --in-place content-file
        // behavior under test (config conversion is covered separately by
        // test_config_file_conversion / test_config_only).
        myst2quarto_cmd()
            .arg(&tmp)
            .arg("--in-place")
            .arg("--no-config")
            .assert()
            .success();

        assert!(tmp.join("intro.qmd").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_dry_run() {
        let tmp = tempdir("cli-options-dry-run");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--dry-run")
            .assert()
            .success();

        if output_dir.exists() {
            assert!(!output_dir.join("intro.qmd").exists());
        }

        cleanup(&tmp);
    }

    /// Ports `test_cli.py::TestCLIOptions::test_config_only`.
    #[test]
    fn test_config_only() {
        let tmp = tempdir("cli-options-config-only");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--config-only")
            .assert()
            .success();

        assert!(output_dir.join("_quarto.yml").exists());
        assert!(!output_dir.join("intro.qmd").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_no_config() {
        let tmp = tempdir("cli-options-no-config");
        myst_project(&tmp);
        let output_dir = tmp.join("output");

        myst2quarto_cmd()
            .arg(&tmp)
            .arg("-o")
            .arg(&output_dir)
            .arg("--no-config")
            .assert()
            .success();

        assert!(!output_dir.join("_quarto.yml").exists());
        assert!(output_dir.join("intro.qmd").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_strict_mode() {
        let tmp = tempdir("cli-options-strict");
        fs::write(tmp.join("doc.md"), "# Hello\n").unwrap();
        // A sibling directory, not nested inside `tmp` — output nested in
        // the input tree triggers the (correct, unrelated) H1
        // output-writes-into-input-tree warning, which `--strict` now
        // legitimately promotes; that is not what this test is about.
        let output_dir = tempdir("cli-options-strict-out");

        // A conversion with nothing lossy in it succeeds under --strict.
        myst2quarto_cmd()
            .arg(tmp.join("doc.md"))
            .arg("-o")
            .arg(&output_dir)
            .arg("--strict")
            .assert()
            .success();

        cleanup(&tmp);
        cleanup(&output_dir);
    }
}

// =======================================================================
// Ports tests/test_cli.py::TestMainSubcommands (3 bucket-B tests: 1 real,
// 2 ignored).
// =======================================================================
mod main_subcommands {
    use super::*;

    #[test]
    fn test_main_to_quarto_subcommand() {
        let tmp = tempdir("main-to-quarto");
        fs::write(tmp.join("doc.md"), "# Hello\n\nSee {cite}`ref1`.\n").unwrap();
        let output_dir = tmp.join("output");

        mystquarto_cmd()
            .arg("to-quarto")
            .arg(tmp.join("doc.md"))
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        assert!(output_dir.join("doc.qmd").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_main_to_myst_subcommand() {
        let tmp = tempdir("main-to-myst");
        fs::write(tmp.join("doc.qmd"), "# Hello\n\nSee [@ref1].\n").unwrap();
        let output_dir = tmp.join("output");

        mystquarto_cmd()
            .arg("to-myst")
            .arg(tmp.join("doc.qmd"))
            .arg("-o")
            .arg(&output_dir)
            .assert()
            .success();

        assert!(output_dir.join("doc.md").exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_main_no_subcommand() {
        mystquarto_cmd()
            .assert()
            .success()
            .stdout(predicate::str::contains("to-quarto"))
            .stdout(predicate::str::contains("to-myst"));
    }
}

// =======================================================================
// Beyond the Python port: this phase's own contract, exercised end to end
// through the built binaries (black-box). The more granular cases live as
// direct-call unit tests in `crate::orchestrate::tests` — see this file's
// module doc.
// =======================================================================

#[test]
fn dry_run_writes_zero_bytes_end_to_end_across_flag_combinations() {
    for extra_args in [
        vec!["--dry-run"],
        vec!["--dry-run", "--config-only"],
        vec!["--dry-run", "--no-config"],
        vec!["--dry-run", "--strict"],
    ] {
        let tmp = tempdir("e2e-dry-run-zero-bytes");
        myst_project(&tmp);
        let before = tree_snapshot(&tmp);
        let output_dir = tmp.join("output");

        let mut cmd = myst2quarto_cmd();
        cmd.arg(&tmp).arg("-o").arg(&output_dir);
        for a in &extra_args {
            cmd.arg(a);
        }
        cmd.assert().success();

        let after = tree_snapshot(&tmp);
        assert_eq!(before, after, "input tree changed for args {extra_args:?}");
        assert!(
            !output_dir.exists(),
            "--dry-run must not create the output directory at all (args {extra_args:?})"
        );

        cleanup(&tmp);
    }
}

#[test]
fn in_place_dry_run_writes_zero_bytes() {
    let tmp = tempdir("e2e-in-place-dry-run");
    myst_project(&tmp);
    let before = tree_snapshot(&tmp);

    myst2quarto_cmd()
        .arg(&tmp)
        .arg("--in-place")
        .arg("--dry-run")
        .assert()
        .success();

    let after = tree_snapshot(&tmp);
    assert_eq!(before, after, "--in-place --dry-run must write zero bytes");

    cleanup(&tmp);
}

#[test]
fn discovery_and_asset_copy_exclude_an_output_dir_nested_inside_the_input() {
    // The D16 shape, exercised through the real binary: an output
    // directory already sitting inside the input tree (as if from a prior
    // run), which must never be walked into again.
    let tmp = tempdir("e2e-d16");
    myst_project(&tmp);
    let output_dir = tmp.join("docs-quarto");
    fs::create_dir_all(&output_dir).unwrap();
    fs::write(output_dir.join("stale.qmd"), "# Stale prior output\n").unwrap();

    myst2quarto_cmd()
        .arg(&tmp)
        .arg("-o")
        .arg(&output_dir)
        .output()
        .unwrap();

    assert!(
        !output_dir.join("docs-quarto").exists(),
        "the output dir must never be walked into and re-copied into itself"
    );

    cleanup(&tmp);
}

#[cfg(unix)]
#[test]
fn symlink_assets_are_skipped_end_to_end() {
    let tmp = tempdir("e2e-symlink-skip");
    myst_project(&tmp);
    let secret = tempdir("e2e-symlink-secret");
    fs::write(secret.join("secret.txt"), "do not copy\n").unwrap();
    std::os::unix::fs::symlink(secret.join("secret.txt"), tmp.join("linked-secret.txt")).unwrap();
    let output_dir = tempdir("e2e-symlink-skip-out");

    let assert = myst2quarto_cmd()
        .arg(&tmp)
        .arg("-o")
        .arg(&output_dir)
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);

    assert!(
        stderr.contains("linked-secret.txt") && stderr.contains("MQ0604"),
        "skipped symlink should be reported as a diagnostic, got:\n{stderr}"
    );
    assert!(
        !output_dir.join("linked-secret.txt").exists(),
        "symlink target must not be copied into the output tree"
    );

    cleanup(&tmp);
    cleanup(&secret);
    cleanup(&output_dir);
}

#[test]
fn include_traversal_is_preserved_with_a_strict_all_diagnostic() {
    let tmp = tempdir("e2e-include-traversal");
    let input = tmp.join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(tmp.join("secret.md"), "outside\n").unwrap();
    fs::write(
        input.join("myst.yml"),
        "project:\n  title: Traversal\n  toc:\n    - file: index\n",
    )
    .unwrap();
    fs::write(
        input.join("index.md"),
        "# Index\n\n```{include} ../secret.md\n```\n",
    )
    .unwrap();
    let output_dir = tempdir("e2e-include-traversal-out");

    let assert = myst2quarto_cmd()
        .arg(&input)
        .arg("-o")
        .arg(&output_dir)
        .arg("--strict")
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("escapes") || stderr.contains("MQ0605"),
        "include traversal should produce a strict Warning diagnostic, got:\n{stderr}"
    );
    assert!(
        !fs::read_to_string(output_dir.join("index.qmd"))
            .unwrap_or_default()
            .contains("outside"),
        "escaped include target content must never be read into output"
    );

    cleanup(&tmp);
    cleanup(&output_dir);
}

#[test]
fn d12_fixture_strict_now_fails_on_real_warnings() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/corpus/defects/d12-silent-warnings/input_dir")
        .canonicalize()
        .unwrap();
    let output_dir = tempdir("e2e-d12-out");

    let assert = myst2quarto_cmd()
        .arg(fixture)
        .arg("-o")
        .arg(&output_dir)
        .arg("--strict=all")
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("[MQ0401]") || stderr.contains("[MQ0402]"),
        "D12 should now be visible to --strict=all, got:\n{stderr}"
    );

    cleanup(&output_dir);
}

#[test]
fn d16_fixture_does_not_recurse_into_nested_output() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/corpus/defects/d16-output-recursion/input_dir")
        .canonicalize()
        .unwrap();
    let tmp = tempdir("e2e-d16-fixture");
    copy_dir(&source, &tmp);
    let output_dir = tmp.join("docs-quarto");

    myst2quarto_cmd()
        .arg(&tmp)
        .arg("-o")
        .arg(&output_dir)
        .assert()
        .success();
    myst2quarto_cmd()
        .arg(&tmp)
        .arg("-o")
        .arg(&output_dir)
        .arg("--force")
        .assert()
        .success();

    assert!(
        !output_dir.join("docs-quarto").exists(),
        "D16 fixture must not create recursive output nesting"
    );

    cleanup(&tmp);
}

#[test]
fn foreign_dialect_preservation_sidecar_is_not_reparsed_by_the_cli() {
    let tmp = tempdir("e2e-foreign-sidecar");
    fs::create_dir_all(tmp.join(".mystquarto")).unwrap();
    fs::write(
        tmp.join("article.qmd"),
        "<!-- mystquarto MQ0201: preserved — see .mystquarto/preserved.json#evil -->\n",
    )
    .unwrap();
    fs::write(
        tmp.join(".mystquarto/preserved.json"),
        "{\n  \"version\": 1,\n  \"entries\": {\n    \"evil\": {\n      \"file\": \"article.md\",\n      \"line\": 1,\n      \"code\": \"MQ0201\",\n      \"kind\": \"hostile\",\n      \"dialect\": \"Myst\",\n      \"original\": [\"```{glossary}\", \"term\", \": definition\", \"```\"]\n    }\n  }\n}\n",
    )
    .unwrap();
    let output_dir = tempdir("e2e-foreign-sidecar-out");

    quarto2myst_cmd()
        .arg(&tmp)
        .arg("-o")
        .arg(&output_dir)
        .assert()
        .success();
    let output = fs::read_to_string(output_dir.join("article.md")).unwrap();
    assert!(
        output.contains("```{glossary}") && output.contains(": definition"),
        "foreign preserved content should be passed through opaquely, got:\n{output}"
    );

    cleanup(&tmp);
    cleanup(&output_dir);
}

#[test]
fn help_flag_exits_zero_for_all_three_binaries() {
    myst2quarto_cmd().arg("--help").assert().success();
    quarto2myst_cmd().arg("--help").assert().success();
    mystquarto_cmd().arg("--help").assert().success();
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}
