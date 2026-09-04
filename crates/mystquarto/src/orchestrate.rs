//! Shared orchestration for all three binaries: resolve input/output paths
//! through the path guard, apply the `--in-place`/`--force`/`--dry-run`
//! contract, discover files and notebooks, and run the batch conversion
//! pipeline ([`mystquarto_core::pipeline`]).
//!
//! **The `run_conversion(input, output, direction)` stub Phase 3 built this
//! module around does not survive contact with Phase 5.** Its own docs
//! said the signature "is stable across that change" — that turned out to
//! be wrong: [`mystquarto_core::LabelRegistry`] is run-scoped (RT-08), so it
//! must see every document in a conversion set *before* any single one is
//! written, which a genuinely per-file function cannot provide. This module
//! now calls [`mystquarto_core::pipeline::convert_myst_to_quarto_batch`] /
//! [`mystquarto_core::pipeline::convert_quarto_to_myst_batch`] once per
//! directory-mode run, then the existing per-file loop below only writes
//! bytes and applies the (unchanged, still fully tested) `--in-place`/
//! `--dry-run` contract.
//!
//! ### The `--in-place` contract, precisely
//!
//! 1. **Delete-only-after-success.** A source content file is removed only
//!    after its batch-rendered output has been written and renamed
//!    successfully.
//! 2. **Config overwrite gate.** An existing hand-authored `myst.yml`/
//!    `_quarto.yml` at the computed output location is never overwritten
//!    without `--force`; instead the CLI reports a conflict, naming the path
//!    it would have written the real conversion to (`<name>.new`) without
//!    actually writing it.
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
//! 5. **Notebook relabelling and the sidecar write are refused whenever
//!    the output writes into the input tree** (`-o` aimed at or inside the
//!    input directory, whether or not `--in-place` is set — H1 fix), unless
//!    `--force` (Phase 5 spec's Risk Assessment: relabelling patches a file
//!    the user may consider a source input, not build output) — skipped,
//!    with a warning, rather than silently mutating a file in place.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use mystquarto_core::diagnostics::{codes, Diagnostic, Severity};
use mystquarto_core::fs::assets::{self, AssetCopyReport};
use mystquarto_core::fs::atomic::write_atomic;
use mystquarto_core::fs::path_guard;
use mystquarto_core::pipeline::{self, BatchFileError};
use mystquarto_core::preserve::{self, PreservedEntry};
use mystquarto_core::{notebook, registry::sidecar};

use crate::args::{ConvertArgs, StrictLevel};
use crate::discover::{self, Direction};

/// Relative path (from an output root) of the label/preservation sidecar
/// directory.
const SIDECAR_DIR: &str = ".mystquarto";
const LABELS_FILE: &str = "labels.json";
const PRESERVED_CONFIG_FILE: &str = "preserved.json";
const DOI_REFERENCES_FILE: &str = "doi-references.bib";
/// A user-authored allowlist of diagnostic codes to drop from the report
/// entirely (reference §11: "a `.mystquarto/suppress.toml` can baseline
/// specific codes") — read from the *input* tree, since it documents the
/// project's own accepted exceptions, not a per-run flag.
const SUPPRESS_FILE: &str = "suppress.toml";

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
    /// the source was removed.
    Converted,
    /// `--dry-run` was set: this is what would happen. Nothing was written.
    WouldConvert,
    /// Conversion was attempted and failed — the message is the batch
    /// pipeline's own per-file error (a read/parse failure — see
    /// `mystquarto_core::pipeline::BatchFileError`) or, for a file the
    /// batch never got to, an I/O error at write time.
    Failed(String),
}

/// The full result of one [`execute`] call.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub effective_output_root: PathBuf,
    pub outcomes: Vec<FileOutcome>,
    pub assets: Option<AssetCopyReport>,
    /// Set if a hand-authored config file existed at the computed output
    /// location and `--force` was not passed — the path this run *would*
    /// have written the real conversion to, reported so the user can
    /// inspect it manually. Never actually written (the existing file is
    /// left untouched instead); see `alongside_new_path`'s call site.
    pub config_conflict: Option<PathBuf>,
    /// Non-fatal diagnostics from the batch pipeline (label collisions, an
    /// unreadable notebook, a stale/malformed sidecar, a preserved
    /// construct, …) plus this module's own (e.g. "notebook relabelling
    /// skipped under --in-place") — reference §11's `Diagnostic`, with real
    /// severities and stable codes. A `.mystquarto/suppress.toml` entry has
    /// already dropped any suppressed code by the time this is populated —
    /// see [`execute`].
    pub warnings: Vec<Diagnostic>,
}

impl RunReport {
    /// Number of files actually converted.
    #[must_use]
    pub fn converted_count(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.status == FileStatus::Converted)
            .count()
    }

    /// `true` if this run should exit non-zero regardless of `--strict`:
    /// any real (non-dry-run) file failure, or an unresolved config
    /// conflict. Severity::Error-class diagnostics (see
    /// `mystquarto_core::diagnostics::Severity`) already flow through this
    /// path today via `FileStatus::Failed`, not a constructed `Diagnostic`
    /// — see that type's docs.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.config_conflict.is_some()
            || self
                .outcomes
                .iter()
                .any(|o| matches!(o.status, FileStatus::Failed(_)))
    }

    /// `true` if `strict` promotes at least one diagnostic already in
    /// `self.warnings` to a failure: bare `--strict` (`Warn`) promotes
    /// `Severity::Warning` and above; `--strict=all` (`All`) also promotes
    /// `Severity::LossyExpected`.
    #[must_use]
    pub fn has_promoted_diagnostics(&self, strict: Option<StrictLevel>) -> bool {
        let Some(level) = strict else { return false };
        let threshold = match level {
            StrictLevel::Warn => Severity::Warning,
            StrictLevel::All => Severity::LossyExpected,
        };
        self.warnings.iter().any(|d| d.severity >= threshold)
    }
}

fn batch_error_message(errors: &[BatchFileError], file: &Path) -> Option<String> {
    errors
        .iter()
        .find(|e| e.file == file)
        .map(|e| e.message.clone())
}

/// Runs one conversion invocation end to end: resolves paths, applies the
/// `--in-place`/`--dry-run`/`--force` contract, discovers files, and runs
/// the batch conversion pipeline.
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

    let mut report = if is_dir {
        execute_directory(args, direction, &canonical_input, &effective_output_root)?
    } else {
        execute_single_file(args, direction, &canonical_input, &effective_output_root)?
    };

    let suppressed = load_suppressed_codes(&canonical_input.join(SIDECAR_DIR).join(SUPPRESS_FILE));
    if !suppressed.is_empty() {
        report.warnings.retain(|d| !suppressed.contains(d.code));
    }

    Ok(report)
}

/// Reads `.mystquarto/suppress.toml`'s `codes = ["MQ0410", ...]` array, as
/// untrusted input like every other sidecar file in this crate: missing or
/// unparseable degrades to "nothing suppressed," never a hard error. A
/// hand-rolled parser for exactly this one shape (a single top-level
/// string-array key) rather than a `toml` crate dependency — the format
/// this file needs has no case a general TOML parser would handle that
/// this does not.
fn load_suppressed_codes(path: &Path) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let Ok(text) = fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some(rest) = line.strip_prefix("codes") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim().trim_start_matches('[').trim_end_matches(']');
        for part in rest.split(',') {
            let code = part.trim().trim_matches('"').trim_matches('\'').trim();
            if !code.is_empty() {
                out.insert(code.to_string());
            }
        }
    }
    out
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
    let mut warnings: Vec<Diagnostic> = Vec::new();

    // Batch-convert every content file up front (Phase 5's run-scoped
    // `LabelRegistry` requires seeing every document before deciding any
    // single one's ids — see this module's docs). `None` for a dry run or
    // `--config-only`: nothing will be written, so there is no reason to
    // read and convert content at all.
    let batch: Option<ContentBatch> = if args.dry_run || args.config_only {
        None
    } else {
        Some(run_content_batch(
            &content_files,
            canonical_input,
            effective_output_root,
            direction,
        ))
    };
    if let Some(b) = &batch {
        warnings.extend(b.warnings_ref().iter().cloned());
        refuse_if_in_place_would_lose_preserved_content(
            args,
            canonical_input,
            effective_output_root,
            b.preserved_entries_ref(),
        )?;
    }

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
                         would write {} instead",
                        out_config_path.display(),
                        conflict_path.display()
                    )),
                });
                config_conflict = Some(conflict_path);
                continue;
            }

            match convert_config_file(
                config_path,
                direction,
                canonical_input,
                effective_output_root,
                &batch,
            ) {
                Ok((text, config_warnings)) => {
                    write_atomic(&out_config_path, text.as_bytes()).with_context(|| {
                        format!("could not write {}", out_config_path.display())
                    })?;
                    outcomes.push(FileOutcome {
                        input: config_path.clone(),
                        output: out_config_path,
                        status: FileStatus::Converted,
                    });
                    warnings.extend(config_warnings);
                }
                Err(e) => {
                    outcomes.push(FileOutcome {
                        input: config_path.clone(),
                        output: out_config_path,
                        status: FileStatus::Failed(e.to_string()),
                    });
                }
            }
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

            let batch = batch
                .as_ref()
                .expect("batch is Some whenever dry_run/config_only are false");
            match batch.rendered().get(content_path) {
                Some(text) => {
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!("could not create output directory {}", parent.display())
                        })?;
                    }
                    write_atomic(&out_path, text.as_bytes())
                        .with_context(|| format!("could not write {}", out_path.display()))?;
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
                None => {
                    let message = batch_error_message(batch.errors_ref(), content_path)
                        .unwrap_or_else(|| "conversion failed for an unknown reason".to_string());
                    outcomes.push(FileOutcome {
                        input: content_path.clone(),
                        output: out_path,
                        status: FileStatus::Failed(message),
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
            let report = assets::copy_assets(
                canonical_input,
                effective_output_root,
                &content_ext,
                &config_names,
            )
            .context("asset copy failed")?;
            for path in &report.skipped_symlinks {
                warnings.push(
                    Diagnostic::new(
                        Severity::Warning,
                        codes::io::SYMLINK_ASSET_SKIPPED,
                        "asset symlink skipped without dereferencing its target".to_string(),
                    )
                    .with_file(path.clone()),
                );
            }
            assets_report = Some(report);
        }

        if !args.dry_run {
            if let Some(ContentBatch::MystToQuarto {
                notebook_renames,
                sidecar,
                ..
            }) = &batch
            {
                relabel_and_write_sidecar(
                    args,
                    canonical_input,
                    effective_output_root,
                    notebook_renames,
                    sidecar,
                    &mut warnings,
                )?;
            }
            if let Some(b) = &batch {
                write_preserved_content_sidecar(
                    b.preserved_entries_ref(),
                    args,
                    canonical_input,
                    effective_output_root,
                    &mut warnings,
                )?;
            }
        }
    }

    Ok(RunReport {
        effective_output_root: effective_output_root.to_path_buf(),
        outcomes,
        assets: assets_report,
        config_conflict,
        warnings,
    })
}

/// One direction's batch-conversion result, kept as an enum (rather than two
/// separate `Option` fields) so a caller cannot accidentally read
/// `notebook_renames`/`sidecar` — concepts that only exist for
/// MyST->Quarto — while actually holding a Quarto->MyST result.
enum ContentBatch {
    MystToQuarto {
        rendered: std::collections::BTreeMap<PathBuf, String>,
        errors: Vec<BatchFileError>,
        notebook_renames:
            std::collections::BTreeMap<PathBuf, std::collections::BTreeMap<String, String>>,
        sidecar: mystquarto_core::registry::sidecar::LabelSidecar,
        used_citation_keys: std::collections::BTreeSet<String>,
        preserved_entries: std::collections::BTreeMap<String, PreservedEntry>,
        warnings: Vec<Diagnostic>,
    },
    QuartoToMyst {
        rendered: std::collections::BTreeMap<PathBuf, String>,
        errors: Vec<BatchFileError>,
        preserved_entries: std::collections::BTreeMap<String, PreservedEntry>,
        warnings: Vec<Diagnostic>,
    },
}

impl ContentBatch {
    fn rendered(&self) -> &std::collections::BTreeMap<PathBuf, String> {
        match self {
            ContentBatch::MystToQuarto { rendered, .. }
            | ContentBatch::QuartoToMyst { rendered, .. } => rendered,
        }
    }
    fn errors_ref(&self) -> &[BatchFileError] {
        match self {
            ContentBatch::MystToQuarto { errors, .. }
            | ContentBatch::QuartoToMyst { errors, .. } => errors,
        }
    }
    fn warnings_ref(&self) -> &[Diagnostic] {
        match self {
            ContentBatch::MystToQuarto { warnings, .. }
            | ContentBatch::QuartoToMyst { warnings, .. } => warnings,
        }
    }
    fn preserved_entries_ref(&self) -> &std::collections::BTreeMap<String, PreservedEntry> {
        match self {
            ContentBatch::MystToQuarto {
                preserved_entries, ..
            }
            | ContentBatch::QuartoToMyst {
                preserved_entries, ..
            } => preserved_entries,
        }
    }
}

/// Discovers notebooks and runs the direction-appropriate batch converter
/// (`mystquarto_core::pipeline`). The content-file loop only ever needs
/// `.rendered()`/`.errors_ref()` (uniform across both directions); it
/// matches on the full enum only once, after the loop, to reach
/// MyST->Quarto's notebook-relabelling and sidecar data — see
/// [`relabel_and_write_sidecar`]'s call site.
fn run_content_batch(
    content_files: &[PathBuf],
    canonical_input: &Path,
    effective_output_root: &Path,
    direction: Direction,
) -> ContentBatch {
    let notebooks = discover::discover_notebooks(canonical_input, Some(effective_output_root));

    match direction {
        Direction::MystToQuarto => {
            let result =
                pipeline::convert_myst_to_quarto_batch(content_files, &notebooks, canonical_input);
            ContentBatch::MystToQuarto {
                rendered: result.rendered,
                errors: result.errors,
                notebook_renames: result.notebook_renames,
                sidecar: result.sidecar,
                used_citation_keys: result.used_citation_keys,
                preserved_entries: result.preserved_entries,
                warnings: result.warnings,
            }
        }
        Direction::QuartoToMyst => {
            let sidecar_path = canonical_input.join(SIDECAR_DIR).join(LABELS_FILE);
            let sidecar_path = sidecar_path.exists().then_some(sidecar_path);
            let result = pipeline::convert_quarto_to_myst_batch(
                content_files,
                &notebooks,
                canonical_input,
                sidecar_path.as_deref(),
            );
            ContentBatch::QuartoToMyst {
                rendered: result.rendered,
                errors: result.errors,
                preserved_entries: result.preserved_entries,
                warnings: result.warnings,
            }
        }
    }
}

/// **C1 fix.** Refuses the whole run *before any file is written or
/// deleted* when this batch produced preserved block content, the
/// preservation sidecar cannot be written (output writes into the input
/// tree, no `--force`), and the run is `--in-place` — the one combination
/// where [`write_preserved_content_sidecar`]'s ordinary "warn and skip"
/// degrade is not safe: `--in-place` deletes each source file immediately
/// after its output is written (see this module's docs, rule 1), so a
/// preserved construct whose *only* record is the sidecar we just declined
/// to write would be gone from the working tree entirely, with no warning
/// strong enough to stop that from happening before it does. Every other
/// mode (`-o` elsewhere, even one that happens to alias the input root)
/// never deletes a source file, so the existing warn-and-skip in
/// [`write_preserved_content_sidecar`] is sufficient there — this check
/// exists only to run *before* that deletion, not to duplicate its policy.
fn refuse_if_in_place_would_lose_preserved_content(
    args: &ConvertArgs,
    canonical_input: &Path,
    effective_output_root: &Path,
    entries: &std::collections::BTreeMap<String, PreservedEntry>,
) -> Result<()> {
    if !args.in_place || args.force || entries.is_empty() {
        return Ok(());
    }
    // `--in-place` means `effective_output_root == canonical_input`, so
    // this is always true here — computed explicitly anyway so the
    // condition reads the same way as every other sidecar-refusal gate in
    // this module, rather than relying on that equivalence silently.
    let writes_into_source = path_guard::is_descendant(canonical_input, effective_output_root);
    if writes_into_source {
        bail!(
            "--in-place refuses to run: {} construct(s) in this conversion set have no target \
             in the other dialect and would be recorded only in .mystquarto/preserved.json, \
             but that sidecar cannot be written without --force. Proceeding would delete each \
             source file immediately after conversion, permanently losing this content. Pass \
             --force to write the sidecar and proceed.",
            entries.len()
        );
    }
    Ok(())
}

/// Merge-writes `entries` (this run's block-preservation sidecar data — see
/// `mystquarto_core::preserve`) at the output root, under the same
/// output-writes-into-source-tree refusal gate as
/// [`relabel_and_write_sidecar`]'s label sidecar (same Risk Assessment
/// rationale: it mutates a sidecar file, not build output, so it must not
/// silently patch the input tree without `--force`). Safe to warn-and-skip
/// here (rather than fail) in every mode except `--in-place`, which
/// [`refuse_if_in_place_would_lose_preserved_content`] already refused
/// earlier, before any source file could have been deleted. A no-op — no
/// warning — when `entries` is empty, unlike the config-field sidecar
/// (`mystquarto_core::config::sidecar`, always written to clear stale
/// fields): this is additive per-run data, not a fixed field set, so an
/// empty run genuinely has nothing new to record.
fn write_preserved_content_sidecar(
    entries: &std::collections::BTreeMap<String, PreservedEntry>,
    args: &ConvertArgs,
    canonical_input: &Path,
    effective_output_root: &Path,
    warnings: &mut Vec<Diagnostic>,
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let writes_into_source = path_guard::is_descendant(canonical_input, effective_output_root);
    if writes_into_source && !args.force {
        warnings.push(Diagnostic::new(
            Severity::Warning,
            codes::block::PRESERVATION_SIDECAR_NOT_WRITTEN,
            "preservation sidecar not written because the output writes into the input tree \
             (pass --force to write it anyway); a preserved construct's marker will not \
             resolve on a later reverse conversion"
                .to_string(),
        ));
        return Ok(());
    }
    let path = effective_output_root
        .join(SIDECAR_DIR)
        .join(PRESERVED_CONFIG_FILE);
    preserve::write_entries(entries, &path)
        .with_context(|| format!("could not write preservation sidecar {}", path.display()))?;
    Ok(())
}

/// Converts one config file (`myst.yml`/`_quarto.yml`) and returns its
/// rendered text plus any non-fatal notices — bibliography synthesis and the
/// RT-14 missing-citation diagnostic for `MystToQuarto`, or the restored
/// `.mystquarto/preserved.json` fields for `QuartoToMyst`.
///
/// The RT-14 citation-key check only runs when `batch` actually holds a
/// `MystToQuarto` result (i.e. content was parsed this run — not under
/// `--dry-run`/`--config-only`, where nothing was read to check citations
/// against); bibliography *synthesis* itself only needs a `.bib` file to
/// exist, so it still runs under `--config-only`.
///
/// # Errors
/// Returns an error if `config_path` cannot be read, is not valid YAML, or
/// the sidecar write (`MystToQuarto`) fails.
fn convert_config_file(
    config_path: &Path,
    direction: Direction,
    canonical_input: &Path,
    effective_output_root: &Path,
    batch: &Option<ContentBatch>,
) -> Result<(String, Vec<Diagnostic>)> {
    let text = fs::read_to_string(config_path)
        .with_context(|| format!("could not read {}", config_path.display()))?;
    let with_file = |ds: Vec<Diagnostic>| -> Vec<Diagnostic> {
        ds.into_iter()
            .map(|d| d.with_file(config_path.to_path_buf()))
            .collect()
    };

    match direction {
        Direction::MystToQuarto => {
            let bib_path = find_bib_file(canonical_input);
            let mut result =
                mystquarto_core::config::myst_to_quarto::convert(&text, bib_path.as_deref())
                    .map_err(|e| anyhow::anyhow!("{}: {e}", config_path.display()))?;

            let mut warnings: Vec<Diagnostic> = with_file(result.warnings);

            // Written unconditionally, even when empty: this sidecar is the
            // authoritative recovery channel (RT-11), so a run that removes
            // every previously-unmappable field from myst.yml must clear the
            // stale sidecar too, not leave it holding fields the source no
            // longer has.
            let sidecar_path = effective_output_root
                .join(SIDECAR_DIR)
                .join(PRESERVED_CONFIG_FILE);
            mystquarto_core::config::sidecar::write(&result.preserved_fields, &sidecar_path)
                .with_context(|| format!("could not write {}", sidecar_path.display()))?;

            if let Some(ContentBatch::MystToQuarto {
                used_citation_keys, ..
            }) = batch
            {
                let defined = if let Some(bib_rel) = &bib_path {
                    if let Ok(bib_text) = fs::read_to_string(canonical_input.join(bib_rel)) {
                        mystquarto_core::config::bibliography::bib_defined_keys(&bib_text)
                    } else {
                        std::collections::BTreeSet::new()
                    }
                } else {
                    std::collections::BTreeSet::new()
                };

                let missing: std::collections::BTreeSet<String> =
                    used_citation_keys.difference(&defined).cloned().collect();
                let generated_doi_refs: std::collections::BTreeSet<String> =
                    missing.into_iter().filter(|key| is_doi_key(key)).collect();

                if !generated_doi_refs.is_empty() {
                    let rel = format!("{SIDECAR_DIR}/{DOI_REFERENCES_FILE}");
                    let path = effective_output_root.join(&rel);
                    write_doi_bibliography_supplement(&generated_doi_refs, &path)
                        .with_context(|| format!("could not write {}", path.display()))?;
                    result.text = add_bibliography_path(&result.text, &rel);
                    warnings.push(Diagnostic::new(
                        Severity::Info,
                        codes::bibliography::BIBLIOGRAPHY_SYNTHESIZED,
                        format!(
                            "generated {rel} with {} DOI citation fallback entrie(s) so \
                             Quarto can resolve citations without MyST's live DOI lookup",
                            generated_doi_refs.len()
                        ),
                    ));
                }

                let defined_or_generated = defined.union(&generated_doi_refs).cloned().collect();
                warnings.extend(with_file(
                    mystquarto_core::config::bibliography::missing_citation_warnings(
                        used_citation_keys,
                        &defined_or_generated,
                    ),
                ));
            }

            Ok((result.text, warnings))
        }
        Direction::QuartoToMyst => {
            let sidecar_path = canonical_input
                .join(SIDECAR_DIR)
                .join(PRESERVED_CONFIG_FILE);
            let preserved = mystquarto_core::config::sidecar::read(&sidecar_path).map(|p| {
                p.fields
                    .iter()
                    .map(|(k, v)| (k.clone(), mystquarto_core::config::sidecar::json_to_yaml(v)))
                    .collect::<std::collections::BTreeMap<_, _>>()
            });
            let result =
                mystquarto_core::config::quarto_to_myst::convert(&text, preserved.as_ref())
                    .map_err(|e| anyhow::anyhow!("{}: {e}", config_path.display()))?;
            Ok((result.text, with_file(result.warnings)))
        }
    }
}

fn is_doi_key(key: &str) -> bool {
    key.starts_with("10.")
        && key.contains('/')
        && !key
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '{' | '}' | ',' | '"' | '%' | '\\' | '#'))
}

fn write_doi_bibliography_supplement(
    keys: &std::collections::BTreeSet<String>,
    path: &Path,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }

    let mut text = String::new();
    for key in keys {
        text.push_str(&format!(
            "@misc{{{key},\n  doi = {{{key}}},\n  url = {{https://doi.org/{key}}},\n  title = {{DOI reference {key}}},\n}}\n\n"
        ));
    }
    write_atomic(path, text.as_bytes())?;
    Ok(())
}

fn add_bibliography_path(text: &str, extra_path: &str) -> String {
    let mut out = Vec::new();
    let mut inserted = false;
    let mut in_bib_seq = false;
    for line in text.lines() {
        if let Some(existing) = line.strip_prefix("bibliography: ") {
            let trimmed = existing.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let inner = &trimmed[1..trimmed.len() - 1];
                let mut items: Vec<&str> = inner
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                items.push(extra_path);
                out.push(format!("bibliography: [{}]", items.join(", ")));
            } else {
                out.push("bibliography:".to_string());
                out.push(format!("  - {trimmed}"));
                out.push(format!("  - {extra_path}"));
            }
            inserted = true;
        } else if line.trim() == "bibliography:" {
            out.push(line.to_string());
            in_bib_seq = true;
        } else if in_bib_seq {
            if line.starts_with("  - ") || line.starts_with("    - ") {
                out.push(line.to_string());
            } else {
                out.push(format!("  - {extra_path}"));
                out.push(line.to_string());
                inserted = true;
                in_bib_seq = false;
            }
        } else {
            out.push(line.to_string());
        }
    }
    if in_bib_seq && !inserted {
        out.push(format!("  - {extra_path}"));
        inserted = true;
    }
    if !inserted {
        out.push("bibliography:".to_string());
        out.push(format!("  - {extra_path}"));
    }

    let mut rendered = out.join("\n");
    if text.ends_with('\n') || !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

/// Finds a `.bib` file directly inside `dir` (the conventional location for
/// `references.bib` alongside `myst.yml`) for RT-14's bibliography synthesis
/// and citation-key diagnostic — not a recursive search: a `.bib` anywhere
/// deeper in the tree is not what MyST's own auto-load convention looks for
/// either. Returns its file name (not a full path) since that is exactly
/// what a synthesized `bibliography:` field should contain — a project-
/// relative reference, not a local-filesystem path. Ties break
/// alphabetically for determinism if more than one `.bib` exists.
fn find_bib_file(dir: &Path) -> Option<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|name| name.to_ascii_lowercase().ends_with(".bib"))
        .collect();
    names.sort();
    names.into_iter().next()
}

/// Relabels every notebook `notebook_renames` names (against its
/// **output-tree** copy, already placed there by the asset copy that runs
/// immediately before this is called) and merge-writes the label sidecar,
/// unless `--no-label-map`.
///
/// Skips relabelling entirely — and skips writing the sidecar into a source
/// tree — whenever `effective_output_root` **is or is inside**
/// `canonical_input`, unless `--force` (Phase 5 spec's Risk Assessment:
/// relabelling patches a file the user may consider a source input).
///
/// H1 fix: the original gate keyed only off `args.in_place`, so
/// `myst2quarto . -o .` (or any `-o` aimed back at the input tree without
/// `--in-place`) bypassed it entirely — every other in-place safety gate
/// (the VCS check, the config-overwrite gate) is skipped too when
/// `in_place` is false, but this is the one gate that must not be, because
/// relabelling mutates a file regardless of which flag caused the output
/// root to coincide with the input root. Checking the *actual* path
/// relationship, not the flag that usually (but not always) causes it,
/// closes that gap.
fn relabel_and_write_sidecar(
    args: &ConvertArgs,
    canonical_input: &Path,
    effective_output_root: &Path,
    notebook_renames: &std::collections::BTreeMap<
        PathBuf,
        std::collections::BTreeMap<String, String>,
    >,
    sidecar_data: &mystquarto_core::registry::sidecar::LabelSidecar,
    warnings: &mut Vec<Diagnostic>,
) -> Result<()> {
    let writes_into_source = path_guard::is_descendant(canonical_input, effective_output_root);
    let refuse_relabel = writes_into_source && !args.force;

    for (notebook_rel, renames) in notebook_renames {
        if refuse_relabel {
            warnings.push(
                Diagnostic::new(
                    Severity::Warning,
                    codes::label::RELABEL_SKIPPED_OUTPUT_IN_INPUT_TREE,
                    "notebook relabelling skipped because the output writes into the input \
                     tree (pass --force to allow it); embed(s) referencing it may not resolve"
                        .to_string(),
                )
                .with_file(notebook_rel.clone()),
            );
            continue;
        }
        let notebook_path = effective_output_root.join(notebook_rel);
        let Ok(text) = fs::read_to_string(&notebook_path) else {
            warnings.push(
                Diagnostic::new(
                    Severity::Warning,
                    codes::io::NOTEBOOK_RELABEL_FAILED,
                    "could not read notebook to relabel it (was it copied?)".to_string(),
                )
                .with_file(notebook_path.clone()),
            );
            continue;
        };
        match notebook::relabel(&text, renames) {
            Ok(relabelled) => {
                write_atomic(&notebook_path, relabelled.as_bytes()).with_context(|| {
                    format!("could not write relabelled {}", notebook_path.display())
                })?;
            }
            Err(e) => warnings.push(
                Diagnostic::new(
                    Severity::Warning,
                    codes::io::NOTEBOOK_RELABEL_FAILED,
                    format!("could not relabel notebook: {e}"),
                )
                .with_file(notebook_path.clone()),
            ),
        }
    }

    if !args.no_label_map {
        let sidecar_path = effective_output_root.join(SIDECAR_DIR).join(LABELS_FILE);
        if refuse_relabel {
            // Writing a sidecar into the source tree is the exact case
            // `--no-label-map` exists for (see that flag's doc) — but we do
            // not force the user to pass it too; skip silently is wrong
            // (nothing-is-destroyed), so warn instead.
            warnings.push(
                Diagnostic::new(
                    Severity::Warning,
                    codes::label::RELABEL_SKIPPED_OUTPUT_IN_INPUT_TREE,
                    "label sidecar not written because the output writes into the input tree \
                     (pass --no-label-map to suppress this notice, or --force to write it \
                     anyway)"
                        .to_string(),
                )
                .with_file(sidecar_path.clone()),
            );
        } else {
            sidecar::write_merged(sidecar_data, &sidecar_path).with_context(|| {
                format!("could not write label sidecar {}", sidecar_path.display())
            })?;
        }
    }

    Ok(())
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
            warnings: Vec::new(),
        });
    }

    fs::create_dir_all(effective_output_root).with_context(|| {
        format!(
            "could not create output directory {}",
            effective_output_root.display()
        )
    })?;

    // A lone file has no project root of its own; its parent directory is
    // the natural scope for include/notebook resolution, matching how a
    // relative `{include}`/`#nb:` target in the file would already be
    // resolved on disk.
    let input_root = canonical_input_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let batch = run_content_batch(
        std::slice::from_ref(&canonical_input_file.to_path_buf()),
        &input_root,
        effective_output_root,
        direction,
    );
    let mut warnings: Vec<Diagnostic> = batch.warnings_ref().to_vec();
    refuse_if_in_place_would_lose_preserved_content(
        args,
        &input_root,
        effective_output_root,
        batch.preserved_entries_ref(),
    )?;

    let status = match batch.rendered().get(canonical_input_file) {
        Some(text) => {
            write_atomic(&out_path, text.as_bytes())
                .with_context(|| format!("could not write {}", out_path.display()))?;
            if args.in_place && canonical_input_file != out_path && canonical_input_file.exists() {
                fs::remove_file(canonical_input_file).with_context(|| {
                    format!("could not remove source {}", canonical_input_file.display())
                })?;
            }
            if let ContentBatch::MystToQuarto {
                notebook_renames,
                sidecar,
                ..
            } = &batch
            {
                relabel_and_write_sidecar(
                    args,
                    &input_root,
                    effective_output_root,
                    notebook_renames,
                    sidecar,
                    &mut warnings,
                )?;
            }
            write_preserved_content_sidecar(
                batch.preserved_entries_ref(),
                args,
                &input_root,
                effective_output_root,
                &mut warnings,
            )?;
            FileStatus::Converted
        }
        None => {
            let message = batch_error_message(batch.errors_ref(), canonical_input_file)
                .unwrap_or_else(|| "conversion failed for an unknown reason".to_string());
            FileStatus::Failed(message)
        }
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
        warnings,
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
/// should use: `0` for a `--dry-run` invocation regardless of what it would
/// have done, otherwise `0` unless [`RunReport::has_failures`] or `strict`
/// promotes at least one diagnostic (reference §11, RD-4).
///
/// `Info`-severity diagnostics are counted but not individually printed —
/// they document routine, always-correct behavior (a label normalized, a
/// sidecar absent on a reverse conversion with none expected), and printing
/// one per label would drown out the warnings/lossy lines that matter. Every
/// count still prints, including a `0` for a class with nothing to report —
/// "a visible `0 warnings` is the signal that silence was earned."
pub fn print_summary(report: &RunReport, dry_run: bool, strict: Option<StrictLevel>) -> i32 {
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

    // Deterministic order (reference §11's requirement): (file, line), with
    // no-file diagnostics (a run-scoped sidecar notice) sorting first.
    let mut printable: Vec<&Diagnostic> = report
        .warnings
        .iter()
        .filter(|d| d.severity != Severity::Info)
        .collect();
    printable.sort_by(|a, b| (&a.file, a.span.start_line).cmp(&(&b.file, b.span.start_line)));
    for d in &printable {
        let loc = d.file.as_ref().map_or_else(
            || "-".to_string(),
            |f| format!("{}:{}", f.display(), d.span.start_line),
        );
        eprintln!("{loc}: {}[{}]: {}", d.severity.label(), d.code, d.message);
        if let Some(r) = d.reference {
            eprintln!("    see docs/dialect-comparison.md {r}");
        }
        if let Some(id) = &d.preserved {
            eprintln!("    preserved in .mystquarto/preserved.json#{id}");
        }
    }

    let error_count = report
        .outcomes
        .iter()
        .filter(|o| matches!(o.status, FileStatus::Failed(_)))
        .count()
        + usize::from(report.config_conflict.is_some());
    let warning_count = report
        .warnings
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count();
    let lossy_count = report
        .warnings
        .iter()
        .filter(|d| d.severity == Severity::LossyExpected)
        .count();
    eprintln!(
        "Converted {} file(s): {error_count} error(s), {warning_count} warning(s), \
         {lossy_count} expected-lossy.",
        report.converted_count()
    );
    if strict.is_none() {
        eprintln!(
            "Run with --strict to fail on warnings, --strict=all to fail on expected-lossy too."
        );
    }

    i32::from(report.has_failures() || report.has_promoted_diagnostics(strict))
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
    //! [`RunReport`] instead of only observable filesystem side effects.

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
            strict: None,
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
            "every discovered content file should get an outcome, got {:?}",
            report.outcomes
        );
        assert!(content_outcomes
            .iter()
            .all(|o| o.status == FileStatus::Converted));

        cleanup(&tmp);
    }

    #[test]
    fn output_pointed_at_the_input_tree_without_in_place_still_refuses_to_mutate_notebooks() {
        // H1 regression: `-o` aimed back at (or inside) the input tree
        // bypasses the clean-VCS gate and the config-overwrite gate, both
        // of which key off `args.in_place` alone — but notebook
        // relabelling and the sidecar write must *not* follow that same
        // flag-only gate, because they mutate a file regardless of which
        // flag caused the output root to land inside the input tree.
        let tmp = tempdir("output-into-input-no-in-place");
        fs::write(tmp.join("myst.yml"), "project:\n  title: Test\n").unwrap();
        fs::write(
            tmp.join("article.md"),
            ":::{figure} #nb:analysis\n:label: fig:environment\n:::\n",
        )
        .unwrap();
        let notebook_source =
            "{\"cells\":[{\"cell_type\":\"code\",\"source\":[\"#| label: nb:analysis\\n\"]}]}";
        fs::write(tmp.join("analysis.ipynb"), notebook_source).unwrap();

        let mut args = base_args(tmp.clone());
        args.output = Some(tmp.clone()); // -o pointed at the input tree itself
        args.in_place = false; // the flag the original bug keyed off of
        args.no_config = true; // isolate this test to the notebook-relabel gate under test

        let report = execute(&args, Direction::MystToQuarto).unwrap();
        assert!(!report.has_failures(), "{:?}", report.outcomes);

        // The source notebook must be byte-identical — never relabelled.
        assert_eq!(
            fs::read_to_string(tmp.join("analysis.ipynb")).unwrap(),
            notebook_source
        );
        // No sidecar written into the source tree.
        assert!(!tmp.join(".mystquarto").join("labels.json").exists());
        // The skip is surfaced, not silent.
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.message.contains("output writes into the input tree")),
            "{:?}",
            report.warnings
        );

        cleanup(&tmp);
    }

    #[test]
    fn in_place_stops_the_batch_after_the_first_failure() {
        let tmp = tempdir("in-place-stop-on-failure");
        fs::write(tmp.join("myst.yml"), "project:\n  title: Test\n").unwrap();
        // "broken.md" sorts alphabetically before "zzz.md" (discovery is
        // sorted — see `discover.rs`), and is a genuine, real conversion
        // failure: invalid UTF-8, which `std::fs::read_to_string` rejects.
        fs::write(tmp.join("broken.md"), [0xFF, 0xFE, 0x00, 0x01]).unwrap();
        fs::write(tmp.join("zzz.md"), "# Fine\n").unwrap();

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
        assert!(matches!(content_outcomes[0].status, FileStatus::Failed(_)));
        // "zzz.md" was never attempted (rule 4), so it is untouched.
        assert!(tmp.join("zzz.md").exists());
        // M2: "broken.md" itself — the file whose conversion actually
        // failed — must also survive. Delete-only-after-success means a
        // failed conversion never deletes its own source either; this was
        // previously unasserted, the exact rule that mattered most before
        // real conversion existed to fail.
        assert!(
            tmp.join("broken.md").exists(),
            "a failed conversion must not delete its source"
        );

        cleanup(&tmp);
    }

    #[test]
    fn in_place_deletes_the_source_only_after_its_output_is_written() {
        // The positive half of the delete-only-after-success rule (M2):
        // proven directly against the filesystem, not just against
        // `intro.qmd` existing (which a stray leftover file could also
        // satisfy).
        let tmp = tempdir("in-place-deletes-after-success");
        write_myst_project(&tmp);
        init_clean_git_repo(&tmp);

        let mut args = base_args(tmp.clone());
        args.in_place = true;
        args.no_config = true; // isolate this test to the delete-only-after-success behavior under test
        let report = execute(&args, Direction::MystToQuarto).unwrap();
        assert!(!report.has_failures(), "{:?}", report.outcomes);

        assert!(tmp.join("intro.qmd").exists());
        assert!(
            !tmp.join("intro.md").exists(),
            "a successful in-place conversion must delete its source"
        );
        assert!(tmp.join("methods.qmd").exists());
        assert!(!tmp.join("methods.md").exists());

        cleanup(&tmp);
    }

    #[test]
    fn in_place_refuses_rather_than_lose_preserved_content_without_force() {
        // C1 regression: `--in-place` deletes each source file immediately
        // after its own output is written (rule 1) — combined with the
        // ordinary "warn and skip" degrade the preservation sidecar write
        // uses everywhere else, this used to delete the only copy of an
        // unmappable construct's original source with no recovery path,
        // exit 0. The whole run must now refuse up front instead, before
        // anything is written or deleted.
        let tmp = tempdir("in-place-preserve-refuse");
        fs::write(tmp.join("myst.yml"), "project:\n  title: Test\n").unwrap();
        fs::write(
            tmp.join("a.md"),
            "# H\n\n:::{glossary}\nIRREPLACEABLE\n:::\n",
        )
        .unwrap();
        init_clean_git_repo(&tmp);

        let mut args = base_args(tmp.clone());
        args.in_place = true;
        args.no_config = true;
        let err = execute(&args, Direction::MystToQuarto)
            .expect_err("must refuse rather than risk losing preserved content");
        assert!(format!("{err}").contains("preserved.json"), "{err}");

        // Nothing was touched: the source survives, untouched, with its
        // original content intact.
        assert_eq!(
            fs::read_to_string(tmp.join("a.md")).unwrap(),
            "# H\n\n:::{glossary}\nIRREPLACEABLE\n:::\n"
        );
        assert!(!tmp.join("a.qmd").exists());
        assert!(!tmp.join(".mystquarto").join("preserved.json").exists());

        cleanup(&tmp);
    }

    #[test]
    fn in_place_with_force_writes_the_preservation_sidecar_and_proceeds() {
        let tmp = tempdir("in-place-preserve-force");
        fs::write(tmp.join("myst.yml"), "project:\n  title: Test\n").unwrap();
        fs::write(
            tmp.join("a.md"),
            "# H\n\n:::{glossary}\nIRREPLACEABLE\n:::\n",
        )
        .unwrap();
        init_clean_git_repo(&tmp);

        let mut args = base_args(tmp.clone());
        args.in_place = true;
        args.force = true;
        args.no_config = true;
        let report = execute(&args, Direction::MystToQuarto).unwrap();
        assert!(!report.has_failures(), "{:?}", report.outcomes);

        assert!(
            !tmp.join("a.md").exists(),
            "a successful conversion deletes its source"
        );
        let sidecar = fs::read_to_string(tmp.join(".mystquarto").join("preserved.json")).unwrap();
        assert!(sidecar.contains("IRREPLACEABLE"));

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
        // `.new` is a *reported* conflict path, never an actually written
        // file — see `RunReport::config_conflict`'s docs.
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
        // --force allows the overwrite, and the write is now a real,
        // converted `_quarto.yml` — not the hand-authored placeholder.
        assert_eq!(fs::read_to_string(&existing).unwrap(), "title: Test\n");

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
