//! `myst.yml` <-> `_quarto.yml` project configuration mapping (reference
//! §8.1-8.3). Fixes D6 (`format: {}` from a template-only export), D7
//! (extension rewriting that ignores `.ipynb`), D8 (book vs manuscript
//! misdetection), D10 (silently dropped unmappable fields), and RT-14 (no
//! bibliography synthesis, so citations render as literal text even though
//! `quarto render` exits 0).
//!
//! Page-level frontmatter (reference §8.4) is [`crate::frontmatter`], a
//! separate module: it edits an existing [`crate::ir::Frontmatter::raw`] via
//! [`crate::yaml::surgery`], where this module *synthesizes* a whole
//! `_quarto.yml`/`myst.yml` from scratch via [`crate::yaml::emit`] — the same
//! reading-vs-synthesis split the `yaml` module's own docs describe.
//!
//! Every function here operates on `myst.yml`'s parsed shape — a top-level
//! mapping with `project`, `site`; §8.2's field rows are all `project.*`. The
//! accessor helpers below exist because [`YamlValue`] (deliberately, see its
//! own docs) has no query methods of its own — adding them here, `pub(crate)`
//! and scoped to what config lookups need, keeps that type's public surface
//! unchanged.

pub mod bibliography;
pub mod exports;
pub mod myst_to_quarto;
pub mod project_type;
pub mod quarto_to_myst;
pub mod sidecar;

pub use project_type::ProjectType;

use crate::yaml::YamlValue;

pub use crate::diagnostics::{Diagnostic, Severity};

/// Builds a config-sourced [`Diagnostic`]. `span` defaults to line 1 (see
/// [`crate::diagnostics`]'s module docs on why config/frontmatter-sourced
/// diagnostics have no precise line number: neither `crate::yaml`'s parser
/// nor `crate::yaml::surgery` track source positions).
pub(crate) fn warn(
    severity: Severity,
    code: &'static str,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic::new(severity, code, message)
}

/// Looks up `key` in an order-preserving parsed mapping — the shape
/// [`crate::yaml::parse_mapping`] returns and every accessor below consumes.
pub(crate) fn get<'a>(mapping: &'a [(String, YamlValue)], key: &str) -> Option<&'a YamlValue> {
    mapping.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

pub(crate) fn as_str(v: &YamlValue) -> Option<&str> {
    match v {
        YamlValue::String(s) | YamlValue::BlockLiteral(s) => Some(s.as_str()),
        _ => None,
    }
}

pub(crate) fn as_mapping(v: &YamlValue) -> Option<&[(String, YamlValue)]> {
    match v {
        YamlValue::Mapping(m) => Some(m.as_slice()),
        _ => None,
    }
}

pub(crate) fn as_sequence(v: &YamlValue) -> Option<&[YamlValue]> {
    match v {
        YamlValue::Sequence(s) => Some(s.as_slice()),
        _ => None,
    }
}

/// `get(mapping, key)` narrowed to a nested mapping, defaulting to empty —
/// the common case of reaching into `project`/`site`/an `authors[]` entry
/// without a chain of `Option` matches at every call site.
pub(crate) fn mapping_field<'a>(
    mapping: &'a [(String, YamlValue)],
    key: &str,
) -> &'a [(String, YamlValue)] {
    get(mapping, key).and_then(as_mapping).unwrap_or(&[])
}

/// `get(mapping, key)` narrowed to a sequence, defaulting to empty.
pub(crate) fn sequence_field<'a>(mapping: &'a [(String, YamlValue)], key: &str) -> &'a [YamlValue] {
    get(mapping, key).and_then(as_sequence).unwrap_or(&[])
}

/// `get(mapping, key)` narrowed to a string, as an owned value (both
/// `String` and `BlockLiteral` read the same way — see [`as_str`]).
pub(crate) fn string_field(mapping: &[(String, YamlValue)], key: &str) -> Option<String> {
    get(mapping, key).and_then(as_str).map(str::to_string)
}
