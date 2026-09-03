//! Reference §8.1 project-type inference (fixes D8). MyST has no `type`
//! field; Quarto's `project.type` selects an entire rendering pipeline, so it
//! must be inferred from other signals.
//!
//! The Python implementation ([`super`]'s module docs) treats **any**
//! `project.toc` as a book, mis-typing `article-template/` (an article with
//! `site.template: article-theme` and a `lapreprint-typst` export) as `book`.
//! These four ordered rules — first match wins — fix that.

use super::{as_mapping, as_str, get, mapping_field, sequence_field};
use crate::yaml::YamlValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectType {
    Book,
    Manuscript,
    Default,
}

/// Infers the Quarto project type from a parsed `myst.yml`'s top-level
/// mapping (as returned by [`crate::yaml::parse_mapping`]).
#[must_use]
pub fn infer(myst_root: &[(String, YamlValue)]) -> ProjectType {
    let project = mapping_field(myst_root, "project");
    let site = mapping_field(myst_root, "site");

    if get(site, "template").and_then(as_str) == Some("book-theme") {
        return ProjectType::Book;
    }

    let has_export_template = sequence_field(project, "exports")
        .iter()
        .any(|e| as_mapping(e).and_then(|m| get(m, "template")).is_some());
    if has_export_template || get(site, "template").and_then(as_str) == Some("article-theme") {
        return ProjectType::Manuscript;
    }

    if sequence_field(project, "toc").len() >= 2 {
        return ProjectType::Book;
    }

    ProjectType::Default
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::parse_mapping;

    fn infer_text(text: &str) -> ProjectType {
        infer(&parse_mapping(text).expect("valid YAML"))
    }

    #[test]
    fn book_theme_site_template_wins_outright() {
        assert_eq!(
            infer_text("project:\n  toc: []\nsite:\n  template: book-theme\n"),
            ProjectType::Book
        );
    }

    #[test]
    fn export_template_makes_a_manuscript() {
        assert_eq!(
            infer_text("project:\n  exports:\n    - template: lapreprint-typst\n"),
            ProjectType::Manuscript
        );
    }

    #[test]
    fn article_theme_site_template_makes_a_manuscript() {
        assert_eq!(
            infer_text("project:\n  toc: []\nsite:\n  template: article-theme\n"),
            ProjectType::Manuscript
        );
    }

    #[test]
    fn toc_with_two_or_more_entries_and_no_article_template_is_a_book() {
        assert_eq!(
            infer_text("project:\n  toc:\n    - file: a\n    - file: b\n"),
            ProjectType::Book
        );
    }

    #[test]
    fn toc_with_one_entry_is_default() {
        assert_eq!(
            infer_text("project:\n  toc:\n    - file: a\n"),
            ProjectType::Default
        );
    }

    #[test]
    fn no_signals_at_all_is_default() {
        assert_eq!(
            infer_text("project:\n  title: Solo page\n"),
            ProjectType::Default
        );
    }

    /// D8's real-world regression: `article-template/myst.yml` has both an
    /// `exports[].template` and `site.template: article-theme`, and its
    /// `toc` has two entries — under the Python implementation's "any toc
    /// means book" rule this becomes `book`; the correct answer, and what
    /// these ordered rules produce, is `manuscript` (rule 2 fires before
    /// rule 3 is even reached).
    #[test]
    fn article_template_fixture_is_a_manuscript_not_a_book() {
        const MYST_YML: &str = include_str!("../../../../article-template/myst.yml");
        assert_eq!(infer_text(MYST_YML), ProjectType::Manuscript);
    }
}
