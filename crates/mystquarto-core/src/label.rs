//! A typed home for cross-reference labels.
//!
//! This is deliberately a **stub type**, not a stub algorithm: normalizing
//! MyST's free-form `kind:name` labels into Quarto's constrained
//! `kind-name` identifiers (colon→hyphen, the `tab:`→`tbl-` exception,
//! `_`→`-`, lowercasing, collision suffixing — reference §3.3/§3.4) is
//! explicitly Phase 5's job per the plan's phase table. `Label` here just
//! gives every labelable `BlockKind` variant (`Heading`, `Figure`, `Table`,
//! `Math`, `CodeCell`, `Target`, …) a place to carry whatever string the
//! reader saw, unmodified, so Phase 4's readers have something to
//! construct and Phase 5's normalizer has something to consume.

/// A cross-reference label as read from the source, before any
/// normalization. `raw` holds exactly what the source wrote — e.g.
/// `"fig:samples"` from MyST's `:label: fig:samples`, or `"fig-samples"`
/// from Quarto's `{#fig-samples}` — with no colon/hyphen rewriting applied.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label {
    /// The label exactly as it appeared in the source.
    pub raw: String,
}

impl Label {
    /// Wraps a raw label string as read from the source.
    pub fn new(raw: impl Into<String>) -> Self {
        Label { raw: raw.into() }
    }
}

impl From<String> for Label {
    fn from(raw: String) -> Self {
        Label { raw }
    }
}

impl From<&str> for Label {
    fn from(raw: &str) -> Self {
        Label::new(raw)
    }
}

impl std::fmt::Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::Label;

    #[test]
    fn preserves_raw_label_unmodified() {
        let l = Label::new("fig:samples");
        assert_eq!(l.raw, "fig:samples");
        assert_eq!(l.to_string(), "fig:samples");
    }

    #[test]
    fn from_str_and_from_string_agree() {
        assert_eq!(
            Label::from("tab:results"),
            Label::from("tab:results".to_string())
        );
    }
}
