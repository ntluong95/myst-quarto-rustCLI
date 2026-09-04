//! The run-scoped label registry (RT-08): normalizes every label in a
//! conversion set exactly once, before any file is written, so a cross-file
//! `@fig:samples` resolves to the same Quarto id its definition gets, and two
//! files that happen to define the same raw label collide predictably
//! (`-2`, `-3`, …) instead of silently emitting duplicate `{#fig-samples}`
//! ids across a project.
//!
//! Deliberately **not** document-scoped (the original phase draft's mistake,
//! caught in red-team review): Quarto's crossref namespace is project-global,
//! and building the registry per-file cannot see a collision that spans two
//! files, nor resolve a reference to a label defined in a different file.

pub mod normalize;
pub mod sidecar;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use normalize::RefKind;

use crate::diagnostics::{codes, Diagnostic, Severity};
use crate::{Block, BlockKind, Document, Label};

/// One entry collected from a document before normalization: which file it
/// came from, the raw label as read, and what kind of construct owns it
/// (needed only when the label has no recognized colon prefix — see
/// `normalize::normalize`'s rule 3).
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawLabel {
    source: PathBuf,
    label: Label,
    kind: RefKind,
}

/// Builds a registry-sourced [`Diagnostic`], `span` defaulted to line 1
/// (label collisions are detected registry-wide, after every document's
/// been read, not at one line the registry itself tracks — the writer that
/// actually emits the suffixed id does have a real span, but by then the
/// collision has already been decided here).
fn warn(code: &'static str, file: PathBuf, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Severity::Warning, code, message).with_file(file)
}

/// The run-scoped label registry. Built once, over every document in a
/// conversion set, before any writer runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelRegistry {
    /// `(source file, raw MyST-side label)` -> normalized Quarto id. Keyed
    /// by file because two files may legitimately both define `fig:samples`
    /// (they get disambiguated, not merged) — a flat `{label: id}` map
    /// cannot represent that, which is exactly the sidecar-shape problem
    /// RT-08/RT-09 raised against the original draft.
    forward: BTreeMap<(PathBuf, Label), String>,
    /// Every quarto id already assigned in this run, so an inline
    /// cross-reference to a *bare* (unprefixed-at-lookup) label can find
    /// which file defined it — see [`LabelRegistry::resolve_reference`].
    by_raw_label: BTreeMap<Label, String>,
}

impl LabelRegistry {
    /// Builds a registry over every document in `documents`, in the order
    /// given. Collision suffixes (`-2`, `-3`, …) are seeded from a **stable
    /// sort of `(source_path, label)`**, not collection order — so adding an
    /// unrelated file to a later run never renumbers a label inside a file
    /// that run did not touch (the exact failure mode RT-08 reproduced
    /// against traversal-order suffixing).
    #[must_use]
    pub fn build(documents: &[(PathBuf, Document)]) -> (Self, Vec<Diagnostic>) {
        let mut raw = Vec::new();
        for (source, doc) in documents {
            collect_labels(&doc.blocks, source, &mut raw);
        }
        // Stable, deterministic ordering independent of collection order —
        // `BTreeMap`'s own ordering only helps once entries exist; the
        // *assignment* order (who gets the bare id vs. `-2`) must be decided
        // up front from a sort, not from however `documents` happened to be
        // passed in.
        raw.sort_by(|a, b| (&a.source, &a.label).cmp(&(&b.source, &b.label)));

        let mut registry = LabelRegistry::default();
        let mut warnings = Vec::new();
        // Every id already assigned in this run — not just base names and
        // their counts, but every *actual* id handed out (including
        // suffixed ones). A counter keyed only by base name (the original
        // design) can hand out an already-taken suffixed id: if `a.md`
        // takes `fig-samples` and `b.md` independently defines the raw
        // label `fig:samples-2` (normalizing straight to `fig-samples-2`),
        // a naive third collision on `fig:samples` would also compute
        // `fig-samples-2` and collide with `b.md` silently. Checking
        // membership in `taken` directly (not a per-base counter) closes
        // that gap.
        let mut taken: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for entry in &raw {
            let base = normalize::normalize(&entry.label.raw, entry.kind.clone());
            let id = if taken.insert(base.clone()) {
                base.clone()
            } else {
                let mut n = 2;
                let suffixed = loop {
                    let candidate = format!("{base}-{n}");
                    if taken.insert(candidate.clone()) {
                        break candidate;
                    }
                    n += 1;
                };
                warnings.push(warn(
                    codes::label::COLLISION_DISAMBIGUATED,
                    entry.source.clone(),
                    format!(
                        "label `{}` normalizes to `{base}`, which is already taken \
                         in this conversion set; using `{suffixed}` instead",
                        entry.label.raw
                    ),
                ));
                suffixed
            };

            registry
                .forward
                .insert((entry.source.clone(), entry.label.clone()), id.clone());
            registry
                .by_raw_label
                .entry(entry.label.clone())
                .or_insert(id);
        }

        (registry, warnings)
    }

    /// Looks up the Quarto id for a label as defined in `source` — the
    /// primary lookup a writer uses for the label attached to the block it
    /// is currently emitting.
    #[must_use]
    pub fn quarto_id(&self, source: &Path, label: &Label) -> Option<&str> {
        self.forward
            .get(&(source.to_path_buf(), label.clone()))
            .map(String::as_str)
    }

    /// Resolves an inline cross-reference's raw label to its final Quarto
    /// id. `source` is the file **containing the reference** (not
    /// necessarily the file defining the label) — checked first, because a
    /// same-file definition is unambiguous and, critically, is the only way
    /// to get a *collision-suffixed* id right: pure string normalization
    /// (`fig:samples` -> `fig-samples`) knows nothing about whether this
    /// conversion set's registry actually suffixed that label to
    /// `fig-samples-2` for this particular file.
    ///
    /// Falls back to a project-wide search (any file's exact `(file, raw)`
    /// entry, keyed only by label spelling) for the case a reference in one
    /// file points at a label defined in *another* — this is inherently
    /// ambiguous if two different files coincidentally define the identical
    /// raw label (the reference cannot know which one was meant), so it
    /// resolves to whichever this registry's own `by_raw_label` index
    /// happened to keep (first-registered, per [`LabelRegistry::build`]'s
    /// sorted assignment order) rather than guaranteeing correctness in
    /// that specific cross-file-ambiguous case.
    ///
    /// Only as a last resort — no definition anywhere in the conversion set
    /// — does this fall through to bare string normalization, which is a
    /// best-effort guess, not a verified resolution.
    #[must_use]
    pub fn resolve_reference(&self, source: &Path, raw: &str) -> String {
        let label = Label::new(raw);
        if let Some(id) = self.forward.get(&(source.to_path_buf(), label.clone())) {
            return id.clone();
        }
        if let Some(id) = self.by_raw_label.get(&label) {
            return id.clone();
        }
        normalize::normalize(raw, RefKind::Generic)
    }

    /// All `(source, myst_label, quarto_id)` triples, sorted, for
    /// [`sidecar`] serialization.
    pub fn entries(&self) -> impl Iterator<Item = (&Path, &Label, &str)> {
        self.forward
            .iter()
            .map(|((source, label), id)| (source.as_path(), label, id.as_str()))
    }
}

/// Recursively walks `blocks`, appending one [`RawLabel`] for every
/// labelable construct that carries a label. Recurses into every variant
/// with nested `Vec<Block>` body content (`Admonition`, `Margin`, `TabSet`'s
/// items, `Blockquote`, `Theorem`, `Directive`) so a label nested inside one
/// of those is not missed.
fn collect_labels(blocks: &[Block], source: &Path, out: &mut Vec<RawLabel>) {
    for block in blocks {
        match &block.kind {
            BlockKind::Heading {
                label: Some(label), ..
            } => push(out, source, label, RefKind::Section),
            BlockKind::Figure {
                label: Some(label), ..
            } => push(out, source, label, RefKind::Figure),
            BlockKind::Table {
                label: Some(label), ..
            } => push(out, source, label, RefKind::Table),
            BlockKind::Math {
                label: Some(label), ..
            } => push(out, source, label, RefKind::Equation),
            BlockKind::CodeCell {
                label: Some(label), ..
            } => push(out, source, label, RefKind::CodeCell),
            BlockKind::Theorem {
                thm_type,
                label: Some(label),
                body,
            } => {
                push(
                    out,
                    source,
                    label,
                    RefKind::Theorem {
                        subtype: thm_type.clone(),
                    },
                );
                collect_labels(body, source, out);
            }
            BlockKind::Directive {
                label: Some(label),
                body,
                ..
            } => {
                push(out, source, label, RefKind::Generic);
                collect_labels(body, source, out);
            }
            BlockKind::Target { label } => push(out, source, label, RefKind::Generic),
            BlockKind::Admonition { body, .. }
            | BlockKind::Margin { body }
            | BlockKind::Blockquote { body, .. } => collect_labels(body, source, out),
            BlockKind::TabSet { items } => {
                for item in items {
                    collect_labels(&item.body, source, out);
                }
            }
            BlockKind::Theorem { body, .. } | BlockKind::Directive { body, .. } => {
                // label: None cases of these two variants — still recurse
                // into their bodies even though this block itself
                // contributed nothing.
                collect_labels(body, source, out);
            }
            _ => {}
        }
    }
}

fn push(out: &mut Vec<RawLabel>, source: &Path, label: &Label, kind: RefKind) {
    out.push(RawLabel {
        source: source.to_path_buf(),
        label: label.clone(),
        kind,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Attrs, FigureSource};
    use crate::{Engine, Span};

    fn doc(source: &str, blocks: Vec<Block>) -> (PathBuf, Document) {
        (
            PathBuf::from(source),
            Document {
                frontmatter: None,
                blocks,
                source: PathBuf::from(source),
                engine: Some(Engine::Jupyter),
            },
        )
    }

    fn block(kind: BlockKind) -> Block {
        Block {
            kind,
            span: Span::single(1),
            blank_lines_before: 0,
        }
    }

    #[test]
    fn tab_colon_label_normalizes_through_the_registry_to_tbl_hyphen() {
        let docs = vec![doc(
            "a.md",
            vec![block(BlockKind::Table {
                caption: vec![],
                rows: vec![],
                label: Some(Label::new("tab:results")),
            })],
        )];
        let (registry, warnings) = LabelRegistry::build(&docs);
        assert!(warnings.is_empty());
        assert_eq!(
            registry.quarto_id(Path::new("a.md"), &Label::new("tab:results")),
            Some("tbl-results")
        );
    }

    #[test]
    fn unprefixed_figure_label_infers_fig_prefix_from_owning_block_type() {
        let docs = vec![doc(
            "a.md",
            vec![block(BlockKind::Figure {
                src: FigureSource::Path(PathBuf::from("img.png")),
                caption: vec![],
                label: Some(Label::new("samples")),
                attrs: Attrs::new(),
            })],
        )];
        let (registry, _) = LabelRegistry::build(&docs);
        assert_eq!(
            registry.quarto_id(Path::new("a.md"), &Label::new("samples")),
            Some("fig-samples")
        );
    }

    #[test]
    fn same_label_in_two_files_collides_and_is_disambiguated_deterministically() {
        let docs = vec![
            doc(
                "b.md",
                vec![block(BlockKind::Figure {
                    src: FigureSource::Path(PathBuf::from("img.png")),
                    caption: vec![],
                    label: Some(Label::new("fig:samples")),
                    attrs: Attrs::new(),
                })],
            ),
            doc(
                "a.md",
                vec![block(BlockKind::Figure {
                    src: FigureSource::Path(PathBuf::from("img.png")),
                    caption: vec![],
                    label: Some(Label::new("fig:samples")),
                    attrs: Attrs::new(),
                })],
            ),
        ];
        let (registry, warnings) = LabelRegistry::build(&docs);
        assert_eq!(warnings.len(), 1);
        // "a.md" sorts before "b.md" regardless of `documents` order, so
        // "a.md" keeps the bare id and "b.md" is suffixed — this is the
        // stable-sort seeding, not collection order.
        assert_eq!(
            registry.quarto_id(Path::new("a.md"), &Label::new("fig:samples")),
            Some("fig-samples")
        );
        assert_eq!(
            registry.quarto_id(Path::new("b.md"), &Label::new("fig:samples")),
            Some("fig-samples-2")
        );
    }

    #[test]
    fn a_suffixed_id_that_collides_with_another_label_is_re_suffixed_not_duplicated() {
        // H3 regression: three files sorted so that "a.md" claims
        // `fig-samples-2` directly (an unrelated raw label that happens to
        // normalize to that exact string), then "b.md" and "c.md" both
        // define plain `fig:samples`. The naive per-base counter computed
        // `fig-samples-2` for the *second* `fig:samples` regardless of
        // whether that id was already taken by something else entirely —
        // producing a silent duplicate `{#fig-samples-2}` across two files.
        let docs = vec![
            doc(
                "a.md",
                vec![block(BlockKind::Figure {
                    src: FigureSource::Path(PathBuf::from("img.png")),
                    caption: vec![],
                    label: Some(Label::new("fig:samples-2")),
                    attrs: Attrs::new(),
                })],
            ),
            doc(
                "b.md",
                vec![block(BlockKind::Figure {
                    src: FigureSource::Path(PathBuf::from("img.png")),
                    caption: vec![],
                    label: Some(Label::new("fig:samples")),
                    attrs: Attrs::new(),
                })],
            ),
            doc(
                "c.md",
                vec![block(BlockKind::Figure {
                    src: FigureSource::Path(PathBuf::from("img.png")),
                    caption: vec![],
                    label: Some(Label::new("fig:samples")),
                    attrs: Attrs::new(),
                })],
            ),
        ];
        let (registry, _warnings) = LabelRegistry::build(&docs);
        let a = registry
            .quarto_id(Path::new("a.md"), &Label::new("fig:samples-2"))
            .unwrap()
            .to_string();
        let b = registry
            .quarto_id(Path::new("b.md"), &Label::new("fig:samples"))
            .unwrap()
            .to_string();
        let c = registry
            .quarto_id(Path::new("c.md"), &Label::new("fig:samples"))
            .unwrap()
            .to_string();
        let ids = [a, b, c];
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(
            unique.len(),
            3,
            "all three ids must be distinct, got {ids:?}"
        );
    }

    #[test]
    fn adding_an_unrelated_file_does_not_renumber_a_file_it_does_not_collide_with() {
        let one_file = vec![doc(
            "a.md",
            vec![block(BlockKind::Figure {
                src: FigureSource::Path(PathBuf::from("img.png")),
                caption: vec![],
                label: Some(Label::new("fig:samples")),
                attrs: Attrs::new(),
            })],
        )];
        let (registry_before, _) = LabelRegistry::build(&one_file);
        let id_before = registry_before
            .quarto_id(Path::new("a.md"), &Label::new("fig:samples"))
            .unwrap()
            .to_string();

        let mut two_files = one_file;
        two_files.push(doc(
            "z-unrelated.md",
            vec![block(BlockKind::Heading {
                level: 1,
                text: "Unrelated".to_string(),
                label: Some(Label::new("sec:unrelated")),
            })],
        ));
        let (registry_after, _) = LabelRegistry::build(&two_files);
        let id_after = registry_after
            .quarto_id(Path::new("a.md"), &Label::new("fig:samples"))
            .unwrap();
        assert_eq!(id_before, id_after);
    }

    #[test]
    fn cross_file_reference_resolves_via_colon_prefix_without_needing_the_defining_file() {
        let docs = vec![doc(
            "other.md",
            vec![block(BlockKind::Figure {
                src: FigureSource::Path(PathBuf::from("img.png")),
                caption: vec![],
                label: Some(Label::new("fig:samples")),
                attrs: Attrs::new(),
            })],
        )];
        let (registry, _) = LabelRegistry::build(&docs);
        // Referencing from a file that does not itself define the label —
        // this is the cross-file case, not the same-file-priority case
        // `resolve_reference`'s docs describe.
        assert_eq!(
            registry.resolve_reference(Path::new("referencing.md"), "fig:samples"),
            "fig-samples"
        );
    }

    #[test]
    fn unprefixed_reference_resolves_through_by_raw_label_when_a_definition_exists() {
        let docs = vec![doc(
            "other.md",
            vec![block(BlockKind::Figure {
                src: FigureSource::Path(PathBuf::from("img.png")),
                caption: vec![],
                label: Some(Label::new("samples")),
                attrs: Attrs::new(),
            })],
        )];
        let (registry, _) = LabelRegistry::build(&docs);
        assert_eq!(
            registry.resolve_reference(Path::new("referencing.md"), "samples"),
            "fig-samples"
        );
    }

    #[test]
    fn same_file_reference_resolves_to_the_suffixed_id_the_registry_actually_assigned() {
        // H2 regression: a reference inside the file whose own label got
        // collision-suffixed must resolve to *that file's* id, not to the
        // bare base id another file already claimed.
        let docs = vec![
            doc(
                "a.md",
                vec![block(BlockKind::Figure {
                    src: FigureSource::Path(PathBuf::from("img.png")),
                    caption: vec![],
                    label: Some(Label::new("fig:samples")),
                    attrs: Attrs::new(),
                })],
            ),
            doc(
                "b.md",
                vec![block(BlockKind::Figure {
                    src: FigureSource::Path(PathBuf::from("img.png")),
                    caption: vec![],
                    label: Some(Label::new("fig:samples")),
                    attrs: Attrs::new(),
                })],
            ),
        ];
        let (registry, _) = LabelRegistry::build(&docs);
        // "a.md" sorts first and keeps the bare id; "b.md" is suffixed.
        assert_eq!(
            registry.resolve_reference(Path::new("b.md"), "fig:samples"),
            "fig-samples-2",
            "a reference inside b.md must resolve to b.md's own (suffixed) id"
        );
        assert_eq!(
            registry.resolve_reference(Path::new("a.md"), "fig:samples"),
            "fig-samples"
        );
    }

    #[test]
    fn labels_nested_inside_admonitions_and_tabsets_are_collected() {
        let docs = vec![doc(
            "a.md",
            vec![
                block(BlockKind::Admonition {
                    kind: crate::AdmonitionKind::Note,
                    title: None,
                    body: vec![block(BlockKind::Figure {
                        src: FigureSource::Path(PathBuf::from("x.png")),
                        caption: vec![],
                        label: Some(Label::new("fig:nested")),
                        attrs: Attrs::new(),
                    })],
                    collapse: None,
                }),
                block(BlockKind::TabSet {
                    items: vec![crate::TabItem {
                        label: "Tab A".to_string(),
                        body: vec![block(BlockKind::Table {
                            caption: vec![],
                            rows: vec![],
                            label: Some(Label::new("tab:in-tab")),
                        })],
                    }],
                }),
            ],
        )];
        let (registry, _) = LabelRegistry::build(&docs);
        assert_eq!(
            registry.quarto_id(Path::new("a.md"), &Label::new("fig:nested")),
            Some("fig-nested")
        );
        assert_eq!(
            registry.quarto_id(Path::new("a.md"), &Label::new("tab:in-tab")),
            Some("tbl-in-tab")
        );
    }

    #[test]
    fn theorem_label_uses_its_subtype_abbreviation() {
        let docs = vec![doc(
            "a.md",
            vec![block(BlockKind::Theorem {
                thm_type: "lemma".to_string(),
                label: Some(Label::new("main")),
                body: vec![],
            })],
        )];
        let (registry, _) = LabelRegistry::build(&docs);
        assert_eq!(
            registry.quarto_id(Path::new("a.md"), &Label::new("main")),
            Some("lem-main")
        );
    }
}
