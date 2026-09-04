//! Page-level frontmatter mapping (reference §8.4) — fixes the "QuartoWriter
//! passes frontmatter through verbatim" gap the D6/D7/D8/D9/D10 writers left
//! open ([`crate::writer`] previously emitted [`crate::ir::Frontmatter::raw`]
//! byte-for-byte, deferring cross-dialect field mapping to this phase).
//!
//! Edits an **existing** frontmatter block via [`crate::yaml::surgery`]
//! rather than synthesizing one from scratch — the opposite half of the
//! `yaml` module's reading-vs-synthesis split from [`crate::config`], which
//! *does* synthesize (there is no existing `_quarto.yml`/`myst.yml` text to
//! anchor project-config edits to). Every key this module does not touch —
//! critically `abstract`, whose block-literal style D9 exists to protect —
//! survives byte-identically because [`crate::yaml::surgery::apply_edits`]
//! never re-serializes untouched segments.
//!
//! `label` (§8.4: "Currently mapped to `id`, which Quarto ignores") is
//! deliberately **not** mapped to `id` here — repeating that mapping would
//! keep emitting a field the target dialect silently discards. The correct
//! fix (promoting it to a heading anchor) needs document-level access this
//! module does not have; until that lands, `label` is dropped with a
//! warning rather than round-tripped incorrectly.

use crate::config::exports;
use crate::config::quarto_to_myst::format_to_exports;
use crate::config::{as_str, get, string_field, warn, Diagnostic, Severity};
use crate::diagnostics::codes::config as codes;
use crate::ir::Frontmatter;
use crate::yaml::surgery::{apply_edits, FrontmatterEdit};
use crate::yaml::YamlValue;

/// Converts a MyST-sourced [`Frontmatter`]'s raw text to Quarto's dialect.
/// Every unrecognized key passes through untouched (both its value and its
/// original YAML style).
#[must_use]
pub fn myst_to_quarto(fm: &Frontmatter) -> (String, Vec<Diagnostic>) {
    let crate::yaml::YamlValue::Mapping(parsed) = &fm.parsed else {
        return (fm.raw.clone(), Vec::new());
    };
    let mut edits = Vec::new();
    let mut warnings = Vec::new();

    if let Some(kernelspec) = get(parsed, "kernelspec").and_then(|v| match v {
        YamlValue::Mapping(m) => Some(m.as_slice()),
        _ => None,
    }) {
        edits.push(FrontmatterEdit::Remove {
            key: "kernelspec".to_string(),
        });
        let name = string_field(kernelspec, "name").unwrap_or_else(|| "python3".to_string());
        if name == "ir" {
            edits.push(FrontmatterEdit::Set {
                key: "engine".to_string(),
                value: YamlValue::String("knitr".to_string()),
            });
        } else {
            edits.push(FrontmatterEdit::Set {
                key: "jupyter".to_string(),
                value: YamlValue::String(name),
            });
        }
    }

    if get(parsed, "jupytext").is_some() {
        edits.push(FrontmatterEdit::Remove {
            key: "jupytext".to_string(),
        });
    }

    if get(parsed, "math").is_some() {
        edits.push(FrontmatterEdit::Remove {
            key: "math".to_string(),
        });
        warnings.push(warn(
            Severity::LossyExpected,
            codes::FRONTMATTER_FIELD_DROPPED,
            "page frontmatter `math` (LaTeX macros) has no Quarto page-level equivalent; \
             dropped (reference §8.4)"
                .to_string(),
        ));
    }

    if get(parsed, "label").is_some() {
        edits.push(FrontmatterEdit::Remove {
            key: "label".to_string(),
        });
        warnings.push(warn(
            Severity::LossyExpected,
            codes::FRONTMATTER_FIELD_DROPPED,
            "page frontmatter `label` has no correct Quarto target yet (mapping it to `id` \
             would repeat a known-wrong behavior — Quarto ignores `id`); dropped rather than \
             mismapped (reference §8.4)"
                .to_string(),
        ));
    }

    if let Some(export_seq) = get(parsed, "exports").and_then(|v| match v {
        YamlValue::Sequence(s) => Some(s.as_slice()),
        _ => None,
    }) {
        edits.push(FrontmatterEdit::Remove {
            key: "exports".to_string(),
        });
        let (format_field, export_warnings) = exports::exports_to_format(export_seq);
        warnings.extend(export_warnings);
        if let Some(f) = format_field {
            edits.push(FrontmatterEdit::Set {
                key: "format".to_string(),
                value: f.value,
            });
        }
    }

    if let Some(numbering) = get(parsed, "numbering").and_then(|v| match v {
        YamlValue::Mapping(m) => Some(m.as_slice()),
        _ => None,
    }) {
        let equation = get(numbering, "equation").and_then(|v| match v {
            YamlValue::Mapping(m) => Some(m.as_slice()),
            _ => None,
        });
        if let Some(template) = equation.and_then(|e| string_field(e, "template")) {
            edits.push(FrontmatterEdit::Remove {
                key: "numbering".to_string(),
            });
            edits.push(FrontmatterEdit::Set {
                key: "crossref".to_string(),
                value: YamlValue::Mapping(vec![(
                    "eq-prefix".to_string(),
                    YamlValue::String(template),
                )]),
            });
        }
    }

    if let Some(parts) = get(parsed, "parts").and_then(|v| match v {
        YamlValue::Mapping(m) => Some(m.as_slice()),
        _ => None,
    }) {
        if let Some(abstract_text) = get(parts, "abstract") {
            // `parse_mapping` never produces `BlockLiteral` (block style is
            // write-side only — see `yaml::YamlValue`'s docs), so a
            // multi-line `parts.abstract` reads back as a plain `String`.
            // Re-emitting a multi-line value as a quoted plain scalar is
            // valid but not the `|` block style D9 exists to preserve;
            // force it back to block style whenever it has interior
            // newlines to keep round-tripped output idiomatic.
            let value = match abstract_text {
                YamlValue::String(s) if s.contains('\n') => YamlValue::BlockLiteral(s.clone()),
                other => other.clone(),
            };
            edits.push(FrontmatterEdit::Set {
                key: "abstract".to_string(),
                value,
            });
            if parts.len() == 1 {
                edits.push(FrontmatterEdit::Remove {
                    key: "parts".to_string(),
                });
            }
        }
    }

    let text = apply_edits(&fm.raw, &edits).unwrap_or_else(|_| fm.raw.clone());
    (text, warnings)
}

/// Converts a Quarto-sourced [`Frontmatter`]'s raw text to MyST's dialect.
#[must_use]
pub fn quarto_to_myst(fm: &Frontmatter) -> (String, Vec<Diagnostic>) {
    let crate::yaml::YamlValue::Mapping(parsed) = &fm.parsed else {
        return (fm.raw.clone(), Vec::new());
    };
    let mut edits = Vec::new();
    let mut warnings = Vec::new();

    if let Some(name) = get(parsed, "jupyter").and_then(as_str) {
        edits.push(FrontmatterEdit::Remove {
            key: "jupyter".to_string(),
        });
        edits.push(FrontmatterEdit::Set {
            key: "kernelspec".to_string(),
            value: kernelspec_for(name),
        });
    } else if let Some(engine) = get(parsed, "engine").and_then(as_str) {
        if engine == "knitr" {
            edits.push(FrontmatterEdit::Remove {
                key: "engine".to_string(),
            });
            edits.push(FrontmatterEdit::Set {
                key: "kernelspec".to_string(),
                value: kernelspec_for("ir"),
            });
        }
    }

    if let Some(format) = get(parsed, "format").and_then(|v| match v {
        YamlValue::Mapping(m) => Some(m.as_slice()),
        _ => None,
    }) {
        edits.push(FrontmatterEdit::Remove {
            key: "format".to_string(),
        });
        let (exports_field, export_warnings) = format_to_exports(format);
        warnings.extend(export_warnings);
        if let Some(e) = exports_field {
            edits.push(FrontmatterEdit::Set {
                key: "exports".to_string(),
                value: e,
            });
        }
    }

    if let Some(eq_prefix) = get(parsed, "crossref")
        .and_then(|v| match v {
            YamlValue::Mapping(m) => Some(m.as_slice()),
            _ => None,
        })
        .and_then(|m| string_field(m, "eq-prefix"))
    {
        edits.push(FrontmatterEdit::Remove {
            key: "crossref".to_string(),
        });
        edits.push(FrontmatterEdit::Set {
            key: "numbering".to_string(),
            value: YamlValue::Mapping(vec![(
                "equation".to_string(),
                YamlValue::Mapping(vec![("template".to_string(), YamlValue::String(eq_prefix))]),
            )]),
        });
    }

    let text = apply_edits(&fm.raw, &edits).unwrap_or_else(|_| fm.raw.clone());
    (text, warnings)
}

/// Mirrors the Python implementation's display-name heuristic
/// (`frontmatter.py::quarto_to_myst_frontmatter`): `python3` -> `Python 3`,
/// `ir` -> `R`, anything else uses the kernel name itself as a fallback
/// display name.
fn kernelspec_for(name: &str) -> YamlValue {
    let display_name = match name {
        "python3" => "Python 3".to_string(),
        "ir" => "R".to_string(),
        other => other.to_string(),
    };
    YamlValue::Mapping(vec![
        ("name".to_string(), YamlValue::String(name.to_string())),
        ("display_name".to_string(), YamlValue::String(display_name)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::parse_mapping;

    fn fm(raw: &str) -> Frontmatter {
        Frontmatter {
            raw: raw.to_string(),
            parsed: YamlValue::Mapping(parse_mapping(raw).expect("valid YAML")),
        }
    }

    #[test]
    fn kernelspec_python3_maps_to_jupyter() {
        let (text, warnings) = myst_to_quarto(&fm("title: My Doc\nkernelspec:\n  name: python3\n"));
        assert!(text.contains("jupyter: python3"));
        assert!(!text.contains("kernelspec"));
        assert!(warnings.is_empty());
        assert!(text.contains("title: My Doc"));
    }

    #[test]
    fn kernelspec_ir_maps_to_knitr_engine() {
        let (text, _) = myst_to_quarto(&fm("kernelspec:\n  name: ir\n  display_name: R\n"));
        assert!(text.contains("engine: knitr"));
        assert!(!text.contains("kernelspec"));
        assert!(!text.contains("jupyter"));
    }

    #[test]
    fn abstract_block_literal_survives_untouched() {
        let raw = "title: Foo\nabstract: |\n  line one\n  line two\nkernelspec:\n  name: python3\n";
        let (text, _) = myst_to_quarto(&fm(raw));
        assert!(text.contains("abstract: |\n  line one\n  line two\n"));
    }

    #[test]
    fn jupytext_and_math_are_dropped() {
        let raw = "title: Foo\njupytext:\n  formats: md:myst\nmath:\n  R: \\mathbb{R}\n";
        let (text, warnings) = myst_to_quarto(&fm(raw));
        assert!(!text.contains("jupytext"));
        assert!(!text.contains("math"));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn label_is_dropped_not_mismapped_to_id() {
        let (text, warnings) = myst_to_quarto(&fm("title: Foo\nlabel: nb:analysis\n"));
        assert!(!text.contains("label"));
        assert!(!text.contains("id:"));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn jupyter_python3_maps_back_to_kernelspec_with_display_name() {
        let (text, _) = quarto_to_myst(&fm("title: Foo\njupyter: python3\n"));
        assert!(text.contains("kernelspec:"));
        assert!(text.contains("name: python3"));
        assert!(text.contains("display_name: Python 3"));
        assert!(!text.contains("jupyter:"));
    }

    #[test]
    fn engine_knitr_maps_back_to_kernelspec_ir() {
        let (text, _) = quarto_to_myst(&fm("title: Foo\nengine: knitr\n"));
        assert!(text.contains("name: ir"));
        assert!(text.contains("display_name: R"));
        assert!(!text.contains("engine:"));
    }

    #[test]
    fn round_trip_kernelspec_python3() {
        let original = fm("title: Foo\nkernelspec:\n  name: python3\n  display_name: Python 3\n");
        let (forward, _) = myst_to_quarto(&original);
        let (back, _) = quarto_to_myst(&fm(&forward));
        assert!(back.contains("name: python3"));
        assert!(back.contains("display_name: Python 3"));
    }
}
