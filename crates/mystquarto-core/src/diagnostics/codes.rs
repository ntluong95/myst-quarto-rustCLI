//! Stable diagnostic codes, one per distinct disposition this crate's
//! existing warning call sites already produce (reference §11's six code
//! ranges). `docs/diagnostics.md` documents each with cause and remedy.
//!
//! **Scope note.** `mappings.toml` has 57 `fidelity = "lossy"` rows and 13
//! `fidelity = "unmappable"` rows — far more than the codes below. Most
//! `lossy` rows describe a fixed 1:1 syntax narrowing the writer always
//! performs unconditionally (e.g. `:tags: [hide-input]` -> `code-fold:
//! true`), with no runtime branch that currently distinguishes "this
//! specific row's construct was converted" from ordinary output — adding a
//! diagnostic call to every one of those match arms is real, but separate,
//! work (tracked in `plans/260903-1749-rust-port-dialect-fidelity/phase-07-diagnostics.md`).
//! The codes here cover every place a diagnostic is *actually emitted
//! today*: every existing `BatchWarning`/`ConfigWarning`/`RegistryWarning`/
//! `SidecarWarning` construction site, plus the writer's `Unmappable`/
//! `Preserved` block handling this phase adds real preservation for.

/// MQ01xx — label / cross-reference / notebook-cell identity.
pub mod label {
    /// Two labels normalized to the same id within one conversion set;
    /// disambiguated with a numeric suffix. Severity: Warning.
    pub const COLLISION_DISAMBIGUATED: &str = "MQ0101";
    /// A notebook cell was claimed under one id by an earlier document and
    /// a different id by a later one; the later document's embed will not
    /// resolve. Severity: Warning.
    pub const NOTEBOOK_CELL_CLAIMED_BY_ANOTHER_DOCUMENT: &str = "MQ0102";
    /// The label sidecar (`.mystquarto/labels.json`) was refused outright —
    /// missing, oversized, malformed, wrong version, too many entries, or
    /// generated for the other direction. Severity: Info.
    pub const LABEL_SIDECAR_REFUSED: &str = "MQ0103";
    /// One malformed entry was dropped from an otherwise-valid label
    /// sidecar. Severity: Info.
    pub const LABEL_SIDECAR_ENTRY_DROPPED: &str = "MQ0104";
    /// Notebook relabelling (and/or the label sidecar write) was skipped
    /// because the output writes into the input tree and `--force` was not
    /// passed (`mystquarto::orchestrate`'s H1 gate). Severity: Warning.
    pub const RELABEL_SKIPPED_OUTPUT_IN_INPUT_TREE: &str = "MQ0105";
}

/// MQ02xx — block construct lossy or unmappable.
pub mod block {
    /// A construct has no equivalent in the target dialect; preserved
    /// verbatim (marker in the document, original in the preservation
    /// sidecar). Severity: LossyExpected.
    pub const UNMAPPABLE_PRESERVED: &str = "MQ0201";
    /// A preservation marker was read back and its sidecar entry restored,
    /// but the restored content did not re-parse as a single block (kept as
    /// an opaque preserved block rather than guessed at). Severity:
    /// LossyExpected.
    pub const PRESERVED_RESTORED_OPAQUE: &str = "MQ0202";
    /// A preservation marker was read back but its id was not found in the
    /// sidecar (missing/stale/hand-edited `.mystquarto/preserved.json`);
    /// the original content could not be restored. Severity: Warning (this
    /// is real content loss, not an expected one).
    pub const PRESERVATION_ENTRY_MISSING: &str = "MQ0203";
    /// The block-content preservation sidecar was not written because the
    /// output writes into the input tree and `--force` was not passed —
    /// non-destructively (nothing was deleted; see
    /// `mystquarto::orchestrate::refuse_if_in_place_would_lose_preserved_content`
    /// for the case that *would* lose data, which aborts the run instead
    /// of warning). Severity: Warning.
    pub const PRESERVATION_SIDECAR_NOT_WRITTEN: &str = "MQ0204";
}

/// MQ03xx — inline construct, citations, bibliography.
pub mod bibliography {
    /// A citation key is used somewhere in the conversion set but defined
    /// in no reachable `.bib` file (RT-14). Severity: Warning.
    pub const CITATION_KEY_MISSING: &str = "MQ0301";
    /// `myst.yml` had no `bibliography:` key but a `.bib` file exists in
    /// the conversion set; one was synthesized. Severity: Info (a helpful
    /// autofix, not a loss).
    pub const BIBLIOGRAPHY_SYNTHESIZED: &str = "MQ0302";
}

/// MQ04xx — config / frontmatter.
pub mod config {
    /// A `myst.yml` field has no `_quarto.yml` equivalent; preserved as a
    /// comment and in `.mystquarto/preserved.json`. Severity: LossyExpected.
    pub const UNMAPPABLE_FIELD_PRESERVED: &str = "MQ0401";
    /// A manuscript's `project.toc` has entries beyond its article and
    /// notebooks (e.g. an appendix); Quarto's manuscript shape has no slot
    /// for them, so they were preserved rather than dropped. Severity:
    /// LossyExpected.
    pub const MANUSCRIPT_TOC_ENTRY_PRESERVED: &str = "MQ0402";
    /// A `_quarto.yml` book's `part:`-grouped chapters were flattened into
    /// a plain myst.yml toc list (the grouping label has no myst.yml
    /// target). Severity: LossyExpected.
    pub const BOOK_PART_GROUPING_FLATTENED: &str = "MQ0403";
    /// A `_quarto.yml` book's `appendices` were appended to the myst.yml
    /// toc as regular entries (myst.yml has no appendix/main-matter
    /// distinction). Severity: LossyExpected.
    pub const BOOK_APPENDICES_FLATTENED: &str = "MQ0404";
    /// `_quarto.yml` `categories` had more than one entry; only the first
    /// was mapped back to myst.yml's single-valued `subject`. Severity:
    /// Warning (the rest are genuinely lost).
    pub const CATEGORIES_NARROWED_TO_SUBJECT: &str = "MQ0405";
    /// An export/format value has no equivalent in the target dialect and
    /// was dropped entirely (not preserved). Severity: Warning.
    pub const EXPORT_FORMAT_DROPPED: &str = "MQ0406";
    /// An export's Quarto format was guessed from a non-portable
    /// `template:` name's suffix. Severity: Warning (the guess may be
    /// wrong).
    pub const EXPORT_FORMAT_GUESSED: &str = "MQ0407";
    /// A `_quarto.yml` `format:` key has no exact myst.yml export
    /// equivalent; passed through as a `format:` value of the same name.
    /// Severity: Warning.
    pub const FORMAT_PASSED_THROUGH: &str = "MQ0408";
    /// A root-level `_quarto.yml` key has no myst.yml equivalent at all
    /// (e.g. `execute:`, `csl:`); dropped. Severity: Warning.
    pub const UNRECOGNIZED_TOP_LEVEL_KEY_DROPPED: &str = "MQ0409";
    /// A page-frontmatter field has no correct target in the other dialect
    /// (`label`, `math`); dropped rather than mismapped. Severity:
    /// LossyExpected.
    pub const FRONTMATTER_FIELD_DROPPED: &str = "MQ0410";
    /// `myst.yml` set both `banner` and `thumbnail`; `_quarto.yml` has one
    /// `image:` slot, so `banner` was used and `thumbnail` silently has no
    /// output otherwise. Severity: Warning.
    pub const BANNER_AND_THUMBNAIL_BOTH_SET: &str = "MQ0411";
}

/// MQ06xx — file, IO, path safety, discovery.
pub mod io {
    /// A notebook in the conversion set could not be read. Severity:
    /// Warning (its cells cannot be indexed, so an embed referencing them
    /// will not resolve; the run still proceeds).
    pub const NOTEBOOK_UNREADABLE: &str = "MQ0601";
    /// A notebook in the conversion set was read but its cells could not be
    /// indexed (malformed JSON). Severity: Warning.
    pub const NOTEBOOK_INDEX_FAILED: &str = "MQ0602";
    /// A notebook's output-tree copy could not be read back to relabel it
    /// (`mystquarto::orchestrate`), or `crate::notebook::relabel` itself
    /// failed. Severity: Warning.
    pub const NOTEBOOK_RELABEL_FAILED: &str = "MQ0603";
    /// An asset path was a symlink and was skipped without dereferencing
    /// the target. Severity: Warning.
    pub const SYMLINK_ASSET_SKIPPED: &str = "MQ0604";
    /// A path-safety check refused an include or embed target (escapes root,
    /// include cycle, depth exceeded, or absolute target). Severity: Warning.
    pub const PATH_SAFETY_REFUSED: &str = "MQ0605";
}
