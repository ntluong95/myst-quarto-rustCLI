//! YAML strategy for `mystquarto`.
//!
//! Two independent, narrow mechanisms — deliberately **not** a general YAML
//! round-trip library. The phase spec's "YAML strategy" section verified
//! against the vendored crate that `saphyr`'s own emitter panics on exactly
//! the case it would be used to fix:
//!
//! ```text
//! saphyr-0.0.12/src/emitter.rs:241: ScalarStyle::Literal => todo!(),
//! saphyr-0.0.12/src/emitter.rs:242: ScalarStyle::Folded  => todo!(),
//! ```
//!
//! `saphyr` / `saphyr-parser` are therefore used here for **reading only**
//! ([`parse_mapping`], safe YAML 1.2 parsing with no alias expansion); their
//! emitter (`YamlEmitter`) is never called anywhere in this crate.
//!
//! - [`surgery`]: targeted line-level edits to an **existing** frontmatter
//!   block, so untouched keys — including multi-line block scalars,
//!   comments, and key order — survive byte-identically because they are
//!   never re-serialized. This is the fix for D9
//!   (`tests/corpus/defects/d09-block-scalar-mangled/`).
//! - [`emit`]: a bounded, deterministic emitter for **synthesizing** YAML
//!   from scratch (e.g. `_quarto.yml` from a parsed `myst.yml`), where there
//!   is no original text to anchor edits to. It is exercised here against a
//!   small demonstration struct, not a general value tree — Phase 6 defines
//!   the real `QuartoConfig` / `MystConfig` structs (reference §8.2) and
//!   calls this emitter directly. Keeping it typed against a closed key set
//!   rather than an open value tree is deliberate: see the phase spec's Risk
//!   Assessment ("the bounded YAML emitter grows into a general one").
//!
//! [`YamlValue`] is the value type shared by both mechanisms and by
//! [`crate::ir::Frontmatter`]'s parsed view. It is intentionally not a fully
//! general YAML value type — it covers exactly what reference §8.2/§8.4's
//! key set needs: strings, strings forced to block-literal style, integers,
//! floats, bools, null, sequences, and order-preserving mappings.

pub mod emit;
pub mod surgery;

use saphyr::{LoadableYamlNode, Scalar, Yaml};

/// A minimal, typed YAML value — not a general YAML value tree (see module
/// docs). Mappings use an order-preserving `Vec<(String, YamlValue)>`
/// rather than a hash map, because preserving key order is one of this
/// crate's explicit requirements (reference §8.4).
#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
    /// A plain scalar string, emitted unquoted or quoted as needed by
    /// [`emit`]/[`surgery`].
    String(String),
    /// A string forced to emit as a YAML block-literal (`|`) scalar,
    /// regardless of content. The **reading** path ([`parse_mapping`]) never
    /// produces this variant — block style is a write-side instruction, not
    /// something inferred from a parsed value. It exists so
    /// [`emit`]/[`surgery`] callers can force `|` style (e.g. for
    /// `abstract`, reference §8.4).
    BlockLiteral(String),
    /// A YAML integer (core schema `!!int`).
    Int(i64),
    /// A YAML floating point number (core schema `!!float`).
    Float(f64),
    /// A YAML boolean (core schema `!!bool`: `true`/`false` only — not the
    /// YAML 1.1 `yes`/`no`/`on`/`off` forms, which the core schema resolves
    /// as strings).
    Bool(bool),
    /// A YAML null (`~`, `null`, or an empty value).
    Null,
    /// A YAML sequence (block `- item` or flow `[a, b]` — style is a write
    /// concern, not carried on this value).
    Sequence(Vec<YamlValue>),
    /// An order-preserving mapping: `(key, value)` pairs in document order
    /// on read, or in caller-specified order on write.
    Mapping(Vec<(String, YamlValue)>),
}

/// Error produced by [`parse_mapping`].
#[derive(Debug)]
pub enum YamlReadError {
    /// The underlying YAML text failed to scan/parse.
    Scan(String),
    /// The document parsed, but its top level was not a single mapping (the
    /// input contained zero or more than one YAML document, or the single
    /// document's root was not a mapping).
    NotAMapping,
}

impl std::fmt::Display for YamlReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            YamlReadError::Scan(msg) => write!(f, "YAML scan error: {msg}"),
            YamlReadError::NotAMapping => write!(f, "top-level YAML node is not a mapping"),
        }
    }
}

impl std::error::Error for YamlReadError {}

/// Parses `text` — the content of a frontmatter block (without the `---`
/// delimiters), or any other single-document YAML mapping — using
/// `saphyr`'s safe YAML 1.2 parser: no alias expansion (so no
/// billion-laughs), and Core Schema scalar resolution, so `open_access: no`
/// reads as the string `"no"`, not a bool (YAML 1.1's `yes`/`no`/`on`/`off`
/// booleans are not part of the YAML 1.2 Core Schema `saphyr` implements).
///
/// Returns an order-preserving `Vec<(String, YamlValue)>` — `saphyr`'s
/// `Mapping` iterates in document order, and this function keeps that order
/// rather than sorting.
///
/// This is the **reading** path only (see module docs): callers that need
/// to change a value and write it back out should use [`surgery`] (existing
/// frontmatter) or [`emit`] (synthesizing new YAML), never a
/// parse-then-re-emit round trip through this function's return value.
///
/// # Errors
/// Returns [`YamlReadError::Scan`] if `text` is not well-formed YAML, or
/// [`YamlReadError::NotAMapping`] if it does not parse to exactly one
/// document whose root is a mapping.
pub fn parse_mapping(text: &str) -> Result<Vec<(String, YamlValue)>, YamlReadError> {
    let docs = Yaml::load_from_str(text).map_err(|e| YamlReadError::Scan(e.to_string()))?;
    let doc = match docs.as_slice() {
        [single] => single,
        _ => return Err(YamlReadError::NotAMapping),
    };
    let Yaml::Mapping(mapping) = doc else {
        return Err(YamlReadError::NotAMapping);
    };
    let mut out = Vec::with_capacity(mapping.len());
    for (k, v) in mapping.iter() {
        let key = k.as_str().ok_or(YamlReadError::NotAMapping)?.to_string();
        out.push((key, yaml_to_value(v)));
    }
    Ok(out)
}

/// Converts a parsed `saphyr::Yaml` node into our minimal [`YamlValue`].
/// `Representation`/`Alias`/`BadValue` nodes only occur with non-default
/// loader settings or malformed input respectively; `saphyr`'s default
/// loader (used by [`parse_mapping`] via `load_from_str`) eagerly resolves
/// scalars, so in practice only `Value`, `Sequence`, `Mapping`, and
/// `Tagged` appear. The remaining arms are handled defensively rather than
/// treated as unreachable, since a future `saphyr` version or an unusual
/// document could still produce them.
fn yaml_to_value(y: &Yaml) -> YamlValue {
    match y {
        Yaml::Value(Scalar::Null) => YamlValue::Null,
        Yaml::Value(Scalar::Boolean(b)) => YamlValue::Bool(*b),
        Yaml::Value(Scalar::Integer(i)) => YamlValue::Int(*i),
        Yaml::Value(Scalar::FloatingPoint(f)) => YamlValue::Float(f.0),
        Yaml::Value(Scalar::String(s)) => YamlValue::String(s.to_string()),
        Yaml::Sequence(seq) => YamlValue::Sequence(seq.iter().map(yaml_to_value).collect()),
        Yaml::Mapping(map) => YamlValue::Mapping(
            map.iter()
                .map(|(k, v)| (k.as_str().unwrap_or_default().to_string(), yaml_to_value(v)))
                .collect(),
        ),
        Yaml::Tagged(_, inner) => yaml_to_value(inner),
        Yaml::Representation(s, _, _) => YamlValue::String(s.to_string()),
        Yaml::Alias(_) | Yaml::BadValue => YamlValue::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_access_no_reads_as_string_not_bool() {
        let parsed = parse_mapping("open_access: no\n").expect("valid YAML");
        assert_eq!(
            parsed,
            vec![(
                "open_access".to_string(),
                YamlValue::String("no".to_string())
            )]
        );
    }

    #[test]
    fn yaml_1_1_boolean_words_all_read_as_strings() {
        // Core Schema (YAML 1.2) booleans are `true`/`false` only. `yes`,
        // `on`, `off`, etc. are YAML 1.1 booleans and must read as strings.
        for word in ["yes", "no", "on", "off", "Yes", "No", "On", "Off"] {
            let text = format!("k: {word}\n");
            let parsed = parse_mapping(&text).unwrap_or_else(|e| panic!("{word}: {e}"));
            assert_eq!(
                parsed,
                vec![("k".to_string(), YamlValue::String(word.to_string()))],
                "{word} must parse as a string under the Core Schema"
            );
        }
    }

    #[test]
    fn true_false_read_as_bool() {
        let parsed = parse_mapping("a: true\nb: false\n").expect("valid YAML");
        assert_eq!(
            parsed,
            vec![
                ("a".to_string(), YamlValue::Bool(true)),
                ("b".to_string(), YamlValue::Bool(false)),
            ]
        );
    }

    #[test]
    fn preserves_document_key_order() {
        let parsed = parse_mapping("z: 1\na: 2\nm: 3\n").expect("valid YAML");
        let keys: Vec<&str> = parsed.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["z", "a", "m"]);
    }

    #[test]
    fn block_literal_reads_as_plain_string_value() {
        // The parsed *value* has no memory of block-literal style — that is
        // exactly why surgery.rs operates on original text rather than a
        // re-emitted parsed value.
        let text = "abstract: |\n  line one\n  line two\n";
        let parsed = parse_mapping(text).expect("valid YAML");
        assert_eq!(
            parsed,
            vec![(
                "abstract".to_string(),
                YamlValue::String("line one\nline two\n".to_string())
            )]
        );
    }

    #[test]
    fn nested_sequence_and_mapping_read_correctly() {
        let text = "authors:\n  - name: Ada\n    orcid: \"0000\"\n  - name: Bob\n";
        let parsed = parse_mapping(text).expect("valid YAML");
        let YamlValue::Sequence(authors) = &parsed[0].1 else {
            panic!("expected sequence");
        };
        assert_eq!(authors.len(), 2);
        let YamlValue::Mapping(first) = &authors[0] else {
            panic!("expected mapping");
        };
        assert_eq!(
            first[0],
            ("name".to_string(), YamlValue::String("Ada".to_string()))
        );
    }

    #[test]
    fn non_mapping_top_level_is_rejected() {
        assert!(matches!(
            parse_mapping("[1, 2, 3]"),
            Err(YamlReadError::NotAMapping)
        ));
    }

    #[test]
    fn malformed_yaml_is_rejected() {
        assert!(matches!(
            parse_mapping("key: [unclosed"),
            Err(YamlReadError::Scan(_))
        ));
    }
}
