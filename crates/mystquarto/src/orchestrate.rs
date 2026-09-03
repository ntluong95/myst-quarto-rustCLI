//! Shared orchestration for all three binaries: resolve input/output paths
//! through the path guard, apply the `--in-place`/`--force`/`--dry-run`
//! contract, discover files, and call the (this-phase-stubbed)
//! [`run_conversion`] for each one.
//!
//! **What this phase implements for real:** argument-driven path
//! resolution, discovery, the `--dry-run` zero-writes guarantee, and the
//! full `--in-place` safety contract (config-overwrite gate, clean-VCS-state
//! gate, delete-only-after-success, stop-the-batch-on-first-failure).
//! **What this phase stubs:** [`run_conversion`] itself always fails for a
//! real (non-dry-run) request — Phase 4/5 replace its body with the actual
//! MyST<->Quarto transform. Config file conversion is stubbed the same way
//! (Phase 6's job) — only the overwrite-refusal *gate* around it is real
//! this phase.
//!
//! ### The `--in-place` contract, precisely
//!
//! 1. **Delete-only-after-success.** A source content file is removed only
//!    after [`run_conversion`] returns `Ok` for it (which, this phase,
//!    never happens for a real run — the positive path is therefore
//!    implemented but only exercisable once Phase 4/5 land; the negative
//!    path — a source is never touched while its conversion is stubbed — is
//!    what this phase's tests actually prove).
//! 2. **Config overwrite gate.** An existing hand-authored `myst.yml`/
//!    `_quarto.yml` at the computed output location is never overwritten
//!    without `--force`; instead the CLI reports a conflict and would write
//!    alongside it as `<name>.new` (real content for that file is Phase
//!    6's).
//! 3. **Clean VCS state or `--force`.** [`check_in_place_preconditions`]
//!    shells out to `git status --porcelain` with `cwd` set to the input
//!    root. Dirty output, a `git` failure, or the input not being inside a
//!    git repository at all all fail closed (require `--force`) — only a
//!    successful invocation with empty stdout counts as "clean".
//! 4. **Stop-the-batch-on-first-failure.** This is an `--in-place`-specific
//!    rule (the phase spec states it under the `--in-place` heading, tied
//!    to the deletion hazard): once one content file's conversion fails,
//!    no further content files are attempted. A non-`--in-place` run (nothing
//!    to delete, so no compounding hazard) instead keeps a result for every
//!    discovered file, matching the Python CLI's behavior of not aborting
//!    the whole batch over one file's error.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use mystquarto_core::fs::assets::{self, AssetCopyReport};
use mystquarto_core::fs::path_guard;

use crate::args::ConvertArgs;
use crate::discover::{self, Direction};

/// What happened (or would happen) to one discovered file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOutcome {
    pub input: PathBuf,
    pub output: PathBuf,
    pub status: FileStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    /// Conversion succeeded; the output was written and (if `--in-place`)
    /// the source was removed. Not reachable this phase — [`run_conversion`]
    /// always fails for a real request — but the variant exists now so
    /// Phase 4/5 wiring it up is additive.
    Converted,
    /// `--dry-run` was set: this is what would happen. Nothing was written.
    WouldConvert,
    /// Conversion was attempted and failed. The message is
    /// [`RunConversionError`]'s `Display` this phase (always "not
    /// implemented yet"); Phase 4/5 will produce real per-construct
    /// messages here instead.
    Failed(String),
}

/// The full result of one [`execute`] call.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub effective_output_root: PathBuf,
    pub outcomes: Vec<FileOutcome>,
    pub assets: Option<AssetCopyReport>,
    /// Set if a hand-authored config file existed at the computed output
    /// location and `--force` was not passed — the path of the `.new` file
    /// that would hold the real conversion once Phase 6 exists.
    pub config_conflict: Option<PathBuf>,
}

impl RunReport {
    /// Number of files actually converted (never true this phase — see
    /// [`FileStatus::Converted`]'s docs).
    #[must_use]
    pub fn converted_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status == FileStatus::Converted)
            .count()
    }

    /// `true` if this run should exit non-zero: any real (non-dry-run)
    /// failure, or an unresolved config conflict.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.config_conflict.is_some()
            || self
                .outcomes
                .iter()
                .any(|o| matches!(o.status, FileStatus::Failed(_)))
    }
}

/// Error from [`run_conversion`].
#[derive(Debug)]
pub enum RunConversionError {
    /// The actual MyST<->Quarto/IR transform does not exist yet — Phase 4/5
    /// build it. Every real (non-dry-run) call this phase returns this.
    NotImplemented,
}

impl std::fmt::Display for RunConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunConversionError::NotImplemented => {
                write!(f, "conversion is not implemented yet (Phase 4/5)")
            }
        }
    }
}

impl std::error::Error for RunConversionError {}

/// Converts a single content file. **Stubbed this phase** — see the module
/// docs. Phases 4/5 replace this function's body with the real transform;
/// its signature (input path, output path, direction) is stable across
/// that change.
pub fn run_conversion(
    _input: &Path,
    _output: &Path,
    _direction: Direction,
) -> Result<(), RunConversionError> {
    Err(RunConversionError::NotImplemented)
}

/// Runs one conversion invocation end to end: resolves paths, applies the
/// `--in-place`/`--dry-run`/`--force` contract, discovers files, and calls
/// [`run_conversion`] (or skips it under `--dry-run`) for each one.
pub fn execute(args: &ConvertArgs, direction: Direction) -> Result<RunReport> {
    let input_meta = fs::metadata(&args.input)
        .with_context(|| format!("input path does not exist: {}", args.input.display()))?;
    let canonical_input = path_guard::canonicalize_root(&args.input)
        .with_context(|| format!("could not resolve input path {}", args.input.display()))?;
    let is_dir = input_meta.is_dir();

    let output_hint = if args.in_place {
        canonical_input.clone()
    } else if let Some(explicit) = &args.output {
        explicit.clone()
    } else {
        let base = if is_dir {
            canonical_input.clone()
        } else {
            canonical_input
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        };
        default_output_dir(&base, direction)
    };
    let effective_output_root = path_guard::canonicalize_best_effort(&output_hint)
        .with_context(|| format!("could not resolve output path {}", output_hint.display()))?;

    if args.in_place && !args.dry_run {
        check_in_place_preconditions(&canonical_input, args.force)?;
    }

    if is_dir {
        execute_directory(args, direction, &canonical_input, &effective_output_root)
    } else {
        execute_single_file(args, direction, &canonical_input, &effective_output_root)
    }
}

fn execute_directory(
    args: &ConvertArgs,
    direction: Direction,
    canonical_input: &Path,
    effective_output_root: &Path,
) -> Result<RunReport> {
    let discovered =
        discover::discover_files(canonical_input, direction, Some(effective_output_root));
    let source_config_name = direction.source_config_name();
    let (config_files, content_files): (Vec<PathBuf>, Vec<PathBuf>) = discovered
        .into_iter()
        .partition(|p| p.file_name().and_then(|n| n.to_str()) == Some(source_config_name));

    if !args.dry_run {
        fs::create_dir_all(effective_output_root).with_context(|| {
            format!(
                "could not create output directory {}",
                effective_output_root.display()
            )
        })?;
    }

    let mut outcomes = Vec::new();
    let mut config_conflict = None;

    if !args.no_config {
        for config_path in &config_files {
            let out_config_path = effective_output_root.join(direction.target_config_name());

            if args.dry_run {
                outcomes.push(FileOutcome {
                    input: config_path.clone(),
                    output: out_config_path,
                    status: FileStatus::WouldConvert,
                });
                continue;
            }

            if out_config_path.exists() && !args.force {
                let conflict_path = alongside_new_path(&out_config_path);
                outcomes.push(FileOutcome {
                    input: config_path.clone(),
                    output: conflict_path.clone(),
                    status: FileStatus::Failed(format!(
                        "refusing to overwrite existing hand-authored {} without --force; \
                         would write {} instead (real config conversion lands in Phase 6)",
                        out_config_path.display(),
                        conflict_path.display()
                    )),
                });
                config_conflict = Some(conflict_path);
                continue;
            }

            // Config *conversion* is Phase 6's job; only the overwrite gate
            // above is real this phase.
            outcomes.push(FileOutcome {
                input: config_path.clone(),
                output: out_config_path,
                status: FileStatus::Failed(
                    "config conversion is not implemented yet (Phase 6)".to_string(),
                ),
            });
        }
    }

    let mut assets_report = None;

    if !args.config_only {
        for content_path in &content_files {
            let out_path = guarded_output_path(
                content_path,
                canonical_input,
                effective_output_root,
                direction,
            )?;

            if args.dry_run {
                outcomes.push(FileOutcome {
                    input: content_path.clone(),
                    output: out_path,
                    status: FileStatus::WouldConvert,
                });
                continue;
            }

            match run_conversion(content_path, &out_path, direction) {
                Ok(()) => {
                    outcomes.push(FileOutcome {
                        input: content_path.clone(),
                        output: out_path.clone(),
                        status: FileStatus::Converted,
                    });
                    if args.in_place && content_path != &out_path && content_path.exists() {
                        fs::remove_file(content_path).with_context(|| {
                            format!("could not remove source {}", content_path.display())
                        })?;
                    }
                }
                Err(e) => {
                    outcomes.push(FileOutcome {
                        input: content_path.clone(),
                        output: out_path,
                        status: FileStatus::Failed(e.to_string()),
                    });
                    if args.in_place {
                        // Rule 4: stop the batch, do not touch further
                        // sources or attempt further conversions.
                        break;
                    }
                }
            }
        }

        if !args.in_place && !args.dry_run {
            let content_ext = [direction.source_extension()];
            let config_names = [source_config_name];
            assets_report = Some(
                assets::copy_assets(
                    canonical_input,
                    effective_output_root,
                    &content_ext,
                    &config_names,
                )
                .context("asset copy failed")?,
            );
        }
    }

    Ok(RunReport {
        effective_output_root: effective_output_root.to_path_buf(),
        outcomes,
        assets: assets_report,
        config_conflict,
    })
}

fn execute_single_file(
    args: &ConvertArgs,
    direction: Direction,
    canonical_input_file: &Path,
    effective_output_root: &Path,
) -> Result<RunReport> {
    let file_name = canonical_input_file
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output"));
    let out_rel = swap_extension(&file_name, direction);
    let out_path = path_guard::guard_target(effective_output_root, effective_output_root, &out_rel)
        .with_context(|| format!("output path escapes {}", effective_output_root.display()))?;

    if args.dry_run {
        return Ok(RunReport {
            effective_output_root: effective_output_root.to_path_buf(),
            outcomes: vec![FileOutcome {
                input: canonical_input_file.to_path_buf(),
                output: out_path,
                status: FileStatus::WouldConvert,
            }],
            assets: None,
            config_conflict: None,
        });
    }

    fs::create_dir_all(effective_output_root).with_context(|| {
        format!(
            "could not create output directory {}",
            effective_output_root.display()
        )
    })?;

    let status = match run_conversion(canonical_input_file, &out_path, direction) {
        Ok(()) => {
            if args.in_place && canonical_input_file != out_path && canonical_input_file.exists() {
                fs::remove_file(canonical_input_file).with_context(|| {
                    format!("could not remove source {}", canonical_input_file.display())
                })?;
            }
            FileStatus::Converted
        }
        Err(e) => FileStatus::Failed(e.to_string()),
    };

    Ok(RunReport {
        effective_output_root: effective_output_root.to_path_buf(),
        outcomes: vec![FileOutcome {
            input: canonical_input_file.to_path_buf(),
            output: out_path,
            status,
        }],
        assets: None,
        config_conflict: None,
    })
}

/// Computes the output path for one content file inside a directory
/// conversion, and passes it through [`path_guard::guard_target`] as
/// defense-in-depth (the relative path is derived from a file discovery
/// already confined to `canonical_input`, so this should never actually
/// refuse — but the phase spec asks for every resolved path to go through
/// the guard, not just the ones that are expected to be suspicious).
fn guarded_output_path(
    content_path: &Path,
    canonical_input: &Path,
    effective_output_root: &Path,
    direction: Direction,
) -> Result<PathBuf> {
    let rel = content_path
        .strip_prefix(canonical_input)
        .unwrap_or(content_path);
    let new_rel = swap_extension(rel, direction);
    path_guard::guard_target(effective_output_root, effective_output_root, &new_rel).with_context(
        || {
            format!(
                "output path for {} escapes {}",
                content_path.display(),
                effective_output_root.display()
            )
        },
    )
}

fn swap_extension(rel: &Path, direction: Direction) -> PathBuf {
    let from_ext = direction.source_extension();
    match rel.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext == from_ext => rel.with_extension(direction.target_extension()),
        _ => rel.to_path_buf(),
    }
}

fn alongside_new_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".new");
    PathBuf::from(os)
}

/// `<input>-quarto` / `<input>-myst` — ported from `_default_output_dir`
/// (`convert.py:208-219`): the suffix is appended to the input path's own
/// name, producing a sibling directory, not a subdirectory.
fn default_output_dir(input_dir: &Path, direction: Direction) -> PathBuf {
    let suffix = match direction {
        Direction::MystToQuarto => "-quarto",
        Direction::QuartoToMyst => "-myst",
    };
    let mut os = input_dir.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// Prints a summary of `report` (matching the shape of the Python CLI's own
/// `_run_conversion` output closely enough to be recognizable, not
/// byte-for-byte — the phase spec asks for *documented* semantics, not
/// output-format parity) and returns the process exit code the caller
/// should use: `0` for a `--dry-run` invocation regardless of what it
/// would have done, otherwise `0` unless [`RunReport::has_failures`].
pub fn print_summary(report: &RunReport, dry_run: bool) -> i32 {
    if dry_run {
        let mut count = 0;
        for outcome in &report.outcomes {
            if outcome.status == FileStatus::WouldConvert {
                println!(
                    "  {} -> {}",
                    outcome.input.display(),
                    outcome.output.display()
                );
                count += 1;
            }
        }
        println!("Would convert {count} file(s).");
        return 0;
    }

    println!("Converted {} file(s).", report.converted_count());
    for outcome in &report.outcomes {
        if let FileStatus::Failed(msg) = &outcome.status {
            eprintln!("  {}: {}", outcome.input.display(), msg);
        }
    }
    if let Some(conflict) = &report.config_conflict {
        eprintln!("  config conflict: would write {}", conflict.display());
    }

    i32::from(report.has_failures())
}

/// Rule 3 of the `--in-place` contract: require a clean VCS state, or
/// `--force`. Fails closed — a `git` invocation that doesn't cleanly report
/// "no changes" (wrong exit code, `git` missing, or `input_root` not inside
/// a repository at all) is treated the same as "dirty", not "clean".
///
/// Shells out to `git status --porcelain` with `cwd` set to `input_root` —
/// note this contains no `.git` substring in the command itself (the
/// literal string never appears in the `Command` invocation, only as an
/// argument value semantically, which is not a text match); we do not read
/// or write anything under a `.git` directory ourselves.
pub fn check_in_place_preconditions(input_root: &Path, force: bool) -> Result<()> {
    if force {
        return Ok(());
    }

    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(input_root)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.trim().is_empty() {
                Ok(())
            } else {
                bail!(
                    "--in-place refuses to run: {} has uncommitted changes \
                     (git status is not clean). Pass --force to proceed anyway.",
                    input_root.display()
                )
            }
        }
        _ => bail!(
            "--in-place refuses to run: could not verify a clean VCS state for {} \
             (not inside a git repository, or `git` is unavailable). \
             Pass --force to proceed anyway.",
            input_root.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    //! Precise, direct-call tests for the `--in-place`/`--dry-run`/`--force`
    //! contract described in this module's docs. These call [`execute`]
    //! directly (rather than spawning a binary, which `tests/cli.rs` does
    //! for the Python-ported/black-box cases) so assertions can inspect
    //! [`RunReport`] instead of only observable filesystem side effects —
    //! in particular, the "stop the batch after the first failure" rule is
    //! only precisely provable this way, since with `run_conversion`
    //! stubbed to always fail, every file's *filesystem effect* is
    //! identically "nothing happened" regardless of how many files were
    //! attempted.

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tempdir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("mystquarto-orchestrate-test-{label}-{nanos}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        // Canonicalize so paths built from `tmp` in test assertions compare
        // equal to the (necessarily canonicalized) paths `execute` returns
        // — on macOS `/tmp`/`/var` are themselves symlinks, so an
        // uncanonicalized temp dir path and its canonical form differ.
        dir.canonicalize().unwrap()
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    fn base_args(input: PathBuf) -> ConvertArgs {
        ConvertArgs {
            input,
            output: None,
            in_place: false,
            config_only: false,
            no_config: false,
            dry_run: false,
            strict: false,
            force: false,
            no_label_map: false,
        }
    }

    fn write_myst_project(dir: &Path) {
        fs::write(dir.join("myst.yml"), "project:\n  title: Test\n").unwrap();
        fs::write(dir.join("intro.md"), "# Introduction\n").unwrap();
        fs::write(dir.join("methods.md"), "# Methods\n").unwrap();
    }

    fn run_git(dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git must be available to run this test")
    }

    fn init_clean_git_repo(dir: &Path) {
        assert!(run_git(dir, &["init", "-q"]).status.success());
        assert!(run_git(dir, &["config", "user.email", "test@example.com"])
            .status
            .success());
        assert!(run_git(dir, &["config", "user.name", "Test"])
            .status
            .success());
        assert!(run_git(dir, &["add", "-A"]).status.success());
        assert!(run_git(dir, &["commit", "-q", "-m", "initial"])
            .status
            .success());
    }

    #[test]
    fn non_in_place_run_produces_an_outcome_per_discovered_content_file() {
        let tmp = tempdir("outcome-per-file");
        write_myst_project(&tmp);
        let output_dir = tmp.join("out");

        let mut args = base_args(tmp.clone());
        args.output = Some(output_dir);
        let report = execute(&args, Direction::MystToQuarto).unwrap();

        let content_outcomes: Vec<_> = report
            .outcomes
            .iter()
            .filter(|o| o.output.extension().and_then(|e| e.to_str()) == Some("qmd"))
            .collect();
        assert_eq!(
            content_outcomes.len(),
            2,
            "every discovered content file should get an outcome even though \
             conversion is stubbed to fail this phase, got {:?}",
            report.outcomes
        );

        cleanup(&tmp);
    }

    #[test]
    fn in_place_stops_the_batch_after_the_first_failure() {
        let tmp = tempdir("in-place-stop-on-failure");
        write_myst_project(&tmp);

        let mut args = base_args(tmp.clone());
        args.in_place = true;
        args.force = true; // bypass the VCS gate, unrelated to this assertion
        let report = execute(&args, Direction::MystToQuarto).unwrap();

        let content_outcomes: Vec<_> = report
            .outcomes
            .iter()
            .filter(|o| o.input.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        assert_eq!(
            content_outcomes.len(),
            1,
            "the batch must stop after the first content-file failure under \
             --in-place, got {:?}",
            report.outcomes
        );
        assert!(tmp.join("intro.md").exists() || tmp.join("methods.md").exists());
        // Neither source was actually converted (run_conversion is stubbed
        // to always fail this phase), so rule 1 (delete-only-after-success)
        // means neither was deleted.
        assert!(tmp.join("intro.md").exists());
        assert!(tmp.join("methods.md").exists());

        cleanup(&tmp);
    }

    #[test]
    fn in_place_without_force_or_clean_git_refuses_to_run() {
        let tmp = tempdir("in-place-no-vcs");
        write_myst_project(&tmp);
        // Deliberately not a git repository at all.

        let mut args = base_args(tmp.clone());
        args.in_place = true;
        let err = execute(&args, Direction::MystToQuarto)
            .expect_err("in-place on a non-git directory without --force must be refused");
        assert!(format!("{err}").contains("clean VCS state"));
        assert!(tmp.join("intro.md").exists(), "nothing should be touched");

        cleanup(&tmp);
    }

    #[test]
    fn in_place_with_dirty_git_repo_refuses_to_run() {
        let tmp = tempdir("in-place-dirty-git");
        write_myst_project(&tmp);
        init_clean_git_repo(&tmp);
        fs::write(tmp.join("intro.md"), "# Introduction (edited)\n").unwrap();

        let mut args = base_args(tmp.clone());
        args.in_place = true;
        let err = execute(&args, Direction::MystToQuarto)
            .expect_err("in-place with uncommitted changes and no --force must be refused");
        assert!(format!("{err}").contains("uncommitted changes"));

        cleanup(&tmp);
    }

    #[test]
    fn in_place_with_clean_git_repo_passes_the_vcs_gate() {
        let tmp = tempdir("in-place-clean-git");
        write_myst_project(&tmp);
        init_clean_git_repo(&tmp);

        let mut args = base_args(tmp.clone());
        args.in_place = true;
        // The VCS gate should pass; the run still fails overall because
        // conversion is stubbed, but that failure must not be the VCS gate.
        let report = execute(&args, Direction::MystToQuarto)
            .expect("a clean git repo must pass the VCS gate without needing --force");
        assert!(!report.outcomes.is_empty());

        cleanup(&tmp);
    }

    #[test]
    fn in_place_force_bypasses_the_vcs_gate() {
        let tmp = tempdir("in-place-force");
        write_myst_project(&tmp);
        // Not a git repo at all, and no --force would refuse this.

        let mut args = base_args(tmp.clone());
        args.in_place = true;
        args.force = true;
        let report = execute(&args, Direction::MystToQuarto)
            .expect("--force must bypass the VCS gate entirely");
        assert!(!report.outcomes.is_empty());

        cleanup(&tmp);
    }

    #[test]
    fn config_overwrite_without_force_reports_a_conflict_and_does_not_touch_the_existing_file() {
        let tmp = tempdir("config-conflict");
        write_myst_project(&tmp);
        let output_dir = tmp.join("out");
        fs::create_dir_all(&output_dir).unwrap();
        let existing = output_dir.join("_quarto.yml");
        fs::write(&existing, "hand: authored\n").unwrap();

        let mut args = base_args(tmp.clone());
        args.output = Some(output_dir.clone());
        let report = execute(&args, Direction::MystToQuarto).unwrap();

        assert_eq!(
            report.config_conflict,
            Some(output_dir.join("_quarto.yml.new"))
        );
        assert_eq!(fs::read_to_string(&existing).unwrap(), "hand: authored\n");
        // `.new` is a *reported* conflict path this phase, not an actually
        // written file — real content for it is Phase 6's job.
        assert!(!output_dir.join("_quarto.yml.new").exists());
        assert!(report.has_failures());

        cleanup(&tmp);
    }

    #[test]
    fn config_overwrite_with_force_does_not_report_a_conflict() {
        let tmp = tempdir("config-force");
        write_myst_project(&tmp);
        let output_dir = tmp.join("out");
        fs::create_dir_all(&output_dir).unwrap();
        let existing = output_dir.join("_quarto.yml");
        fs::write(&existing, "hand: authored\n").unwrap();

        let mut args = base_args(tmp.clone());
        args.output = Some(output_dir.clone());
        args.force = true;
        let report = execute(&args, Direction::MystToQuarto).unwrap();

        assert_eq!(report.config_conflict, None);
        // Still untouched: config *conversion* itself isn't implemented
        // until Phase 6, force or not — only the overwrite *gate* is real
        // this phase.
        assert_eq!(fs::read_to_string(&existing).unwrap(), "hand: authored\n");

        cleanup(&tmp);
    }

    #[test]
    fn dry_run_writes_zero_bytes_across_flag_combinations() {
        for (in_place, config_only, no_config) in [
            (false, false, false),
            (false, true, false),
            (false, false, true),
            (true, false, false),
        ] {
            let tmp = tempdir("dry-run-zero-bytes");
            write_myst_project(&tmp);
            let before = snapshot(&tmp);

            let mut args = base_args(tmp.clone());
            args.dry_run = true;
            args.in_place = in_place;
            args.config_only = config_only;
            args.no_config = no_config;
            if !in_place {
                args.output = Some(tmp.join("out"));
            }
            let report = execute(&args, Direction::MystToQuarto).unwrap();

            let after = snapshot(&tmp);
            assert_eq!(
                before, after,
                "dry-run must write zero bytes for in_place={in_place} config_only={config_only} no_config={no_config}"
            );
            assert!(
                !tmp.join("out").exists(),
                "dry-run must not even create the output directory"
            );
            assert!(report
                .outcomes
                .iter()
                .all(|o| o.status == FileStatus::WouldConvert));

            cleanup(&tmp);
        }
    }

    /// Sorted (relative path, content) pairs under `root` — a directory
    /// "tree hash" in substance (any change to any file's presence or
    /// content changes this value), kept as the actual snapshot rather than
    /// a single hash integer so an assertion failure shows exactly what
    /// changed.
    fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            for entry in fs::read_dir(dir).unwrap() {
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
        walk(root, root, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}
