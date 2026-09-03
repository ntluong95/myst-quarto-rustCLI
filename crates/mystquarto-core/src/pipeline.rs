//! Batch conversion: the four-pass pipeline Phase 5's spec describes as
//! "read -> notebook index -> registry -> write -> relabel -> sidecar",
//! packaged as one call so the CLI orchestration layer (`crate::fs` primitives'
//! caller, `mystquarto`'s `orchestrate.rs`) does not need to re-derive it.
//!
//! **Why a batch, not per-file.** [`crate::LabelRegistry`] is run-scoped
//! (RT-08): it must see every document in a conversion set before deciding
//! any single document's ids, so a collision between two files is caught
//! and suffixed deterministically instead of producing duplicate Quarto
//! ids. That is impossible to get right from a per-file
//! `run_conversion(input, output)` call, so this module's entry points take
//! the whole file set at once and return every file's rendered text
//! together — the caller then only needs to write bytes to disk (and apply
//! whatever `--dry-run`/`--in-place`/atomicity contract it already has;
//! this module performs no I/O writes itself, only reads).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::registry::sidecar::{self, LabelSidecar};
use crate::writer::{
    known_reference_labels, resolve_embed_id, MystWriter, QuartoWriter, RestoreMap,
};
use crate::{
    Block, BlockKind, Document, EmbedTarget, FigureSource, LabelRegistry, MystReader,
    NotebookCellIndex, QuartoReader, ReaderContext, ReaderError,
};

/// Direction-agnostic non-fatal notice from a batch conversion — mirrors
/// [`crate::registry::RegistryWarning`]/[`crate::registry::sidecar::SidecarWarning`]'s
/// shape for the same reason: Phase 7 does not exist yet to assign these
/// real diagnostic codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchWarning {
    pub file: Option<PathBuf>,
    pub message: String,
}

/// A file this batch could not read or parse. Distinct from
/// [`BatchWarning`]: a read/parse failure means that file has no rendered
/// output at all, which the caller (already tracking per-file results) must
/// surface as a hard per-file failure, not a warning attached to output
/// that does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchFileError {
    pub file: PathBuf,
    pub message: String,
}

/// Builds a [`NotebookCellIndex`] over every notebook in `notebooks`
/// (absolute paths). A notebook that fails to parse is recorded as a
/// [`BatchWarning`] and simply contributes no entries — a malformed
/// notebook should not abort conversion of every `.md`/`.qmd` file in the
/// project, only degrade any embed that would have referenced it (which
/// becomes `Unmappable`, per `crate::reader::myst::MystReader::figure`'s
/// existing "not found in the conversion set" handling).
///
/// Indexes every notebook in `notebooks` (absolute paths), keyed in the
/// resulting [`NotebookCellIndex`] by each notebook's path **relative to
/// `input_root`** — not its absolute filesystem path. This matters for two
/// downstream consumers, both of which need a path meaningful in the
/// *output* tree, not this machine's filesystem: the `{{< embed >}}`
/// shortcode `crate::writer::quarto::QuartoWriter::figure` emits (an
/// absolute path there would be both wrong — Quarto resolves it relative to
/// the rendered document — and a local-filesystem leak into published
/// output), and [`collect_notebook_renames`]'s map, which
/// `crate::notebook::relabel` applies against the notebook's copy in the
/// *output* tree (`crate::fs::assets::copy_assets` preserves each asset's
/// path relative to the root, so the same relative path identifies the
/// notebook on both sides).
fn build_notebook_index(
    notebooks: &[PathBuf],
    input_root: &Path,
) -> (NotebookCellIndex, Vec<BatchWarning>) {
    let mut index = NotebookCellIndex::default();
    let mut warnings = Vec::new();
    for path in notebooks {
        let rel = path.strip_prefix(input_root).unwrap_or(path).to_path_buf();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if let Err(e) = index.add_notebook_json(rel, &text) {
                    warnings.push(BatchWarning {
                        file: Some(path.clone()),
                        message: format!("could not index notebook cells: {e}"),
                    });
                }
            }
            Err(e) => warnings.push(BatchWarning {
                file: Some(path.clone()),
                message: format!("could not read notebook: {e}"),
            }),
        }
    }
    (index, warnings)
}

/// Result of [`convert_myst_to_quarto_batch`].
#[derive(Debug, Clone)]
pub struct MystToQuartoBatch {
    /// Input path -> rendered `.qmd` text, for every file that read and
    /// wrote successfully.
    pub rendered: BTreeMap<PathBuf, String>,
    /// Files that failed to read or parse — not present in `rendered`.
    pub errors: Vec<BatchFileError>,
    /// Notebook path -> `{old cell label -> new Quarto id}`, for
    /// `crate::notebook::relabel` to apply to each notebook's *output-tree*
    /// copy (never the source — see that module's docs).
    pub notebook_renames: BTreeMap<PathBuf, BTreeMap<String, String>>,
    /// The label sidecar to merge-write at the output root (unless the
    /// caller's `--no-label-map` is set) — `crate::registry::sidecar::write_merged`.
    pub sidecar: LabelSidecar,
    /// Every citation key referenced anywhere across `documents`, computed
    /// with the batch's own [`known_reference_labels`] so a bare `@token`
    /// that is actually a cross-reference (e.g. `@sec:data-analysis`) is
    /// never misclassified as a citation — RT-14's diagnostic (`crate::config::bibliography`)
    /// needs this list but has no access to `documents` or the registry
    /// itself, both of which live only inside this function.
    pub used_citation_keys: std::collections::BTreeSet<String>,
    pub warnings: Vec<BatchWarning>,
}

/// Converts every file in `files` (absolute `.md` paths) from MyST to
/// Quarto, as one run-scoped batch.
///
/// `files` and `notebooks` must both be absolute paths already resolved
/// through the caller's path guard / discovery step — this function does no
/// containment checking of its own (that is a discovery-time concern, not a
/// conversion-time one).
#[must_use]
pub fn convert_myst_to_quarto_batch(
    files: &[PathBuf],
    notebooks: &[PathBuf],
    input_root: &Path,
) -> MystToQuartoBatch {
    let (notebook_index, mut warnings) = build_notebook_index(notebooks, input_root);

    let mut documents: Vec<(PathBuf, Document)> = Vec::new();
    let mut errors = Vec::new();
    for path in files {
        match read_myst(path, &notebook_index, input_root) {
            Ok(doc) => documents.push((path.clone(), doc)),
            Err(e) => errors.push(BatchFileError {
                file: path.clone(),
                message: e.to_string(),
            }),
        }
    }

    let (registry, registry_warnings) = LabelRegistry::build(&documents);
    warnings.extend(registry_warnings.into_iter().map(|w| BatchWarning {
        file: Some(w.file),
        message: w.message,
    }));

    let writer = QuartoWriter::new(&registry);
    let mut rendered = BTreeMap::new();
    for (path, doc) in &documents {
        let (text, fm_warnings) = writer.write(doc);
        rendered.insert(path.clone(), text);
        warnings.extend(fm_warnings.into_iter().map(|w| BatchWarning {
            file: Some(path.clone()),
            message: w.message,
        }));
    }

    let (notebook_renames, rename_warnings) = collect_notebook_renames(&registry, &documents);
    warnings.extend(rename_warnings);

    let citation_known_labels = known_reference_labels(&registry);
    let mut used_citation_keys = std::collections::BTreeSet::new();
    for (_, doc) in &documents {
        used_citation_keys.extend(crate::config::bibliography::citation_keys_in_document(
            doc,
            &citation_known_labels,
        ));
    }

    // The sidecar's `content_hash` must be of the *rendered Quarto output*,
    // not the MyST source — that is the file a later reverse conversion
    // will actually re-read and hash to check staleness against
    // (`convert_quarto_to_myst_batch` below). Hashing the source instead
    // would compare `.md` bytes against `.qmd` bytes on the way back,
    // which can never match and would make every entry look stale
    // unconditionally.
    let content_hashes: BTreeMap<PathBuf, String> = rendered
        .iter()
        .map(|(path, text)| (path.clone(), sidecar::content_hash(text.as_bytes())))
        .collect();
    let sidecar = sidecar::build(&registry, "myst_to_quarto", input_root, &content_hashes);

    MystToQuartoBatch {
        rendered,
        errors,
        notebook_renames,
        sidecar,
        used_citation_keys,
        warnings,
    }
}

fn read_myst(
    path: &Path,
    notebook_index: &NotebookCellIndex,
    input_root: &Path,
) -> Result<Document, ReaderError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        ReaderError::Yaml(crate::yaml::YamlReadError::Scan(format!(
            "could not read {}: {e}",
            path.display()
        )))
    })?;
    let context = ReaderContext {
        notebook_index: notebook_index.clone(),
        ..ReaderContext::new(path).with_input_root(input_root)
    };
    MystReader::new(context).read_str(&text)
}

/// Walks every document's blocks for a notebook-cell figure or embed,
/// resolving each one's final crossref id the same way
/// [`crate::writer::quarto::QuartoWriter`] did when it rendered the
/// `{{< embed >}}` shortcode (via the same [`resolve_embed_id`]), and
/// groups the resulting `{old cell label -> new id}` renames by notebook
/// path. This is what makes the two sides agree: the writer decided the id
/// text-by-text; this function decides the *same* id block-by-block so the
/// notebook relabelling step (run separately, against the output-tree copy)
/// produces a file whose labels actually match what the writer already
/// emitted.
///
/// A single physical notebook cell can only ever have **one** `#| label:`
/// value, but two different documents can each embed that same cell with
/// their own, different document-level label (H5: `a.md` embeds
/// `#nb:analysis` as `fig:environment`, `b.md` embeds the *same* cell as
/// `fig:setup`) — which the writer, working one document at a time, cannot
/// see coming. This is an unsatisfiable request from the source documents,
/// not something this function can silently resolve correctly: it keeps the
/// **first** requested id (by `documents`' own — already sorted — order,
/// the same "first registered wins" precedent
/// `LabelRegistry::build`'s collision handling sets) and returns a warning
/// for every later, losing document, whose already-rendered `{{< embed >}}`
/// shortcode will not resolve.
fn collect_notebook_renames(
    registry: &LabelRegistry,
    documents: &[(PathBuf, Document)],
) -> (
    BTreeMap<PathBuf, BTreeMap<String, String>>,
    Vec<BatchWarning>,
) {
    let mut out: BTreeMap<PathBuf, BTreeMap<String, String>> = BTreeMap::new();
    let mut warnings = Vec::new();
    for (source, doc) in documents {
        walk_for_renames(registry, source, &doc.blocks, &mut out, &mut warnings);
    }
    (out, warnings)
}

/// `document_label`, passed to [`resolve_embed_id`], must be the label's
/// **normalized Quarto id** (what `registry.quarto_id` returns), not its raw
/// spelling — `QuartoWriter::figure`/`embed` look it up the same way, and
/// this function has to make the identical choice or the notebook gets
/// relabelled to a different id than the one the writer already emitted
/// into the `.qmd` text.
fn walk_for_renames(
    registry: &LabelRegistry,
    source: &Path,
    blocks: &[Block],
    out: &mut BTreeMap<PathBuf, BTreeMap<String, String>>,
    warnings: &mut Vec<BatchWarning>,
) {
    for block in blocks {
        match &block.kind {
            BlockKind::Figure {
                src:
                    FigureSource::CellRef {
                        label: cell_label,
                        notebook: Some(notebook),
                    },
                label,
                ..
            } => {
                let own_id = label.as_ref().and_then(|l| registry.quarto_id(source, l));
                let new_id = resolve_embed_id(cell_label, own_id);
                record_notebook_rename(out, warnings, source, notebook, cell_label, new_id);
            }
            BlockKind::Embed {
                target:
                    EmbedTarget::NotebookCell {
                        notebook,
                        cell_label,
                    },
                label,
            } => {
                let own_id = label.as_ref().and_then(|l| registry.quarto_id(source, l));
                let new_id = resolve_embed_id(cell_label, own_id);
                record_notebook_rename(out, warnings, source, notebook, cell_label, new_id);
            }
            BlockKind::Admonition { body, .. }
            | BlockKind::Margin { body }
            | BlockKind::Blockquote { body, .. }
            | BlockKind::Theorem { body, .. }
            | BlockKind::Directive { body, .. } => {
                walk_for_renames(registry, source, body, out, warnings);
            }
            BlockKind::TabSet { items } => {
                for item in items {
                    walk_for_renames(registry, source, &item.body, out, warnings);
                }
            }
            _ => {}
        }
    }
}

/// Records `cell_label -> new_id` for `notebook`, or — if some earlier
/// document already claimed that exact cell under a *different* id — keeps
/// the earlier claim and warns instead of silently overwriting it. See
/// [`collect_notebook_renames`]'s docs for why this conflict is inherent to
/// the source documents, not something to silently paper over.
fn record_notebook_rename(
    out: &mut BTreeMap<PathBuf, BTreeMap<String, String>>,
    warnings: &mut Vec<BatchWarning>,
    source: &Path,
    notebook: &Path,
    cell_label: &crate::Label,
    new_id: String,
) {
    let cell_map = out.entry(notebook.to_path_buf()).or_default();
    match cell_map.get(&cell_label.raw) {
        Some(existing) if existing != &new_id => {
            warnings.push(BatchWarning {
                file: Some(source.to_path_buf()),
                message: format!(
                    "{}: another document already claimed notebook cell `{}` as \
                     `{existing}`; this document's embed of it as `{new_id}` will not \
                     resolve — a single notebook cell can only have one label",
                    notebook.display(),
                    cell_label.raw,
                ),
            });
        }
        Some(_) => {} // identical request from another document — fine
        None => {
            cell_map.insert(cell_label.raw.clone(), new_id);
        }
    }
}

/// Every label actually defined across `documents`' blocks — used to seed
/// [`crate::writer::MystWriter`]'s `known_labels` for the Quarto->MyST
/// direction (H4 fix): without this, `read_at_reference`
/// (`crate::reader::inline`) has no way to tell a cross-reference token
/// (`@fig-samples`) apart from a citation, so it defaults every `@token` to
/// `Citation` and `MystWriter`'s sidecar-restore logic for inline references
/// never actually runs.
///
/// Deliberately walks the *documents themselves*, not
/// `crate::registry::sidecar::restore_labels`'s output: a sidecar only
/// covers labels known at the time of a *previous* forward conversion, so a
/// Quarto project with no sidecar at all (or one missing a label added
/// since) would otherwise still misclassify its own, perfectly real
/// cross-references as citations.
fn labels_defined_in(documents: &[(PathBuf, Document)]) -> Vec<String> {
    let mut labels = Vec::new();
    for (_, doc) in documents {
        collect_defined_labels(&doc.blocks, &mut labels);
    }
    labels.sort();
    labels.dedup();
    labels
}

fn collect_defined_labels(blocks: &[Block], out: &mut Vec<String>) {
    for block in blocks {
        match &block.kind {
            BlockKind::Heading { label: Some(l), .. }
            | BlockKind::Figure { label: Some(l), .. }
            | BlockKind::Table { label: Some(l), .. }
            | BlockKind::Math { label: Some(l), .. }
            | BlockKind::CodeCell { label: Some(l), .. }
            | BlockKind::Theorem { label: Some(l), .. }
            | BlockKind::Directive { label: Some(l), .. }
            | BlockKind::Target { label: l } => out.push(l.raw.clone()),
            _ => {}
        }
        match &block.kind {
            BlockKind::Admonition { body, .. }
            | BlockKind::Margin { body }
            | BlockKind::Blockquote { body, .. }
            | BlockKind::Theorem { body, .. }
            | BlockKind::Directive { body, .. } => collect_defined_labels(body, out),
            BlockKind::TabSet { items } => {
                for item in items {
                    collect_defined_labels(&item.body, out);
                }
            }
            _ => {}
        }
    }
}

/// Result of [`convert_quarto_to_myst_batch`].
#[derive(Debug, Clone)]
pub struct QuartoToMystBatch {
    pub rendered: BTreeMap<PathBuf, String>,
    pub errors: Vec<BatchFileError>,
    pub warnings: Vec<BatchWarning>,
}

/// Converts every file in `files` (absolute `.qmd` paths) from Quarto to
/// MyST. Unlike the forward direction, this is **not** run-scoped in the
/// normalization sense — see `crate::writer` module docs on why the reverse
/// direction only ever restores-or-passes-through, never re-normalizes —
/// but is still batched for a uniform API and so notebook indexing is done
/// once.
///
/// `sidecar_path`, if given, is read via
/// `crate::registry::sidecar::read_untrusted` (untrusted input — RT-09) and
/// used to restore original MyST-side label spellings; absent or invalid,
/// every label is passed through as its Quarto id unchanged (still valid
/// MyST label text — reference §3.3, "MyST imposes no constraint").
#[must_use]
pub fn convert_quarto_to_myst_batch(
    files: &[PathBuf],
    notebooks: &[PathBuf],
    input_root: &Path,
    sidecar_path: Option<&Path>,
) -> QuartoToMystBatch {
    let (notebook_index, mut warnings) = build_notebook_index(notebooks, input_root);

    let mut documents: Vec<(PathBuf, Document)> = Vec::new();
    let mut errors = Vec::new();
    for path in files {
        match read_quarto(path, &notebook_index, input_root) {
            Ok(doc) => documents.push((path.clone(), doc)),
            Err(e) => errors.push(BatchFileError {
                file: path.clone(),
                message: e.to_string(),
            }),
        }
    }

    // The sidecar keys every entry by the *original MyST-side* file's path
    // relative to `input_root` (e.g. `"a.md"`) — that is what
    // `crate::registry::sidecar::build` recorded during the forward run.
    // The documents this function just read, though, are Quarto-side files
    // at their own *absolute* `.qmd` paths (`Document.source`). Bridging
    // the two needs one assumption this whole tool already makes elsewhere
    // (`crate::discover`'s content-file extension swap): a `.qmd` file's
    // corresponding sidecar entry sits at the same relative path with the
    // extension swapped back to `.md`. `content_hashes_by_myst_key` builds
    // exactly that bridge so both the staleness check inside
    // `restore_labels` and the final lookup key line up with what the
    // sidecar actually recorded.
    let mut content_hashes_by_myst_key = BTreeMap::new();
    let mut quarto_path_by_myst_key: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for path in files {
        let rel = path.strip_prefix(input_root).unwrap_or(path);
        let myst_key = rel.with_extension("md");
        quarto_path_by_myst_key.insert(myst_key.clone(), path.clone());
        if let Ok(bytes) = std::fs::read(path) {
            content_hashes_by_myst_key.insert(myst_key, sidecar::content_hash(&bytes));
        }
    }

    let restore: RestoreMap = match sidecar_path {
        Some(p) => {
            // A useful sidecar was written *by the forward run*
            // (`convert_myst_to_quarto_batch`, the only direction that
            // currently calls `sidecar::write_merged`) — so the direction
            // this reverse run expects to find recorded is
            // `"myst_to_quarto"`, not its own `"quarto_to_myst"`. These are
            // opposite by design: forward writes it, reverse reads it back.
            let (sidecar, sidecar_warnings) = sidecar::read_untrusted(p, "myst_to_quarto");
            warnings.extend(sidecar_warnings.into_iter().map(|w| BatchWarning {
                file: None,
                message: w.message,
            }));
            let by_myst_key = sidecar
                .map(|s| sidecar::restore_labels(&s, &content_hashes_by_myst_key))
                .unwrap_or_default();
            // Remap `(myst-relative-path, id) -> label` to
            // `(quarto-absolute-path, id) -> label`, since that is the key
            // `crate::writer::resolve_myst_label` looks up by
            // (`Document.source`, the id as read).
            by_myst_key
                .into_iter()
                .filter_map(|((myst_key, id), label)| {
                    quarto_path_by_myst_key
                        .get(&myst_key)
                        .map(|quarto_path| ((quarto_path.clone(), id), label))
                })
                .collect()
        }
        None => RestoreMap::new(),
    };

    let known_labels = labels_defined_in(&documents);
    let writer = MystWriter::new(&restore, known_labels);
    let mut rendered = BTreeMap::new();
    for (path, doc) in &documents {
        let (text, fm_warnings) = writer.write(doc);
        rendered.insert(path.clone(), text);
        warnings.extend(fm_warnings.into_iter().map(|w| BatchWarning {
            file: Some(path.clone()),
            message: w.message,
        }));
    }

    QuartoToMystBatch {
        rendered,
        errors,
        warnings,
    }
}

fn read_quarto(
    path: &Path,
    notebook_index: &NotebookCellIndex,
    input_root: &Path,
) -> Result<Document, ReaderError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        ReaderError::Yaml(crate::yaml::YamlReadError::Scan(format!(
            "could not read {}: {e}",
            path.display()
        )))
    })?;
    let context = ReaderContext {
        notebook_index: notebook_index.clone(),
        ..ReaderContext::new(path).with_input_root(input_root)
    };
    QuartoReader::new(context).read_str(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn tempdir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("mystquarto-pipeline-test-{label}-{nanos}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cross_file_label_collision_is_disambiguated_across_the_whole_batch() {
        let tmp = tempdir("collision");
        write(
            &tmp.join("a.md"),
            ":::{figure} x.png\n:label: fig:samples\n:::\n",
        );
        write(
            &tmp.join("b.md"),
            ":::{figure} y.png\n:label: fig:samples\n:::\n",
        );

        let files = vec![tmp.join("a.md"), tmp.join("b.md")];
        let batch = convert_myst_to_quarto_batch(&files, &[], &tmp);

        assert!(batch.errors.is_empty(), "{:?}", batch.errors);
        assert!(batch.rendered[&tmp.join("a.md")].contains("{#fig-samples}"));
        assert!(batch.rendered[&tmp.join("b.md")].contains("{#fig-samples-2}"));
        assert_eq!(batch.warnings.len(), 1);

        cleanup(&tmp);
    }

    #[test]
    fn notebook_rename_map_matches_what_the_writer_emitted() {
        let tmp = tempdir("rename-map");
        write(
            &tmp.join("article.md"),
            ":::{figure} #nb:analysis\n:label: fig:environment\n:::\n",
        );
        write(
            &tmp.join("analysis.ipynb"),
            "{\"cells\":[{\"cell_type\":\"code\",\"source\":[\"#| label: nb:analysis\\n\"]}]}",
        );

        let files = vec![tmp.join("article.md")];
        let notebooks = vec![tmp.join("analysis.ipynb")];
        let batch = convert_myst_to_quarto_batch(&files, &notebooks, &tmp);

        assert!(batch.errors.is_empty(), "{:?}", batch.errors);
        assert!(batch.rendered[&tmp.join("article.md")]
            .contains("{{< embed analysis.ipynb#fig-environment >}}"));
        // Keyed relative to input_root, not absolute — see
        // `build_notebook_index`'s docs on why.
        let renames = &batch.notebook_renames[Path::new("analysis.ipynb")];
        assert_eq!(
            renames.get("nb:analysis"),
            Some(&"fig-environment".to_string())
        );

        cleanup(&tmp);
    }

    #[test]
    fn two_documents_claiming_the_same_notebook_cell_with_different_ids_keeps_the_first_and_warns()
    {
        // H5 regression: a single physical notebook cell can only carry one
        // `#| label:`. Two documents embedding the *same* cell under
        // different document-level labels is unsatisfiable — the original
        // bug silently let the second document's request overwrite the
        // first's in the rename map, so document A's already-rendered
        // `{{< embed >}}` (using A's own id) pointed at a cell that would
        // end up relabelled to B's id instead — a dangling embed with no
        // diagnostic at all.
        let tmp = tempdir("notebook-rename-conflict");
        write(
            &tmp.join("a.md"),
            ":::{figure} #nb:analysis\n:label: fig:environment\n:::\n",
        );
        write(
            &tmp.join("b.md"),
            ":::{figure} #nb:analysis\n:label: fig:setup\n:::\n",
        );
        write(
            &tmp.join("analysis.ipynb"),
            "{\"cells\":[{\"cell_type\":\"code\",\"source\":[\"#| label: nb:analysis\\n\"]}]}",
        );

        let files = vec![tmp.join("a.md"), tmp.join("b.md")];
        let notebooks = vec![tmp.join("analysis.ipynb")];
        let batch = convert_myst_to_quarto_batch(&files, &notebooks, &tmp);

        assert!(batch.errors.is_empty(), "{:?}", batch.errors);
        // a.md sorts first, so it keeps its requested id.
        let renames = &batch.notebook_renames[Path::new("analysis.ipynb")];
        assert_eq!(
            renames.get("nb:analysis"),
            Some(&"fig-environment".to_string())
        );
        // b.md's conflicting request produced a warning, not a silent
        // overwrite.
        assert!(
            batch
                .warnings
                .iter()
                .any(|w| w.message.contains("fig-setup") && w.message.contains("will not resolve")),
            "expected a conflict warning mentioning fig-setup, got {:?}",
            batch.warnings
        );

        cleanup(&tmp);
    }

    #[test]
    fn a_file_that_fails_to_read_is_an_error_not_a_panic_and_does_not_block_others() {
        let tmp = tempdir("read-error");
        write(&tmp.join("ok.md"), "# Fine\n");
        let missing = tmp.join("missing.md");

        let files = vec![tmp.join("ok.md"), missing.clone()];
        let batch = convert_myst_to_quarto_batch(&files, &[], &tmp);

        assert!(batch.rendered.contains_key(&tmp.join("ok.md")));
        assert_eq!(batch.errors.len(), 1);
        assert_eq!(batch.errors[0].file, missing);

        cleanup(&tmp);
    }

    #[test]
    fn quarto_to_myst_restores_original_label_from_a_written_sidecar() {
        let tmp = tempdir("restore");
        write(
            &tmp.join("a.md"),
            ":::{figure} x.png\n:label: fig:samples\n:::\n\nSee @fig:samples.\n",
        );
        let files = vec![tmp.join("a.md")];
        let forward = convert_myst_to_quarto_batch(&files, &[], &tmp);
        let sidecar_path = tmp.join(".mystquarto").join("labels.json");
        sidecar::write_merged(&forward.sidecar, &sidecar_path).unwrap();

        let qmd_path = tmp.join("a.qmd");
        write(&qmd_path, &forward.rendered[&tmp.join("a.md")]);

        let reverse = convert_quarto_to_myst_batch(
            std::slice::from_ref(&qmd_path),
            &[],
            &tmp,
            Some(&sidecar_path),
        );
        assert!(reverse.errors.is_empty(), "{:?}", reverse.errors);
        let out = &reverse.rendered[&qmd_path];
        assert!(out.contains(":label: fig:samples"));
        // H4 regression: the *inline* `@fig-samples` reference must also be
        // restored to `@fig:samples`, not just the block's own label — the
        // original bug left `MystWriter`'s `known_labels` empty, so every
        // `@token` was misclassified as a citation and passed through
        // unchanged, silently skipping this exact restoration.
        assert!(
            out.contains("See @fig:samples."),
            "inline reference was not restored, got:\n{out}"
        );
        assert!(!out.contains("@fig-samples"));

        cleanup(&tmp);
    }

    #[test]
    fn two_identical_runs_produce_identical_bytes() {
        let tmp = tempdir("determinism");
        write(
            &tmp.join("a.md"),
            ":::{figure} x.png\n:label: fig:samples\n:::\n\nSee @fig:samples.\n",
        );
        let files = vec![tmp.join("a.md")];

        let run1 = convert_myst_to_quarto_batch(&files, &[], &tmp);
        let run2 = convert_myst_to_quarto_batch(&files, &[], &tmp);
        assert_eq!(run1.rendered, run2.rendered);
        assert_eq!(
            serde_json::to_string(&run1.sidecar).unwrap(),
            serde_json::to_string(&run2.sidecar).unwrap()
        );

        cleanup(&tmp);
    }
}
