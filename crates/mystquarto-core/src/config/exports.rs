//! Reference §8.3: MyST `project.exports[]` (a list of `{format|template,
//! ...}`) <-> Quarto `format` (a map). Fixes D6: an export entry carrying
//! only a `template:` (no `format:`) previously produced `format: {}`,
//! invalid Quarto — the template name is inspected and mapped to a real
//! format, with the (non-portable) template name preserved as a comment.

use super::{as_mapping, as_str, get, warn, Diagnostic, Severity};
use crate::diagnostics::codes::config as codes;
use crate::yaml::YamlValue;

/// The Quarto `format:` field this phase would emit, plus any comment that
/// should render above it (documenting a guessed/non-portable format — see
/// [`crate::yaml::emit`]'s "one comment above a given top-level key" model,
/// which is why every export's note is folded into a single multi-line
/// comment on the one `format` field rather than attached per-entry).
pub struct FormatField {
    pub value: YamlValue,
    pub comment: Option<String>,
}

/// Converts every entry of `exports` (MyST's `project.exports[]`, already
/// narrowed to a `&[YamlValue]`) into Quarto's `format:` map. Returns `None`
/// when no entry produced a usable format (an empty `exports: []`, or every
/// entry being an unmappable `format: meca`) — callers must not emit
/// `format: {}` in that case, only skip the field.
#[must_use]
pub fn exports_to_format(exports: &[YamlValue]) -> (Option<FormatField>, Vec<Diagnostic>) {
    let mut entries: Vec<(String, YamlValue)> = Vec::new();
    let mut comment_lines: Vec<String> = Vec::new();
    let mut warnings = Vec::new();

    for export in exports {
        let Some(m) = as_mapping(export) else {
            continue;
        };
        if let Some(fmt) = get(m, "format").and_then(as_str) {
            match known_format(fmt) {
                Some(quarto_key) => {
                    entries.push((quarto_key.to_string(), YamlValue::Mapping(vec![])))
                }
                None if fmt == "meca" => warnings.push(warn(
                    Severity::Warning,
                    codes::EXPORT_FORMAT_DROPPED,
                    "myst.yml project.exports: format `meca` has no Quarto equivalent \
                     (reference §8.3); dropped"
                        .to_string(),
                )),
                None => warnings.push(warn(
                    Severity::Warning,
                    codes::EXPORT_FORMAT_DROPPED,
                    format!(
                        "myst.yml project.exports: unrecognized format `{fmt}`; dropped (no \
                         known Quarto equivalent)"
                    ),
                )),
            }
        } else if let Some(template) = get(m, "template").and_then(as_str) {
            let (quarto_fmt, guessed) = infer_format_from_template(template);
            entries.push((quarto_fmt.to_string(), YamlValue::Mapping(vec![])));
            comment_lines.push(if guessed {
                format!(
                    "mystquarto: could not infer a format from template `{template}`; \
                     guessed `{quarto_fmt}` — verify and override if wrong"
                )
            } else {
                format!(
                    "mystquarto: `template: {template}` is not portable to Quarto; inferred \
                     format `{quarto_fmt}`"
                )
            });
            if guessed {
                warnings.push(warn(
                    Severity::Warning,
                    codes::EXPORT_FORMAT_GUESSED,
                    format!(
                        "myst.yml project.exports: template `{template}`'s suffix is not \
                         recognized; guessed format `{quarto_fmt}`"
                    ),
                ));
            }
        }
    }

    if entries.is_empty() {
        return (None, warnings);
    }
    let comment = (!comment_lines.is_empty()).then(|| comment_lines.join("\n"));
    (
        Some(FormatField {
            value: YamlValue::Mapping(entries),
            comment,
        }),
        warnings,
    )
}

fn known_format(fmt: &str) -> Option<&'static str> {
    match fmt {
        "pdf" => Some("pdf"),
        "docx" => Some("docx"),
        "tex" => Some("latex"),
        "jats" => Some("jats"),
        _ => None,
    }
}

/// Template-suffix -> inferred format (reference §8.3's table). Unrecognized
/// suffixes fall back to `pdf` with `guessed = true`, so callers always warn
/// rather than silently guessing.
fn infer_format_from_template(template: &str) -> (&'static str, bool) {
    if template.ends_with("-typst") {
        ("typst", false)
    } else if template.ends_with("-tex") || template.ends_with("-latex") {
        ("pdf", false)
    } else if template.ends_with("-docx") {
        ("docx", false)
    } else if template.ends_with("-jats") {
        ("jats", false)
    } else {
        ("pdf", true)
    }
}

/// Reference architecture note: "`manuscript.article` derives from
/// `exports[].article`." Returns the first `article:` value found across
/// `exports`, if any.
#[must_use]
pub fn manuscript_article(exports: &[YamlValue]) -> Option<String> {
    exports
        .iter()
        .filter_map(as_mapping)
        .find_map(|m| get(m, "article").and_then(as_str))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::parse_mapping;

    fn exports_from(text: &str) -> Vec<YamlValue> {
        let parsed = parse_mapping(text).expect("valid YAML");
        match &parsed[0].1 {
            YamlValue::Sequence(s) => s.clone(),
            _ => panic!("expected a sequence"),
        }
    }

    #[test]
    fn explicit_format_maps_directly() {
        let exports = exports_from("exports:\n  - format: pdf\n");
        let (field, warnings) = exports_to_format(&exports);
        assert!(warnings.is_empty());
        let YamlValue::Mapping(m) = field.unwrap().value else {
            panic!()
        };
        assert_eq!(m, vec![("pdf".to_string(), YamlValue::Mapping(vec![]))]);
    }

    #[test]
    fn tex_maps_to_latex() {
        let exports = exports_from("exports:\n  - format: tex\n");
        let (field, _) = exports_to_format(&exports);
        let YamlValue::Mapping(m) = field.unwrap().value else {
            panic!()
        };
        assert_eq!(m[0].0, "latex");
    }

    #[test]
    fn meca_is_dropped_with_a_warning_never_format_empty() {
        let exports = exports_from("exports:\n  - format: meca\n");
        let (field, warnings) = exports_to_format(&exports);
        assert!(
            field.is_none(),
            "an all-unmappable export list must not emit format: {{}}"
        );
        assert_eq!(warnings.len(), 1);
    }

    /// D6: `template:`-only export must infer a real format, never
    /// `format: {}`, and must preserve the template name as a comment.
    #[test]
    fn template_only_export_infers_typst_and_preserves_template_as_comment() {
        let exports =
            exports_from("exports:\n  - template: lapreprint-typst\n    article: article.md\n");
        let (field, warnings) = exports_to_format(&exports);
        assert!(warnings.is_empty(), "a recognized suffix should not warn");
        let field = field.expect("template-only export must still produce a format");
        let YamlValue::Mapping(m) = &field.value else {
            panic!()
        };
        assert_eq!(m, &vec![("typst".to_string(), YamlValue::Mapping(vec![]))]);
        assert!(field.comment.unwrap().contains("lapreprint-typst"));
    }

    #[test]
    fn unrecognized_template_suffix_guesses_pdf_and_warns() {
        let exports = exports_from("exports:\n  - template: my-custom-house-style\n");
        let (field, warnings) = exports_to_format(&exports);
        let YamlValue::Mapping(m) = &field.unwrap().value else {
            panic!()
        };
        assert_eq!(m[0].0, "pdf");
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn empty_exports_produces_no_format_field() {
        let (field, warnings) = exports_to_format(&[]);
        assert!(field.is_none());
        assert!(warnings.is_empty());
    }

    #[test]
    fn manuscript_article_reads_the_first_article_field() {
        let exports =
            exports_from("exports:\n  - template: lapreprint-typst\n    article: article.md\n");
        assert_eq!(manuscript_article(&exports), Some("article.md".to_string()));
    }
}
