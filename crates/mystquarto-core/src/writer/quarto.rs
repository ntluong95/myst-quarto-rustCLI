//! IR -> Quarto writer. Fixes D1 (dead cross-references), D2 (dropped
//! figure labels), D3 (dropped table captions/labels), and D11 (broken
//! notebook-embed image links) — see each render function's doc comment for
//! which defect it closes and why the old Python transform lost the
//! information.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use crate::diagnostics::Diagnostic;
use crate::preserve::PreservedEntry;
use crate::reader::inline::InlineEvent;
use crate::registry::normalize;
use crate::writer::{known_reference_labels, push_spacing, rewrite_lines};
use crate::{AdmonitionKind, Block, BlockKind, Document, EmbedTarget, FigureSource, LabelRegistry};

/// Renders one MyST-sourced [`Document`] as Quarto (`.qmd`) text.
pub struct QuartoWriter<'a> {
    registry: &'a LabelRegistry,
    known_labels: Vec<String>,
    /// Accumulated by [`crate::writer::render_preserved`] while rendering
    /// one document — drained (not cleared-on-next-call state; see
    /// `write`) at the end of [`write`](Self::write). A side channel rather
    /// than a `render_block` parameter because `render_block` recurses
    /// through many block kinds (admonitions, tab sets, block quotes, …)
    /// and a preserved/unmappable construct can appear at any depth.
    preserved: RefCell<BTreeMap<String, PreservedEntry>>,
    diagnostics: RefCell<Vec<Diagnostic>>,
}

impl<'a> QuartoWriter<'a> {
    #[must_use]
    pub fn new(registry: &'a LabelRegistry) -> Self {
        Self {
            registry,
            known_labels: known_reference_labels(registry),
            preserved: RefCell::new(BTreeMap::new()),
            diagnostics: RefCell::new(Vec::new()),
        }
    }

    /// Renders `doc` to a complete `.qmd` file's text, plus any non-fatal
    /// diagnostics (frontmatter drops at §8.4, plus one per preserved
    /// block — reference §11) and every preservation-sidecar entry a
    /// preserved/unmappable block in `doc` produced (reference RD-2/RT-02;
    /// see [`crate::preserve`]). Frontmatter is mapped field-by-field —
    /// `kernelspec` -> `jupyter`/`engine`, `exports` -> `format`, etc. —
    /// with every untouched key (critically `abstract`, D9) surviving
    /// byte-identically.
    #[must_use]
    pub fn write(
        &self,
        doc: &Document,
    ) -> (String, Vec<Diagnostic>, BTreeMap<String, PreservedEntry>) {
        let mut out = String::new();
        let mut warnings = Vec::new();
        if let Some(fm) = &doc.frontmatter {
            let (mapped, fm_warnings) = crate::frontmatter::myst_to_quarto(fm);
            warnings = fm_warnings;
            out.push_str("---\n");
            out.push_str(&mapped);
            out.push_str("---\n");
        }
        for (i, block) in doc.blocks.iter().enumerate() {
            push_spacing(
                &mut out,
                block.blank_lines_before,
                i == 0 && doc.frontmatter.is_none(),
            );
            if doc.frontmatter.is_some() && i == 0 {
                out.push('\n'); // one blank line after frontmatter, always
            }
            let rendered = self.render_block(block, &doc.source);
            out.push_str(&rendered.join("\n"));
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
        warnings.extend(self.diagnostics.take());
        (out, warnings, self.preserved.take())
    }

    /// `source` is the file whose text is being rewritten — needed so a
    /// cross-reference token resolves against *this* file's own label
    /// definitions first (see [`crate::LabelRegistry::resolve_reference`]'s
    /// docs on why that must take priority over any project-wide guess:
    /// otherwise a reference inside a file whose own label got
    /// collision-suffixed resolves to the *wrong* id).
    fn rewrite(&self, lines: &[String], source: &Path) -> Vec<String> {
        rewrite_lines(lines, &self.known_labels, |event| {
            self.render_event(event, source)
        })
    }

    /// The dialect-specific half of inline rewriting (reference §4/§5/§10):
    /// legacy MyST roles rendered directly to their Quarto form (never via
    /// an intermediate "modern MyST" string), cross-references normalized
    /// through the registry, and Jupyter/knitr eval forms passed through or
    /// translated as needed.
    fn render_event(&self, event: InlineEvent, source: &Path) -> Option<String> {
        match event {
            // See `writer::myst`'s identical arm: `rewrite_line` never
            // invokes `render` for `Citation` (it copies modern citation
            // syntax through verbatim itself). Exists for exhaustiveness.
            InlineEvent::Citation(_) => None,
            InlineEvent::CrossReference(key) => Some(format!(
                "@{}",
                self.registry.resolve_reference(source, &key)
            )),
            InlineEvent::LegacyRole { role, target } => {
                Some(render_role_to_quarto(&role, &target, self.registry, source))
            }
            InlineEvent::JupyterEval { engine, expr } => Some(format!("`{{{engine}}} {expr}`")),
            InlineEvent::KnitrEval(expr) => {
                // knitr has no MyST origin (MyST only reads modern
                // `{eval}` roles, caught above as JupyterEval via
                // read_braced_eval) — a KnitrEval event here means this
                // text was *itself* Quarto-native knitr syntax being
                // passed through a same-dialect-adjacent path (e.g. a
                // `Preserved` block being re-emitted). Pass through
                // unchanged; it is already correct Quarto syntax.
                Some(format!("`r {expr}`"))
            }
        }
    }

    fn render_block(&self, block: &Block, source: &Path) -> Vec<String> {
        match &block.kind {
            BlockKind::Heading { level, text, label } => {
                self.heading(*level, text, label.as_ref(), source)
            }
            BlockKind::Paragraph { lines } => self.rewrite(lines, source),
            BlockKind::CodeCell {
                lang,
                options,
                body,
                label,
            } => self.code_cell(lang, options, body, label.as_ref(), source),
            BlockKind::StaticCode { lang, body, attrs } => static_code(lang, body, attrs),
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
            BlockKind::Margin { body } => self.wrapped_div("column-margin", body, source),
            BlockKind::Include { target, .. } => vec![include_shortcode(target)],
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
                name,
                attrs,
                body,
                label,
            } => self.directive(name, attrs, body, label.as_ref(), source),
            BlockKind::Comment { text, .. } => vec![format!("<!-- {text} -->")],
            BlockKind::Target { .. } => Vec::new(), // no general Quarto anchor; see writer/mod.rs docs
            BlockKind::Raw { format, body } => raw(format, body),
            BlockKind::BlockBreak => vec![
                "<!-- mystquarto: MyST block break (+++) has no Quarto equivalent -->".to_string(),
            ],
            // Empty `original` is the "missing-sidecar-entry"/"foreign
            // dialect, but no real content to fall back to" degrade —
            // `render_preserved` handles it (a fresh marker + a Warning
            // diagnostic), see its docs. The non-empty arm right below
            // passes through verbatim — see `writer::myst::MystWriter`'s
            // identical arm for why that's correct (C2 fix).
            BlockKind::Preserved { original, .. } if original.is_empty() => {
                vec![crate::writer::render_preserved(
                    &crate::writer::PreserveSink {
                        preserved: &self.preserved,
                        diagnostics: &self.diagnostics,
                    },
                    source,
                    block.span,
                    "content",
                    crate::writer::PreservedDisposition {
                        code: crate::diagnostics::codes::block::PRESERVED_RESTORED_OPAQUE,
                        severity: crate::diagnostics::Severity::LossyExpected,
                        dialect: crate::preserve::Dialect::Unknown,
                    },
                    original.clone(),
                )]
            }
            BlockKind::Preserved { original, .. } => original.clone(),
            // `QuartoWriter`'s sole caller (`convert_myst_to_quarto_batch`)
            BlockKind::Unmappable { original, reason } => {
                let is_path_safety = reason.contains("escapes")
                    || reason.contains("cycle")
                    || reason.contains("depth")
                    || reason.contains("absolute path");
                vec![crate::writer::render_preserved(
                    &crate::writer::PreserveSink {
                        preserved: &self.preserved,
                        diagnostics: &self.diagnostics,
                    },
                    source,
                    block.span,
                    &crate::writer::preserved_kind(reason),
                    crate::writer::PreservedDisposition {
                        code: if is_path_safety {
                            crate::diagnostics::codes::io::PATH_SAFETY_REFUSED
                        } else {
                            crate::diagnostics::codes::block::UNMAPPABLE_PRESERVED
                        },
                        severity: if is_path_safety {
                            crate::diagnostics::Severity::Warning
                        } else {
                            crate::diagnostics::Severity::LossyExpected
                        },
                        dialect: crate::preserve::Dialect::Myst,
                    },
                    original.clone(),
                )]
            }
        }
    }

    fn heading(
        &self,
        level: u8,
        text: &str,
        label: Option<&crate::Label>,
        source: &Path,
    ) -> Vec<String> {
        let text = self
            .rewrite(std::slice::from_ref(&text.to_string()), source)
            .remove(0);
        let hashes = "#".repeat(level as usize);
        match label.and_then(|l| self.registry.quarto_id(source, l)) {
            Some(id) => vec![format!("{hashes} {text} {{#{id}}}")],
            None => vec![format!("{hashes} {text}")],
        }
    }

    fn code_cell(
        &self,
        lang: &str,
        options: &crate::CellOptions,
        body: &[String],
        label: Option<&crate::Label>,
        source: &Path,
    ) -> Vec<String> {
        let lang = if lang == "ipython3" { "python" } else { lang };
        let mut out = vec![format!("```{{{lang}}}")];
        if let Some(id) = label.and_then(|l| self.registry.quarto_id(source, l)) {
            out.push(format!("#| label: {id}"));
        }
        for tag in &options.tags {
            if let Some(opt) = tag_to_cell_option(tag) {
                out.push(opt.to_string());
            }
        }
        if let Some(caption) = &options.caption {
            out.push(format!("#| fig-cap: \"{}\"", escape_quoted(caption)));
        }
        for (k, v) in &options.extra {
            out.push(format!("#| {k}: {v}"));
        }
        out.extend(body.iter().cloned());
        out.push("```".to_string());
        out
    }

    fn figure(
        &self,
        src: &FigureSource,
        caption: &[String],
        label: Option<&crate::Label>,
        attrs: &BTreeMap<String, String>,
        source: &Path,
    ) -> Vec<String> {
        let id = label.and_then(|l| self.registry.quarto_id(source, l));
        match src {
            FigureSource::CellRef {
                label: cell_label,
                notebook,
            } => {
                // D11: the notebook-cell embed. `embed_id` is what both the
                // `{{< embed >}}` anchor *and* the notebook cell's own
                // relabelled `#| label:` (`crate::notebook::relabel`, run by
                // the orchestration layer against the *output* copy) must
                // agree on — this writer decides the id, orchestration
                // applies it to the notebook.
                //
                // Verified against `quarto 1.9.36`: **there is no
                // document-level alias for an embedded output.** An earlier
                // version of this function tried emitting a second `#id`
                // after the cell anchor (`{{< embed nb#cell-id #fig-x >}}`),
                // assuming that would give the embed an extra crossref id —
                // it does not; Quarto ignored it and
                // `Unable to resolve crossref @fig-environment` came back
                // from the real renderer. The embedded output's crossref id
                // *is* its cell's label, full stop. So: prefer the figure
                // block's own document-level label (e.g. `fig:environment`)
                // as `embed_id` when present — that is the identity the
                // *document* wants to reference it by — falling back to the
                // cell's own name (`nb:analysis` -> `fig-analysis`) only
                // when the figure has no label of its own. Orchestration
                // must relabel the notebook cell to this same `embed_id`,
                // not to the cell's own name unconditionally.
                let embed_id = resolve_embed_id(cell_label, id);
                let notebook = notebook
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                vec![format!("{{{{< embed {notebook}#{embed_id} >}}}}")]
            }
            FigureSource::Path(path) => {
                let caption_text = self.rewrite(caption, source).join(" ");
                let path = path.display().to_string();
                if caption.len() > 1 {
                    // Multi-block caption: reference §2.1's div form.
                    let mut out = Vec::new();
                    let open = match id {
                        Some(id) => format!("::: {{#{id}}}"),
                        None => "::: {}".to_string(),
                    };
                    out.push(open);
                    out.push(format!("![]({path})"));
                    out.push(String::new());
                    out.extend(self.rewrite(caption, source));
                    out.push(":::".to_string());
                    out
                } else {
                    let mut attr_parts = Vec::new();
                    if let Some(id) = id {
                        attr_parts.push(format!("#{id}"));
                    }
                    for (k, v) in attrs {
                        attr_parts.push(format!("{k}=\"{}\"", escape_quoted(v)));
                    }
                    let attr_str = if attr_parts.is_empty() {
                        String::new()
                    } else {
                        format!("{{{}}}", attr_parts.join(" "))
                    };
                    vec![format!("![{caption_text}]({path}){attr_str}")]
                }
            }
        }
    }

    /// D3: the caption and the table's own label both survive here — the
    /// old Python `_transform_table` read the caption from the directive
    /// *argument* (which MyST doesn't put it in; the caption is the
    /// directive *body*, per reference §2.1) and never emitted `:label:`'s
    /// value as `{#tbl-id}` at all.
    fn table(
        &self,
        caption: &[String],
        rows: &[String],
        label: Option<&crate::Label>,
        source: &Path,
    ) -> Vec<String> {
        let mut out: Vec<String> = rows.to_vec();
        let caption_text = self.rewrite(caption, source).join(" ");
        match label.and_then(|l| self.registry.quarto_id(source, l)) {
            Some(id) => out.push(format!(": {caption_text} {{#{id}}}")),
            None if !caption_text.is_empty() => out.push(format!(": {caption_text}")),
            None => {}
        }
        out
    }

    fn math(&self, body: &[String], label: Option<&crate::Label>, source: &Path) -> Vec<String> {
        let mut out = vec!["$$".to_string()];
        out.extend(body.iter().cloned());
        match label.and_then(|l| self.registry.quarto_id(source, l)) {
            Some(id) => out.push(format!("$$ {{#{id}}}")),
            None => out.push("$$".to_string()),
        }
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
        let class = admonition_class(kind);
        let mut header = format!("::: {{.{class}");
        if let Some(title) = title {
            header.push_str(&format!(" title=\"{}\"", escape_quoted(title)));
        }
        if let Some(collapse) = collapse {
            header.push_str(&format!(" collapse=\"{collapse}\""));
        }
        header.push('}');
        let mut out = vec![header];
        out.extend(self.render_nested(body, source));
        out.push(":::".to_string());
        out
    }

    fn tab_set(&self, items: &[crate::TabItem], source: &Path) -> Vec<String> {
        let mut out = vec!["::: {.panel-tabset}".to_string()];
        for item in items {
            out.push(format!("## {}", item.label));
            out.extend(self.render_nested(&item.body, source));
        }
        out.push(":::".to_string());
        out
    }

    fn wrapped_div(&self, class: &str, body: &[Block], source: &Path) -> Vec<String> {
        let mut out = vec![format!("::: {{.{class}}}")];
        out.extend(self.render_nested(body, source));
        out.push(":::".to_string());
        out
    }

    /// Same "no document-level alias" constraint as `figure`'s `CellRef`
    /// branch (see [`resolve_embed_id`]'s docs): the id used here must be
    /// the one the notebook cell gets relabelled to, so `label` (this
    /// `Embed` block's own crossref label, if any) takes priority over the
    /// cell's own name.
    fn embed(
        &self,
        target: &EmbedTarget,
        label: Option<&crate::Label>,
        source: &Path,
    ) -> Vec<String> {
        let own_id = label.and_then(|l| self.registry.quarto_id(source, l));
        let (prefix, cell_id) = match target {
            EmbedTarget::NotebookCell {
                notebook,
                cell_label,
            } => (
                notebook.display().to_string(),
                resolve_embed_id(cell_label, own_id),
            ),
            EmbedTarget::Label(l) => (
                String::new(),
                self.registry.resolve_reference(source, &l.raw),
            ),
        };
        vec![format!("{{{{< embed {prefix}#{cell_id} >}}}}")]
    }

    fn blockquote(
        &self,
        body: &[Block],
        attribution: Option<&[String]>,
        source: &Path,
    ) -> Vec<String> {
        let mut out: Vec<String> = self
            .render_nested(body, source)
            .into_iter()
            .map(|l| format!("> {l}"))
            .collect();
        if let Some(attribution) = attribution {
            for line in attribution {
                out.push(format!("> \u{2014} {line}"));
            }
        }
        out
    }

    fn theorem(
        &self,
        thm_type: &str,
        label: Option<&crate::Label>,
        body: &[Block],
        source: &Path,
    ) -> Vec<String> {
        let id = label.and_then(|l| self.registry.quarto_id(source, l));
        let header = match id {
            Some(id) => format!("::: {{#{id} .{thm_type}}}"),
            None => format!("::: {{.{thm_type}}}"),
        };
        let mut out = vec![header];
        out.extend(self.render_nested(body, source));
        out.push(":::".to_string());
        out
    }

    fn directive(
        &self,
        name: &str,
        attrs: &BTreeMap<String, String>,
        body: &[Block],
        label: Option<&crate::Label>,
        source: &Path,
    ) -> Vec<String> {
        // Reference §2: "Drop the directive" for these two — Quarto handles
        // both implicitly, via config.
        if matches!(name, "bibliography" | "tableofcontents") {
            return Vec::new();
        }
        let id = label.and_then(|l| self.registry.quarto_id(source, l));
        let mut attr_parts = vec![format!(".{name}")];
        if let Some(id) = id {
            attr_parts.insert(0, format!("#{id}"));
        }
        for (k, v) in attrs {
            attr_parts.push(format!("{k}=\"{}\"", escape_quoted(v)));
        }
        let mut out = vec![format!("::: {{{}}}", attr_parts.join(" "))];
        out.extend(self.render_nested(body, source));
        out.push(":::".to_string());
        out
    }

    fn render_nested(&self, body: &[Block], source: &Path) -> Vec<String> {
        crate::writer::render_body(body, |b| self.render_block(b, source))
    }
}

/// Resolves the single crossref id an embedded notebook cell's output must
/// use — in the `{{< embed nb#id >}}` shortcode this writer emits *and* in
/// the notebook cell's own relabelled `#| label:` (orchestration's job,
/// `crate::notebook::relabel`; both sides must agree on `id` or the embed
/// will not resolve).
///
/// Verified against `quarto 1.9.36`: **there is no document-level alias for
/// an embedded output.** A `{{< embed >}}` shortcode's crossref id is
/// exactly its target cell's `#| label:` value — nothing else. An earlier
/// version of this writer emitted a second `#id` after the cell anchor
/// (`{{< embed nb#cell-id #fig-x >}}`) assuming Quarto would treat that as
/// an additional alias id for the embed; it does not, and the real
/// renderer reported `Unable to resolve crossref @fig-x` for exactly that
/// id. So there is only one id to choose, and this function chooses it:
///
/// - if the *document* gave this figure/embed its own label (`document_label`,
///   e.g. MyST's `:label: fig:environment` on a `:::{figure} #nb:analysis`
///   block) — that is the identity the *document* wants to reference this
///   embedded content by, so it wins;
/// - otherwise, fall back to normalizing the notebook cell's own name
///   (`nb:analysis` -> `fig-analysis`).
#[must_use]
pub fn resolve_embed_id(cell_label: &crate::Label, document_label: Option<&str>) -> String {
    match document_label {
        Some(id) => id.to_string(),
        None => normalize::normalize(&cell_label.raw, crate::RefKind::CodeCell),
    }
}

/// Escapes `\` and `"` for interpolation into a double-quoted string —
/// Pandoc div/span attribute values (`{k}="v"`, `title="v"`) and Quarto's
/// YAML-flavored `#|` cell options (`#| fig-cap: "v"`) both use the same
/// backslash-then-quote escaping rule. Without this (L2), a caption or
/// title containing `"` breaks out of the attribute — a title of
/// `x" .callout-important collapse="true` would inject an arbitrary class
/// and attribute into the rendered div, not merely produce malformed
/// output.
fn escape_quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// This one emits *raw HTML* (the `{abbr}` role, reference §5's "no Quarto
/// semantic" fallback — `render_role_to_quarto`'s `"abbr"` arm), so it needs
/// real HTML escaping, not [`escape_quoted`]'s Pandoc-attribute rule: `&`
/// first (so escaping the others doesn't get re-escaped), then `<`/`>` for
/// text content, plus `"` for an attribute value.
fn escape_html_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attr(s: &str) -> String {
    escape_html_text(s).replace('"', "&quot;")
}

fn admonition_class(kind: AdmonitionKind) -> &'static str {
    match kind {
        AdmonitionKind::Note
        | AdmonitionKind::Hint
        | AdmonitionKind::SeeAlso
        | AdmonitionKind::Attention => "callout-note",
        AdmonitionKind::Warning => "callout-warning",
        AdmonitionKind::Tip => "callout-tip",
        AdmonitionKind::Important | AdmonitionKind::Danger | AdmonitionKind::Error => {
            "callout-important"
        }
        AdmonitionKind::Caution => "callout-caution",
    }
}

fn tag_to_cell_option(tag: &str) -> Option<&'static str> {
    match tag {
        "remove-input" => Some("#| echo: false"),
        "remove-output" => Some("#| output: false"),
        "remove-cell" => Some("#| include: false"),
        "hide-input" => Some("#| code-fold: true"),
        _ => None,
    }
}

fn static_code(
    lang: &Option<String>,
    body: &[String],
    attrs: &BTreeMap<String, String>,
) -> Vec<String> {
    if lang.as_deref() == Some("mermaid") {
        let mut out = vec!["```{mermaid}".to_string()];
        out.extend(body.iter().cloned());
        out.push("```".to_string());
        return out;
    }
    let mut open = String::from("```{");
    open.push_str(&format!(".{}", lang.as_deref().unwrap_or("text")));
    for (k, v) in attrs {
        open.push_str(&format!(" {k}=\"{v}\""));
    }
    open.push('}');
    let mut out = vec![open];
    out.extend(body.iter().cloned());
    out.push("```".to_string());
    out
}

fn include_shortcode(target: &Path) -> String {
    let dir = target.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("include");
    let stem = if let Some(stripped) = stem.strip_prefix('_') {
        stripped
    } else {
        stem
    };
    let name = format!("_{stem}.qmd");
    let path = if dir.as_os_str().is_empty() {
        name
    } else {
        dir.join(name).display().to_string()
    };
    format!("{{{{< include {path} >}}}}")
}

fn raw(format: &str, body: &[String]) -> Vec<String> {
    let mut out = vec![format!("```{{={format}}}")];
    out.extend(body.iter().cloned());
    out.push("```".to_string());
    out
}

/// Renders every `{name}`content`` form (reference §5/§10 — both truly
/// legacy roles and modern-but-still-`{name}`-shaped roles like `eval`/
/// `del`/`abbr`) to its Quarto equivalent. Consolidated into one match
/// because [`crate::reader::inline::read_legacy_role`] does not distinguish
/// "legacy" from "modern MyST role" syntactically — both are `{name}`
/// followed by a backtick-delimited argument — so the writer must handle
/// every name this matcher can produce, not just the ones `mappings.toml`'s
/// `[[legacy_role]]` table happens to catalog.
fn render_role_to_quarto(
    role: &str,
    target: &str,
    registry: &LabelRegistry,
    source: &Path,
) -> String {
    match role {
        "cite" | "cite:p" => render_citation_group(target),
        "cite:t" => format!("@{target}"),
        "numref" | "ref" => format!(
            "@{}",
            registry.resolve_reference(source, reference_target(target))
        ),
        "eq" => {
            let id = if target.starts_with("eq-") || target.starts_with("eq:") {
                registry.resolve_reference(source, target)
            } else {
                registry.resolve_reference(source, &format!("eq:{target}"))
            };
            format!("@{id}")
        }
        "doc" => {
            let stem = target.strip_suffix(".md").unwrap_or(target);
            format!("[{target}]({stem}.qmd)")
        }
        "eval" => format!("`{{python}} {target}`"),
        "del" | "strike" => format!("~~{target}~~"),
        "u" | "underline" => format!("[{target}]{{.underline}}"),
        "sc" | "smallcaps" => format!("[{target}]{{.smallcaps}}"),
        "sub" => format!("~{target}~"),
        "sup" => format!("^{target}^"),
        "kbd" => format!("[{target}]{{.kbd}}"),
        "abbr" => render_abbr(target),
        _ => format!("{{{role}}}`{target}`"), // unrecognized role: preserve verbatim
    }
}

fn render_citation_group(target: &str) -> String {
    let keys: Vec<&str> = target
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .collect();
    if keys.len() <= 1 {
        format!("[@{}]", target.trim())
    } else {
        format!(
            "[{}]",
            keys.into_iter()
                .map(|key| format!("@{key}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}

fn reference_target(target: &str) -> &str {
    if let Some(open) = target.rfind('<') {
        if let Some(close) = target[open + 1..].find('>') {
            return &target[open + 1..open + 1 + close];
        }
    }
    target
}

fn render_abbr(target: &str) -> String {
    if let Some(open) = target.find('(') {
        let term = target[..open].trim();
        let expansion = target[open + 1..].trim_end_matches(')').trim();
        return format!(
            "<abbr title=\"{}\">{}</abbr>",
            escape_html_attr(expansion),
            escape_html_text(term)
        );
    }
    format!("<abbr>{}</abbr>", escape_html_text(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Attrs, CellOptions, Engine, FigureSource, Frontmatter};
    use crate::{Label, ReaderContext, Span};

    fn registry_for(docs: &[(std::path::PathBuf, Document)]) -> LabelRegistry {
        LabelRegistry::build(docs).0
    }

    fn block(kind: BlockKind, blank: u8) -> Block {
        Block {
            kind,
            span: Span::single(1),
            blank_lines_before: blank,
        }
    }

    #[test]
    fn heading_with_label_emits_sec_id() {
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::Heading {
                        level: 2,
                        text: "Data Analysis".to_string(),
                        label: Some(Label::new("sec:data-analysis")),
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert_eq!(out.trim(), "## Data Analysis {#sec-data-analysis}");
    }

    #[test]
    fn figure_with_single_line_caption_emits_fig_id() {
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::Figure {
                        src: FigureSource::Path("images/fruit-flies.png".into()),
                        caption: vec!["Collection methodology.".to_string()],
                        label: Some(Label::new("fig:samples")),
                        attrs: Attrs::new(),
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert_eq!(
            out.trim(),
            "![Collection methodology.](images/fruit-flies.png){#fig-samples}"
        );
    }

    #[test]
    fn table_caption_and_label_both_survive() {
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::Table {
                        caption: vec!["Phenotypic variation.".to_string()],
                        rows: vec!["| A | B |".to_string(), "|---|---|".to_string()],
                        label: Some(Label::new("tab:phenotypic-variation")),
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert!(out.contains("| A | B |"));
        assert!(out.contains(": Phenotypic variation. {#tbl-phenotypic-variation}"));
    }

    #[test]
    fn every_at_reference_has_a_matching_id_on_the_real_fixture() {
        let mut index = crate::NotebookCellIndex::default();
        index
            .add_notebook_json(
                "analysis.ipynb",
                include_str!("../../../../article-template/analysis.ipynb"),
            )
            .unwrap();
        let reader = crate::MystReader::new(ReaderContext {
            notebook_index: index,
            ..ReaderContext::new("article.md")
        });
        let doc = reader
            .read_str(include_str!("../../../../article-template/article.md"))
            .unwrap();
        let docs = vec![(std::path::PathBuf::from("article.md"), doc)];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);

        // Every bare cross-reference `@fig-x`/`@tbl-x`/`@sec-x`/`@eq-x` in
        // the rendered output must have a matching `{#id}` (or
        // `#| label: id` for code cells) somewhere in the same output —
        // D1's acceptance criterion, checked directly against real text
        // rather than trusting label bookkeeping alone.
        //
        // Citation keys (`[@10.1038/nmeth.1974]`, bare `@numpy`) are
        // deliberately excluded: a citation is resolved against a
        // bibliography, not a `{#id}` definition in this same document, and
        // real DOI-shaped keys contain `.`/`/`, which is exactly why this
        // check only looks at `@`-tokens matching Quarto's own
        // `{#kind-name}` id shape (`[a-z]+-` prefix, `[a-z0-9-]*` rest) —
        // the fixture's citations (`10.1038/...`, `numpy`, `pandas`,
        // `scipy`) never match that shape, so they are correctly skipped
        // without needing to also detect "is this inside `[@...]`".
        let known_id_prefixes = ["fig-", "tbl-", "eq-", "sec-"];
        let ids_referenced: Vec<&str> = out
            .split('@')
            .skip(1)
            .filter_map(|s| {
                let end = s
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                    .unwrap_or(s.len());
                let candidate = &s[..end];
                known_id_prefixes
                    .iter()
                    .any(|p| candidate.starts_with(p))
                    .then_some(candidate)
            })
            .collect();
        assert!(
            !ids_referenced.is_empty(),
            "sanity check: fixture should contain cross-references"
        );
        for id in ids_referenced {
            assert!(
                definition_exists(&out, id),
                "@{id} has no matching definition in:\n{out}"
            );
        }
    }

    /// M4 fix: `out.contains("#fig-samples")` is satisfied by
    /// `#fig-samples-2` too (`fig-samples` is a string *prefix* of
    /// `fig-samples-2`), so a naive substring check would not have caught a
    /// D1 regression where a reference resolves to the wrong, merely
    /// similarly-prefixed id. This checks that the character immediately
    /// following each candidate match is a real delimiter, not an
    /// id-continuation character.
    fn definition_exists(out: &str, id: &str) -> bool {
        let is_boundary = |c: char| !(c.is_ascii_alphanumeric() || c == '-');
        for pattern in [
            format!("{{#{id}}}"),
            format!("label: {id}"),
            format!("#{id}"),
        ] {
            let mut start = 0;
            while let Some(rel) = out[start..].find(&pattern) {
                let end = start + rel + pattern.len();
                if out[end..].chars().next().map_or(true, is_boundary) {
                    return true;
                }
                start = start + rel + 1;
            }
        }
        false
    }

    #[test]
    fn cellref_figure_emits_embed_with_normalized_cell_id() {
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::Figure {
                        src: FigureSource::CellRef {
                            label: Label::new("nb:analysis"),
                            notebook: Some("analysis.ipynb".into()),
                        },
                        caption: vec![],
                        label: Some(Label::new("fig:environment")),
                        attrs: Attrs::new(),
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        // The figure's own label (`fig:environment`) wins over the cell's
        // own name (`nb:analysis` -> `fig-analysis`) — see
        // `resolve_embed_id`'s docs on why there is only one id, not two.
        assert_eq!(out.trim(), "{{< embed analysis.ipynb#fig-environment >}}");
    }

    #[test]
    fn cellref_figure_with_no_document_label_falls_back_to_the_cell_name() {
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::Figure {
                        src: FigureSource::CellRef {
                            label: Label::new("nb:analysis"),
                            notebook: Some("analysis.ipynb".into()),
                        },
                        caption: vec![],
                        label: None,
                        attrs: Attrs::new(),
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert_eq!(out.trim(), "{{< embed analysis.ipynb#fig-analysis >}}");
    }

    #[test]
    fn legacy_cite_role_never_reaches_output_as_role_syntax() {
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::Paragraph {
                        lines: vec!["See {cite}`smith2020` for details.".to_string()],
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert_eq!(out.trim(), "See [@smith2020] for details.");
    }

    #[test]
    fn doi_citation_key_survives_myst_to_quarto_unchanged() {
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::Paragraph {
                        lines: vec!["See [@10.1038/nmeth.1974].".to_string()],
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert_eq!(out.trim(), "See [@10.1038/nmeth.1974].");
    }

    #[test]
    fn untouched_frontmatter_fields_pass_through_unchanged() {
        let raw = "title: Sample\nabstract: |\n  Line one.\n  Line two.\n";
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: Some(Frontmatter {
                    raw: raw.to_string(),
                    parsed: crate::YamlValue::Mapping(
                        crate::yaml::parse_mapping(raw).expect("valid YAML"),
                    ),
                }),
                blocks: vec![block(
                    BlockKind::Paragraph {
                        lines: vec!["Body.".to_string()],
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert!(out.starts_with("---\ntitle: Sample\nabstract: |\n  Line one.\n  Line two.\n---\n"));
        assert!(out.contains("Body."));
    }

    /// Proves the writer actually calls `crate::frontmatter::myst_to_quarto`
    /// (not just that module's own unit tests) — `kernelspec` must become
    /// `jupyter` in the rendered `.qmd` text.
    #[test]
    fn kernelspec_frontmatter_is_mapped_to_jupyter_by_the_writer() {
        let raw = "title: Sample\nkernelspec:\n  name: python3\n";
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: Some(Frontmatter {
                    raw: raw.to_string(),
                    parsed: crate::YamlValue::Mapping(
                        crate::yaml::parse_mapping(raw).expect("valid YAML"),
                    ),
                }),
                blocks: vec![block(
                    BlockKind::Paragraph {
                        lines: vec!["Body.".to_string()],
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert!(out.contains("jupyter: python3"));
        assert!(!out.contains("kernelspec"));
    }

    #[test]
    fn code_cell_tags_map_to_quarto_options() {
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::CodeCell {
                        lang: "python".to_string(),
                        options: CellOptions {
                            tags: vec!["remove-input".to_string()],
                            caption: None,
                            extra: BTreeMap::new(),
                        },
                        body: vec!["x = 1".to_string()],
                        label: None,
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert!(out.contains("```{python}"));
        assert!(out.contains("#| echo: false"));
        assert!(out.contains("x = 1"));
    }

    #[test]
    fn admonition_title_containing_a_quote_cannot_inject_extra_attributes() {
        // L2 regression: a title of `x" .callout-important collapse="true`
        // must not close the `title="..."` attribute early and inject a
        // second class/attribute into the div — reproduced concretely: the
        // original unescaped code produced
        // `title="x" .callout-important collapse="true"` (a real,
        // additional `.callout-important` class applied to the div), not
        // merely cosmetically wrong output.
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::Admonition {
                        kind: AdmonitionKind::Note,
                        title: Some(r#"x" .callout-important collapse="true"#.to_string()),
                        body: vec![],
                        collapse: None,
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        assert!(
            !out.contains(".callout-important collapse=\"true\"}"),
            "an unescaped title must not be able to inject a second class/attribute, got:\n{out}"
        );
        // The title's own embedded quote was escaped (backslash then
        // quote), not passed through as a raw attribute delimiter.
        assert!(out.contains("title=\"x\\\""));
    }

    #[test]
    fn figure_caption_containing_a_quote_does_not_break_the_attribute_block() {
        let docs = vec![(
            std::path::PathBuf::from("a.md"),
            Document {
                frontmatter: None,
                blocks: vec![block(
                    BlockKind::Figure {
                        src: FigureSource::Path("img.png".into()),
                        caption: vec![],
                        label: Some(Label::new("fig:samples")),
                        attrs: Attrs::from([("alt".to_string(), r#"a "quoted" alt"#.to_string())]),
                    },
                    0,
                )],
                source: std::path::PathBuf::from("a.md"),
                engine: Some(Engine::Jupyter),
            },
        )];
        let registry = registry_for(&docs);
        let writer = QuartoWriter::new(&registry);
        let (out, _, _) = writer.write(&docs[0].1);
        // The escaped quote (`\"`, two characters: backslash then quote)
        // must appear on both sides of the escaped value — proving the
        // attribute's own delimiter quotes were not closed early by the
        // caption's embedded quotes.
        assert!(out.contains("alt=\"a \\\"quoted\\\" alt\""));
    }
}
