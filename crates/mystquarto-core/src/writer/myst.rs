//! IR -> MyST writer. Emits **modern mystmd v1 only** — no legacy role ever
//! reaches output, regardless of whether it originated as legacy MyST syntax
//! (read by [`crate::MystReader`], which accepts it) or was never legacy at
//! all (a same-dialect round-trip of already-modern text). This is the
//! accepted "modern MyST only" decision (`plan.md`'s Accepted Decisions
//! table) enforced at the one place it can be violated: text rendering.
//!
//! See this module's parent for why label handling here is a `restore`
//! lookup, not a [`crate::LabelRegistry`] — that struct solves the
//! MyST->Quarto normalization problem, which does not exist in this
//! direction.

use std::path::Path;

use crate::reader::inline::InlineEvent;
use crate::writer::{push_spacing, resolve_myst_label, RestoreMap};
use crate::{AdmonitionKind, Block, BlockKind, Document, EmbedTarget, FigureSource, Label};

/// Renders a [`Document`] (from either dialect's reader) as MyST (`.md`)
/// text.
pub struct MystWriter<'a> {
    restore: &'a RestoreMap,
    known_labels: Vec<String>,
}

impl<'a> MystWriter<'a> {
    /// `restore` is `crate::registry::sidecar::restore_labels`'s output —
    /// pass an empty map for a same-dialect round-trip (labels then pass
    /// through completely unchanged, which is what makes byte-identical
    /// round-trip on the Stable class possible).
    ///
    /// `known_labels` is every label actually **defined** across the
    /// documents in this conversion set — not from a
    /// [`crate::LabelRegistry`] (that struct solves MyST->Quarto
    /// normalization, a problem that does not exist in this direction; see
    /// this module's docs), and deliberately **not** only from `restore`'s
    /// keys either: a Quarto project with no sidecar (never round-tripped)
    /// still has real, locally-defined labels that must be recognized as
    /// cross-references rather than misclassified as citations. See
    /// `crate::pipeline::labels_defined_in` for how a caller builds this
    /// list from a batch's parsed documents.
    #[must_use]
    pub fn new(restore: &'a RestoreMap, known_labels: Vec<String>) -> Self {
        Self {
            restore,
            known_labels,
        }
    }

    #[must_use]
    pub fn write(&self, doc: &Document) -> String {
        let mut out = String::new();
        if let Some(fm) = &doc.frontmatter {
            out.push_str("---\n");
            out.push_str(&fm.raw);
            out.push_str("---\n");
        }
        for (i, block) in doc.blocks.iter().enumerate() {
            push_spacing(
                &mut out,
                block.blank_lines_before,
                i == 0 && doc.frontmatter.is_none(),
            );
            if doc.frontmatter.is_some() && i == 0 {
                out.push('\n');
            }
            let rendered = self.render_block(block, &doc.source);
            out.push_str(&rendered.join("\n"));
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }

    fn rewrite(&self, lines: &[String]) -> Vec<String> {
        crate::writer::rewrite_lines(lines, &self.known_labels, |event| self.render_event(event))
    }

    fn render_event(&self, event: InlineEvent) -> Option<String> {
        match event {
            // `rewrite_line` never invokes `render` for a `Citation` event
            // — it always copies modern citation syntax through verbatim
            // itself (reference §4: identical in both dialects). This arm
            // exists only so the match is exhaustive.
            InlineEvent::Citation(_) => None,
            // A cross-reference token passes through unchanged unless a
            // restore map (built from the sidecar) says otherwise — see
            // `resolve_reference_label`.
            InlineEvent::CrossReference(key) => {
                let restored = self.resolve_reference_label(&key);
                (restored != key).then(|| format!("@{restored}"))
            }
            InlineEvent::LegacyRole { role, target } => render_role_to_myst(&role, &target),
            // Modern MyST inline eval — `{eval}`expr`` — regardless of
            // which Quarto engine tag the source used, since MyST's own
            // eval role carries no engine parameter (reference §5).
            InlineEvent::JupyterEval { expr, .. } => Some(format!("{{eval}}`{expr}`")),
            // Quarto knitr-only inline syntax has no MyST equivalent
            // (reference §6): rendered as MyST's eval role, same as the
            // Jupyter case, since MyST only ever executes via Jupyter — an
            // `IRkernel` (or equivalent) must exist for this to actually
            // run, which Phase 7's diagnostics will warn about once it
            // exists.
            InlineEvent::KnitrEval(expr) => Some(format!("{{eval}}`{expr}`")),
        }
    }

    /// Looks up `key` (a raw reference token as it appeared in the source
    /// being converted — either a MyST colon-label or a Quarto hyphen-id)
    /// against `restore`, trying every source file recorded there (the
    /// same cross-file resolution [`crate::LabelRegistry::resolve_reference`]
    /// does, but over the restore map instead of a registry). Falls back to
    /// `key` unchanged when nothing matches.
    fn resolve_reference_label(&self, key: &str) -> String {
        self.restore
            .iter()
            .find(|((_, id), _)| id == key)
            .map(|(_, label)| label.raw.clone())
            .unwrap_or_else(|| key.to_string())
    }

    fn render_block(&self, block: &Block, source: &Path) -> Vec<String> {
        match &block.kind {
            BlockKind::Heading { level, text, label } => {
                self.heading(*level, text, label.as_ref(), source)
            }
            BlockKind::Paragraph { lines } => self.rewrite(lines),
            BlockKind::CodeCell {
                lang,
                options,
                body,
                label,
            } => self.code_cell(lang, options, body, label.as_ref(), source),
            BlockKind::StaticCode { lang, body, .. } => static_code(lang, body),
            BlockKind::Figure {
                src,
                caption,
                label,
                attrs,
            } => self.figure(src, caption, label.as_ref(), attrs, source),
            BlockKind::Table {
                caption,
                rows,
                label,
            } => self.table(caption, rows, label.as_ref(), source),
            BlockKind::Math { body, label } => self.math(body, label.as_ref(), source),
            BlockKind::Admonition {
                kind,
                title,
                body,
                collapse,
            } => self.admonition(*kind, title.as_deref(), body, *collapse, source),
            BlockKind::TabSet { items } => self.tab_set(items, source),
            BlockKind::Margin { body } => self.directive_wrap("margin", body, source),
            BlockKind::Include { target, .. } => {
                vec![format!("```{{include}} {}", target.display())]
            }
            BlockKind::Embed { target, label } => self.embed(target, label.as_ref(), source),
            BlockKind::Blockquote { body, attribution } => {
                self.blockquote(body, attribution.as_deref(), source)
            }
            BlockKind::Theorem {
                thm_type,
                label,
                body,
            } => self.theorem(thm_type, label.as_ref(), body, source),
            BlockKind::Directive {
                name, body, label, ..
            } => self.directive(name, label.as_ref(), body, source),
            BlockKind::Comment { text, .. } => vec![format!("% {text}")],
            BlockKind::Target { label } => {
                vec![format!("({})=", self.myst_label(source, label).raw)]
            }
            BlockKind::Raw { format, body } => raw(format, body),
            BlockKind::BlockBreak => vec!["+++".to_string()],
            BlockKind::Preserved { original, .. } => original.clone(),
            BlockKind::Unmappable { original, reason } => preserved(reason, original),
        }
    }

    fn myst_label(&self, source: &Path, label: &Label) -> Label {
        resolve_myst_label(source, label, self.restore)
    }

    fn heading(&self, level: u8, text: &str, label: Option<&Label>, source: &Path) -> Vec<String> {
        let text = self
            .rewrite(std::slice::from_ref(&text.to_string()))
            .remove(0);
        let hashes = "#".repeat(level as usize);
        match label {
            Some(l) => vec![
                format!("({})=", self.myst_label(source, l).raw),
                String::new(),
                format!("{hashes} {text}"),
            ],
            None => vec![format!("{hashes} {text}")],
        }
    }

    fn code_cell(
        &self,
        lang: &str,
        options: &crate::CellOptions,
        body: &[String],
        label: Option<&Label>,
        source: &Path,
    ) -> Vec<String> {
        let mut out = vec![format!("```{{code-cell}} {lang}")];
        if let Some(l) = label {
            out.push(format!(":label: {}", self.myst_label(source, l).raw));
        }
        if !options.tags.is_empty() {
            out.push(format!(":tags: [{}]", options.tags.join(", ")));
        }
        if let Some(caption) = &options.caption {
            out.push(format!(":caption: {caption}"));
        }
        if out.len() > 1 {
            out.push(String::new());
        }
        out.extend(body.iter().cloned());
        out.push("```".to_string());
        out
    }

    fn figure(
        &self,
        src: &FigureSource,
        caption: &[String],
        label: Option<&Label>,
        attrs: &std::collections::BTreeMap<String, String>,
        source: &Path,
    ) -> Vec<String> {
        let arg = match src {
            FigureSource::Path(p) => p.display().to_string(),
            FigureSource::CellRef { label, .. } => format!("#{}", label.raw),
        };
        let mut out = vec![format!(":::{{figure}} {arg}")];
        if let Some(l) = label {
            out.push(format!(":label: {}", self.myst_label(source, l).raw));
        }
        for (k, v) in attrs {
            out.push(format!(":{k}: {v}"));
        }
        if !caption.is_empty() {
            out.push(String::new());
            out.extend(self.rewrite(caption));
        }
        out.push(":::".to_string());
        out
    }

    fn table(
        &self,
        caption: &[String],
        rows: &[String],
        label: Option<&Label>,
        source: &Path,
    ) -> Vec<String> {
        let mut out = vec![":::{table}".to_string()];
        if let Some(l) = label {
            out.push(format!(":label: {}", self.myst_label(source, l).raw));
        }
        out.push(String::new());
        out.extend(self.rewrite(caption));
        if !caption.is_empty() {
            out.push(String::new());
        }
        out.extend(rows.iter().cloned());
        out.push(":::".to_string());
        out
    }

    fn math(&self, body: &[String], label: Option<&Label>, source: &Path) -> Vec<String> {
        let mut out = vec!["```{math}".to_string()];
        if let Some(l) = label {
            out.push(format!(":label: {}", self.myst_label(source, l).raw));
            out.push(String::new());
        }
        out.extend(body.iter().cloned());
        out.push("```".to_string());
        out
    }

    fn admonition(
        &self,
        kind: AdmonitionKind,
        title: Option<&str>,
        body: &[Block],
        collapse: Option<bool>,
        source: &Path,
    ) -> Vec<String> {
        let name = admonition_name(kind);
        let mut out = if let Some(title) = title {
            vec![format!("```{{admonition}} {title}")]
        } else {
            vec![format!("```{{{name}}}")]
        };
        if let Some(collapse) = collapse {
            out.push(":class: dropdown".to_string());
            out.push(format!(":open: {}", !collapse));
        }
        out.extend(self.render_nested(body, source));
        out.push("```".to_string());
        out
    }

    fn tab_set(&self, items: &[crate::TabItem], source: &Path) -> Vec<String> {
        let mut out = vec!["::::{tab-set}".to_string()];
        for item in items {
            out.push(format!(":::{{tab-item}} {}", item.label));
            out.extend(self.render_nested(&item.body, source));
            out.push(":::".to_string());
        }
        out.push("::::".to_string());
        out
    }

    fn embed(&self, target: &EmbedTarget, label: Option<&Label>, source: &Path) -> Vec<String> {
        let arg = match target {
            EmbedTarget::Label(l) => l.raw.clone(),
            EmbedTarget::NotebookCell { cell_label, .. } => cell_label.raw.clone(),
        };
        let mut out = vec![format!("```{{embed}} #{arg}")];
        if let Some(l) = label {
            out.push(format!(":label: {}", self.myst_label(source, l).raw));
        }
        out.push("```".to_string());
        out
    }

    fn blockquote(
        &self,
        body: &[Block],
        attribution: Option<&[String]>,
        source: &Path,
    ) -> Vec<String> {
        let mut out = vec!["```{blockquote}".to_string()];
        out.extend(self.render_nested(body, source));
        if let Some(attribution) = attribution {
            for line in attribution {
                out.push(format!("-- {line}"));
            }
        }
        out.push("```".to_string());
        out
    }

    fn theorem(
        &self,
        thm_type: &str,
        label: Option<&Label>,
        body: &[Block],
        source: &Path,
    ) -> Vec<String> {
        let mut out = vec![format!("```{{prf:{thm_type}}}")];
        if let Some(l) = label {
            out.push(format!(":label: {}", self.myst_label(source, l).raw));
        }
        out.extend(self.render_nested(body, source));
        out.push("```".to_string());
        out
    }

    fn directive_wrap(&self, name: &str, body: &[Block], source: &Path) -> Vec<String> {
        let mut out = vec![format!("```{{{name}}}")];
        out.extend(self.render_nested(body, source));
        out.push("```".to_string());
        out
    }

    fn directive(
        &self,
        name: &str,
        label: Option<&Label>,
        body: &[Block],
        source: &Path,
    ) -> Vec<String> {
        let mut out = vec![format!("```{{{name}}}")];
        if let Some(l) = label {
            out.push(format!(":label: {}", self.myst_label(source, l).raw));
        }
        out.extend(self.render_nested(body, source));
        out.push("```".to_string());
        out
    }

    fn render_nested(&self, body: &[Block], source: &Path) -> Vec<String> {
        crate::writer::render_body(body, |b| self.render_block(b, source))
    }
}

fn admonition_name(kind: AdmonitionKind) -> &'static str {
    match kind {
        AdmonitionKind::Note => "note",
        AdmonitionKind::Warning => "warning",
        AdmonitionKind::Tip => "tip",
        AdmonitionKind::Important => "important",
        AdmonitionKind::Caution => "caution",
        AdmonitionKind::Danger => "danger",
        AdmonitionKind::Error => "error",
        AdmonitionKind::Hint => "hint",
        AdmonitionKind::SeeAlso => "seealso",
        AdmonitionKind::Attention => "attention",
    }
}

fn static_code(lang: &Option<String>, body: &[String]) -> Vec<String> {
    if lang.as_deref() == Some("mermaid") {
        let mut out = vec!["```{mermaid}".to_string()];
        out.extend(body.iter().cloned());
        out.push("```".to_string());
        return out;
    }
    let mut out = vec![format!("```{}", lang.as_deref().unwrap_or(""))];
    out.extend(body.iter().cloned());
    out.push("```".to_string());
    out
}

fn raw(format: &str, body: &[String]) -> Vec<String> {
    let mut out = vec![format!("```{{raw}} {format}")];
    out.extend(body.iter().cloned());
    out.push("```".to_string());
    out
}

/// See `writer::quarto::preserved`'s docs — same RT-02 rationale: unmappable
/// content goes inside a fenced code block, never an HTML comment, so it can
/// never be interpreted as markup regardless of blank lines it contains.
fn preserved(note: &str, original: &[String]) -> Vec<String> {
    let mut out = vec![format!("% mystquarto: {note}"), "```text".to_string()];
    out.extend(original.iter().cloned());
    out.push("```".to_string());
    out
}

/// Renders every `{name}`content`` form to **modern MyST**, never
/// re-emitting the legacy spelling — this is the one function a grep-based
/// test can point at to prove "no MyST output contains any legacy role."
fn render_role_to_myst(role: &str, target: &str) -> Option<String> {
    Some(match role {
        "cite" | "cite:p" => format!("[@{target}]"),
        "cite:t" => format!("@{target}"),
        "numref" | "ref" => format!("@{target}"),
        "eq" => {
            let label = if target.starts_with("eq-") || target.starts_with("eq:") {
                target.to_string()
            } else {
                format!("eq-{target}")
            };
            format!("@{label}")
        }
        "doc" => {
            let stem = target.strip_suffix(".qmd").unwrap_or(target);
            format!("[{target}]({stem}.md)")
        }
        // Already-modern MyST role syntax: passes through unchanged (this
        // function only exists to guarantee *legacy* forms never survive;
        // modern roles are, definitionally, already correct MyST).
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Attrs, Engine};
    use crate::{PreservationStore, ReaderContext, Span};

    fn block(kind: BlockKind, blank: u8) -> Block {
        Block {
            kind,
            span: Span::single(1),
            blank_lines_before: blank,
        }
    }

    #[test]
    fn same_dialect_round_trip_is_byte_identical_on_a_modern_myst_paragraph() {
        let input = "(sec:data)=\n\n## Data\n\nSome prose with [@smith2020] and @sec:data.\n";
        let reader = crate::MystReader::new(ReaderContext::new("a.md"));
        let doc = reader.read_str(input).unwrap();
        let restore = RestoreMap::new();
        let writer = MystWriter::new(&restore, Vec::new());
        let out = writer.write(&doc);
        assert_eq!(out, input);
    }

    #[test]
    fn legacy_cite_role_is_rewritten_to_modern_bracket_syntax() {
        let doc = Document {
            frontmatter: None,
            blocks: vec![block(
                BlockKind::Paragraph {
                    lines: vec!["See {cite}`smith2020` here.".to_string()],
                },
                0,
            )],
            source: "a.md".into(),
            engine: Some(Engine::Jupyter),
        };
        let restore = RestoreMap::new();
        let writer = MystWriter::new(&restore, Vec::new());
        let out = writer.write(&doc);
        assert_eq!(out.trim(), "See [@smith2020] here.");
        assert!(!out.contains("{cite}"));
    }

    #[test]
    fn no_legacy_role_syntax_survives_a_battery_of_legacy_inputs() {
        let text = "\
{cite}`a` and {cite:t}`b` and {cite:p}`c`.
{numref}`fig-x` and {ref}`label` and {eq}`eq-y`.
{doc}`other.md`
";
        let reader = crate::MystReader::new(ReaderContext::new("a.md"));
        let doc = reader.read_str(text).unwrap();
        let restore = RestoreMap::new();
        let writer = MystWriter::new(&restore, Vec::new());
        let out = writer.write(&doc);
        for legacy in [
            "{cite}", "{cite:t}", "{cite:p}", "{numref}", "{ref}", "{eq}", "{doc}",
        ] {
            assert!(
                !out.contains(legacy),
                "found legacy role {legacy} in:\n{out}"
            );
        }
    }

    #[test]
    fn sidecar_restored_label_is_used_instead_of_the_quarto_id() {
        let doc = Document {
            frontmatter: None,
            blocks: vec![block(
                BlockKind::Figure {
                    src: FigureSource::Path("img.png".into()),
                    caption: vec![],
                    label: Some(Label::new("fig-samples")),
                    attrs: Attrs::new(),
                },
                0,
            )],
            source: "article.md".into(),
            engine: Some(Engine::Jupyter),
        };
        let mut restore = RestoreMap::new();
        restore.insert(
            ("article.md".into(), "fig-samples".to_string()),
            Label::new("fig:samples"),
        );
        let writer = MystWriter::new(&restore, Vec::new());
        let out = writer.write(&doc);
        assert!(out.contains(":label: fig:samples"));
    }

    #[test]
    fn preserved_block_reader_writer_round_trip() {
        let mut preserved = PreservationStore::default();
        preserved.insert("abc", vec!["% restored comment".to_string()]);
        let reader = crate::MystReader::new(ReaderContext {
            preserved,
            ..ReaderContext::new("a.md")
        });
        let doc = reader
            .read_str("<!-- mystquarto MQ0203: x see .mystquarto/preserved.json#abc -->\n")
            .unwrap();
        let restore = RestoreMap::new();
        let writer = MystWriter::new(&restore, Vec::new());
        let out = writer.write(&doc);
        assert_eq!(out.trim(), "% restored comment");
    }
}
