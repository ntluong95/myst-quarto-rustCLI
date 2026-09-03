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

use std::path::{Path, PathBuf};

use crate::reader::inline::{rewrite_line, InlineEvent};
use crate::{Block, Label};

pub use myst::MystWriter;
pub use quarto::{resolve_embed_id, QuartoWriter};

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
