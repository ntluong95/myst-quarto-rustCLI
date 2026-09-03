//! Typed accessors over `mappings.toml`, the MyST ↔ Quarto conversion
//! contract transcribed from `docs/dialect-comparison.md`.
//!
//! The TOML file is embedded at compile time via `include_str!` and parsed
//! once into a lazily-initialized static. Two `HashMap` indices give O(1)
//! lookup of directive mappings by MyST name and by Quarto target, in place
//! of a linear scan over the vector.

use std::collections::HashMap;
use std::sync::LazyLock;

/// Fidelity class for a single conversion rule, matching
/// `docs/dialect-comparison.md`'s legend: ✅ -> `exact`, ⚠️ -> `lossy`,
/// ❌ / ➖(no-equivalent) -> `unmappable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fidelity {
    Exact,
    Lossy,
    Unmappable,
}

/// §2 non-structural block construct: a directive/attribute name swap.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DirectiveMapping {
    pub myst: String,
    pub quarto: Option<String>,
    pub fidelity: Fidelity,
    pub note: Option<String>,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// §2 structural construct that requires hand-written transform code.
/// Recorded only so diagnostics can look up its fidelity class — there is
/// no `quarto` target field because there is no name-swap to record.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct StructuralMapping {
    pub myst: String,
    pub fidelity: Fidelity,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// §5 inline construct expressed as MyST role syntax (`` {name}`...` ``).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RoleMapping {
    pub myst: Option<String>,
    pub quarto: Option<String>,
    pub fidelity: Fidelity,
    pub note: Option<String>,
    #[serde(default)]
    pub legacy: bool,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// §5 inline construct expressed as plain (non-role) syntax.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct InlineMapping {
    pub myst: String,
    pub quarto: String,
    pub fidelity: Fidelity,
    pub note: Option<String>,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// §8.2 `myst.yml` (`project.*`) ↔ `_quarto.yml` field mapping.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConfigFieldMapping {
    pub myst: Option<String>,
    pub quarto: Option<String>,
    pub fidelity: Fidelity,
    pub note: Option<String>,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// §8.3 MyST `exports[]` entry ↔ Quarto `format` map entry.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExportFormatMapping {
    pub myst: String,
    pub quarto: Option<String>,
    pub fidelity: Fidelity,
    pub note: Option<String>,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// §10 legacy (Sphinx/Jupyter Book v1) construct: accepted on read, never
/// emitted. Records the legacy syntax, its modern-MyST equivalent, and what
/// the writer emits into Quarto.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LegacyRoleMapping {
    pub myst: String,
    pub modern_myst: String,
    pub quarto: String,
    pub fidelity: Fidelity,
    pub note: Option<String>,
    #[serde(default)]
    pub legacy: bool,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// §3.3 colon-prefix → hyphen-prefix label normalization rule.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LabelPrefixMapping {
    pub myst: String,
    pub quarto: String,
    pub fidelity: Fidelity,
    pub note: Option<String>,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// The full parsed contents of `mappings.toml`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Mappings {
    #[serde(default)]
    pub directive: Vec<DirectiveMapping>,
    #[serde(default)]
    pub structural: Vec<StructuralMapping>,
    #[serde(default)]
    pub role: Vec<RoleMapping>,
    #[serde(default)]
    pub inline: Vec<InlineMapping>,
    #[serde(default)]
    pub config_field: Vec<ConfigFieldMapping>,
    #[serde(default)]
    pub export_format: Vec<ExportFormatMapping>,
    #[serde(default)]
    pub legacy_role: Vec<LegacyRoleMapping>,
    #[serde(default)]
    pub label_prefix: Vec<LabelPrefixMapping>,
}

/// The raw `mappings.toml` source, embedded at compile time.
pub const MAPPINGS_TOML: &str = include_str!("../../../mappings.toml");

static MAPPINGS: LazyLock<Mappings> = LazyLock::new(|| {
    toml::from_str(MAPPINGS_TOML).expect("mappings.toml must parse into Mappings")
});

/// Returns the parsed, static conversion contract.
pub fn mappings() -> &'static Mappings {
    &MAPPINGS
}

static DIRECTIVE_BY_MYST: LazyLock<HashMap<&'static str, &'static DirectiveMapping>> =
    LazyLock::new(|| {
        mappings()
            .directive
            .iter()
            .map(|d| (d.myst.as_str(), d))
            .collect()
    });

// Several MyST directives collapse onto the same Quarto target (e.g. `note`,
// `hint`, `seealso` and `attention` all render as `callout-note`; see the
// admonition rows transcribed from §2). Building this index with
// `entry(..).or_insert(..)` keeps the *first* — canonical — match in
// `mappings.toml`'s array order rather than silently letting a later,
// lossy collapse-target overwrite it, as a plain `collect()` would.
static DIRECTIVE_BY_QUARTO: LazyLock<HashMap<&'static str, &'static DirectiveMapping>> =
    LazyLock::new(|| {
        let mut index = HashMap::new();
        for d in &mappings().directive {
            if let Some(q) = d.quarto.as_deref() {
                index.entry(q).or_insert(d);
            }
        }
        index
    });

/// Looks up a §2 directive mapping by its MyST name (e.g. `"note"`,
/// `"code-cell"`). O(1) via a pre-built index, not a linear scan.
pub fn directive_by_myst_name(name: &str) -> Option<&'static DirectiveMapping> {
    DIRECTIVE_BY_MYST.get(name).copied()
}

/// Looks up a §2 directive mapping by its Quarto target (e.g.
/// `"callout-note"`). O(1) via a pre-built index, not a linear scan.
pub fn directive_by_quarto_class(class: &str) -> Option<&'static DirectiveMapping> {
    DIRECTIVE_BY_QUARTO.get(class).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_table_kinds() {
        let m = mappings();
        assert_eq!(m.directive.len(), 33);
        assert_eq!(m.structural.len(), 11);
        assert_eq!(m.role.len(), 11);
        assert_eq!(m.inline.len(), 5);
        assert_eq!(m.config_field.len(), 32);
        assert_eq!(m.export_format.len(), 6);
        assert_eq!(m.legacy_role.len(), 9);
        assert_eq!(m.label_prefix.len(), 22);
    }

    #[test]
    fn directive_lookup_is_indexed_both_directions() {
        let by_name = directive_by_myst_name("note").expect("note directive exists");
        assert_eq!(by_name.quarto.as_deref(), Some("callout-note"));
        assert_eq!(by_name.fidelity, Fidelity::Exact);

        let by_class = directive_by_quarto_class("callout-note").expect("callout-note exists");
        assert_eq!(by_class.myst, "note");

        assert!(directive_by_myst_name("does-not-exist").is_none());
        assert!(directive_by_quarto_class("does-not-exist").is_none());
    }
}
