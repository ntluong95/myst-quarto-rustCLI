//! RT-14: MyST auto-loads `references.bib` from the project directory **and
//! resolves DOI citation keys over the network**; Quarto does neither. A
//! field-for-field `myst.yml` -> `_quarto.yml` mapping (there is no
//! `bibliography` key to map — MyST never had one) therefore produces a
//! `_quarto.yml` with no bibliography, citeproc never runs, and every
//! citation renders as literal text while `quarto render` still exits 0.
//!
//! Two independent checks, both needed: **synthesis** (add a `bibliography:`
//! key when a `.bib` exists and `myst.yml` doesn't set one) and
//! **diagnosis** (a citation key used in the documents but absent from every
//! reachable `.bib` — DOI keys MyST resolved live, which this tool does not
//! fetch, per the plan's non-goals).

use std::collections::BTreeSet;

use super::{warn, ConfigWarning};
use crate::reader::inline::{scan_line, InlineEvent};
use crate::{Block, BlockKind, Document};

/// Parses `@type{key, ...}` entries out of raw BibTeX text and returns every
/// citation key defined. Deliberately not a full BibTeX parser (see this
/// crate's non-goals on scope) — just enough structure recognition to answer
/// "is this key defined anywhere."
#[must_use]
pub fn bib_defined_keys(bib_text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut search_from = 0usize;
    while let Some(at_rel) = bib_text[search_from..].find('@') {
        let at = search_from + at_rel;
        let after_at = &bib_text[at + 1..];
        let type_len = after_at
            .char_indices()
            .take_while(|(_, c)| c.is_ascii_alphabetic())
            .last()
            .map_or(0, |(idx, ch)| idx + ch.len_utf8());
        let after_type = &after_at[type_len..];
        if type_len > 0 {
            if let Some(rest) = after_type.strip_prefix('{') {
                let key_len = rest
                    .char_indices()
                    .find(|(_, c)| matches!(c, ',' | '}' | '\n'))
                    .map_or(rest.len(), |(idx, _)| idx);
                let key = rest[..key_len].trim();
                if !key.is_empty() {
                    out.insert(key.to_string());
                }
            }
        }
        search_from = at + 1;
    }
    out
}

/// Every citation key referenced anywhere in `doc` — walks every text-bearing
/// line context ([`Block::span`]-adjacent prose, captions, headings) the same
/// way [`crate::pipeline`]'s label collector walks for labels, using
/// [`scan_line`] (the same recognizer the writers use) so a key here is
/// exactly what a writer would also treat as a citation, not an
/// independently-reimplemented guess.
#[must_use]
pub fn citation_keys_in_document(doc: &Document, known_labels: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect(&doc.blocks, known_labels, &mut out);
    out
}

fn collect(blocks: &[Block], known_labels: &[String], out: &mut BTreeSet<String>) {
    for block in blocks {
        let lines: Vec<&String> = match &block.kind {
            BlockKind::Paragraph { lines } => lines.iter().collect(),
            BlockKind::Figure { caption, .. } | BlockKind::Table { caption, .. } => {
                caption.iter().collect()
            }
            _ => Vec::new(),
        };
        for line in lines {
            for event in scan_line(line, known_labels).events {
                if let InlineEvent::Citation(key) = event {
                    out.insert(key);
                }
            }
        }
        match &block.kind {
            BlockKind::Admonition { body, .. }
            | BlockKind::Margin { body }
            | BlockKind::Blockquote { body, .. }
            | BlockKind::Theorem { body, .. }
            | BlockKind::Directive { body, .. } => collect(body, known_labels, out),
            BlockKind::TabSet { items } => {
                for item in items {
                    collect(&item.body, known_labels, out);
                }
            }
            _ => {}
        }
    }
}

/// A warning per citation key that is used somewhere in the conversion set
/// but defined in none of `bib_keys` — the direct RT-14 diagnostic.
#[must_use]
pub fn missing_citation_warnings(
    used: &BTreeSet<String>,
    bib_keys: &BTreeSet<String>,
) -> Vec<ConfigWarning> {
    used.difference(bib_keys)
        .map(|key| {
            warn(format!(
                "citation key `{key}` is used but defined in no reachable .bib file; MyST may \
                 have resolved it live from a DOI (reference RT-14) — Quarto requires a local \
                 bibliography entry, so this citation will render as literal text"
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Attrs, Engine, FigureSource};
    use crate::{Label, Span};
    use std::path::PathBuf;

    #[test]
    fn parses_article_and_software_entry_keys() {
        let bib =
            "@article{matplotlib,\n  title = {X},\n}\n\n@software{pandas,\n  title = {Y},\n}\n";
        let keys = bib_defined_keys(bib);
        assert_eq!(
            keys,
            BTreeSet::from(["matplotlib".to_string(), "pandas".to_string()])
        );
    }

    #[test]
    fn ignores_non_entry_at_signs() {
        // An email-shaped `@` in an author field must not be mistaken for an
        // entry start.
        let bib = "@article{key1,\n  author = {a@example.com},\n}\n";
        assert_eq!(bib_defined_keys(bib), BTreeSet::from(["key1".to_string()]));
    }

    fn doc_with_paragraph(text: &str) -> Document {
        Document {
            frontmatter: None,
            blocks: vec![Block {
                kind: BlockKind::Paragraph {
                    lines: vec![text.to_string()],
                },
                span: Span::single(1),
                blank_lines_before: 0,
            }],
            source: PathBuf::from("article.md"),
            engine: Some(Engine::Jupyter),
        }
    }

    #[test]
    fn finds_bracket_citation_keys_including_doi_style() {
        let doc = doc_with_paragraph("See [@10.1038/nmeth.1974] for details.");
        let keys = citation_keys_in_document(&doc, &[]);
        assert_eq!(keys, BTreeSet::from(["10.1038/nmeth.1974".to_string()]));
    }

    #[test]
    fn does_not_confuse_a_cross_reference_with_a_citation() {
        let doc = doc_with_paragraph("See @sec:intro for background.");
        let keys = citation_keys_in_document(&doc, &["sec:intro".to_string()]);
        assert!(
            keys.is_empty(),
            "a known label must not be treated as a citation"
        );
    }

    #[test]
    fn walks_figure_captions_too() {
        let doc = Document {
            frontmatter: None,
            blocks: vec![Block {
                kind: BlockKind::Figure {
                    src: FigureSource::Path(PathBuf::from("x.png")),
                    caption: vec!["From [@numpy].".to_string()],
                    label: Some(Label::new("fig:x")),
                    attrs: Attrs::new(),
                },
                span: Span::single(1),
                blank_lines_before: 0,
            }],
            source: PathBuf::from("article.md"),
            engine: Some(Engine::Jupyter),
        };
        assert_eq!(
            citation_keys_in_document(&doc, &[]),
            BTreeSet::from(["numpy".to_string()])
        );
    }

    /// RT-14's concrete regression: the two DOI keys in `article.md` are
    /// used but not defined in `references.bib` (which only has matplotlib,
    /// numpy, pandas, scipy).
    #[test]
    fn missing_citation_warnings_flags_undefined_dois() {
        let used = BTreeSet::from(["10.1038/nmeth.1974".to_string(), "matplotlib".to_string()]);
        let defined = BTreeSet::from(["matplotlib".to_string()]);
        let warnings = missing_citation_warnings(&used, &defined);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("10.1038/nmeth.1974"));
    }
}
