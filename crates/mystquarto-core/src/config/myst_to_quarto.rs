//! `myst.yml` -> `_quarto.yml` (reference §8.1-8.3). Synthesizes a whole new
//! document via [`crate::yaml::emit`] — there is no existing `_quarto.yml`
//! text to anchor edits to, unlike page frontmatter (see [`crate::frontmatter`]
//! and the [`super`] module docs).
//!
//! `myst.yml`'s `version` key (a schema-version marker, not user content) is
//! deliberately never mapped or preserved — it carries no information a
//! round trip needs to recover. `site.template` is consumed by
//! [`super::project_type::infer`] to decide `project.type`/`project` vs
//! `book`/`manuscript` shape, and is likewise not separately preserved: its
//! effect is visible in the output's `project.type`, so preserving it again
//! as a comment would be redundant, not missing information.

use std::collections::BTreeMap;

use super::{
    as_mapping, as_sequence, as_str, exports, get, mapping_field, sequence_field, string_field,
    warn, ConfigWarning, ProjectType,
};
use crate::yaml::emit::{emit, render_field, EmitField, YamlDoc};
use crate::yaml::{parse_mapping, YamlReadError, YamlValue};

/// The four §8.2 fields with no Quarto target at all — preserved as a
/// combined, human-readable comment block (matching the phase spec's worked
/// example) **and** returned in [`ConversionResult::preserved_fields`] for
/// the caller to write to `.mystquarto/preserved.json` via [`super::sidecar`]
/// — the authoritative recovery channel; the comment is informational only.
const UNMAPPABLE_FIELDS: &[&str] = &["abbreviations", "open_access", "venue", "id"];

/// Every `project.*` key this function reads by name, plus `version` (a
/// schema marker deliberately never mapped or preserved — see module docs).
/// Anything present in `project` that is *not* in this list is an
/// unknown/future field; closing D10 means preserving it too (comment +
/// sidecar, same channel as [`UNMAPPABLE_FIELDS`]), not just the four named
/// ones — see the loop at the end of [`convert`].
const HANDLED_PROJECT_KEYS: &[&str] = &[
    "version",
    "title",
    "authors",
    "subtitle",
    "short_title",
    "description",
    "subject",
    "keywords",
    "date",
    "license",
    "doi",
    "github",
    "bibliography",
    "exports",
    "downloads",
    "banner",
    "thumbnail",
    "funding",
    "numbering",
    "toc",
    "abbreviations",
    "open_access",
    "venue",
    "id",
];

pub struct ConversionResult {
    pub text: String,
    pub project_type: ProjectType,
    pub warnings: Vec<ConfigWarning>,
    pub preserved_fields: BTreeMap<String, YamlValue>,
}

/// Converts `myst_text` (a whole `myst.yml` file's contents) to `_quarto.yml`
/// text. `bib_path_for_synthesis`, if given, is the conversion-set-relative
/// path of a `.bib` file to synthesize a `bibliography:` key from when
/// `myst.yml` doesn't already set one (RT-14) — the caller decides whether
/// one exists and which to use (this function has no filesystem access).
///
/// # Errors
/// Propagates [`crate::yaml::YamlReadError`] if `myst_text` is not a valid
/// single-document YAML mapping.
pub fn convert(
    myst_text: &str,
    bib_path_for_synthesis: Option<&str>,
) -> Result<ConversionResult, YamlReadError> {
    let root = parse_mapping(myst_text)?;
    let project = mapping_field(&root, "project");
    let ptype = super::project_type::infer(&root);

    let mut warnings = Vec::new();
    let mut preserved_fields = BTreeMap::new();
    let mut fields: Vec<EmitField> = Vec::new();

    match ptype {
        ProjectType::Book => fields.push(EmitField::new(
            "project",
            YamlValue::Mapping(vec![(
                "type".to_string(),
                YamlValue::String("book".to_string()),
            )]),
        )),
        ProjectType::Manuscript => fields.push(EmitField::new(
            "project",
            YamlValue::Mapping(vec![(
                "type".to_string(),
                YamlValue::String("manuscript".to_string()),
            )]),
        )),
        ProjectType::Default => {}
    }

    let title = string_field(project, "title");
    let author_field = convert_authors(sequence_field(project, "authors"));

    if ptype == ProjectType::Book {
        let mut book_fields = Vec::new();
        if let Some(t) = &title {
            book_fields.push(("title".to_string(), YamlValue::String(t.clone())));
        }
        if let Some(a) = author_field.clone() {
            book_fields.push(("author".to_string(), a));
        }
        if let Some(chapters) = toc_to_chapters(sequence_field(project, "toc")) {
            book_fields.push(("chapters".to_string(), chapters));
        }
        fields.push(EmitField::new("book", YamlValue::Mapping(book_fields)));
    } else {
        if let Some(t) = &title {
            fields.push(EmitField::new("title", YamlValue::String(t.clone())));
        }
        if let Some(a) = author_field {
            fields.push(EmitField::new("author", a));
        }
    }

    if let Some(v) = string_field(project, "subtitle") {
        fields.push(EmitField::new("subtitle", YamlValue::String(v)));
    }
    if let Some(v) = string_field(project, "short_title") {
        fields.push(EmitField::new("short-title", YamlValue::String(v)));
    }
    if let Some(v) = string_field(project, "description") {
        fields.push(EmitField::new("description", YamlValue::String(v)));
    }
    if let Some(v) = string_field(project, "subject") {
        fields.push(EmitField::new(
            "categories",
            YamlValue::Sequence(vec![YamlValue::String(v)]),
        ));
    }
    if let Some(seq) = get(project, "keywords").and_then(as_sequence) {
        if !seq.is_empty() {
            fields.push(EmitField::new(
                "keywords",
                YamlValue::Sequence(seq.to_vec()),
            ));
        }
    }
    if let Some(v) = get(project, "date") {
        fields.push(EmitField::new("date", v.clone()));
    }
    if let Some(v) = string_field(project, "license") {
        fields.push(EmitField::new("license", YamlValue::String(v)));
    }
    if let Some(v) = string_field(project, "doi") {
        fields.push(EmitField::new("doi", YamlValue::String(v)));
    }
    if let Some(v) = string_field(project, "github") {
        fields.push(EmitField::new("repo-url", YamlValue::String(v)));
    }

    let mut bibliography_set = false;
    if let Some(v) = string_field(project, "bibliography") {
        fields.push(EmitField::new("bibliography", YamlValue::String(v)));
        bibliography_set = true;
    }
    if !bibliography_set {
        if let Some(bib_path) = bib_path_for_synthesis {
            fields.push(EmitField::new(
                "bibliography",
                YamlValue::String(bib_path.to_string()),
            ));
            warnings.push(warn(format!(
                "myst.yml has no `bibliography` key but {bib_path} exists in the conversion \
                 set; synthesized `bibliography: {bib_path}` so citations resolve under Quarto \
                 (reference RT-14)"
            )));
        }
    }

    let (format_field, export_warnings) =
        exports::exports_to_format(sequence_field(project, "exports"));
    warnings.extend(export_warnings);
    if let Some(f) = format_field {
        let mut ef = EmitField::new("format", f.value);
        if let Some(c) = f.comment {
            ef = ef.with_comment(c);
        }
        fields.push(ef);
    }

    if let Some(seq) = get(project, "downloads").and_then(as_sequence) {
        if !seq.is_empty() {
            fields.push(
                EmitField::new("downloads", YamlValue::Sequence(seq.to_vec()))
                    .with_comment("mystquarto: partial analogue only (reference §8.2)"),
            );
        }
    }

    match (
        string_field(project, "banner"),
        string_field(project, "thumbnail"),
    ) {
        (Some(b), Some(_)) => {
            fields.push(EmitField::new("image", YamlValue::String(b)));
            warnings.push(warn(
                "myst.yml has both banner and thumbnail; banner was used for _quarto.yml's \
                 image (reference §8.2)"
                    .to_string(),
            ));
        }
        (Some(b), None) => fields.push(EmitField::new("image", YamlValue::String(b))),
        (None, Some(t)) => fields.push(EmitField::new("image", YamlValue::String(t))),
        (None, None) => {}
    }

    if let Some(v) = get(project, "funding") {
        fields.push(
            EmitField::new("funding", v.clone())
                .with_comment("mystquarto: shapes differ between myst.yml and _quarto.yml; passed through as-is (reference §8.2)"),
        );
    }

    let numbering = mapping_field(project, "numbering");
    let eq_template = string_field(mapping_field(numbering, "equation"), "template");
    if let Some(t) = eq_template {
        fields.push(EmitField::new(
            "crossref",
            YamlValue::Mapping(vec![("eq-prefix".to_string(), YamlValue::String(t))]),
        ));
    } else if let Some(v) = get(project, "numbering") {
        preserved_fields.insert("numbering".to_string(), v.clone());
    }

    if ptype == ProjectType::Manuscript {
        let mut manuscript_fields = Vec::new();
        let article_file = exports::manuscript_article(sequence_field(project, "exports"));
        if let Some(article) = &article_file {
            manuscript_fields.push((
                "article".to_string(),
                YamlValue::String(rewrite_content_extension(article)),
            ));
        }
        let toc = sequence_field(project, "toc");
        let notebooks: Vec<YamlValue> = toc
            .iter()
            .filter_map(toc_entry_file)
            .filter(|f| f.ends_with(".ipynb"))
            .map(|f| YamlValue::Mapping(vec![("notebook".to_string(), YamlValue::String(f))]))
            .collect();
        if !notebooks.is_empty() {
            manuscript_fields.push(("notebooks".to_string(), YamlValue::Sequence(notebooks)));
        }
        if !manuscript_fields.is_empty() {
            fields.push(EmitField::new(
                "manuscript",
                YamlValue::Mapping(manuscript_fields),
            ));
        }
        // A manuscript's Quarto shape only has room for one `article` and a
        // list of `notebooks`; any other toc entry (an appendix or
        // supplement `.md`, say) has no target there. Preserve it rather
        // than silently dropping it — previously happened for every toc
        // entry beyond the article and any `.ipynb`s.
        let leftover: Vec<YamlValue> = toc
            .iter()
            .filter(|entry| {
                toc_entry_file(entry)
                    .is_some_and(|f| !f.ends_with(".ipynb") && Some(&f) != article_file.as_ref())
            })
            .cloned()
            .collect();
        if !leftover.is_empty() {
            preserved_fields.insert("toc".to_string(), YamlValue::Sequence(leftover));
            warnings.push(warn(
                "myst.yml project.toc has entries beyond the manuscript's article and \
                 notebooks (e.g. an appendix or supplement); Quarto's manuscript project shape \
                 has no equivalent, so they were preserved rather than dropped (reference §8.1)"
                    .to_string(),
            ));
        }
    } else if ptype == ProjectType::Default {
        if let Some(v) = get(project, "toc") {
            if as_sequence(v).is_some_and(|s| !s.is_empty()) {
                preserved_fields.insert("toc".to_string(), v.clone());
            }
        }
    }

    for key in UNMAPPABLE_FIELDS {
        if let Some(v) = get(project, key) {
            preserved_fields.insert((*key).to_string(), v.clone());
        }
    }

    // D10, closed for real: any `project.*` key not explicitly read above
    // (an unknown/future field — §8.2's `math` included, which has no
    // implementation here yet) is preserved through the same comment +
    // sidecar channel rather than silently vanishing.
    for (key, value) in project {
        if !HANDLED_PROJECT_KEYS.contains(&key.as_str()) && !preserved_fields.contains_key(key) {
            preserved_fields.insert(key.clone(), value.clone());
        }
    }

    // Mirrors the `project.*` handling above for `site.*`: `template` is
    // usually already represented via the inferred `project.type`
    // (`project_type::infer` only recognizes `book-theme`/`article-theme`),
    // but any other value — a custom theme name — has no other trace in the
    // output, so preserve it. Every other `site.*` key has never had any
    // handling here at all; preserve those too rather than drop them.
    let site = mapping_field(&root, "site");
    for (key, value) in site {
        if key == "template" {
            if !matches!(as_str(value), Some("book-theme") | Some("article-theme")) {
                preserved_fields.insert("site.template".to_string(), value.clone());
            }
            continue;
        }
        preserved_fields.insert(format!("site.{key}"), value.clone());
    }

    let mut text = emit(&YamlDoc(fields));
    if !preserved_fields.is_empty() {
        text.push_str("# mystquarto: no Quarto equivalent for these myst.yml fields\n");
        for (key, value) in &preserved_fields {
            for line in render_field(key, value, 0).lines() {
                text.push_str("# ");
                text.push_str(line);
                text.push('\n');
            }
        }
    }

    Ok(ConversionResult {
        text,
        project_type: ptype,
        warnings,
        preserved_fields,
    })
}

fn toc_entry_file(entry: &YamlValue) -> Option<String> {
    as_mapping(entry)
        .and_then(|m| get(m, "file"))
        .and_then(as_str)
        .or_else(|| as_str(entry))
        .map(str::to_string)
}

/// Reference §8.2's type-aware toc extension rewrite (fixes D7): `.md` ->
/// `.qmd`, `.ipynb` unchanged, no extension gets `.qmd` appended.
fn rewrite_content_extension(name: &str) -> String {
    if name.ends_with(".ipynb") {
        name.to_string()
    } else if let Some(stem) = name.strip_suffix(".md") {
        format!("{stem}.qmd")
    } else {
        format!("{name}.qmd")
    }
}

fn toc_to_chapters(toc: &[YamlValue]) -> Option<YamlValue> {
    if toc.is_empty() {
        return None;
    }
    let chapters: Vec<YamlValue> = toc
        .iter()
        .filter_map(toc_entry_file)
        .map(|f| YamlValue::String(rewrite_content_extension(&f)))
        .collect();
    Some(YamlValue::Sequence(chapters))
}

/// Reference §8.2: `authors[]` -> `author[]`, with `affiliation` (a bare
/// string MyST allows) widened to Quarto's `affiliations[].name` list-of-
/// objects shape; every other field passes through under its own name.
fn convert_authors(authors: &[YamlValue]) -> Option<YamlValue> {
    if authors.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for author in authors {
        let Some(m) = as_mapping(author) else {
            // MyST allows a bare-string author entry (just a name, no other
            // fields) — widen it to Quarto's `{name: ...}` object shape
            // rather than dropping the author entirely, matching how a
            // bare-string `affiliation` is widened below.
            if let Some(s) = as_str(author) {
                out.push(YamlValue::Mapping(vec![(
                    "name".to_string(),
                    YamlValue::String(s.to_string()),
                )]));
            }
            continue;
        };
        let mut entry = Vec::new();
        for (key, value) in m {
            if key == "affiliation" {
                let affiliations = if let Some(s) = as_str(value) {
                    YamlValue::Sequence(vec![YamlValue::Mapping(vec![(
                        "name".to_string(),
                        YamlValue::String(s.to_string()),
                    )])])
                } else {
                    value.clone()
                };
                entry.push(("affiliations".to_string(), affiliations));
            } else {
                entry.push((key.clone(), value.clone()));
            }
        }
        out.push(YamlValue::Mapping(entry));
    }
    Some(YamlValue::Sequence(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_theme_project_places_title_and_chapters_under_book() {
        let myst = "project:\n  title: Test Project\n  authors:\n    - name: Test Author\n  toc:\n    - file: intro\n    - file: methods\nsite:\n  template: book-theme\n";
        let result = convert(myst, None).unwrap();
        assert_eq!(result.project_type, ProjectType::Book);
        assert!(result.text.contains("project:\n  type: book\n"));
        assert!(result.text.contains("book:\n  title: Test Project\n"));
        assert!(result.text.contains("author:\n    - name: Test Author\n"));
        assert!(result
            .text
            .contains("chapters:\n    - intro.qmd\n    - methods.qmd\n"));
    }

    #[test]
    fn bare_string_affiliation_widens_to_a_list_of_objects() {
        let myst = "project:\n  authors:\n    - name: A\n      affiliation: Curvenote Inc.\n";
        let result = convert(myst, None).unwrap();
        assert!(result.text.contains("affiliations:\n"));
        assert!(result.text.contains("name: Curvenote Inc."));
        assert!(!result.text.contains("affiliation:"));
    }

    #[test]
    fn subject_maps_to_categories_not_description() {
        let myst = "project:\n  subject: Original Research\n  description: A real description.\n";
        let result = convert(myst, None).unwrap();
        assert!(result.text.contains("description: A real description."));
        assert!(result.text.contains("categories:\n  - Original Research\n"));
    }

    #[test]
    fn template_only_export_never_emits_format_empty() {
        let myst =
            "project:\n  exports:\n    - template: lapreprint-typst\n      article: article.md\n";
        let result = convert(myst, None).unwrap();
        assert!(!result.text.contains("format: {}"));
        assert!(result.text.contains("format:\n  typst: {}\n"));
        assert!(result.text.contains("lapreprint-typst"));
    }

    #[test]
    fn unmappable_fields_appear_as_a_comment_block_and_in_preserved_fields() {
        let myst = "project:\n  open_access: true\n  venue: The Morganton Scientific\n  id: proj-1\n  abbreviations:\n    CRISPR: gene editing\n";
        let result = convert(myst, None).unwrap();
        assert!(result
            .text
            .contains("# mystquarto: no Quarto equivalent for these myst.yml fields"));
        assert!(result.text.contains("# open_access: true"));
        assert!(result.text.contains("# venue: The Morganton Scientific"));
        assert!(result.text.contains("# id: proj-1"));
        assert!(result.text.contains("#   CRISPR: gene editing"));
        assert_eq!(result.preserved_fields.len(), 4);
        assert!(result.preserved_fields.contains_key("abbreviations"));
    }

    #[test]
    fn bibliography_is_synthesized_when_a_bib_exists_and_myst_yml_omits_it() {
        let myst = "project:\n  title: Test\n";
        let result = convert(myst, Some("references.bib")).unwrap();
        assert!(result.text.contains("bibliography: references.bib"));
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].message.contains("synthesized"));
    }

    #[test]
    fn explicit_bibliography_is_not_overridden_by_synthesis() {
        let myst = "project:\n  bibliography: my-refs.bib\n";
        let result = convert(myst, Some("references.bib")).unwrap();
        assert!(result.text.contains("bibliography: my-refs.bib"));
        assert!(!result.text.contains("references.bib"));
        assert!(result.warnings.is_empty());
    }

    /// D10's exhaustive coverage requirement, exercised against the real
    /// `article-template/myst.yml` fixture: every field present must show up
    /// either mapped or as a preservation comment — nothing silently
    /// dropped.
    #[test]
    fn article_template_fixture_drops_no_field() {
        const MYST_YML: &str = include_str!("../../../../article-template/myst.yml");
        let result = convert(MYST_YML, None).unwrap();
        let text = &result.text;

        assert_eq!(result.project_type, ProjectType::Manuscript);
        assert!(text.contains("type: manuscript"));
        assert!(text.contains(
            "title: \"Morgan's Marvelous Mutations: Unraveling the Mysteries of Genetic Variation\""
        ));
        assert!(text.contains("subtitle:"));
        assert!(text.contains("short-title: Genetic Variation"));
        assert!(text.contains("description:"));
        assert!(text.contains("categories:\n  - Original Research\n"));
        assert!(text.contains("keywords:\n  - template\n  - genetics\n"));
        assert!(text.contains("author:\n"));
        assert!(text.contains("Rowan Cockett"));
        assert!(text.contains("Thomas Hunt Morgan"));
        assert!(text.contains("repo-url: https://github.com/rowanc1/article-template"));
        assert!(text.contains("license: CC-BY-4.0"));
        assert!(text.contains("image: banner.png"));
        assert!(text.contains("format:\n  typst: {}\n"));
        assert!(text.contains("lapreprint-typst"));
        assert!(text.contains("manuscript:\n  article: article.qmd\n"));

        // Unmappable fields: comment + sidecar, never silently dropped.
        assert!(text.contains("# mystquarto: no Quarto equivalent for these myst.yml fields"));
        assert!(text.contains("# open_access: true"));
        assert!(text.contains("# venue: The Morganton Scientific"));
        assert!(text.contains("# id: morganton-2024-template"));
        assert!(text.contains("CRISPR"));
        assert_eq!(
            result.preserved_fields.keys().cloned().collect::<Vec<_>>(),
            vec!["abbreviations", "id", "open_access", "venue"]
        );

        // The actual D10 guarantee, checked mechanically rather than by
        // remembering to hand-write an assertion per field: every key
        // present in the fixture's `project`/`site` mappings must be either
        // explicitly handled by name (`HANDLED_PROJECT_KEYS`, or `site`'s
        // `template` when its value is represented via `project.type`) or
        // present in `preserved_fields` — never neither. This is what
        // catches a field that HANDLED_PROJECT_KEYS's own list of
        // hand-written `assert!(text.contains(...))` calls above happens
        // not to exercise.
        let root = parse_mapping(MYST_YML).expect("fixture is valid YAML");
        let project = mapping_field(&root, "project");
        let site = mapping_field(&root, "site");
        for (key, _) in project {
            assert!(
                HANDLED_PROJECT_KEYS.contains(&key.as_str())
                    || result.preserved_fields.contains_key(key),
                "myst.yml project.{key} is neither mapped nor preserved"
            );
        }
        for (key, _) in site {
            let represented_via_project_type = key == "template"
                && (text.contains("type: manuscript") || text.contains("type: book"));
            assert!(
                represented_via_project_type
                    || result.preserved_fields.contains_key(&format!("site.{key}")),
                "myst.yml site.{key} is neither represented nor preserved"
            );
        }
    }

    #[test]
    fn unknown_project_key_is_preserved_not_dropped() {
        // D10's actual failure mode: a key this function has never heard of
        // (a stand-in for §8.2's unimplemented `math` row, or any future
        // myst.yml field) must not silently vanish.
        let myst = "project:\n  title: T\n  math:\n    R: \\mathbb{R}\n";
        let result = convert(myst, None).unwrap();
        assert!(result.preserved_fields.contains_key("math"));
        assert!(result.text.contains("# math:"));
    }

    #[test]
    fn unrecognized_site_template_is_preserved() {
        let myst = "project:\n  title: T\nsite:\n  template: my-custom-theme\n";
        let result = convert(myst, None).unwrap();
        assert_eq!(
            result.preserved_fields.get("site.template"),
            Some(&YamlValue::String("my-custom-theme".to_string()))
        );
    }

    #[test]
    fn recognized_site_template_is_not_redundantly_preserved() {
        let myst =
            "project:\n  toc:\n    - file: a\n    - file: b\nsite:\n  template: book-theme\n";
        let result = convert(myst, None).unwrap();
        assert!(!result.preserved_fields.contains_key("site.template"));
    }

    #[test]
    fn other_site_key_is_preserved() {
        let myst = "project:\n  title: T\nsite:\n  nav:\n    - title: Home\n";
        let result = convert(myst, None).unwrap();
        assert!(result.preserved_fields.contains_key("site.nav"));
    }

    #[test]
    fn bare_string_author_is_widened_not_dropped() {
        let myst = "project:\n  authors:\n    - Rowan Cockett\n    - name: Thomas Morgan\n";
        let result = convert(myst, None).unwrap();
        assert!(
            result.text.contains("Rowan Cockett"),
            "a bare-string author must survive; got:\n{}",
            result.text
        );
        assert!(result.text.contains("Thomas Morgan"));
    }

    #[test]
    fn manuscript_toc_entry_beyond_article_and_notebooks_is_preserved() {
        let myst = "project:\n  exports:\n    - template: lapreprint-typst\n      article: article.md\n  toc:\n    - file: article.md\n    - file: appendix.md\n    - file: analysis.ipynb\n";
        let result = convert(myst, None).unwrap();
        assert_eq!(result.project_type, ProjectType::Manuscript);
        assert!(result.text.contains("notebook: analysis.ipynb"));
        let YamlValue::Sequence(leftover) = result
            .preserved_fields
            .get("toc")
            .expect("appendix.md must be preserved, not dropped")
        else {
            panic!("expected a sequence");
        };
        assert_eq!(leftover.len(), 1);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.message.contains("beyond the manuscript's article")));
    }
}
