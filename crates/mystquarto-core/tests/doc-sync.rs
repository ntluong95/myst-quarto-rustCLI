//! Fails the build when `mappings.toml` and the sentinel-bounded tables in
//! `docs/dialect-comparison.md` drift apart.
//!
//! This reimplements `scripts/render-mapping-tables.py`'s rendering logic
//! independently in Rust (small, self-contained functions below — not a
//! shared module with the Python script). Both are expected to agree with
//! the committed doc; if they don't, this test is the tripwire.

use mystquarto_core::{mappings, Fidelity};

/// Escapes a Markdown table cell, rendering `None` as an em dash.
fn esc(value: Option<&str>) -> String {
    match value {
        None => "—".to_string(),
        Some(s) => s.replace('|', "\\|"),
    }
}

/// Notes render as an empty cell (not an em dash) when absent.
fn note_cell(value: Option<&str>) -> String {
    match value {
        None => String::new(),
        Some(s) => s.replace('|', "\\|"),
    }
}

fn fidelity_symbol(f: Fidelity) -> &'static str {
    match f {
        Fidelity::Exact => "✅",
        Fidelity::Lossy => "⚠️",
        Fidelity::Unmappable => "❌",
    }
}

fn render_table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut lines = Vec::with_capacity(rows.len() + 2);
    lines.push(format!("| {} |", header.join(" | ")));
    lines.push(format!("|{}|", vec!["---"; header.len()].join("|")));
    for row in rows {
        lines.push(format!("| {} |", row.join(" | ")));
    }
    lines.join("\n")
}

fn render_directive() -> String {
    let rows: Vec<Vec<String>> = mappings()
        .directive
        .iter()
        .map(|d| {
            vec![
                esc(Some(d.myst.as_str())),
                esc(d.quarto.as_deref()),
                fidelity_symbol(d.fidelity).to_string(),
                note_cell(d.note.as_deref()),
            ]
        })
        .collect();
    render_table(&["MyST", "Quarto", "Fidelity", "Note"], &rows)
}

fn render_inline() -> String {
    let mut rows: Vec<Vec<String>> = mappings()
        .role
        .iter()
        .map(|r| {
            vec![
                esc(r.myst.as_deref()),
                esc(r.quarto.as_deref()),
                fidelity_symbol(r.fidelity).to_string(),
                note_cell(r.note.as_deref()),
            ]
        })
        .collect();
    rows.extend(mappings().inline.iter().map(|i| {
        vec![
            esc(Some(i.myst.as_str())),
            esc(Some(i.quarto.as_str())),
            fidelity_symbol(i.fidelity).to_string(),
            note_cell(i.note.as_deref()),
        ]
    }));
    render_table(&["MyST", "Quarto", "Fidelity", "Note"], &rows)
}

fn render_config_field() -> String {
    let rows: Vec<Vec<String>> = mappings()
        .config_field
        .iter()
        .map(|c| {
            vec![
                esc(c.myst.as_deref()),
                esc(c.quarto.as_deref()),
                fidelity_symbol(c.fidelity).to_string(),
                note_cell(c.note.as_deref()),
            ]
        })
        .collect();
    render_table(
        &[
            "`myst.yml` field",
            "`_quarto.yml` field",
            "Fidelity",
            "Note",
        ],
        &rows,
    )
}

fn render_export_format() -> String {
    let rows: Vec<Vec<String>> = mappings()
        .export_format
        .iter()
        .map(|e| {
            vec![
                esc(Some(e.myst.as_str())),
                esc(e.quarto.as_deref()),
                fidelity_symbol(e.fidelity).to_string(),
                note_cell(e.note.as_deref()),
            ]
        })
        .collect();
    render_table(&["MyST export", "Quarto format", "Fidelity", "Note"], &rows)
}

fn render_legacy_role() -> String {
    let rows: Vec<Vec<String>> = mappings()
        .legacy_role
        .iter()
        .map(|l| {
            vec![
                esc(Some(l.myst.as_str())),
                esc(Some(l.modern_myst.as_str())),
                esc(Some(l.quarto.as_str())),
                fidelity_symbol(l.fidelity).to_string(),
                note_cell(l.note.as_deref()),
            ]
        })
        .collect();
    render_table(
        &[
            "Legacy construct",
            "Modern MyST",
            "Quarto",
            "Fidelity",
            "Note",
        ],
        &rows,
    )
}

fn render_label_prefix() -> String {
    let rows: Vec<Vec<String>> = mappings()
        .label_prefix
        .iter()
        .map(|p| {
            vec![
                esc(Some(p.myst.as_str())),
                esc(Some(p.quarto.as_str())),
                fidelity_symbol(p.fidelity).to_string(),
                note_cell(p.note.as_deref()),
            ]
        })
        .collect();
    render_table(&["MyST prefix", "Quarto prefix", "Fidelity", "Note"], &rows)
}

/// The committed reference doc, embedded at compile time so this test needs
/// no runtime path resolution.
const DOC: &str = include_str!("../../../docs/dialect-comparison.md");

/// Extracts the content strictly between a `<!-- generated: do not edit
/// (id) -->` / `<!-- end generated (id) -->` sentinel pair. Panics with a
/// message naming the section on any lookup failure, since that itself
/// indicates the doc and the generator have drifted (a missing/renamed
/// sentinel).
fn extract_section<'a>(doc: &'a str, id: &str) -> &'a str {
    let start_marker = format!("<!-- generated: do not edit ({id}) -->\n");
    let end_marker = format!("\n<!-- end generated ({id}) -->");
    let start = doc
        .find(&start_marker)
        .unwrap_or_else(|| panic!("section {id:?}: start sentinel not found in committed doc"))
        + start_marker.len();
    let end = doc[start..]
        .find(&end_marker)
        .unwrap_or_else(|| panic!("section {id:?}: end sentinel not found in committed doc"))
        + start;
    &doc[start..end]
}

/// The comparison helper the drift test exercises directly (see
/// `comparison_helper_detects_a_deliberate_mismatch` below) and that
/// `mappings_toml_matches_committed_doc` uses for every section.
fn sections_match(rendered: &str, committed: &str) -> bool {
    rendered == committed
}

#[test]
#[allow(clippy::type_complexity)] // a fixed-size array of (section id, renderer fn) pairs; a named alias would add indirection without adding clarity here
fn mappings_toml_matches_committed_doc() {
    let sections: [(&str, fn() -> String); 6] = [
        ("directive", render_directive),
        ("inline", render_inline),
        ("config_field", render_config_field),
        ("export_format", render_export_format),
        ("legacy_role", render_legacy_role),
        ("label_prefix", render_label_prefix),
    ];

    let mut mismatches = Vec::new();
    for (id, render) in sections {
        let rendered = render();
        let committed = extract_section(DOC, id);
        if !sections_match(&rendered, committed) {
            mismatches.push(format!(
                "section {id:?} disagrees with docs/dialect-comparison.md:\n\
                 --- rendered from mappings.toml ---\n{rendered}\n\
                 --- committed in the doc ---\n{committed}\n"
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "mappings.toml and docs/dialect-comparison.md have drifted apart; \
         run `scripts/render-mapping-tables.py` (or `just regen-docs`) and \
         commit the result.\n\n{}",
        mismatches.join("\n")
    );
}

/// Proves `sections_match` can actually catch drift, rather than trivially
/// always passing, by feeding it two deliberately different strings — one
/// with a `fidelity` value changed from ✅ to ⚠️ — instead of mutating the
/// real TOML file.
#[test]
fn comparison_helper_detects_a_deliberate_mismatch() {
    let expected = "| note | callout-note | ✅ | Only these five overlap |";
    let corrupted = "| note | callout-note | ⚠️ | Only these five overlap |";

    assert!(
        sections_match(expected, expected),
        "identical strings must compare equal"
    );
    assert!(
        !sections_match(expected, corrupted),
        "a single corrupted fidelity value must be detected as a mismatch"
    );
}
