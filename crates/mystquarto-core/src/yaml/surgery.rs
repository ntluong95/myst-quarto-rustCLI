//! Targeted line-level edits to an **existing** frontmatter block — the fix
//! for D9 (`tests/corpus/defects/d09-block-scalar-mangled/`).
//!
//! The defect: the Python implementation edits frontmatter by
//! `yaml.safe_load` + mutate + `yaml.dump`. Round-tripping through a parsed
//! value loses everything the parse doesn't model — block-scalar style
//! (`abstract: |` becomes a single-quoted folded string), comments, and
//! (with a naive dumper) key order.
//!
//! [`apply_edits`] never re-serializes the whole document. It slices the
//! **original text** into one segment per top-level key (plus a segment for
//! stray leading/trailing comments), and only the segments named by an edit
//! are replaced — using the same field-rendering routine the synthesis
//! emitter (`super::emit`) uses internally, so a `Set` with a
//! [`super::YamlValue::BlockLiteral`] produces the same `|` style. Every
//! other segment's lines are copied through byte-for-byte from the input.
//!
//! # Scope
//!
//! This targets flat, single-document, single-level frontmatter mappings —
//! exactly the shape reference §8.4's page-frontmatter keys have. It does
//! not parse nested structure within a segment; a segment's continuation
//! lines (block scalars, nested sequences/mappings under a key) are opaque
//! text, copied verbatim. Editing a key that holds a nested structure
//! replaces the whole thing via [`FrontmatterEdit::Set`], not a sub-path.
//!
//! A standalone `#`-comment or blank line at column 0 is attached to the
//! **next** key's segment (the common "comment documents the following
//! key" convention). This means [`FrontmatterEdit::Remove`] on a key also
//! removes any comment lines immediately above it — documented here rather
//! than guessed at silently. CRLF line endings are normalized to LF on
//! output (`str::lines` strips a trailing `\r`); the phase spec's Risk
//! Assessment flags CRLF/tabs/multi-document YAML as inputs that could
//! defeat targeted edits and names "fall back to full re-emission and warn"
//! as the mitigation — that fallback is not implemented in this phase, only
//! the primary mechanism the success criteria require.

use super::{emit::render_field, YamlValue};

/// A single change to apply to a frontmatter block. Expressive enough to
/// cover Phase 6's config-field remapping (rename `subject` → `categories`,
/// drop `abbreviations` with no Quarto target, set `open_access` as a
/// preserved comment, …) without needing a richer API yet.
#[derive(Debug, Clone, PartialEq)]
pub enum FrontmatterEdit {
    /// Set `key` to `value`, rewriting only that key's line(s). If `key`
    /// does not exist, a new segment is appended at the end.
    Set { key: String, value: YamlValue },
    /// Remove `key` and its value lines entirely (see module docs re:
    /// attached leading comments). Errors if `key` does not exist.
    Remove { key: String },
    /// Rename `key` from `from` to `to`, keeping its value lines and style
    /// byte-identical — only the key token on the first line changes.
    /// Errors if `from` does not exist.
    Rename { from: String, to: String },
}

/// Error applying a [`FrontmatterEdit`].
#[derive(Debug, PartialEq, Eq)]
pub enum SurgeryError {
    /// [`FrontmatterEdit::Remove`] or the `from` side of
    /// [`FrontmatterEdit::Rename`] named a key not present in the text.
    KeyNotFound(String),
}

impl std::fmt::Display for SurgeryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SurgeryError::KeyNotFound(k) => write!(f, "frontmatter key not found: {k:?}"),
        }
    }
}

impl std::error::Error for SurgeryError {}

/// One top-level key's worth of original text, plus any comment/blank
/// lines immediately preceding it. `key`/`key_line` are `None` only for a
/// trailing keyless segment holding comments/blanks that appear after the
/// last key (nothing follows them to attach to).
struct Segment {
    key: Option<String>,
    /// Comment/blank lines immediately above `key_line`, verbatim.
    leading: Vec<String>,
    /// The exact original `key: value` (or `key:`) line. `None` only for
    /// the trailing keyless segment.
    key_line: Option<String>,
    /// Lines after `key_line` and before the next segment: block-scalar
    /// content, nested sequence/mapping lines, or (rarely) more
    /// comments/blanks that happened not to precede a following key.
    continuation: Vec<String>,
}

impl Segment {
    fn into_lines(self) -> Vec<String> {
        let mut out = self.leading;
        if let Some(k) = self.key_line {
            out.push(k);
        }
        out.extend(self.continuation);
        out
    }
}

/// Returns the key name if `line` opens a new top-level mapping entry:
/// unindented, not a comment, not a sequence item, and its text before the
/// first `:` looks like a key (the `:` is followed by end-of-line or a
/// space — rejects things like a bare `http://...` value line that could
/// only appear, incorrectly unindented, in malformed input).
fn top_level_key(line: &str) -> Option<String> {
    if line.starts_with(' ') || line.starts_with('\t') {
        return None;
    }
    let trimmed = line.trim_end();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let colon_pos = trimmed.find(':')?;
    let key_part = trimmed[..colon_pos].trim();
    if key_part.is_empty() {
        return None;
    }
    let rest = &trimmed[colon_pos + 1..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some(key_part.to_string())
}

/// True for an unindented comment or blank line — the two things that can
/// legally sit between top-level keys outside of a key's own continuation.
fn is_top_level_comment_or_blank(line: &str) -> bool {
    if line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#')
}

/// A key segment being accumulated, before it's known where it ends.
/// `leading` is captured from `pending_leading` at the moment this segment
/// *starts* (not when it finalizes) — see [`segment_text`].
struct InProgress {
    leading: Vec<String>,
    key: String,
    key_line: String,
    continuation: Vec<String>,
}

fn finalize(current: &mut Option<InProgress>, segments: &mut Vec<Segment>) {
    if let Some(seg) = current.take() {
        segments.push(Segment {
            key: Some(seg.key),
            leading: seg.leading,
            key_line: Some(seg.key_line),
            continuation: seg.continuation,
        });
    }
}

/// True if `line` — already known to be a top-level `key: ...` line — opens
/// a YAML block scalar (`|` or `>`, with an optional chomping indicator
/// `-`/`+` and/or an explicit indentation digit, in either order, optionally
/// followed by a comment). Once inside one, every following line — blank or
/// indented — belongs to its content until a non-blank, unindented line
/// appears (YAML block scalars are indentation-delimited; a blank line,
/// even a zero-width one between paragraphs, never ends one). Without
/// tracking this, [`segment_text`] would treat a blank line inside a
/// multi-paragraph `abstract: |` as the top-level blank/comment separator
/// it is everywhere else, splitting the scalar's tail into its own
/// keyless segment that a later edit on a *different* key could delete.
fn is_block_scalar_opener(line: &str) -> bool {
    let trimmed = line.trim_end();
    let Some(colon) = trimmed.find(':') else {
        return false;
    };
    let rest = trimmed[colon + 1..].trim_start();
    let mut chars = rest.chars();
    match chars.next() {
        Some('|' | '>') => {}
        _ => return false,
    }
    let indicator_part = chars.as_str().split('#').next().unwrap_or("").trim();
    indicator_part
        .chars()
        .all(|c| c == '-' || c == '+' || c.is_ascii_digit())
}

fn segment_text(text: &str) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    // Comment/blank lines seen since the last segment was finalized, not
    // yet attached to any segment. Drained into a new key segment's
    // `leading` the moment that segment starts (comments document the
    // *following* key); left over at end-of-input, they become a trailing
    // keyless segment.
    let mut pending_leading: Vec<String> = Vec::new();
    let mut current: Option<InProgress> = None;
    // `true` from the line after a block-scalar-opening key line until a
    // non-blank, unindented line ends it — see `is_block_scalar_opener`.
    let mut in_block_scalar = false;

    for line in text.lines() {
        if in_block_scalar {
            let is_blank = line.trim().is_empty();
            let is_indented = line.starts_with(' ') || line.starts_with('\t');
            if is_blank || is_indented {
                if let Some(seg) = current.as_mut() {
                    seg.continuation.push(line.to_string());
                }
                continue;
            }
            in_block_scalar = false;
        }

        if let Some(key) = top_level_key(line) {
            finalize(&mut current, &mut segments);
            in_block_scalar = is_block_scalar_opener(line);
            current = Some(InProgress {
                leading: std::mem::take(&mut pending_leading),
                key,
                key_line: line.to_string(),
                continuation: Vec::new(),
            });
        } else if is_top_level_comment_or_blank(line) {
            // Whatever key was in progress ends here — a column-0
            // comment/blank can't be part of its continuation (that would
            // be indented). Finalize it with the `leading` it was given
            // when *it* started, then begin buffering for whatever is next.
            finalize(&mut current, &mut segments);
            pending_leading.push(line.to_string());
        } else if let Some(seg) = current.as_mut() {
            seg.continuation.push(line.to_string());
        } else {
            // A continuation-shaped line with no open key segment:
            // malformed input for our narrow grammar. Keep it rather than
            // drop it, as a keyless leading line, so surgery never
            // silently loses text.
            pending_leading.push(line.to_string());
        }
    }
    finalize(&mut current, &mut segments);
    if !pending_leading.is_empty() {
        segments.push(Segment {
            key: None,
            leading: pending_leading,
            key_line: None,
            continuation: Vec::new(),
        });
    }
    segments
}

fn find_index(segments: &[Segment], key: &str) -> Option<usize> {
    segments.iter().position(|s| s.key.as_deref() == Some(key))
}

/// Applies `edits` to `original` (the frontmatter's text, delimiters
/// excluded) and returns the edited text. Keys untouched by any edit are
/// copied through byte-for-byte, including their block-scalar style and any
/// leading comment lines.
///
/// # Errors
/// Returns [`SurgeryError::KeyNotFound`] if [`FrontmatterEdit::Remove`] or
/// [`FrontmatterEdit::Rename`]'s `from` names a key not present in
/// `original`.
pub fn apply_edits(original: &str, edits: &[FrontmatterEdit]) -> Result<String, SurgeryError> {
    let mut segments = segment_text(original);

    for edit in edits {
        match edit {
            FrontmatterEdit::Set { key, value } => {
                let rendered = render_field(key, value, 0);
                let mut lines = rendered.lines().map(str::to_string);
                let key_line = lines.next().unwrap_or_default();
                let continuation: Vec<String> = lines.collect();
                match find_index(&segments, key) {
                    Some(i) => {
                        segments[i].key_line = Some(key_line);
                        segments[i].continuation = continuation;
                    }
                    None => segments.push(Segment {
                        key: Some(key.clone()),
                        leading: Vec::new(),
                        key_line: Some(key_line),
                        continuation,
                    }),
                }
            }
            FrontmatterEdit::Remove { key } => {
                let i = find_index(&segments, key)
                    .ok_or_else(|| SurgeryError::KeyNotFound(key.clone()))?;
                segments.remove(i);
            }
            FrontmatterEdit::Rename { from, to } => {
                let i = find_index(&segments, from)
                    .ok_or_else(|| SurgeryError::KeyNotFound(from.clone()))?;
                let old_line = segments[i].key_line.as_deref().unwrap_or_default();
                let colon = old_line
                    .find(':')
                    .expect("key segments always have a colon");
                segments[i].key_line = Some(format!("{to}{}", &old_line[colon..]));
                segments[i].key = Some(to.clone());
            }
        }
    }

    let ends_with_newline = original.ends_with('\n');
    let mut all_lines: Vec<String> = Vec::new();
    for seg in segments {
        all_lines.extend(seg.into_lines());
    }
    let mut out = all_lines.join("\n");
    if ends_with_newline && !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::YamlValue;

    /// D9's real fixture input, embedded at compile time.
    const D09_INPUT: &str =
        include_str!("../../../../tests/corpus/defects/d09-block-scalar-mangled/input.md");

    /// Strips the `---`-delimited frontmatter out of a full `.md` file,
    /// returning just its inner text — the shape [`apply_edits`] expects.
    fn extract_frontmatter(full_text: &str) -> &str {
        let after_open = full_text
            .strip_prefix("---\n")
            .expect("fixture starts with ---");
        let end = after_open.find("\n---").expect("fixture has a closing ---");
        &after_open[..end + 1] // keep the trailing newline before the closing ---
    }

    #[test]
    fn d09_abstract_block_literal_survives_unrelated_edit_byte_identical() {
        let fm = extract_frontmatter(D09_INPUT);
        let edits = [FrontmatterEdit::Set {
            key: "title".to_string(),
            value: YamlValue::String("Renamed".to_string()),
        }];
        let out = apply_edits(fm, &edits).expect("edit applies");

        let expected_abstract_block = "abstract: |\n  This is a multi-line abstract\n  with a hard line break preserved\n  by the block literal style.\n";
        assert!(
            out.contains(expected_abstract_block),
            "abstract block scalar must survive byte-identically; got:\n{out}"
        );
        assert!(
            out.contains("title: Renamed"),
            "title must be updated; got:\n{out}"
        );
        assert!(
            !out.contains("Sample Article"),
            "old title value must be gone; got:\n{out}"
        );
    }

    #[test]
    fn set_on_unrelated_key_preserves_block_style_and_key_order() {
        let original = "title: Foo\nabstract: |\n  line one\n  line two\ndescription: Bar\n";
        let edits = [FrontmatterEdit::Set {
            key: "description".to_string(),
            value: YamlValue::String("Baz".to_string()),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert_eq!(
            out,
            "title: Foo\nabstract: |\n  line one\n  line two\ndescription: Baz\n"
        );
    }

    #[test]
    fn comments_and_key_order_preserved() {
        let original = "# leading comment\ntitle: Foo\n# comment about abstract\nabstract: |\n  line one\n  line two\ndescription: Bar\n";
        let edits = [FrontmatterEdit::Set {
            key: "description".to_string(),
            value: YamlValue::String("Baz".to_string()),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert_eq!(
            out,
            "# leading comment\ntitle: Foo\n# comment about abstract\nabstract: |\n  line one\n  line two\ndescription: Baz\n"
        );
    }

    #[test]
    fn block_chomp_strip_style_preserved() {
        // `|-` strips the trailing newline; must survive verbatim.
        let original = "title: Foo\nnotes: |-\n  no trailing blank line\ndescription: Bar\n";
        let edits = [FrontmatterEdit::Set {
            key: "title".to_string(),
            value: YamlValue::String("New".to_string()),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert!(out.contains("notes: |-\n  no trailing blank line\n"));
    }

    #[test]
    fn folded_block_style_preserved() {
        // `>` folds newlines into spaces; must survive verbatim.
        let original = "title: Foo\nsummary: >\n  folded\n  text\ndescription: Bar\n";
        let edits = [FrontmatterEdit::Set {
            key: "title".to_string(),
            value: YamlValue::String("New".to_string()),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert!(out.contains("summary: >\n  folded\n  text\n"));
    }

    #[test]
    fn set_new_key_appends_at_end() {
        let original = "title: Foo\n";
        let edits = [FrontmatterEdit::Set {
            key: "doi".to_string(),
            value: YamlValue::String("10.1/x".to_string()),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert_eq!(out, "title: Foo\ndoi: 10.1/x\n");
    }

    #[test]
    fn set_block_literal_value_emits_pipe_style() {
        let original = "title: Foo\n";
        let edits = [FrontmatterEdit::Set {
            key: "abstract".to_string(),
            value: YamlValue::BlockLiteral("line one\nline two\n".to_string()),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert_eq!(out, "title: Foo\nabstract: |\n  line one\n  line two\n");
    }

    #[test]
    fn remove_deletes_key_and_value_lines() {
        let original = "title: Foo\nabstract: |\n  line one\n  line two\ndescription: Bar\n";
        let edits = [FrontmatterEdit::Remove {
            key: "abstract".to_string(),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert_eq!(out, "title: Foo\ndescription: Bar\n");
    }

    #[test]
    fn remove_missing_key_errors() {
        let original = "title: Foo\n";
        let edits = [FrontmatterEdit::Remove {
            key: "nope".to_string(),
        }];
        assert_eq!(
            apply_edits(original, &edits),
            Err(SurgeryError::KeyNotFound("nope".to_string()))
        );
    }

    #[test]
    fn rename_keeps_value_lines_byte_identical() {
        let original = "subject: Biology\nabstract: |\n  line one\n  line two\n";
        let edits = [FrontmatterEdit::Rename {
            from: "subject".to_string(),
            to: "categories".to_string(),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert_eq!(
            out,
            "categories: Biology\nabstract: |\n  line one\n  line two\n"
        );
    }

    #[test]
    fn rename_missing_key_errors() {
        let original = "title: Foo\n";
        let edits = [FrontmatterEdit::Rename {
            from: "nope".to_string(),
            to: "x".to_string(),
        }];
        assert_eq!(
            apply_edits(original, &edits),
            Err(SurgeryError::KeyNotFound("nope".to_string()))
        );
    }

    #[test]
    fn blank_line_inside_block_scalar_survives_an_unrelated_key_removal() {
        // Regression: a blank line between paragraphs of a multi-paragraph
        // `abstract: |` used to be misread as the top-level separator
        // between keys, splitting the second paragraph into its own
        // segment that got silently deleted by an edit on a later key.
        let original = "title: Foo\nabstract: |\n  First paragraph.\n\n  Second paragraph.\nkernelspec:\n  name: python3\n";
        let edits = [FrontmatterEdit::Remove {
            key: "kernelspec".to_string(),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert_eq!(
            out,
            "title: Foo\nabstract: |\n  First paragraph.\n\n  Second paragraph.\n"
        );
    }

    #[test]
    fn blank_line_inside_folded_block_scalar_survives_an_unrelated_key_removal() {
        let original =
            "title: Foo\nsummary: >\n  First.\n\n  Second.\nkernelspec:\n  name: python3\n";
        let edits = [FrontmatterEdit::Remove {
            key: "kernelspec".to_string(),
        }];
        let out = apply_edits(original, &edits).expect("edit applies");
        assert_eq!(out, "title: Foo\nsummary: >\n  First.\n\n  Second.\n");
    }

    #[test]
    fn no_edits_returns_input_byte_identical() {
        let original = "title: Foo\nabstract: |\n  line one\n  line two\n# trailing comment\n";
        let out = apply_edits(original, &[]).expect("no edits always applies");
        assert_eq!(out, original);
    }
}
