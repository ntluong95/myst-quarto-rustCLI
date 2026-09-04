//! Quarto Markdown reader.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::reader::inline::{scan_line, InlineEvent};
use crate::reader::{
    block, fence, parse_cell_option, parse_cell_options, preservation_marker_id, split_frontmatter,
    unquote,
};
use crate::reader::{ReaderContext, ReaderError};
use crate::{
    AdmonitionKind, Attrs, Block, BlockKind, CommentStyle, Document, EmbedTarget, Engine,
    FigureSource, IncludeOpts, Label, TabItem,
};

#[derive(Debug, Clone)]
pub struct QuartoReader {
    context: ReaderContext,
}

impl QuartoReader {
    #[must_use]
    pub fn new(context: ReaderContext) -> Self {
        Self { context }
    }

    pub fn read_str(&self, text: &str) -> Result<Document, ReaderError> {
        let (frontmatter, lines, start_line) = split_frontmatter(text)?;
        let blocks = self.parse_blocks(&lines, start_line)?;
        let engine = detect_engine(&lines, &blocks);
        Ok(Document {
            frontmatter,
            blocks,
            source: self.context.source.clone(),
            engine,
        })
    }

    fn parse_blocks(&self, lines: &[&str], start_line: u32) -> Result<Vec<Block>, ReaderError> {
        let mut out = Vec::new();
        let mut para = Vec::new();
        let mut para_start = 0;
        let mut para_blank = 0;
        let mut blank = 0u8;
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            let line_no = start_line + i as u32;
            if line.trim().is_empty() {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                blank = blank.saturating_add(1);
                i += 1;
                continue;
            }
            if let Some(id) = preservation_marker_id(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                self.push_preserved_or_marker(&mut out, id, line_no, blank)?;
                blank = 0;
                i += 1;
                continue;
            }
            if let Some(kind) = self.shortcode(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                out.push(block(kind, line_no, line_no, blank));
                blank = 0;
                i += 1;
                continue;
            }
            if let Some(open) = fence::parse_quarto_code_open(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let (body, _, end) =
                    fence::take_fenced_body(lines, i, '`', open.fence_count, open.indent);
                if let Some(format) = open.lang.strip_prefix('=') {
                    out.push(block(
                        BlockKind::Raw {
                            format: format.to_string(),
                            body,
                        },
                        line_no,
                        start_line + end as u32,
                        blank,
                    ));
                    blank = 0;
                    i = end + 1;
                    continue;
                }
                let (options, consumed) = parse_cell_options(&body);
                let label = body
                    .iter()
                    .find_map(|l| parse_cell_option(l, "label"))
                    .map(Label::new);
                out.push(block(
                    BlockKind::CodeCell {
                        lang: open.lang,
                        options,
                        body: body[consumed..].to_vec(),
                        label,
                    },
                    line_no,
                    start_line + end as u32,
                    blank,
                ));
                blank = 0;
                i = end + 1;
                continue;
            }
            if let Some((fence_char, count, indent, lang)) = fence::parse_regular_code_open(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let (body, _, end) = fence::take_fenced_body(lines, i, fence_char, count, indent);
                out.push(block(
                    BlockKind::StaticCode {
                        lang,
                        body,
                        attrs: Attrs::new(),
                    },
                    line_no,
                    start_line + end as u32,
                    blank,
                ));
                blank = 0;
                i = end + 1;
                continue;
            }
            if let Some(open) = fence::parse_quarto_div_open(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let (body, original, end) =
                    fence::take_fenced_body(lines, i, ':', open.fence_count, open.indent);
                let kind = self.div_to_block(&open.attrs, &body, &original, line_no + 1)?;
                out.push(block(kind, line_no, start_line + end as u32, blank));
                blank = 0;
                i = end + 1;
                continue;
            }
            if let Some(kind) = parse_image(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                out.push(block(kind, line_no, line_no, blank));
                blank = 0;
                i += 1;
                continue;
            }
            if line.trim_start().starts_with("$$") {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let (kind, end) = parse_math(lines, i);
                out.push(block(kind, line_no, start_line + end as u32, blank));
                blank = 0;
                i = end + 1;
                continue;
            }
            if line.trim_start().starts_with('|') {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let (kind, end) = parse_table(lines, i);
                out.push(block(kind, line_no, start_line + end as u32, blank));
                blank = 0;
                i = end + 1;
                continue;
            }
            if let Some((level, text, label)) = parse_heading(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                out.push(block(
                    BlockKind::Heading { level, text, label },
                    line_no,
                    line_no,
                    blank,
                ));
                blank = 0;
                i += 1;
                continue;
            }
            if line.trim_start().starts_with("<!--") && line.trim_end().ends_with("-->") {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let text = line
                    .trim()
                    .trim_start_matches("<!--")
                    .trim_end_matches("-->")
                    .trim()
                    .to_string();
                out.push(block(
                    BlockKind::Comment {
                        text,
                        style: CommentStyle::Html,
                    },
                    line_no,
                    line_no,
                    blank,
                ));
                blank = 0;
                i += 1;
                continue;
            }
            if para.is_empty() {
                para_start = line_no;
                para_blank = blank;
                blank = 0;
            }
            para.push(line.to_string());
            i += 1;
        }
        flush_paragraph(
            &mut out,
            &mut para,
            para_start,
            start_line + lines.len() as u32,
            para_blank,
        );
        Ok(out)
    }

    fn div_to_block(
        &self,
        attrs: &str,
        body: &[String],
        original: &[String],
        body_start: u32,
    ) -> Result<BlockKind, ReaderError> {
        let parsed_attrs = parse_attrs(attrs);
        if let Some(kind) = callout_kind(attrs) {
            return Ok(BlockKind::Admonition {
                kind,
                title: parsed_attrs.get("title").cloned(),
                body: self.parse_owned_body(body, body_start)?,
                collapse: parsed_attrs.get("collapse").map(|v| v == "true"),
            });
        }
        if attrs.contains(".panel-tabset") {
            return Ok(BlockKind::TabSet {
                items: self.parse_tabs(body, body_start)?,
            });
        }
        if attrs.contains(".column-margin") {
            return Ok(BlockKind::Margin {
                body: self.parse_owned_body(body, body_start)?,
            });
        }
        if attrs.contains(".theorem") {
            return Ok(BlockKind::Theorem {
                thm_type: "theorem".to_string(),
                label: parsed_attrs.get("id").map(Label::new),
                body: self.parse_owned_body(body, body_start)?,
            });
        }
        if parsed_attrs.contains_key("id") {
            return Ok(BlockKind::Figure {
                src: FigureSource::Path(PathBuf::new()),
                caption: body.to_vec(),
                label: parsed_attrs.get("id").map(Label::new),
                attrs: parsed_attrs,
            });
        }
        if attrs.contains(".grid") || attrs.contains(".card") {
            return Ok(BlockKind::Directive {
                name: attrs.trim_start_matches('.').to_string(),
                attrs: parsed_attrs,
                body: self.parse_owned_body(body, body_start)?,
                label: None,
            });
        }
        Ok(BlockKind::Unmappable {
            original: original.to_vec(),
            reason: format!("unrecognized Quarto div {{{attrs}}}"),
        })
    }

    fn parse_owned_body(&self, lines: &[String], start: u32) -> Result<Vec<Block>, ReaderError> {
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        self.parse_blocks(&refs, start)
    }

    fn parse_tabs(&self, lines: &[String], start: u32) -> Result<Vec<TabItem>, ReaderError> {
        let mut items = Vec::new();
        let mut current_label: Option<String> = None;
        let mut current = Vec::new();
        let mut current_start = start;
        for (idx, line) in lines.iter().enumerate() {
            if let Some(label) = line.strip_prefix("## ") {
                if let Some(label) = current_label.replace(label.to_string()) {
                    items.push(TabItem {
                        label,
                        body: self.parse_owned_body(&current, current_start)?,
                    });
                    current.clear();
                }
                current_start = start + idx as u32 + 1;
            } else {
                current.push(line.clone());
            }
        }
        if let Some(label) = current_label {
            items.push(TabItem {
                label,
                body: self.parse_owned_body(&current, current_start)?,
            });
        }
        Ok(items)
    }

    fn shortcode(&self, line: &str) -> Option<BlockKind> {
        let trimmed = line.trim();
        if !trimmed.contains("{{<") {
            return None;
        }
        if !trimmed.starts_with("{{<") || !trimmed.ends_with(">}}") {
            return Some(BlockKind::Unmappable {
                original: vec![line.to_string()],
                reason: "Quarto shortcodes must be supported block-level forms".to_string(),
            });
        }
        let inner = trimmed.strip_prefix("{{<")?.strip_suffix(">}}")?.trim();
        let mut parts = inner.split_whitespace();
        match parts.next()? {
            "include" => {
                let target = PathBuf::from(parts.next().unwrap_or_default());
                match self.context.resolve_include(&target) {
                    Ok(path) => Some(BlockKind::Include {
                        target: path,
                        opts: shortcode_include_opts(parts),
                    }),
                    Err(e) => Some(BlockKind::Unmappable {
                        original: vec![line.to_string()],
                        reason: e.to_string(),
                    }),
                }
            }
            "embed" => Some(self.parse_embed(parts.next().unwrap_or_default(), line)),
            name @ ("video" | "pagebreak" | "meta" | "var") => Some(BlockKind::Unmappable {
                original: vec![line.to_string()],
                reason: format!("Quarto shortcode {name} has no MyST equivalent"),
            }),
            name => Some(BlockKind::Unmappable {
                original: vec![line.to_string()],
                reason: format!("Quarto shortcode {name} is not recognized"),
            }),
        }
    }

    /// See `crate::reader::myst::MystReader::push_preserved_or_marker`'s
    /// doc — identical fix, mirrored: this reader only ever reparses an
    /// entry explicitly recorded as `Dialect::Quarto`; anything else
    /// (`Dialect::Myst`, `Unknown`, or no entry at all) becomes an opaque
    /// `Preserved` block, never passed to `self.parse_blocks`.
    fn push_preserved_or_marker(
        &self,
        out: &mut Vec<Block>,
        id: &str,
        line: u32,
        blank: u8,
    ) -> Result<(), ReaderError> {
        use crate::preserve::Dialect;

        if let Some(original) = self.context.preserved.get_matching(id, Dialect::Quarto) {
            let refs: Vec<&str> = original.iter().map(String::as_str).collect();
            let parsed = self.parse_blocks(&refs, line)?;
            if parsed.len() == 1 {
                out.push(Block {
                    span: crate::Span::single(line),
                    blank_lines_before: blank,
                    ..parsed[0].clone()
                });
                return Ok(());
            }
            out.push(block(
                BlockKind::Preserved {
                    original: original.clone(),
                    code: "preserved-sidecar",
                },
                line,
                line,
                blank,
            ));
        } else if let Some(original) = self.context.preserved.get(id) {
            out.push(block(
                BlockKind::Preserved {
                    original: original.clone(),
                    code: "preserved-foreign-dialect",
                },
                line,
                line,
                blank,
            ));
        } else {
            out.push(block(
                BlockKind::Preserved {
                    original: Vec::new(),
                    code: "missing-sidecar-entry",
                },
                line,
                line,
                blank,
            ));
        }
        Ok(())
    }
}

fn flush_paragraph(out: &mut Vec<Block>, lines: &mut Vec<String>, start: u32, end: u32, blank: u8) {
    if !lines.is_empty() {
        out.push(block(
            BlockKind::Paragraph {
                lines: std::mem::take(lines),
            },
            start,
            end.saturating_sub(1),
            blank,
        ));
    }
}

fn parse_image(line: &str) -> Option<BlockKind> {
    let trimmed = line.trim();
    let alt_start = trimmed.strip_prefix("![")?;
    let alt_end = alt_start.find("](")?;
    let after_alt = &alt_start[alt_end + 2..];
    let src_end = after_alt.find(')')?;
    let alt = &alt_start[..alt_end];
    let src = &after_alt[..src_end];
    let attrs = after_alt[src_end + 1..].trim();
    let parsed_attrs = parse_attrs(attrs.trim_start_matches('{').trim_end_matches('}'));
    let mut figure_attrs = parsed_attrs.clone();
    if !alt.is_empty() {
        figure_attrs.insert("alt".to_string(), alt.to_string());
    }
    Some(BlockKind::Figure {
        src: FigureSource::Path(PathBuf::from(src)),
        caption: if alt.is_empty() {
            Vec::new()
        } else {
            vec![alt.to_string()]
        },
        label: parsed_attrs.get("id").map(Label::new),
        attrs: figure_attrs,
    })
}

fn parse_math(lines: &[&str], start: usize) -> (BlockKind, usize) {
    let mut body = Vec::new();
    let mut label = None;
    let mut i = start + 1;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if let Some(after) = trimmed.strip_prefix("$$") {
            label = parse_attrs(after.trim().trim_start_matches('{').trim_end_matches('}'))
                .get("id")
                .map(Label::new);
            return (BlockKind::Math { body, label }, i);
        }
        body.push(lines[i].to_string());
        i += 1;
    }
    (
        BlockKind::Math { body, label },
        lines.len().saturating_sub(1),
    )
}

fn parse_table(lines: &[&str], start: usize) -> (BlockKind, usize) {
    let mut rows = Vec::new();
    let mut i = start;
    while i < lines.len() && lines[i].trim_start().starts_with('|') {
        rows.push(lines[i].to_string());
        i += 1;
    }
    let mut probe = i;
    while probe < lines.len() && lines[probe].trim().is_empty() {
        probe += 1;
    }
    if probe < lines.len() && lines[probe].trim_start().starts_with(':') {
        let cap = lines[probe].trim_start().trim_start_matches(':').trim();
        let (caption, label) = split_caption_label(cap);
        return (
            BlockKind::Table {
                caption: vec![caption],
                rows,
                label: label.map(Label::new),
            },
            probe,
        );
    }
    (
        BlockKind::Table {
            caption: Vec::new(),
            rows,
            label: None,
        },
        i.saturating_sub(1),
    )
}

fn split_caption_label(caption: &str) -> (String, Option<String>) {
    if let Some(pos) = caption.rfind("{#") {
        let id = caption[pos + 2..].trim_end_matches('}').trim().to_string();
        return (caption[..pos].trim().to_string(), Some(id));
    }
    (caption.to_string(), None)
}

fn parse_heading(line: &str) -> Option<(u8, String, Option<Label>)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) || !trimmed[level..].starts_with(' ') {
        return None;
    }
    let text = trimmed[level + 1..].trim();
    let (text, label) = split_caption_label(text);
    Some((level as u8, text, label.map(Label::new)))
}

fn parse_attrs(attrs: &str) -> Attrs {
    let mut out = BTreeMap::new();
    for part in attrs.split_whitespace() {
        if let Some(id) = part.strip_prefix('#') {
            out.insert("id".to_string(), id.to_string());
        } else if let Some(class) = part.strip_prefix('.') {
            out.insert("class".to_string(), class.to_string());
        } else if let Some((key, value)) = part.split_once('=') {
            out.insert(key.to_string(), unquote(value));
        }
    }
    out
}

fn callout_kind(attrs: &str) -> Option<AdmonitionKind> {
    if attrs.contains(".callout-note") {
        Some(AdmonitionKind::Note)
    } else if attrs.contains(".callout-warning") {
        Some(AdmonitionKind::Warning)
    } else if attrs.contains(".callout-tip") {
        Some(AdmonitionKind::Tip)
    } else if attrs.contains(".callout-important") {
        Some(AdmonitionKind::Important)
    } else if attrs.contains(".callout-caution") {
        Some(AdmonitionKind::Caution)
    } else {
        None
    }
}

impl QuartoReader {
    fn parse_embed(&self, target: &str, line: &str) -> BlockKind {
        if let Some((path, _)) = target.split_once('#') {
            if !path.is_empty() {
                let candidate = PathBuf::from(path);
                if let Err(e) = self.context.resolve_include(&candidate) {
                    return BlockKind::Unmappable {
                        original: vec![line.to_string()],
                        reason: e.to_string(),
                    };
                }
            }
        }
        parse_embed_target(target)
    }
}

fn parse_embed_target(target: &str) -> BlockKind {
    let (path, label) = target.split_once('#').unwrap_or(("", target));
    let label = Label::new(label);
    let target = if path.is_empty() {
        EmbedTarget::Label(label)
    } else {
        EmbedTarget::NotebookCell {
            notebook: PathBuf::from(path),
            cell_label: label,
        }
    };
    BlockKind::Embed {
        target,
        label: None,
    }
}

fn shortcode_include_opts<'a>(parts: impl Iterator<Item = &'a str>) -> IncludeOpts {
    let mut opts = IncludeOpts::default();
    for part in parts {
        if let Some((key, value)) = part.split_once('=') {
            match key {
                "start-line" => opts.start_line = value.parse().ok(),
                "end-line" => opts.end_line = value.parse().ok(),
                _ => {}
            }
        }
    }
    opts
}

fn detect_engine(lines: &[&str], blocks: &[Block]) -> Option<Engine> {
    let mut saw_jupyter_inline = false;
    for line in lines {
        for event in scan_line(line, &[]).events {
            match event {
                InlineEvent::KnitrEval(_) => return Some(Engine::Knitr),
                InlineEvent::JupyterEval { .. } => saw_jupyter_inline = true,
                _ => {}
            }
        }
    }
    for block in blocks {
        if matches!(&block.kind, BlockKind::CodeCell { lang, .. } if lang == "r") {
            return Some(Engine::Knitr);
        }
    }
    blocks
        .iter()
        .any(|b| matches!(b.kind, BlockKind::CodeCell { .. }))
        .then_some(Engine::Jupyter)
        .or_else(|| saw_jupyter_inline.then_some(Engine::Jupyter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_code_options_shortcodes_and_engine() {
        let reader = QuartoReader::new(ReaderContext::new("article.qmd"));
        let doc = reader
            .read_str("```{r}\n#| label: fig-analysis\n#| echo: false\nplot(x)\n```\n\n{{< include _part.qmd >}}\n\n`r x + 1`\n")
            .unwrap();
        assert_eq!(doc.engine, Some(Engine::Knitr));
        assert!(
            matches!(&doc.blocks[0].kind, BlockKind::CodeCell { label: Some(l), options, .. } if l.raw == "fig-analysis" && options.tags == vec!["remove-input"])
        );
        assert!(
            matches!(&doc.blocks[1].kind, BlockKind::Include { target, .. } if target == &PathBuf::from("_part.qmd"))
        );
    }

    #[test]
    fn parses_figures_tables_math_and_embed() {
        let reader = QuartoReader::new(ReaderContext::new("article.qmd"));
        let doc = reader
            .read_str("![Caption](img.png){#fig-x width=50%}\n\n| a |\n|---|\n: Table cap {#tbl-x}\n\n$$\nx\n$$ {#eq-x}\n\n{{< embed analysis.ipynb#fig-analysis >}}\n")
            .unwrap();
        assert!(
            matches!(&doc.blocks[0].kind, BlockKind::Figure { label: Some(l), .. } if l.raw == "fig-x")
        );
        assert!(
            matches!(&doc.blocks[1].kind, BlockKind::Table { label: Some(l), caption, .. } if l.raw == "tbl-x" && caption[0] == "Table cap")
        );
        assert!(
            matches!(&doc.blocks[2].kind, BlockKind::Math { label: Some(l), .. } if l.raw == "eq-x")
        );
        assert!(
            matches!(&doc.blocks[3].kind, BlockKind::Embed { target: EmbedTarget::NotebookCell { notebook, .. }, .. } if notebook == &PathBuf::from("analysis.ipynb"))
        );
    }
}
