//! Notebook cell relabelling (RD-3): patches `#| label:` lines inside a
//! `.ipynb` file's JSON so a Quarto `{{< embed nb.ipynb#fig-x >}}` shortcode
//! actually resolves.
//!
//! **Why this exists.** Verified against `quarto 1.9.36`: an embed whose
//! anchor does not match any cell's `#| label:` value renders no output at
//! all —
//! ```text
//! ERROR: The cell fig-analysis does not exist in notebook
//! ```
//! MyST's `#nb:analysis` convention and Quarto's `fig-`-prefixed embed
//! anchors are different spellings for the same concept, so without this
//! step the notebook embed defect (D11) cannot actually be fixed — the
//! writer could emit a syntactically valid `{{< embed >}}` shortcode that
//! still points at nothing.
//!
//! **Scope boundary**, from the phase spec: only `#| label:` lines are
//! patched. Notebook cell *content* (code, outputs, other metadata) is
//! never touched, and this module never writes to a *source* notebook —
//! callers apply it only to the copy already placed in the output tree
//! (`crate::fs::assets`' job, not this module's).
//!
//! **Precision over convenience.** [`relabel`] does not parse-then-
//! reserialize the notebook through `serde_json::Value` — doing so would
//! reformat the whole file (key order, number formatting, whitespace),
//! which is exactly the kind of collateral change the scope boundary above
//! rules out. Instead, `serde_json` is used only to *locate* which JSON
//! string literal needs to change; the edit itself is a literal,
//! byte-exact substring replacement of that one string's JSON-encoded form
//! in the original text — the same "surgery, not re-emission" strategy
//! `crate::yaml::surgery` uses for frontmatter, and for the same reason.

use std::collections::BTreeMap;

use crate::reader::parse_cell_option;

/// Rewrites every `#| label: <old>` line inside `text` (a notebook's raw
/// JSON, already read from the output-tree copy) to `#| label: <new>` for
/// each `(old, new)` pair in `renames`, and returns the edited text.
/// Anything not on a matched `#| label:` line — including every other cell,
/// every other line of a matched cell, and all notebook metadata/outputs —
/// is byte-identical to `text`.
///
/// A label in `renames` with no matching cell in `text` is simply not
/// applied (not an error): the notebook may not be the one a given label
/// belongs to, or may already have been relabelled by an earlier pass.
///
/// # Errors
/// Returns the underlying [`serde_json::Error`] if `text` is not valid JSON.
/// Malformed notebook JSON is refused rather than guessed at — see this
/// module's docs on why textual surgery, not reserialization, is used for
/// the edit itself; that strategy only works once the *location* of the
/// edit has been found via a real parse.
pub fn relabel(
    text: &str,
    renames: &BTreeMap<String, String>,
) -> Result<String, serde_json::Error> {
    if renames.is_empty() {
        return Ok(text.to_string());
    }
    let value: serde_json::Value = serde_json::from_str(text)?;
    let mut out = text.to_string();
    // A cursor that only ever moves forward across cells (M1 fix), so a
    // literal already consumed by an earlier cell's match can never be
    // re-matched by a later one. `serde_json::Value` carries no byte spans
    // (a spanning parser would be needed for that, which this module
    // deliberately avoids — see its docs on "surgery, not reserialization"),
    // so this alone would not stop a stray literal *within the same cell*
    // from being matched ahead of that cell's own real `source` line if the
    // JSON object happens to serialize `outputs` before `source` (a real,
    // common key order from some notebook writers). The per-cell anchor
    // below closes that gap: before searching for a cell's own label lines,
    // the cursor is advanced to that cell's `"source"` key position first,
    // so nothing before it — including that same cell's own `outputs` —
    // can ever be matched.
    let mut search_from = 0usize;

    let Some(cells) = value.get("cells").and_then(|v| v.as_array()) else {
        return Ok(out);
    };
    for cell in cells {
        // Only code cells carry an executable `#| label:` convention;
        // excluding other cell types also narrows which part of the file a
        // stray matching literal (e.g. in prose) could be mistaken for.
        if cell.get("cell_type").and_then(|v| v.as_str()) != Some("code") {
            continue;
        }
        let Some(source) = cell.get("source") else {
            continue;
        };
        // Anchor to this cell's own `"source"` key so a match can never
        // land in that same cell's `outputs`/`metadata`/anything else that
        // precedes `source` in the serialized object.
        if let Some(rel) = out[search_from..].find("\"source\"") {
            search_from += rel;
        }
        match source {
            serde_json::Value::Array(items) => {
                for item in items {
                    let Some(line) = item.as_str() else { continue };
                    if let Some((old_label, new_label)) = matching_rename(line, renames) {
                        if let Some(new_pos) = replace_source_string(
                            &mut out,
                            search_from,
                            line,
                            &line.replacen(&old_label, &new_label, 1),
                        ) {
                            search_from = new_pos;
                        }
                    }
                }
            }
            serde_json::Value::String(full) => {
                for line in full.lines() {
                    if let Some((old_label, new_label)) = matching_rename(line, renames) {
                        if let Some(new_pos) = replace_source_string(
                            &mut out,
                            search_from,
                            full,
                            &full.replacen(&old_label, &new_label, 1),
                        ) {
                            search_from = new_pos;
                        }
                        break; // one `#| label:` line per cell, at most
                    }
                }
            }
            _ => {}
        }
    }

    Ok(out)
}

/// If `line` (one source-array element, or one line of a single-string
/// `source`) is a `#| label: <value>` comment whose `<value>` is a key in
/// `renames`, returns `(old_label, new_label)`.
fn matching_rename(line: &str, renames: &BTreeMap<String, String>) -> Option<(String, String)> {
    let trimmed = line.trim_end_matches('\n');
    let value = parse_cell_option(trimmed, "label")?;
    renames.get(&value).map(|new| (value, new.clone()))
}

/// Replaces the JSON-encoded form of `old` with the JSON-encoded form of
/// `new` inside `text`, searching only from byte offset `search_from`
/// onward (M1 — see [`relabel`]'s docs on why this must not search from the
/// start of the file every time). Encoding both sides through `serde_json`
/// (rather than assuming `old`/`new` need no escaping) keeps this correct
/// even if a source line contains a character JSON must escape.
///
/// Returns the byte offset immediately after the replacement (the next
/// valid `search_from` for a subsequent call), or `None` if `old_json` was
/// not found at or after `search_from`.
fn replace_source_string(
    text: &mut String,
    search_from: usize,
    old: &str,
    new: &str,
) -> Option<usize> {
    let old_json = serde_json::to_string(old).expect("a &str always serializes to JSON");
    let new_json = serde_json::to_string(new).expect("a &str always serializes to JSON");
    let rel = text.get(search_from..)?.find(&old_json)?;
    let pos = search_from + rel;
    text.replace_range(pos..pos + old_json.len(), &new_json);
    Some(pos + new_json.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relabels_array_source_line_matching_a_rename() {
        let text = r##"{
  "cells": [
   {
    "cell_type": "code",
    "source": [
     "#| label: nb:analysis\n",
     "import matplotlib.pyplot as plt\n"
    ]
   }
  ]
}"##;
        let renames = BTreeMap::from([("nb:analysis".to_string(), "fig-analysis".to_string())]);
        let out = relabel(text, &renames).unwrap();
        assert!(out.contains("\"#| label: fig-analysis\\n\""));
        assert!(out.contains("\"import matplotlib.pyplot as plt\\n\""));
    }

    #[test]
    fn leaves_unrelated_cells_and_all_other_content_byte_identical() {
        let text = r##"{
 "cells": [
  {"cell_type": "code", "source": ["#| label: nb:analysis\n", "x = 1\n"]},
  {"cell_type": "markdown", "source": ["# A heading\n"]}
 ],
 "metadata": {"kernelspec": {"name": "python3"}}
}"##;
        let renames = BTreeMap::from([("nb:analysis".to_string(), "fig-analysis".to_string())]);
        let out = relabel(text, &renames).unwrap();
        assert!(out.contains("\"# A heading\\n\""));
        assert!(out.contains("\"kernelspec\": {\"name\": \"python3\"}"));
        assert!(out.contains("\"x = 1\\n\""));
    }

    #[test]
    fn unmatched_rename_is_a_no_op_not_an_error() {
        let text = r##"{"cells": [{"cell_type": "code", "source": ["#| label: nb:other\n"]}]}"##;
        let renames = BTreeMap::from([("nb:analysis".to_string(), "fig-analysis".to_string())]);
        let out = relabel(text, &renames).unwrap();
        assert_eq!(out, text);
    }

    #[test]
    fn empty_renames_returns_text_unchanged() {
        let text = r##"{"cells": []}"##;
        let out = relabel(text, &BTreeMap::new()).unwrap();
        assert_eq!(out, text);
    }

    #[test]
    fn malformed_json_is_a_typed_error_not_a_panic() {
        let renames = BTreeMap::from([("nb:analysis".to_string(), "fig-analysis".to_string())]);
        assert!(relabel("{ not json", &renames).is_err());
    }

    #[test]
    fn single_string_source_is_handled() {
        let text =
            r##"{"cells": [{"cell_type": "code", "source": "#| label: nb:analysis\nx = 1"}]}"##;
        let renames = BTreeMap::from([("nb:analysis".to_string(), "fig-analysis".to_string())]);
        let out = relabel(text, &renames).unwrap();
        assert!(out.contains("fig-analysis"));
        assert!(out.contains("x = 1"));
    }

    #[test]
    fn an_echoed_label_literal_in_outputs_preceding_source_is_not_matched_instead_of_the_real_line()
    {
        // M1 regression: `outputs` serialized *before* `source` in the same
        // cell (a real, common key order) contains the exact JSON literal
        // that also appears as this cell's genuine `#| label:` line — e.g.
        // because the cell printed its own source, or a traceback embedded
        // it. Before the fix, a plain `text.find()` from byte 0 matched the
        // `outputs` occurrence first and "relabelled" inert output data
        // while leaving the real, executable label untouched.
        let text = "{\"cells\": [{\"cell_type\": \"code\", \
             \"outputs\": [{\"text\": [\"#| label: nb:analysis\\n\"]}], \
             \"source\": [\"#| label: nb:analysis\\n\", \"x = 1\\n\"]}]}";
        let renames = BTreeMap::from([("nb:analysis".to_string(), "fig-analysis".to_string())]);
        let out = relabel(text, &renames).unwrap();

        // The real source line was relabelled...
        assert!(
            out.contains("\"source\": [\"#| label: fig-analysis\\n\", \"x = 1\\n\"]"),
            "the actual source line must be relabelled, got:\n{out}"
        );
        // ...and the echoed output text was left completely alone.
        assert!(
            out.contains("\"outputs\": [{\"text\": [\"#| label: nb:analysis\\n\"]}]"),
            "output data must never be touched, got:\n{out}"
        );
    }

    #[test]
    fn real_article_template_notebook_relabels_both_cells() {
        let text = include_str!("../../../article-template/analysis.ipynb");
        let renames = BTreeMap::from([
            ("nb:analysis".to_string(), "fig-analysis".to_string()),
            ("nb:chi-squared".to_string(), "fig-chi-squared".to_string()),
        ]);
        let out = relabel(text, &renames).unwrap();
        assert!(out.contains("fig-analysis"));
        assert!(out.contains("fig-chi-squared"));
        assert!(!out.contains("nb:analysis"));
        assert!(!out.contains("nb:chi-squared"));
        // Still valid JSON, still the same cell count.
        let original: serde_json::Value = serde_json::from_str(text).unwrap();
        let relabelled: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            original["cells"].as_array().unwrap().len(),
            relabelled["cells"].as_array().unwrap().len()
        );
    }
}
