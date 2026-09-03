//! `_quarto.yml` -> `myst.yml` (reverse of [`super::myst_to_quarto`]).
//!
//! This direction is inherently lossier than the forward one: Quarto's
//! config has no equivalent of `subject`/`abbreviations`/`open_access`/
//! `venue`/`id` at all, so those can only be recovered from
//! `.mystquarto/preserved.json` (`preserved_fields`, restored verbatim under
//! their original `myst.yml` key) — never reconstructed from `_quarto.yml`
//! text, which never had them. Fields Quarto genuinely widens beyond what
//! MyST expressed (`categories[]` from a single `subject` string,
//! `format:`'s per-format options) are narrowed back with a warning when the
//! narrowing is lossy, rather than silently guessing.

use std::collections::BTreeMap;

use super::{
    as_mapping, as_sequence, as_str, get, mapping_field, sequence_field, string_field, warn,
    ConfigWarning,
};
use crate::yaml::emit::{emit, EmitField, YamlDoc};
use crate::yaml::{parse_mapping, YamlReadError, YamlValue};

pub struct ConversionResult {
    pub text: String,
    pub is_book: bool,
    pub warnings: Vec<ConfigWarning>,
}

/// Every root-level `_quarto.yml` key this function reads by name. Anything
/// else (`execute:`, `csl:`, `number-sections:`, `website:`, `theme:`, …) has
/// no myst.yml equivalent this function knows how to place — unlike the
/// forward direction, there is no per-field sidecar-recovery channel for
/// *this* direction yet (RT-11's sidecar is written by the forward run
/// only), so closing D10 here means warning rather than silently narrowing;
/// see [`convert`]'s trailing loop.
const HANDLED_ROOT_KEYS: &[&str] = &[
    "title",
    "subtitle",
    "short-title",
    "description",
    "author",
    "keywords",
    "date",
    "license",
    "doi",
    "repo-url",
    "bibliography",
    "image",
    "funding",
    "categories",
    "format",
    "crossref",
    "project",
    "book",
    "site",
];

/// Converts `quarto_text` (a whole `_quarto.yml` file's contents) to
/// `myst.yml` text. `preserved_fields`, if given, is
/// [`super::sidecar::PreservedConfig`]'s `fields` (already decoded via
/// [`super::sidecar::json_to_yaml`]) — restored verbatim under `project.*`.
///
/// # Errors
/// Propagates [`crate::yaml::YamlReadError`] if `quarto_text` is not a valid
/// single-document YAML mapping.
pub fn convert(
    quarto_text: &str,
    preserved_fields: Option<&BTreeMap<String, YamlValue>>,
) -> Result<ConversionResult, YamlReadError> {
    let root = parse_mapping(quarto_text)?;
    let project_type = string_field(mapping_field(&root, "project"), "type");
    let is_book = project_type.as_deref() == Some("book") || get(&root, "book").is_some();

    let mut warnings = Vec::new();
    let mut project_fields: Vec<(String, YamlValue)> = Vec::new();
    let mut top_fields: Vec<EmitField> = Vec::new();

    let book = mapping_field(&root, "book");
    let (title, authors, toc, appendices) = if is_book {
        (
            string_field(book, "title"),
            sequence_field(book, "author").to_vec(),
            sequence_field(book, "chapters").to_vec(),
            sequence_field(book, "appendices").to_vec(),
        )
    } else {
        (
            string_field(&root, "title"),
            sequence_field(&root, "author").to_vec(),
            Vec::new(),
            Vec::new(),
        )
    };

    if let Some(t) = title {
        project_fields.push(("title".to_string(), YamlValue::String(t)));
    }
    if let Some(v) = string_field(&root, "subtitle") {
        project_fields.push(("subtitle".to_string(), YamlValue::String(v)));
    }
    if let Some(v) = string_field(&root, "short-title") {
        project_fields.push(("short_title".to_string(), YamlValue::String(v)));
    }
    if let Some(v) = string_field(&root, "description") {
        project_fields.push(("description".to_string(), YamlValue::String(v)));
    }
    if !authors.is_empty() {
        project_fields.push((
            "authors".to_string(),
            YamlValue::Sequence(convert_authors_back(&authors)),
        ));
    }
    if !toc.is_empty() || !appendices.is_empty() {
        let mut flattened = Vec::new();
        let mut had_part_grouping = flatten_chapters(&toc, &mut flattened);
        if !appendices.is_empty() {
            had_part_grouping |= flatten_chapters(&appendices, &mut flattened);
            warnings.push(warn(
                "_quarto.yml book.appendices has no myst.yml toc equivalent for the appendix/ \
                 main-matter distinction; appended to toc as regular entries (reference §8.1)"
                    .to_string(),
            ));
        }
        if had_part_grouping {
            warnings.push(warn(
                "_quarto.yml book chapters include a `part:` grouping, which myst.yml's toc has \
                 no equivalent for; flattened into a plain list (reference §8.1)"
                    .to_string(),
            ));
        }
        project_fields.push(("toc".to_string(), YamlValue::Sequence(flattened)));
    }
    if let Some(seq) = get(&root, "keywords").and_then(as_sequence) {
        if !seq.is_empty() {
            project_fields.push(("keywords".to_string(), YamlValue::Sequence(seq.to_vec())));
        }
    }
    if let Some(v) = get(&root, "date") {
        project_fields.push(("date".to_string(), v.clone()));
    }
    if let Some(v) = string_field(&root, "license") {
        project_fields.push(("license".to_string(), YamlValue::String(v)));
    }
    if let Some(v) = string_field(&root, "doi") {
        project_fields.push(("doi".to_string(), YamlValue::String(v)));
    }
    if let Some(v) = string_field(&root, "repo-url") {
        project_fields.push(("github".to_string(), YamlValue::String(v)));
    }
    if let Some(v) = string_field(&root, "bibliography") {
        project_fields.push(("bibliography".to_string(), YamlValue::String(v)));
    }
    if let Some(v) = string_field(&root, "image") {
        project_fields.push(("banner".to_string(), YamlValue::String(v)));
    }
    if let Some(v) = get(&root, "funding") {
        project_fields.push(("funding".to_string(), v.clone()));
    }

    if let Some(cats) = get(&root, "categories").and_then(as_sequence) {
        if let Some(first) = cats.first().and_then(as_str) {
            project_fields.push(("subject".to_string(), YamlValue::String(first.to_string())));
            if cats.len() > 1 {
                warnings.push(warn(
                    "_quarto.yml categories has more than one entry; only the first was mapped \
                     back to myst.yml's single-valued subject field (reference §8.2)"
                        .to_string(),
                ));
            }
        }
    }

    if let Some(format) = get(&root, "format").and_then(as_mapping) {
        let (exports_field, export_warnings) = format_to_exports(format);
        warnings.extend(export_warnings);
        if let Some(e) = exports_field {
            project_fields.push(("exports".to_string(), e));
        }
    }

    if let Some(eq_prefix) = string_field(mapping_field(&root, "crossref"), "eq-prefix") {
        project_fields.push((
            "numbering".to_string(),
            YamlValue::Mapping(vec![(
                "equation".to_string(),
                YamlValue::Mapping(vec![("template".to_string(), YamlValue::String(eq_prefix))]),
            )]),
        ));
    }

    // `site.<key>`-prefixed entries are how the forward direction
    // (`myst_to_quarto`) preserves an unmapped `site.*` field (see that
    // module's docs) — restore them under `site:`, not `project:`, or a
    // round trip would move them to the wrong place in myst.yml.
    let mut site_fields: Vec<(String, YamlValue)> = Vec::new();
    if is_book {
        site_fields.push((
            "template".to_string(),
            YamlValue::String("book-theme".to_string()),
        ));
    }
    if let Some(fields) = preserved_fields {
        for (key, value) in fields {
            match key.strip_prefix("site.") {
                Some(site_key) => site_fields.push((site_key.to_string(), value.clone())),
                None => project_fields.push((key.clone(), value.clone())),
            }
        }
    }

    if !project_fields.is_empty() {
        top_fields.push(EmitField::new(
            "project",
            YamlValue::Mapping(project_fields),
        ));
    }
    if !site_fields.is_empty() {
        top_fields.push(EmitField::new("site", YamlValue::Mapping(site_fields)));
    }

    // D10 for this (inherently lossier — see module docs) direction: a
    // root-level `_quarto.yml` key this function has no myst.yml target for
    // (`execute:`, `csl:`, `theme:`, …) is warned about rather than
    // silently narrowed away, since there is no per-field sidecar-recovery
    // channel for this direction to fall back on.
    for (key, _) in &root {
        if !HANDLED_ROOT_KEYS.contains(&key.as_str()) {
            warnings.push(warn(format!(
                "_quarto.yml top-level key `{key}` has no myst.yml equivalent; dropped \
                 (reference §8.1-8.3)"
            )));
        }
    }

    Ok(ConversionResult {
        text: emit(&YamlDoc(top_fields)),
        is_book,
        warnings,
    })
}

fn convert_authors_back(authors: &[YamlValue]) -> Vec<YamlValue> {
    authors
        .iter()
        .map(|a| {
            let Some(m) = as_mapping(a) else {
                return a.clone();
            };
            let mut entry = Vec::new();
            for (key, value) in m {
                if key == "affiliations" {
                    // Reverse of the forward widening: a single-entry
                    // affiliations[] with only a `name` narrows back to a
                    // bare string; anything richer is passed through as-is
                    // rather than guessed at.
                    if let Some([one]) = as_sequence(value).map(<[YamlValue]>::to_vec).as_deref() {
                        if let Some(affil_map) = as_mapping(one) {
                            if affil_map.len() == 1 {
                                if let Some(name) = string_field(affil_map, "name") {
                                    entry
                                        .push(("affiliation".to_string(), YamlValue::String(name)));
                                    continue;
                                }
                            }
                        }
                    }
                    entry.push((key.clone(), value.clone()));
                } else {
                    entry.push((key.clone(), value.clone()));
                }
            }
            YamlValue::Mapping(entry)
        })
        .collect()
}

/// Flattens `chapters` (a Quarto `book.chapters`/`book.appendices` list)
/// into `out` as myst.yml toc entries, recursing into any `part:`-grouped
/// sub-list (standard Quarto book structure: `{part: "Part One", chapters:
/// [...]}`) rather than silently dropping the whole group — myst.yml's toc
/// has no concept of a part grouping to map the label onto, so only the
/// label is lost, not the chapters themselves. Returns `true` if any part
/// grouping was encountered, so the caller can warn about the flattening.
fn flatten_chapters(chapters: &[YamlValue], out: &mut Vec<YamlValue>) -> bool {
    let mut had_part_grouping = false;
    for c in chapters {
        if let Some(s) = as_str(c) {
            let name = s.strip_suffix(".qmd").unwrap_or(s);
            out.push(YamlValue::Mapping(vec![(
                "file".to_string(),
                YamlValue::String(name.to_string()),
            )]));
        } else if let Some(m) = as_mapping(c) {
            if let Some(nested) = get(m, "chapters").and_then(as_sequence) {
                had_part_grouping = true;
                had_part_grouping |= flatten_chapters(nested, out);
            }
        }
    }
    had_part_grouping
}

/// `pub(crate)`: also reused by [`crate::frontmatter::quarto_to_myst`] for
/// page-level `format:` -> `exports:` (reference §8.4's row pointing back at
/// §8.3), so the two directions share one mapping table instead of two.
pub(crate) fn format_to_exports(
    format: &[(String, YamlValue)],
) -> (Option<YamlValue>, Vec<ConfigWarning>) {
    let mut exports = Vec::new();
    let mut warnings = Vec::new();
    for (key, _) in format {
        let myst_format = match key.as_str() {
            "pdf" => "pdf",
            "docx" => "docx",
            "latex" => "tex",
            "jats" => "jats",
            other => {
                warnings.push(warn(format!(
                    "_quarto.yml format `{other}` has no exact myst.yml export equivalent; \
                     passed through as `format: {other}` (reference §8.3)"
                )));
                other
            }
        };
        exports.push(YamlValue::Mapping(vec![(
            "format".to_string(),
            YamlValue::String(myst_format.to_string()),
        )]));
    }
    if exports.is_empty() {
        (None, warnings)
    } else {
        (Some(YamlValue::Sequence(exports)), warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_type_project_maps_book_title_authors_and_chapters_back() {
        let quarto = "project:\n  type: book\nbook:\n  title: Test Project\n  author:\n    - name: Test Author\n  chapters:\n    - intro.qmd\n    - methods.qmd\n";
        let result = convert(quarto, None).unwrap();
        assert!(result.is_book);
        assert!(result.text.contains("project:"));
        assert!(result.text.contains("title: Test Project"));
        assert!(result.text.contains("name: Test Author"));
        assert!(result.text.contains("file: intro"));
        assert!(result.text.contains("file: methods"));
        assert!(result.text.contains("site:\n  template: book-theme\n"));
    }

    #[test]
    fn non_book_project_maps_top_level_title_and_author() {
        let quarto = "title: Solo Page\nauthor:\n  - name: A\n";
        let result = convert(quarto, None).unwrap();
        assert!(!result.is_book);
        assert!(result.text.contains("title: Solo Page"));
        assert!(!result.text.contains("site:"));
    }

    #[test]
    fn preserved_fields_are_restored_under_their_original_keys() {
        let mut preserved = BTreeMap::new();
        preserved.insert("open_access".to_string(), YamlValue::Bool(true));
        preserved.insert(
            "venue".to_string(),
            YamlValue::String("The Morganton Scientific".to_string()),
        );
        let quarto = "title: X\n";
        let result = convert(quarto, Some(&preserved)).unwrap();
        assert!(result.text.contains("open_access: true"));
        assert!(result.text.contains("venue: The Morganton Scientific"));
    }

    #[test]
    fn format_pdf_and_docx_map_back_to_exports() {
        let quarto = "title: X\nformat:\n  pdf: {}\n";
        let result = convert(quarto, None).unwrap();
        assert!(result.text.contains("format: pdf"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn unrecognized_format_key_passes_through_with_a_warning() {
        let quarto = "title: X\nformat:\n  typst: {}\n";
        let result = convert(quarto, None).unwrap();
        assert!(result.text.contains("format: typst"));
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn site_prefixed_preserved_field_is_restored_under_site_not_project() {
        let mut preserved = BTreeMap::new();
        preserved.insert(
            "site.template".to_string(),
            YamlValue::String("my-custom-theme".to_string()),
        );
        let quarto = "title: X\n";
        let result = convert(quarto, Some(&preserved)).unwrap();
        assert!(result.text.contains("site:\n  template: my-custom-theme\n"));
        assert!(!result.text.contains("project:\n  site.template"));
    }

    #[test]
    fn book_appendices_are_flattened_into_toc_with_a_warning() {
        let quarto = "project:\n  type: book\nbook:\n  chapters:\n    - intro.qmd\n  appendices:\n    - appendix.qmd\n";
        let result = convert(quarto, None).unwrap();
        assert!(result.text.contains("file: intro"));
        assert!(result.text.contains("file: appendix"));
        assert!(result
            .warnings
            .iter()
            .any(|w| w.message.contains("appendices")));
    }

    #[test]
    fn book_part_grouping_is_flattened_with_a_warning() {
        let quarto = "project:\n  type: book\nbook:\n  chapters:\n    - index.qmd\n    - part: \"Part One\"\n      chapters:\n        - a.qmd\n        - b.qmd\n";
        let result = convert(quarto, None).unwrap();
        assert!(result.text.contains("file: index"));
        assert!(result.text.contains("file: a"));
        assert!(result.text.contains("file: b"));
        assert!(result.warnings.iter().any(|w| w.message.contains("part")));
    }

    #[test]
    fn unrecognized_top_level_key_warns_instead_of_silently_dropping() {
        let quarto = "title: X\nexecute:\n  echo: false\n";
        let result = convert(quarto, None).unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|w| w.message.contains("execute")));
    }
}
