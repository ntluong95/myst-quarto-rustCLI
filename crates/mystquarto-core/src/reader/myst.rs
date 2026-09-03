//! MyST Markdown reader.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::reader::{
    block, fence, label_option, parse_cell_options, preservation_marker_id, split_frontmatter,
};
use crate::reader::{ReaderContext, ReaderError};
use crate::{
    AdmonitionKind, Attrs, Block, BlockKind, CommentStyle, Document, EmbedTarget, Engine,
    FigureSource, IncludeOpts, Label,
};

#[derive(Debug, Clone)]
pub struct MystReader {
    context: ReaderContext,
}

impl MystReader {
    #[must_use]
    pub fn new(context: ReaderContext) -> Self {
        Self { context }
    }

    pub fn read_str(&self, text: &str) -> Result<Document, ReaderError> {
        let (frontmatter, lines, start_line) = split_frontmatter(text)?;
        let blocks = self.parse_blocks(&lines, start_line)?;
        Ok(Document {
            frontmatter,
            blocks,
            source: self.context.source.clone(),
            engine: Some(Engine::Jupyter),
        })
    }

    fn parse_blocks(&self, lines: &[&str], start_line: u32) -> Result<Vec<Block>, ReaderError> {
        let mut out = Vec::new();
        let mut para = Vec::new();
        let mut para_start = 0;
        let mut para_blank = 0;
        let mut blank = 0u8;
        let mut pending_target: Option<(Label, u32, u8)> = None;
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
            if let Some(label) = parse_target(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                if let Some((label, line, blank)) = pending_target.replace((label, line_no, blank))
                {
                    out.push(block(BlockKind::Target { label }, line, line, blank));
                }
                blank = 0;
                i += 1;
                continue;
            }
            if line.trim_start().starts_with('%') {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let text = line
                    .trim_start()
                    .trim_start_matches('%')
                    .trim_start()
                    .to_string();
                push_with_target(
                    &mut out,
                    block(
                        BlockKind::Comment {
                            text,
                            style: CommentStyle::Percent,
                        },
                        line_no,
                        line_no,
                        blank,
                    ),
                    &mut pending_target,
                );
                blank = 0;
                i += 1;
                continue;
            }
            if line.trim() == "+++" {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                push_with_target(
                    &mut out,
                    block(BlockKind::BlockBreak, line_no, line_no, blank),
                    &mut pending_target,
                );
                blank = 0;
                i += 1;
                continue;
            }
            if let Some(frame) = fence::take_myst_directive(lines, i, line_no) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let b = self.directive_to_block(&frame, blank)?;
                push_with_target(&mut out, b, &mut pending_target);
                blank = 0;
                i += (frame.end_line - frame.start_line + 1) as usize;
                continue;
            }
            if let Some((fence_char, count, indent, lang)) = fence::parse_regular_code_open(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let (body, _, end) = fence::take_fenced_body(lines, i, fence_char, count, indent);
                push_with_target(
                    &mut out,
                    block(
                        BlockKind::StaticCode {
                            lang,
                            body,
                            attrs: Attrs::new(),
                        },
                        line_no,
                        start_line + end as u32,
                        blank,
                    ),
                    &mut pending_target,
                );
                blank = 0;
                i = end + 1;
                continue;
            }
            if line.trim_start().starts_with("$$") {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let (kind, end) = parse_dollar_math(lines, i);
                push_with_target(
                    &mut out,
                    block(kind, line_no, start_line + end as u32, blank),
                    &mut pending_target,
                );
                blank = 0;
                i = end + 1;
                continue;
            }
            if let Some((level, text)) = parse_heading(line) {
                flush_paragraph(&mut out, &mut para, para_start, line_no, para_blank);
                let label = pending_target.take().map(|(label, _, _)| label);
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
        if let Some((label, line, blank)) = pending_target.take() {
            out.push(block(BlockKind::Target { label }, line, line, blank));
        }
        Ok(out)
    }

    fn directive_to_block(
        &self,
        frame: &fence::DirectiveFrame,
        blank: u8,
    ) -> Result<Block, ReaderError> {
        let name = frame.open.name.as_str();
        let kind = match name {
            "code-cell" => self.code_cell(frame),
            "code" | "mermaid" => self.static_code(frame),
            "figure" | "image" => self.figure(frame),
            "table" => table(frame),
            "math" => BlockKind::Math {
                body: frame.body.clone(),
                label: label_option(&frame.options),
            },
            "include" => self.include(frame),
            "embed" => self.embed(frame),
            "raw" => BlockKind::Raw {
                format: frame.open.argument.clone(),
                body: frame.body.clone(),
            },
            "tab-set" => self.tab_set(frame)?,
            "tab-item" => BlockKind::Directive {
                name: name.to_string(),
                attrs: options_to_attrs(&frame.options),
                body: self.parse_owned_body(&frame.body, frame.start_line + 1)?,
                label: label_option(&frame.options),
            },
            "margin" | "aside" => BlockKind::Margin {
                body: self.parse_owned_body(&frame.body, frame.start_line + 1)?,
            },
            "admonition" => BlockKind::Admonition {
                kind: AdmonitionKind::Note,
                title: (!frame.open.argument.is_empty()).then(|| frame.open.argument.clone()),
                body: self.parse_owned_body(&frame.body, frame.start_line + 1)?,
                collapse: collapse_from_options(&frame.options),
            },
            n if admonition_kind(n).is_some() => BlockKind::Admonition {
                kind: admonition_kind(n).unwrap(),
                title: frame.options.get("title").cloned(),
                body: self.parse_owned_body(&frame.body, frame.start_line + 1)?,
                collapse: collapse_from_options(&frame.options),
            },
            n if n.starts_with("prf:") => BlockKind::Theorem {
                thm_type: n.trim_start_matches("prf:").to_string(),
                label: label_option(&frame.options),
                body: self.parse_owned_body(&frame.body, frame.start_line + 1)?,
            },
            "grid" | "card" | "bibliography" | "tableofcontents" => BlockKind::Directive {
                name: name.to_string(),
                attrs: options_to_attrs(&frame.options),
                body: self.parse_owned_body(&frame.body, frame.start_line + 1)?,
                label: label_option(&frame.options),
            },
            "epigraph" | "pull-quote" | "glossary" | "list-table" | "csv-table" => {
                BlockKind::Unmappable {
                    original: frame.original.clone(),
                    reason: format!("{{{name}}} has no typed target in the opposite dialect"),
                }
            }
            _ => BlockKind::Unmappable {
                original: frame.original.clone(),
                reason: format!("unrecognized MyST directive {{{name}}}"),
            },
        };
        Ok(block(kind, frame.start_line, frame.end_line, blank))
    }

    fn parse_owned_body(&self, lines: &[String], start: u32) -> Result<Vec<Block>, ReaderError> {
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        self.parse_blocks(&refs, start)
    }

    fn code_cell(&self, frame: &fence::DirectiveFrame) -> BlockKind {
        let mut options = cell_options_from_myst(&frame.options);
        let (quarto_options, consumed) = parse_cell_options(&frame.body);
        options.extra.extend(quarto_options.extra);
        let body = frame.body[consumed..].to_vec();
        BlockKind::CodeCell {
            lang: frame.open.argument.clone(),
            options,
            body,
            label: label_option(&frame.options),
        }
    }

    fn static_code(&self, frame: &fence::DirectiveFrame) -> BlockKind {
        let lang = if frame.open.name == "mermaid" {
            Some("mermaid".to_string())
        } else if frame.open.argument.is_empty() {
            None
        } else {
            Some(frame.open.argument.clone())
        };
        BlockKind::StaticCode {
            lang,
            body: frame.body.clone(),
            attrs: options_to_attrs(&frame.options),
        }
    }

    fn figure(&self, frame: &fence::DirectiveFrame) -> BlockKind {
        let src = if let Some(cell) = frame.open.argument.strip_prefix("#nb:") {
            let raw = format!("nb:{cell}");
            let Some(found) = self.context.notebook_index.resolve(&raw) else {
                return BlockKind::Unmappable {
                    original: frame.original.clone(),
                    reason: format!(
                        "{raw} does not resolve to a notebook cell in the conversion set"
                    ),
                };
            };
            FigureSource::CellRef {
                label: Label::new(raw),
                notebook: Some(found.notebook.clone()),
            }
        } else {
            FigureSource::Path(PathBuf::from(&frame.open.argument))
        };
        BlockKind::Figure {
            src,
            caption: trim_blank_lines(&frame.body),
            label: label_option(&frame.options),
            attrs: options_to_attrs(&frame.options),
        }
    }

    fn include(&self, frame: &fence::DirectiveFrame) -> BlockKind {
        let target = PathBuf::from(&frame.open.argument);
        match self.context.resolve_include(&target) {
            Ok(path) => BlockKind::Include {
                target: path,
                opts: include_opts(&frame.options),
            },
            Err(e) => BlockKind::Unmappable {
                original: frame.original.clone(),
                reason: e.to_string(),
            },
        }
    }

    fn embed(&self, frame: &fence::DirectiveFrame) -> BlockKind {
        let raw = frame.open.argument.trim_start_matches('#');
        BlockKind::Embed {
            target: EmbedTarget::Label(Label::new(raw)),
            label: label_option(&frame.options),
        }
    }

    fn tab_set(&self, frame: &fence::DirectiveFrame) -> Result<BlockKind, ReaderError> {
        let refs: Vec<&str> = frame.body.iter().map(String::as_str).collect();
        let mut items = Vec::new();
        let mut i = 0;
        while i < refs.len() {
            let line_no = frame.start_line + 1 + i as u32;
            if let Some(item) = fence::take_myst_directive(&refs, i, line_no) {
                if item.open.name == "tab-item" {
                    items.push(crate::TabItem {
                        label: item.open.argument.clone(),
                        body: self.parse_owned_body(&item.body, item.start_line + 1)?,
                    });
                    i += (item.end_line - item.start_line + 1) as usize;
                    continue;
                }
            }
            i += 1;
        }
        Ok(BlockKind::TabSet { items })
    }
}

fn push_with_target(
    out: &mut Vec<Block>,
    mut block: Block,
    pending: &mut Option<(Label, u32, u8)>,
) {
    if let Some((label, line, blank)) = pending.take() {
        if !attach_label(&mut block, label.clone()) {
            out.push(crate::reader::block(
                BlockKind::Target { label },
                line,
                line,
                blank,
            ));
        }
    }
    out.push(block);
}

fn attach_label(block: &mut Block, label: Label) -> bool {
    match &mut block.kind {
        BlockKind::Heading { label: slot, .. }
        | BlockKind::Figure { label: slot, .. }
        | BlockKind::Table { label: slot, .. }
        | BlockKind::Math { label: slot, .. }
        | BlockKind::CodeCell { label: slot, .. }
        | BlockKind::Theorem { label: slot, .. }
        | BlockKind::Directive { label: slot, .. } => {
            if slot.is_none() {
                *slot = Some(label);
                true
            } else {
                false
            }
        }
        _ => false,
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

fn parse_target(line: &str) -> Option<Label> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('(')?.strip_suffix(")=")?;
    (!inner.is_empty()).then(|| Label::new(inner))
}

fn parse_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) || !trimmed[level..].starts_with(' ') {
        return None;
    }
    Some((level as u8, trimmed[level + 1..].to_string()))
}

impl MystReader {
    fn push_preserved_or_marker(
        &self,
        out: &mut Vec<Block>,
        id: &str,
        line: u32,
        blank: u8,
    ) -> Result<(), ReaderError> {
        if let Some(original) = self.context.preserved.get(id) {
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

fn cell_options_from_myst(options: &BTreeMap<String, String>) -> crate::CellOptions {
    let mut out = crate::CellOptions::default();
    if let Some(tags) = options.get("tags") {
        out.tags = tags
            .trim_matches(['[', ']'])
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    out.caption = options.get("caption").cloned();
    for (key, value) in options {
        if !matches!(key.as_str(), "tags" | "caption" | "label" | "name") {
            out.extra.insert(key.clone(), value.clone());
        }
    }
    out
}

fn table(frame: &fence::DirectiveFrame) -> BlockKind {
    let table_start = frame
        .body
        .iter()
        .position(|l| l.trim_start().starts_with('|'))
        .unwrap_or(frame.body.len());
    BlockKind::Table {
        caption: trim_blank_lines(&frame.body[..table_start]),
        rows: frame.body[table_start..].to_vec(),
        label: label_option(&frame.options),
    }
}

fn parse_dollar_math(lines: &[&str], start: usize) -> (BlockKind, usize) {
    let mut body = Vec::new();
    let mut i = start + 1;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("$$") {
            return (BlockKind::Math { body, label: None }, i);
        }
        body.push(lines[i].to_string());
        i += 1;
    }
    (
        BlockKind::Math { body, label: None },
        lines.len().saturating_sub(1),
    )
}

fn include_opts(options: &BTreeMap<String, String>) -> IncludeOpts {
    IncludeOpts {
        literal: options.contains_key("literal"),
        lang: options.get("lang").cloned(),
        start_line: options.get("start-line").and_then(|s| s.parse().ok()),
        end_line: options.get("end-line").and_then(|s| s.parse().ok()),
        lines: options.get("lines").cloned(),
    }
}

fn options_to_attrs(options: &BTreeMap<String, String>) -> Attrs {
    options
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "label" | "name"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn collapse_from_options(options: &BTreeMap<String, String>) -> Option<bool> {
    if options.get("class").is_some_and(|v| v.contains("dropdown")) {
        Some(!matches!(
            options.get("open").map(String::as_str),
            Some("true")
        ))
    } else {
        None
    }
}

fn admonition_kind(name: &str) -> Option<AdmonitionKind> {
    match name {
        "note" => Some(AdmonitionKind::Note),
        "warning" => Some(AdmonitionKind::Warning),
        "tip" => Some(AdmonitionKind::Tip),
        "important" => Some(AdmonitionKind::Important),
        "caution" => Some(AdmonitionKind::Caution),
        "danger" => Some(AdmonitionKind::Danger),
        "error" => Some(AdmonitionKind::Error),
        "hint" => Some(AdmonitionKind::Hint),
        "seealso" => Some(AdmonitionKind::SeeAlso),
        "attention" => Some(AdmonitionKind::Attention),
        _ => None,
    }
}

fn trim_blank_lines(lines: &[String]) -> Vec<String> {
    let start = lines
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(start, |i| i + 1);
    lines[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::{NotebookCellIndex, PreservationStore};

    #[test]
    fn parses_targets_comments_labels_and_captions() {
        let reader = MystReader::new(ReaderContext::new("article.md"));
        let doc = reader
            .read_str("(sec:data)=\n\n## Data\n\n% comment\n\n50% of users\n\n:::{figure} img.png\n:name: fig:old\n:label: fig:new\nCaption\n:::\n")
            .unwrap();
        assert!(
            matches!(&doc.blocks[0].kind, BlockKind::Heading { label: Some(l), .. } if l.raw == "sec:data")
        );
        assert!(matches!(
            &doc.blocks[1].kind,
            BlockKind::Comment {
                style: CommentStyle::Percent,
                ..
            }
        ));
        assert!(
            matches!(&doc.blocks[2].kind, BlockKind::Paragraph { lines } if lines[0] == "50% of users")
        );
        assert!(
            matches!(&doc.blocks[3].kind, BlockKind::Figure { label: Some(l), caption, .. } if l.raw == "fig:new" && caption == &vec!["Caption".to_string()])
        );
    }

    #[test]
    fn resolves_notebook_cell_refs_through_index() {
        let mut index = NotebookCellIndex::default();
        index.insert("nb:analysis", "analysis.ipynb", 0);
        let reader = MystReader::new(ReaderContext {
            notebook_index: index,
            ..ReaderContext::new("article.md")
        });
        let doc = reader.read_str(":::{figure} #nb:analysis\n:::\n").unwrap();
        assert!(
            matches!(&doc.blocks[0].kind, BlockKind::Figure { src: FigureSource::CellRef { notebook: Some(p), .. }, .. } if p == &PathBuf::from("analysis.ipynb"))
        );
    }

    #[test]
    fn preserved_marker_reparses_original_block() {
        let mut preserved = PreservationStore::default();
        preserved.insert("b7f3", vec!["% restored".to_string()]);
        let reader = MystReader::new(ReaderContext {
            preserved,
            ..ReaderContext::new("article.md")
        });
        let doc = reader
            .read_str("<!-- mystquarto MQ0203: preserved see .mystquarto/preserved.json#b7f3 -->\n")
            .unwrap();
        assert!(
            matches!(&doc.blocks[0].kind, BlockKind::Comment { text, .. } if text == "restored")
        );
    }
}
