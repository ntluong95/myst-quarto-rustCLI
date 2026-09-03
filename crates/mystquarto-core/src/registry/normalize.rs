//! Colon-prefixed MyST label -> hyphen-prefixed Quarto id normalization.
//!
//! Reference §3.3/§3.4, transcribed into `mappings.toml`'s `[[label_prefix]]`
//! table (22 rows: `fig:`->`fig-`, the `tab:`->`tbl-` exception, 17 more
//! theorem/callout-adjacent prefixes, and the generic `_`->`-` rule). This
//! module is the pure, stateless half of label handling — [`super::registry`]
//! owns the stateful part (collision suffixing, sidecar persistence) that
//! needs to see every label in a conversion set at once.

use crate::mappings::mappings;

/// What kind of construct a label without a recognized colon prefix is
/// attached to — used only when [`normalize`]'s rule 2 (prefix-table lookup)
/// finds no match, per the phase spec's rule 3 ("if no kind token, infer from
/// RefKind"). Every labelable [`crate::BlockKind`] variant maps to exactly
/// one of these (see [`super::registry::collect_labels`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefKind {
    Section,
    Figure,
    Table,
    Equation,
    /// A code cell's own label (not the figure/table it might produce —
    /// this is the *cell's* label, e.g. for a listing). No `[[label_prefix]]`
    /// row exists for this; `fig-` is used because in practice a MyST
    /// code-cell label that gets cross-referenced is overwhelmingly one
    /// that will render a figure (the same real-world pattern the phase
    /// spec's "nb -> fig" rule documents for notebook-cell embeds) — a
    /// documented judgment call, not a reference-doc row.
    CodeCell,
    /// One of Quarto's eleven `crossref` theorem-family types. `subtype` is
    /// the MyST `prf:` directive's own subtype string (`"theorem"`,
    /// `"lemma"`, …), mapped to its `[[label_prefix]]` abbreviation by
    /// [`theorem_prefix`]. An owned `String` (not `&'static str`) because it
    /// comes from [`crate::BlockKind::Theorem`]'s runtime `thm_type` field.
    Theorem {
        subtype: String,
    },
    /// A generic labelled directive (`grid`/`card`/…) or a standalone
    /// `(label)=` target with no directive context at all — reference §3.1
    /// "Arbitrary block". No natural Quarto id family fits either, so an
    /// unprefixed label of this kind is only lowercased/hyphenated, never
    /// given an injected prefix.
    Generic,
}

/// Maps a MyST `prf:` theorem subtype to its `[[label_prefix]]` MyST-prefix
/// key (reference §3.3's eleven theorem-family rows), or `None` for a
/// subtype this table does not carry an abbreviation for (falls back to
/// [`RefKind::Generic`]-style unprefixed handling).
#[must_use]
pub fn theorem_prefix(subtype: &str) -> Option<&'static str> {
    Some(match subtype {
        "theorem" => "thm:",
        "lemma" => "lem:",
        "corollary" => "cor:",
        "proposition" => "prp:",
        "conjecture" => "cnj:",
        "definition" => "def:",
        "example" => "exm:",
        "exercise" => "exr:",
        "solution" => "sol:",
        "remark" => "rem:",
        "algorithm" => "alg:",
        _ => return None,
    })
}

/// The colon-prefix key an inferred [`RefKind`] would carry if it had one —
/// used by [`normalize`]'s rule 3 to reuse rule 2's table lookup rather than
/// duplicating the myst-prefix -> quarto-prefix mapping a second time.
fn inferred_prefix_key(kind: RefKind) -> Option<&'static str> {
    match kind {
        RefKind::Section => Some("sec:"),
        RefKind::Figure => Some("fig:"),
        RefKind::Table => Some("tab:"),
        RefKind::Equation => Some("eq:"),
        RefKind::CodeCell => Some("fig:"),
        RefKind::Theorem { ref subtype } => theorem_prefix(subtype),
        RefKind::Generic => None,
    }
}

/// Normalizes one raw MyST-side label into a Quarto-legal id: hyphen-
/// separated, type-prefixed where a prefix is known, lowercase, `[a-z0-9-]`
/// only. Does **not** handle collision suffixing (`-2`, `-3`, …) — that
/// requires seeing every label in the conversion set at once, which is
/// [`super::LabelRegistry::build`]'s job, run after this function has
/// produced every label's *base* id.
///
/// Rules, applied in order (reference §3.4):
/// 1. Split `raw` on the first `:`.
/// 2. If the prefix (with its colon) is a row in `mappings.toml`'s
///    `[[label_prefix]]` table, use that row's `quarto` prefix — this is
///    where `tab:` -> `tbl-` (not `tab-`) comes from.
/// 3. Otherwise, if `raw` had no colon at all, infer a prefix from `kind`
///    (only possible because the IR carries the owning block's type
///    alongside the label — see `crate::ir` module docs). A colon prefix
///    the table does not recognize (some `custom:` the user invented) is
///    **not** overridden by `kind` here — see this function's doc on that
///    choice below.
/// 4. Lowercase; `_` -> `-`; drop any byte outside `[a-z0-9-]`.
///
/// An unrecognized colon prefix is treated as ordinary label text (the whole
/// `raw` string, colon and all, goes through step 4) rather than being
/// overridden by `kind` — "if no kind token, infer from RefKind" (phase spec)
/// reads as "no colon", not "a colon we don't happen to have a row for".
/// MyST imposes no constraint on labels (reference §3.3), so respecting a
/// user's own semantic tag when we don't recognize it is more conservative
/// than silently replacing it with our own guess.
#[must_use]
pub fn normalize(raw: &str, kind: RefKind) -> String {
    let (base, colon_present) = match raw.split_once(':') {
        // `nb:` is mystquarto's own notebook-cell-relabelling convention
        // (RD-3), not a general MyST label prefix — it has no
        // `[[label_prefix]]` row in `mappings.toml` because reference §3.4
        // documents it as an example of rule 2's *algorithm*, not as one
        // more user-facing prefix for that table to catalog. Handled here,
        // directly, rather than added as a 23rd table row purely to satisfy
        // one internal caller ([`crate::notebook::relabel_notebook_json`]).
        Some(("nb", rest)) => (format!("fig-{rest}"), true),
        Some((prefix, rest)) => {
            let key = format!("{prefix}:");
            match label_prefix_quarto(&key) {
                Some(quarto_prefix) => (format!("{quarto_prefix}{rest}"), true),
                None => (raw.to_string(), true),
            }
        }
        None => (raw.to_string(), false),
    };

    let with_inferred_prefix = if colon_present {
        base
    } else {
        match inferred_prefix_key(kind).and_then(label_prefix_quarto) {
            Some(quarto_prefix) => format!("{quarto_prefix}{base}"),
            None => base,
        }
    };

    sanitize(&with_inferred_prefix)
}

/// Looks up a `[[label_prefix]]` row by its MyST-side key (e.g. `"fig:"`,
/// `"tab:"`), returning its Quarto-side prefix (e.g. `"fig-"`, `"tbl-"`).
fn label_prefix_quarto(myst_key: &str) -> Option<&'static str> {
    mappings()
        .label_prefix
        .iter()
        .find(|row| row.myst == myst_key)
        .map(|row| row.quarto.as_str())
}

/// Rule 4: lowercase, `_`/`:` -> `-`, drop anything outside `[a-z0-9-]`.
/// Applied as a final pass over the whole (possibly already-prefixed)
/// string, so it also cleans up the source text after the prefix
/// substitution above, not just the caller's raw input. `:` is folded to
/// `-` here (not merely dropped) so an *unrecognized* colon prefix — the
/// one case that reaches this function still carrying a colon, since rule 2
/// already stripped every recognized prefix's colon during substitution —
/// degrades to a readable, still-structured id (`custom:thing` ->
/// `custom-thing`) instead of silently mashing the prefix into the rest
/// (`customthing`).
fn sanitize(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            let c = c.to_ascii_lowercase();
            match c {
                'a'..='z' | '0'..='9' | '-' => Some(c),
                '_' | ':' => Some('-'),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fig_colon_prefix_maps_to_fig_hyphen() {
        assert_eq!(normalize("fig:samples", RefKind::Generic), "fig-samples");
    }

    #[test]
    fn tab_colon_prefix_maps_to_tbl_hyphen_not_tab_hyphen() {
        assert_eq!(
            normalize("tab:phenotypic-variation", RefKind::Generic),
            "tbl-phenotypic-variation"
        );
    }

    #[test]
    fn eq_colon_prefix_maps_to_eq_hyphen() {
        assert_eq!(
            normalize("eq:chi-squared", RefKind::Generic),
            "eq-chi-squared"
        );
    }

    #[test]
    fn sec_colon_prefix_maps_to_sec_hyphen() {
        assert_eq!(
            normalize("sec:data-analysis", RefKind::Generic),
            "sec-data-analysis"
        );
    }

    #[test]
    fn nb_colon_prefix_maps_directly_to_fig_hyphen() {
        // The notebook-cell-relabelling case (RD-3): a real notebook cell
        // label is the full `nb:<name>` string (see
        // `crate::reader::myst::MystReader::figure`'s
        // `format!("nb:{cell}")`), not the bare name — this is the special
        // case in `normalize`'s rule 2, not rule 3's RefKind fallback.
        assert_eq!(normalize("nb:analysis", RefKind::CodeCell), "fig-analysis");
    }

    #[test]
    fn unprefixed_figure_label_gets_fig_prefix_injected() {
        assert_eq!(normalize("samples", RefKind::Figure), "fig-samples");
    }

    #[test]
    fn unprefixed_table_label_gets_tbl_prefix_injected() {
        assert_eq!(normalize("results", RefKind::Table), "tbl-results");
    }

    #[test]
    fn unprefixed_section_label_gets_sec_prefix_injected() {
        assert_eq!(normalize("intro", RefKind::Section), "sec-intro");
    }

    #[test]
    fn theorem_subtype_maps_to_its_abbreviation() {
        assert_eq!(
            normalize(
                "main",
                RefKind::Theorem {
                    subtype: "lemma".to_string()
                }
            ),
            "lem-main"
        );
        assert_eq!(
            normalize(
                "main",
                RefKind::Theorem {
                    subtype: "algorithm".to_string()
                }
            ),
            "alg-main"
        );
    }

    #[test]
    fn generic_unprefixed_label_gets_no_injected_prefix() {
        assert_eq!(normalize("my-para", RefKind::Generic), "my-para");
    }

    #[test]
    fn underscore_becomes_hyphen() {
        assert_eq!(normalize("my_label", RefKind::Generic), "my-label");
    }

    #[test]
    fn uppercase_is_lowercased() {
        assert_eq!(normalize("Fig:Samples", RefKind::Generic), "fig-samples");
    }

    #[test]
    fn unrecognized_colon_prefix_is_kept_as_label_text_not_overridden_by_kind() {
        // "custom:" is not a `[[label_prefix]]` row; the whole string is
        // sanitized as-is rather than having `kind`'s prefix injected.
        assert_eq!(normalize("custom:thing", RefKind::Figure), "custom-thing");
    }

    #[test]
    fn characters_outside_the_legal_set_are_dropped() {
        assert_eq!(normalize("fig:a b!c", RefKind::Generic), "fig-abc");
    }
}
