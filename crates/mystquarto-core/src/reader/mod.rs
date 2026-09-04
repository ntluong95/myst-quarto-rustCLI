//! MyST and Quarto source readers.
//!
//! The readers are intentionally lightweight block scanners over the Phase 3
//! IR. They recognize dialect constructs into typed `BlockKind` variants and
//! retain unrecognized source as `Unmappable` instead of discarding it.

pub mod fence;
pub mod inline;
pub mod mask;
pub mod myst;
pub mod quarto;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::fs::path_guard::{guard_target, IncludeChain, PathGuardError};
use crate::ir::Frontmatter;
use crate::yaml::{parse_mapping, YamlReadError, YamlValue};
use crate::{Block, BlockKind, Label, Span};

pub use myst::MystReader;
pub use quarto::QuartoReader;

/// Shared reader state for one conversion set.
#[derive(Debug, Clone, Default)]
pub struct ReaderContext {
    pub source: PathBuf,
    pub input_root: Option<PathBuf>,
    pub notebook_index: NotebookCellIndex,
    pub preserved: PreservationStore,
    pub include_chain: IncludeChain,
}

impl ReaderContext {
    #[must_use]
    pub fn new(source: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_input_root(mut self, input_root: impl Into<PathBuf>) -> Self {
        self.input_root = Some(input_root.into());
        self
    }

    #[must_use]
    pub fn source_dir(&self) -> PathBuf {
        self.source
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub(crate) fn resolve_include(&self, target: &Path) -> Result<PathBuf, PathGuardError> {
        if target.is_absolute() {
            return Err(PathGuardError::AbsoluteTarget {
                path: target.to_path_buf(),
            });
        }
        let chain_target = if let Some(root) = &self.input_root {
            guard_target(root, &self.source_dir(), target)?
        } else {
            target.to_path_buf()
        };
        let mut chain = self.include_chain.clone();
        chain.push(chain_target)?;
        Ok(target.to_path_buf())
    }
}

/// `#| label:` references found in notebooks or executable Quarto documents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotebookCellIndex {
    cells: BTreeMap<String, NotebookCellRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookCellRef {
    pub notebook: PathBuf,
    pub cell_index: usize,
}

impl NotebookCellIndex {
    pub fn insert(
        &mut self,
        label: impl Into<String>,
        notebook: impl Into<PathBuf>,
        cell_index: usize,
    ) {
        self.cells.insert(
            label.into(),
            NotebookCellRef {
                notebook: notebook.into(),
                cell_index,
            },
        );
    }

    #[must_use]
    pub fn resolve(&self, label: &str) -> Option<&NotebookCellRef> {
        self.cells.get(label)
    }

    pub fn add_notebook_json(
        &mut self,
        path: impl Into<PathBuf>,
        text: &str,
    ) -> Result<(), ReaderError> {
        let path = path.into();
        let value: serde_json::Value = serde_json::from_str(text)?;
        let Some(cells) = value.get("cells").and_then(|v| v.as_array()) else {
            return Ok(());
        };
        for (idx, cell) in cells.iter().enumerate() {
            if let Some(source) = cell.get("source") {
                for line in json_source_lines(source) {
                    if let Some(label) = parse_cell_option(&line, "label") {
                        self.insert(label, path.clone(), idx);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn add_qmd_cells(&mut self, path: impl Into<PathBuf>, text: &str) {
        let path = path.into();
        let mut cell_index: usize = 0;
        for line in text.lines() {
            if fence::parse_quarto_code_open(line).is_some() {
                cell_index += 1;
            }
            if let Some(label) = parse_cell_option(line, "label") {
                self.insert(label, path.clone(), cell_index.saturating_sub(1));
            }
        }
    }
}

/// Sidecar entries keyed by the id in a preservation marker comment, each
/// carrying the dialect its content was captured in — see
/// [`crate::preserve::Dialect`]'s docs for why a reader must know this
/// before ever attempting to reparse restored content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreservationStore {
    entries: BTreeMap<String, (crate::preserve::Dialect, Vec<String>)>,
}

impl PreservationStore {
    /// Inserts an entry of unknown dialect — for callers (mainly tests)
    /// that don't need the dialect-matching safety [`Self::insert_dialect`]
    /// provides; a reader treats `Dialect::Unknown` as foreign to every
    /// dialect, so this never risks a cross-dialect reparse.
    pub fn insert(&mut self, id: impl Into<String>, original: Vec<String>) {
        self.insert_dialect(id, crate::preserve::Dialect::Unknown, original);
    }

    pub fn insert_dialect(
        &mut self,
        id: impl Into<String>,
        dialect: crate::preserve::Dialect,
        original: Vec<String>,
    ) {
        self.entries.insert(id.into(), (dialect, original));
    }

    /// Returns `original` only when `dialect` matches the entry's recorded
    /// dialect — the actual fix: a reader can now only ever reparse content
    /// it can prove is native to itself. Returns `None`, not the wrong
    /// dialect's content, on a mismatch; callers fall back to an opaque
    /// [`crate::BlockKind::Preserved`] instead (see
    /// `crate::reader::myst::MystReader::push_preserved_or_marker`).
    #[must_use]
    pub fn get_matching(
        &self,
        id: &str,
        dialect: crate::preserve::Dialect,
    ) -> Option<&Vec<String>> {
        let (entry_dialect, original) = self.entries.get(id)?;
        (*entry_dialect == dialect).then_some(original)
    }

    /// Returns the entry's content regardless of dialect — for the "does
    /// *any* entry exist for this id" question (an opaque
    /// `BlockKind::Preserved` restore doesn't care which dialect it's in;
    /// it never gets reparsed).
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Vec<String>> {
        self.entries.get(id).map(|(_, original)| original)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReaderError {
    #[error("{0}")]
    PathGuard(#[from] PathGuardError),
    #[error("{0}")]
    Yaml(#[from] YamlReadError),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

pub(crate) fn split_frontmatter(
    text: &str,
) -> Result<(Option<Frontmatter>, Vec<&str>, u32), ReaderError> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.first().copied() != Some("---") {
        return Ok((None, lines, 1));
    }
    let Some(end) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(i, l)| (*l == "---").then_some(i))
    else {
        return Ok((None, lines, 1));
    };
    let raw = lines[1..end].join("\n") + "\n";
    let parsed = YamlValue::Mapping(parse_mapping(&raw)?);
    Ok((
        Some(Frontmatter { raw, parsed }),
        lines[end + 1..].to_vec(),
        end as u32 + 2,
    ))
}

pub(crate) fn block(kind: BlockKind, start: u32, end: u32, blank: u8) -> Block {
    Block {
        kind,
        span: Span::new(start, end.max(start)),
        blank_lines_before: blank,
    }
}

pub(crate) fn label_option(options: &BTreeMap<String, String>) -> Option<Label> {
    options
        .get("label")
        .or_else(|| options.get("name"))
        .filter(|s| !s.is_empty())
        .map(|s| Label::new(s.clone()))
}

pub(crate) fn preservation_marker_id(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if !trimmed.starts_with("<!-- mystquarto ") || !trimmed.ends_with("-->") {
        return None;
    }
    // Resolves from the *last* occurrence of the needle, not the first —
    // defense in depth alongside `crate::preserve::marker`'s own escaping
    // of this exact string inside `kind`: `marker()` guarantees only one
    // real occurrence today, but this reader has no way to verify that
    // guarantee held (a hand-edited file, or a future `marker()` bug,
    // could reintroduce it) — the *last* occurrence is the one nearest the
    // line's own trailing `-->`, which is the real anchor, not whatever an
    // earlier, forged occurrence claims.
    let (_, after) = trimmed.rsplit_once(".mystquarto/preserved.json#")?;
    after
        .split_whitespace()
        .next()
        .map(|s| s.trim_end_matches("-->").trim())
        .filter(|s| !s.is_empty())
}

pub(crate) fn parse_cell_options(lines: &[String]) -> (crate::CellOptions, usize) {
    let mut opts = crate::CellOptions::default();
    let mut consumed = 0;
    for line in lines {
        let Some((key, value)) = parse_quarto_option_pair(line) else {
            break;
        };
        consumed += 1;
        match key.as_str() {
            "fig-cap" => opts.caption = Some(unquote(&value)),
            "echo" if value == "false" => opts.tags.push("remove-input".to_string()),
            "output" if value == "false" => opts.tags.push("remove-output".to_string()),
            "include" if value == "false" => opts.tags.push("remove-cell".to_string()),
            "code-fold" if value == "true" => opts.tags.push("hide-input".to_string()),
            _ => {
                opts.extra.insert(key, value);
            }
        }
    }
    (opts, consumed)
}

pub(crate) fn parse_cell_option(line: &str, wanted: &str) -> Option<String> {
    parse_quarto_option_pair(line).and_then(|(key, value)| (key == wanted).then(|| unquote(&value)))
}

fn parse_quarto_option_pair(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start().strip_prefix("#|")?.trim_start();
    let (key, value) = rest.split_once(':')?;
    Some((key.trim().to_string(), value.trim().to_string()))
}

pub(crate) fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn json_source_lines(source: &serde_json::Value) -> Vec<String> {
    if let Some(s) = source.as_str() {
        return s.lines().map(str::to_string).collect();
    }
    source
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim_end_matches('\n').to_string())
                .collect()
        })
        .unwrap_or_default()
}
