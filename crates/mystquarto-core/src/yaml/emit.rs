//! Bounded, deterministic YAML emitter for **synthesizing** documents from
//! scratch — the config-synthesis half of the YAML strategy (see the
//! [module docs](super)). Used when there is no original text to anchor
//! edits to (unlike [`super::surgery`]), e.g. producing `_quarto.yml` from
//! a parsed `myst.yml`.
//!
//! This is deliberately **not** a general YAML value-tree emitter. It is
//! exercised here only against [`YamlDoc`], an ordered `Vec` of fields with
//! an optional comment each — not against arbitrary/open-ended structures.
//! Phase 6 (per the phase spec) defines the real `QuartoConfig` /
//! `MystConfig` structs (reference §8.2) and is expected to build a
//! `YamlDoc` from them field-by-field, then call [`emit`]. Keeping the
//! input to this module a flat, caller-constructed `Vec` — rather than
//! accepting `impl Serialize` or a generic value tree — is what keeps
//! "unsupported shapes" a compile-time concern for Phase 6's struct, not a
//! feature request against this emitter (see the phase spec's Risk
//! Assessment: "the bounded YAML emitter grows into a general one").
//!
//! Supports: block-literal (`|`) scalar emission, key order as given by the
//! caller (not alphabetized — callers pass a `Vec`, so struct-declaration
//! order is whatever order the caller builds the `Vec` in), nested
//! mappings, sequences (including sequences of small mapping objects, e.g.
//! `authors[]`), and one comment line (or several, one per `\n`-separated
//! line) emitted directly above a given top-level key.

use super::YamlValue;

/// One top-level field to emit: a key, its value, and an optional comment
/// rendered as `# <comment>` on the line(s) immediately above it. Comment
/// anchoring is intentionally this simple — "above a given key" — not a
/// general comment-attachment mechanism.
#[derive(Debug, Clone, PartialEq)]
pub struct EmitField {
    pub key: String,
    pub value: YamlValue,
    pub comment: Option<String>,
}

impl EmitField {
    /// Builds a field with no comment.
    #[must_use]
    pub fn new(key: impl Into<String>, value: YamlValue) -> Self {
        EmitField {
            key: key.into(),
            value,
            comment: None,
        }
    }

    /// Attaches a comment, rendered above this field's line(s).
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }
}

/// An ordered document to emit: top-level fields in caller-specified order.
/// Order-preservation is why this is a `Vec` rather than a `HashMap` —
/// `serde` alone does not guarantee map order for arbitrary maps, and key
/// order preservation is an explicit requirement (reference §8.4).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct YamlDoc(pub Vec<EmitField>);

/// Emits `doc` deterministically: same input always produces the same
/// output string, byte for byte.
#[must_use]
pub fn emit(doc: &YamlDoc) -> String {
    let mut out = String::new();
    for field in &doc.0 {
        if let Some(comment) = &field.comment {
            for line in comment.lines() {
                out.push_str("# ");
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push_str(&render_field(&field.key, &field.value, 0));
        out.push('\n');
    }
    out
}

fn indent_str(n: usize) -> String {
    " ".repeat(n)
}

/// Renders one `key: value` field (possibly spanning multiple lines for
/// block scalars, sequences, or nested mappings) at the given indent level.
/// The returned string has no trailing newline; callers add one between
/// fields.
pub(crate) fn render_field(key: &str, value: &YamlValue, indent: usize) -> String {
    let pad = indent_str(indent);
    match value {
        YamlValue::String(s) => format!("{pad}{key}: {}", quote_scalar_string(s)),
        YamlValue::BlockLiteral(s) => render_block_literal(key, s, indent),
        YamlValue::Int(i) => format!("{pad}{key}: {i}"),
        YamlValue::Float(f) => format!("{pad}{key}: {f}"),
        YamlValue::Bool(b) => format!("{pad}{key}: {b}"),
        YamlValue::Null => format!("{pad}{key}: null"),
        YamlValue::Sequence(items) => render_sequence_field(key, items, indent),
        YamlValue::Mapping(fields) => render_mapping_field(key, fields, indent),
    }
}

fn render_block_literal(key: &str, content: &str, indent: usize) -> String {
    let pad = indent_str(indent);
    let inner_pad = indent_str(indent + 2);
    // A trailing `\n` on `content` is the normal "clamp" case (single
    // trailing newline, plain `|`); strip exactly one so we don't emit a
    // spurious blank final line, matching how a single logical trailing
    // newline round-trips through block-literal style.
    let body = content.strip_suffix('\n').unwrap_or(content);
    let mut out = format!("{pad}{key}: |\n");
    if body.is_empty() {
        // Nothing to indent; leave the bare `key: |`.
        out.pop();
        return out;
    }
    let lines: Vec<&str> = body.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        out.push_str(&inner_pad);
        out.push_str(line);
        if i + 1 != lines.len() {
            out.push('\n');
        }
    }
    out
}

fn render_sequence_field(key: &str, items: &[YamlValue], indent: usize) -> String {
    let pad = indent_str(indent);
    if items.is_empty() {
        return format!("{pad}{key}: []");
    }
    let mut out = format!("{pad}{key}:\n");
    for (i, item) in items.iter().enumerate() {
        out.push_str(&render_sequence_item(item, indent + 2));
        if i + 1 != items.len() {
            out.push('\n');
        }
    }
    out
}

fn render_mapping_field(key: &str, fields: &[(String, YamlValue)], indent: usize) -> String {
    let pad = indent_str(indent);
    if fields.is_empty() {
        return format!("{pad}{key}: {{}}");
    }
    let mut out = format!("{pad}{key}:\n");
    for (i, (k, v)) in fields.iter().enumerate() {
        out.push_str(&render_field(k, v, indent + 2));
        if i + 1 != fields.len() {
            out.push('\n');
        }
    }
    out
}

/// Renders one `- item` line of a block sequence. Mapping items (e.g. one
/// `authors[]` entry) render their first key on the `- ` line and align
/// subsequent keys under it, matching conventional block-style YAML:
/// ```yaml
/// authors:
///   - name: Ada
///     orcid: "0000"
/// ```
fn render_sequence_item(item: &YamlValue, indent: usize) -> String {
    let pad = indent_str(indent);
    match item {
        YamlValue::Mapping(fields) if !fields.is_empty() => {
            let mut out = String::new();
            for (i, (k, v)) in fields.iter().enumerate() {
                let rendered = render_field(k, v, indent + 2);
                let trimmed = rendered.trim_start_matches(' ');
                if i == 0 {
                    out.push_str(&pad);
                    out.push_str("- ");
                    out.push_str(trimmed);
                } else {
                    out.push('\n');
                    out.push_str(&rendered);
                }
            }
            out
        }
        YamlValue::Sequence(items) if !items.is_empty() => {
            let mut out = format!("{pad}-\n");
            for (i, sub) in items.iter().enumerate() {
                out.push_str(&render_sequence_item(sub, indent + 2));
                if i + 1 != items.len() {
                    out.push('\n');
                }
            }
            out
        }
        scalar => format!("{pad}- {}", render_scalar_inline(scalar)),
    }
}

fn render_scalar_inline(value: &YamlValue) -> String {
    match value {
        YamlValue::String(s) => quote_scalar_string(s),
        // A forced block-literal inside a sequence item is outside this
        // bounded emitter's scope (reference §8.2/§8.4's key set has no
        // such shape); fall back to a quoted plain scalar rather than
        // silently dropping newlines.
        YamlValue::BlockLiteral(s) => quote_scalar_string(s),
        YamlValue::Int(i) => i.to_string(),
        YamlValue::Float(f) => f.to_string(),
        YamlValue::Bool(b) => b.to_string(),
        YamlValue::Null => "null".to_string(),
        YamlValue::Sequence(_) | YamlValue::Mapping(_) => {
            unreachable!("sequence/mapping items are handled by render_sequence_item directly")
        }
    }
}

/// Quotes a plain string scalar when emitting it unquoted would change its
/// parsed meaning or be invalid YAML: empty strings, reserved words that
/// resolve to bool/null under the Core Schema, strings that parse as a
/// number, strings starting with an indicator character, strings
/// containing `": "` (would be read as a nested mapping), and strings with
/// interior newlines (never valid as a plain scalar).
fn quote_scalar_string(s: &str) -> String {
    if needs_quoting(s) {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn needs_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off"
    ) {
        return true;
    }
    if s.parse::<f64>().is_ok() {
        return true;
    }
    let first = s.chars().next().expect("checked non-empty above");
    if "!&*-?|>%@`\"'#,[]{}:".contains(first) {
        return true;
    }
    if s.contains(": ") || s.ends_with(':') || s.contains(" #") || s.contains('\n') {
        return true;
    }
    if s.trim() != s {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_string_field_emits_unquoted() {
        let doc = YamlDoc(vec![EmitField::new(
            "title",
            YamlValue::String("Sample Article".into()),
        )]);
        assert_eq!(emit(&doc), "title: Sample Article\n");
    }

    #[test]
    fn block_literal_field_emits_pipe_style() {
        let doc = YamlDoc(vec![EmitField::new(
            "abstract",
            YamlValue::BlockLiteral(
                "This is a multi-line abstract\nwith a hard line break preserved\nby the block literal style.\n"
                    .into(),
            ),
        )]);
        let expected = "abstract: |\n  This is a multi-line abstract\n  with a hard line break preserved\n  by the block literal style.\n";
        assert_eq!(emit(&doc), expected);
    }

    #[test]
    fn nested_mapping_field_emits_indented_block() {
        let doc = YamlDoc(vec![EmitField::new(
            "project",
            YamlValue::Mapping(vec![
                (
                    "type".to_string(),
                    YamlValue::String("manuscript".to_string()),
                ),
                (
                    "output-dir".to_string(),
                    YamlValue::String("_output".to_string()),
                ),
            ]),
        )]);
        let expected = "project:\n  type: manuscript\n  output-dir: _output\n";
        assert_eq!(emit(&doc), expected);
    }

    #[test]
    fn list_of_strings_emits_dash_items() {
        let doc = YamlDoc(vec![EmitField::new(
            "keywords",
            YamlValue::Sequence(vec![
                YamlValue::String("genomics".into()),
                YamlValue::String("phenotype".into()),
            ]),
        )]);
        let expected = "keywords:\n  - genomics\n  - phenotype\n";
        assert_eq!(emit(&doc), expected);
    }

    #[test]
    fn list_of_small_objects_emits_authors_style() {
        let doc = YamlDoc(vec![EmitField::new(
            "author",
            YamlValue::Sequence(vec![
                YamlValue::Mapping(vec![
                    (
                        "name".to_string(),
                        YamlValue::String("Ada Lovelace".to_string()),
                    ),
                    (
                        "orcid".to_string(),
                        YamlValue::String("0000-0000".to_string()),
                    ),
                ]),
                YamlValue::Mapping(vec![(
                    "name".to_string(),
                    YamlValue::String("Bob".to_string()),
                )]),
            ]),
        )]);
        let expected = "author:\n  - name: Ada Lovelace\n    orcid: 0000-0000\n  - name: Bob\n";
        assert_eq!(emit(&doc), expected);
    }

    #[test]
    fn comment_emits_above_key() {
        let doc = YamlDoc(vec![
            EmitField::new("title", YamlValue::String("Sample".into())),
            EmitField::new("open_access", YamlValue::String("no".into())).with_comment(
                "preserved from myst.yml; no Quarto equivalent (reference \u{00a7}8.2)",
            ),
        ]);
        let expected = "title: Sample\n# preserved from myst.yml; no Quarto equivalent (reference \u{00a7}8.2)\nopen_access: \"no\"\n";
        assert_eq!(emit(&doc), expected);
    }

    #[test]
    fn key_order_matches_vec_order_not_alphabetical() {
        let doc = YamlDoc(vec![
            EmitField::new("zeta", YamlValue::Int(1)),
            EmitField::new("alpha", YamlValue::Int(2)),
        ]);
        let out = emit(&doc);
        assert!(out.find("zeta").unwrap() < out.find("alpha").unwrap());
    }

    #[test]
    fn open_access_no_string_round_trips_quoted() {
        // Proves the emitter and the reader (parse_mapping) agree: a
        // YamlValue::String("no") that reads back as "no", not `false`.
        let doc = YamlDoc(vec![EmitField::new(
            "open_access",
            YamlValue::String("no".into()),
        )]);
        let text = emit(&doc);
        let parsed = super::super::parse_mapping(&text).expect("emitted YAML must parse");
        assert_eq!(
            parsed,
            vec![(
                "open_access".to_string(),
                YamlValue::String("no".to_string())
            )]
        );
    }

    #[test]
    fn emit_is_deterministic() {
        let doc = YamlDoc(vec![
            EmitField::new("title", YamlValue::String("Sample".into())),
            EmitField::new(
                "author",
                YamlValue::Sequence(vec![YamlValue::Mapping(vec![(
                    "name".to_string(),
                    YamlValue::String("Ada".to_string()),
                )])]),
            ),
            EmitField::new(
                "abstract",
                YamlValue::BlockLiteral("line one\nline two\n".into()),
            ),
        ]);
        assert_eq!(emit(&doc), emit(&doc));
    }

    /// A small, clearly-scoped stand-in for the real `QuartoConfig` /
    /// `MystConfig` structs Phase 6 will define (reference §8.2). It exists
    /// only to prove the emitter mechanism against something struct-shaped
    /// rather than a hand-built `YamlDoc` for every test — Phase 6 is
    /// expected to write an equivalent `to_yaml_doc` for its real config
    /// types, not reuse this one.
    struct DemoConfig {
        title: String,
        keywords: Vec<String>,
        project: Vec<(String, String)>,
        abstract_text: String,
    }

    impl DemoConfig {
        fn to_yaml_doc(&self) -> YamlDoc {
            YamlDoc(vec![
                EmitField::new("title", YamlValue::String(self.title.clone())),
                EmitField::new(
                    "keywords",
                    YamlValue::Sequence(
                        self.keywords
                            .iter()
                            .cloned()
                            .map(YamlValue::String)
                            .collect(),
                    ),
                ),
                EmitField::new(
                    "project",
                    YamlValue::Mapping(
                        self.project
                            .iter()
                            .map(|(k, v)| (k.clone(), YamlValue::String(v.clone())))
                            .collect(),
                    ),
                ),
                EmitField::new(
                    "abstract",
                    YamlValue::BlockLiteral(self.abstract_text.clone()),
                ),
            ])
        }
    }

    #[test]
    fn demo_config_exercises_all_four_required_shapes() {
        let cfg = DemoConfig {
            title: "Sample Article".to_string(),
            keywords: vec!["genomics".to_string(), "phenotype".to_string()],
            project: vec![("type".to_string(), "manuscript".to_string())],
            abstract_text: "line one\nline two\n".to_string(),
        };
        let out = emit(&cfg.to_yaml_doc());
        assert_eq!(
            out,
            "title: Sample Article\n\
             keywords:\n  - genomics\n  - phenotype\n\
             project:\n  type: manuscript\n\
             abstract: |\n  line one\n  line two\n"
        );
    }
}
