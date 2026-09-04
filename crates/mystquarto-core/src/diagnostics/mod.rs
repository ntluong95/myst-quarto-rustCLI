//! Unified lossy-conversion diagnostics (reference §11, decisions RD-4/RT-10).
//!
//! Before this module, four ad hoc "warning" types (`pipeline::BatchWarning`,
//! `config::ConfigWarning`, `registry::RegistryWarning`,
//! `registry::sidecar::SidecarWarning`) each carried a bare `message: String`
//! with no severity or stable code — every one of their doc comments said
//! "Phase 7 does not exist yet to assign these real diagnostic codes." This
//! module is that assignment: each of those types now also carries a
//! [`Severity`] and a stable `code`, so `--strict`/`--strict=all` can promote
//! by class and a user can baseline a specific code in `suppress.toml`
//! without string-matching message text.
//!
//! [`Diagnostic`] is the type every one of those four converts into at the
//! CLI boundary (`crate::orchestrate::RunReport`, in the `mystquarto` binary
//! crate) — the point where they are finally rendered and where `--strict`
//! decides the exit code.
//!
//! # Deviations from the phase spec's literal `Diagnostic` struct
//!
//! - `file` is `Option<PathBuf>`, not `PathBuf`: a handful of existing
//!   diagnostics are run-scoped rather than tied to one file (e.g. "the
//!   label sidecar itself is stale"), matching `BatchWarning`'s existing
//!   shape rather than inventing a placeholder path.
//! - `span` is always present, but for a diagnostic sourced from
//!   `myst.yml`/`_quarto.yml`/page-frontmatter mapping (`crate::config`,
//!   `crate::frontmatter`) it is `Span::single(1)` — neither the YAML parser
//!   (`crate::yaml`) nor the frontmatter surgery module tracks source line
//!   numbers, so "line" for those diagnostics means "the config file", not a
//!   precise position. Block-sourced diagnostics (readers/writers, which
//!   operate on `crate::Block`, always carrying a real `crate::Span`) get a
//!   precise line.

use std::path::PathBuf;

use crate::Span;

pub mod codes;

/// The four-class severity policy (reference §11, RD-4). Ordered so
/// `Severity::Error > Severity::Warning` etc. holds — used to decide
/// `--strict`/`--strict=all` promotion by comparison rather than a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Purely informational — never fails a run, `--strict` or not (e.g. a
    /// label was normalized, or a reverse conversion found no label
    /// sidecar to restore from).
    Info,
    /// A conversion that is correct *and* inherently lossy — the formats
    /// differ, not a defect. Fails only `--strict=all`.
    LossyExpected,
    /// A conversion that lost or could not resolve something a user would
    /// reasonably expect to keep working. Fails `--strict` and
    /// `--strict=all`.
    Warning,
    /// A file could not be read/written, or a path-safety check refused an
    /// operation. Already surfaced today via `FileStatus::Failed` /
    /// `BatchFileError`, which already always fail a run — no code in this
    /// crate currently constructs a `Diagnostic` at `Error` severity; the
    /// variant exists so the type is complete and so a future direct
    /// `Diagnostic`-emitting error path has somewhere to report at.
    Error,
}

impl Severity {
    /// The word used in human output (`warning[MQ0201]`, `lossy[MQ0203]`, …).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::LossyExpected => "lossy",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// One lossy-conversion notice: what happened, how severe, where, and (for
/// a preserved construct) where to find the original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// A stable code from [`codes`], e.g. `"MQ0203"`. Stable across runs and
    /// versions so a user can suppress a class in CI (`suppress.toml`) or a
    /// corpus test can assert on the code rather than message text.
    pub code: &'static str,
    pub message: String,
    /// `None` for a run-scoped notice not tied to one file — see module docs.
    pub file: Option<PathBuf>,
    /// See module docs: precise for block-sourced diagnostics, `Span::single(1)`
    /// for config/frontmatter-sourced ones (no source-position tracking there).
    pub span: Span,
    /// A `docs/dialect-comparison.md` section, e.g. `"§8.2"` — the reference
    /// line in human output.
    pub reference: Option<&'static str>,
    /// Set only for a `LossyExpected` diagnostic backed by a
    /// `.mystquarto/preserved.json` entry — the id to look up there.
    pub preserved: Option<String>,
}

impl Diagnostic {
    #[must_use]
    pub fn new(severity: Severity, code: &'static str, message: impl Into<String>) -> Self {
        Diagnostic {
            severity,
            code,
            message: message.into(),
            file: None,
            span: Span::single(1),
            reference: None,
            preserved: None,
        }
    }

    #[must_use]
    pub fn with_file(mut self, file: impl Into<PathBuf>) -> Self {
        self.file = Some(file.into());
        self
    }

    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = span;
        self
    }

    #[must_use]
    pub fn with_reference(mut self, reference: &'static str) -> Self {
        self.reference = Some(reference);
        self
    }

    #[must_use]
    pub fn with_preserved(mut self, id: impl Into<String>) -> Self {
        self.preserved = Some(id.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ordering_promotes_error_above_warning_above_lossy_above_info() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::LossyExpected);
        assert!(Severity::LossyExpected > Severity::Info);
    }

    #[test]
    fn builder_sets_every_optional_field() {
        let d = Diagnostic::new(Severity::Warning, "MQ0301", "citation missing")
            .with_file("article.md")
            .with_span(Span::new(10, 12))
            .with_reference("§8.3")
            .with_preserved("fnv1a:deadbeef");
        assert_eq!(d.file, Some(PathBuf::from("article.md")));
        assert_eq!(d.span, Span::new(10, 12));
        assert_eq!(d.reference, Some("§8.3"));
        assert_eq!(d.preserved, Some("fnv1a:deadbeef".to_string()));
    }

    #[test]
    fn default_span_is_line_one() {
        let d = Diagnostic::new(Severity::Info, "MQ0101", "x");
        assert_eq!(d.span, Span::single(1));
        assert_eq!(d.file, None);
    }
}
