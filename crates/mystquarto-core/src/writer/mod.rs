//! IR -> MyST / IR -> Quarto writers.
//!
//! **Label direction is asymmetric, deliberately.** [`crate::LabelRegistry`]
//! solves *normalization* (MyST-style `fig:samples` -> Quarto-legal
//! `fig-samples`, with cross-file collision handling) — a concept that only
//! exists in the MyST->Quarto direction, because Quarto's own ids are
//! already legal MyST label text (MyST "imposes no constraint" — reference
//! §3.3). So:
//!
//! - [`quarto::QuartoWriter`] consults a [`crate::LabelRegistry`] built over
//!   the MyST-sourced documents being converted.
//! - [`myst::MystWriter`] does **not** — it either passes a label through
//!   unchanged (the same-dialect MyST->MyST round-trip case, and the default
//!   for Quarto->MyST when no sidecar entry exists) or substitutes a
//!   sidecar-restored original spelling via a `restore` map
//!   (`(file, quarto_id) -> original MyST label`, built by
//!   [`crate::registry::sidecar::restore_labels`]). Building a second,
//!   reverse `LabelRegistry` for this would need to re-normalize already-
//!   normalized ids — a modeling error the phase spec's Phase 3 risk section
//!   warns against repeating ("the IR is wrong and Phase 4 needs an IR
//!   change more than twice"; the same discipline applies here to *not*
//!   forcing one struct to do two directions' jobs).

pub mod myst;
pub mod quarto;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::diagnostics::{codes, Diagnostic, Severity};
use crate::preserve::{self, PreservedEntry};
use crate::reader::inline::{rewrite_line, InlineEvent};
use crate::{Block, Label, Span};

pub use myst::MystWriter;
pub use quarto::{resolve_embed_id, QuartoWriter};

/// Renders a `Preserved`/`Unmappable` block as its single-line, content-free
/// marker (reference §11, RD-2/RT-02) — shared by both writers, since the
/// marker syntax (an HTML comment) is dialect-agnostic: it renders as
/// nothing in both MyST and Quarto/Pandoc output, which is exactly why the
/// same syntax works as the round-trip anchor `crate::reader::preservation_marker_id`
/// recognizes regardless of which dialect is being read.
///
/// `original` empty means there is nothing to preserve (the "sidecar entry
/// for a marker being read back was missing" degrade — see
/// [`codes::block::PRESERVATION_ENTRY_MISSING`]): no sidecar entry is
/// written, and the returned marker says so rather than pointing at a
/// fabricated id that would perpetuate the same problem on a later round
/// trip.
///
/// `sink` bundles the two per-document accumulators
/// ([`QuartoWriter`]/[`MystWriter`]'s `preserved`/`diagnostics` fields) into
/// one argument — purely to keep this function's arity down; the two are
/// otherwise unrelated (one is sidecar data, one is user-facing output).
pub(crate) struct PreserveSink<'a> {
    pub preserved: &'a RefCell<BTreeMap<String, PreservedEntry>>,
    pub diagnostics: &'a RefCell<Vec<Diagnostic>>,
}

/// Bundles `render_preserved`'s per-call disposition (as opposed to
/// `PreserveSink`'s per-document accumulator state) — purely to keep that
/// function's arity down.
pub(crate) struct PreservedDisposition {
    pub code: &'static str,
    pub severity: Severity,
    /// The dialect `original` is written in — always the *writer's own*
    /// input dialect for a fresh `Unmappable` block (a `QuartoWriter` only
    /// ever processes MyST-sourced documents, so its `Unmappable` content
    /// is always `Dialect::Myst`; `MystWriter`'s production caller,
    /// `crate::pipeline::convert_quarto_to_myst_batch`, is symmetric).
    /// Recorded on the sidecar entry so a later reader can refuse to
    /// reparse it through the wrong dialect's parser — see
    /// `crate::preserve::Dialect`'s docs.
    pub dialect: crate::preserve::Dialect,
}

pub(crate) fn render_preserved(
    sink: &PreserveSink,
    file: &Path,
    span: Span,
    kind: &str,
    disposition: PreservedDisposition,
    original: Vec<String>,
) -> String {
    let PreservedDisposition {
        code,
        severity,
        dialect,
    } = disposition;
    if original.is_empty() {
        sink.diagnostics.borrow_mut().push(
            Diagnostic::new(
                Severity::Warning,
                codes::block::PRESERVATION_ENTRY_MISSING,
                "a preservation marker's sidecar entry was not found (missing or stale \
                 .mystquarto/preserved.json); its original content could not be restored",
            )
            .with_file(file.to_path_buf())
            .with_span(span),
        );
        return "<!-- mystquarto: preservation entry missing, original content unavailable -->"
            .to_string();
    }

    let id = preserve::entry_id(&original);
    sink.preserved.borrow_mut().insert(
        id.clone(),
        PreservedEntry {
            file: file.display().to_string(),
            line: span.start_line,
            code: code.to_string(),
            kind: kind.to_string(),
            dialect,
            original,
        },
    );
    sink.diagnostics.borrow_mut().push(
        Diagnostic::new(
            severity,
            code,
            format!("{kind} has no equivalent in the target dialect; preserved"),
        )
        .with_file(file.to_path_buf())
        .with_span(span)
        .with_preserved(id.clone()),
    );
    preserve::marker(code, kind, &id)
}

/// Extracts a short human-readable label for a preserved construct from a
/// reader-produced message (`BlockKind::Unmappable::reason`, e.g.
/// `"unrecognized MyST directive {glossary}"`): the text inside the first
/// `{...}`, matching the phase spec's own marker example
/// (`{glossary} preserved`). Falls back to `"construct"` when `reason` has
/// no such substring (e.g. a Quarto shortcode's reason, which names the
/// shortcode outside braces).
#[must_use]
pub(crate) fn preserved_kind(reason: &str) -> String {
    if let Some(start) = reason.find('{') {
        if let Some(end) = reason[start..].find('}') {
            return reason[start + 1..start + end].to_string();
        }
    }
    "construct".to_string()
}

/// Renders a block's `blank_lines_before` as that many blank lines, except
/// before the very first block in a sequence (nothing to separate it from).
/// Shared by both writers so vertical spacing round-trips identically
/// regardless of target dialect — this is what makes same-dialect
/// byte-identical round-trip possible (`crate::ir::Block::blank_lines_before`'s
/// whole reason for existing, RT-13).
pub(crate) fn push_spacing(out: &mut String, blank_lines_before: u8, is_first: bool) {
    if is_first {
        return;
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for _ in 0..blank_lines_before {
        out.push('\n');
    }
}

/// Joins rendered block bodies (each already a `Vec<String>` of lines) with
/// single newlines, trimming nothing — callers control blank-line placement
/// via [`push_spacing`].
pub(crate) fn join_lines(lines: &[String]) -> String {
    lines.join("\n")
}

/// Collects every label spelling that should be recognized as a
/// cross-reference (rather than a citation) by [`rewrite_line`]'s
/// `known_labels` parameter, from every document in a conversion set —
/// both MyST-side raw labels and their normalized Quarto ids, since a
/// document's inline `@token` text is in whichever dialect *that* document
/// was read from, and a single writer call processes one document at a
/// time without re-deriving which dialect its own input was.
#[must_use]
pub fn known_reference_labels(registry: &crate::LabelRegistry) -> Vec<String> {
    let mut labels = Vec::new();
    for (_, myst_label, quarto_id) in registry.entries() {
        labels.push(myst_label.raw.clone());
        labels.push(quarto_id.to_string());
    }
    labels.sort();
    labels.dedup();
    labels
}

/// Rewrites every text-bearing line context a block can carry
/// (`Paragraph.lines`, a `Figure`/`Table`'s `caption`, a heading's `text`,
/// …) with dialect-specific inline rendering. Delegates the actual
/// per-event decision to `render`, so [`myst::MystWriter`] and
/// [`quarto::QuartoWriter`] each supply their own — this function only
/// owns "call [`rewrite_line`] on every line," not the rendering rules
/// themselves.
pub(crate) fn rewrite_lines(
    lines: &[String],
    known_labels: &[String],
    mut render: impl FnMut(InlineEvent) -> Option<String>,
) -> Vec<String> {
    lines
        .iter()
        .map(|l| rewrite_line(l, known_labels, &mut render))
        .collect()
}

/// Recursively renders nested `body: Vec<Block>` content (admonitions,
/// margins, tab items, blockquotes, theorems, generic directives) by
/// delegating to `render_block` for each child and joining with
/// [`push_spacing`] — shared so both writers implement nested-body
/// rendering identically rather than each hand-rolling the same loop.
pub(crate) fn render_body(
    body: &[Block],
    mut render_block: impl FnMut(&Block) -> Vec<String>,
) -> Vec<String> {
    let mut out = String::new();
    for (i, block) in body.iter().enumerate() {
        push_spacing(&mut out, block.blank_lines_before, i == 0);
        out.push_str(&join_lines(&render_block(block)));
    }
    out.lines().map(str::to_string).collect()
}

/// `(source file, MyST-side label as it will be emitted)` -> the string
/// [`myst::MystWriter`] should actually write for it. Built once per
/// conversion set from [`crate::registry::sidecar::restore_labels`]'s
/// output — see this module's docs on why this, not a second
/// `LabelRegistry`, is the right shape for the reverse direction.
pub type RestoreMap = std::collections::BTreeMap<(PathBuf, String), Label>;

/// Resolves what [`myst::MystWriter`] should emit for `label` as read in
/// `source`: the sidecar-restored original if `restore` has one, else
/// `label` unchanged (identity pass-through — correct both for the
/// same-dialect MyST->MyST case, where `restore` is always empty, and for
/// Quarto->MyST with no sidecar entry for this particular id).
#[must_use]
pub(crate) fn resolve_myst_label(source: &Path, label: &Label, restore: &RestoreMap) -> Label {
    restore
        .get(&(source.to_path_buf(), label.raw.clone()))
        .cloned()
        .unwrap_or_else(|| label.clone())
}
