//! Fence recognition shared by the readers.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveOpen {
    pub indent: usize,
    pub fence_char: char,
    pub fence_count: usize,
    pub name: String,
    pub argument: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveFrame {
    pub open: DirectiveOpen,
    pub options: BTreeMap<String, String>,
    pub body: Vec<String>,
    pub original: Vec<String>,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuartoDivOpen {
    pub indent: usize,
    pub fence_count: usize,
    pub attrs: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuartoCodeOpen {
    pub indent: usize,
    pub fence_count: usize,
    pub lang: String,
}

#[must_use]
pub fn parse_myst_open(line: &str) -> Option<DirectiveOpen> {
    parse_braced_open(line, '`').or_else(|| parse_braced_open(line, ':'))
}

#[must_use]
pub fn parse_quarto_code_open(line: &str) -> Option<QuartoCodeOpen> {
    let indent = count_indent(line);
    let trimmed = line.trim_start();
    let fence_count = trimmed.chars().take_while(|c| *c == '`').count();
    if fence_count < 3 {
        return None;
    }
    let rest = trimmed[fence_count..].strip_prefix('{')?;
    let close = rest.find('}')?;
    (rest[close + 1..].trim().is_empty()).then(|| QuartoCodeOpen {
        indent,
        fence_count,
        lang: rest[..close].trim().to_string(),
    })
}

#[must_use]
pub fn parse_regular_code_open(line: &str) -> Option<(char, usize, usize, Option<String>)> {
    let indent = count_indent(line);
    let trimmed = line.trim_start();
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed.chars().take_while(|c| *c == first).count();
    if count < 3 || trimmed.starts_with("```{") {
        return None;
    }
    let lang = trimmed[count..].trim();
    Some((
        first,
        count,
        indent,
        (!lang.is_empty()).then(|| lang.to_string()),
    ))
}

#[must_use]
pub fn parse_quarto_div_open(line: &str) -> Option<QuartoDivOpen> {
    let indent = count_indent(line);
    let trimmed = line.trim_start();
    let fence_count = trimmed.chars().take_while(|c| *c == ':').count();
    if fence_count < 3 {
        return None;
    }
    let rest = trimmed[fence_count..].trim_start();
    if !rest.starts_with('{') || !rest.ends_with('}') {
        return None;
    }
    Some(QuartoDivOpen {
        indent,
        fence_count,
        attrs: rest[1..rest.len() - 1].trim().to_string(),
    })
}

#[must_use]
pub fn is_close(line: &str, fence_char: char, fence_count: usize, indent: usize) -> bool {
    let current_indent = count_indent(line);
    if current_indent > indent {
        return false;
    }
    let stripped = line.trim();
    !stripped.is_empty()
        && stripped.chars().all(|c| c == fence_char)
        && stripped.chars().count() >= fence_count
}

#[must_use]
pub fn is_colon_close(line: &str, indent: usize) -> bool {
    is_close(line, ':', 3, indent)
}

#[must_use]
pub fn take_myst_directive(lines: &[&str], start: usize, line_no: u32) -> Option<DirectiveFrame> {
    let open = parse_myst_open(lines[start])?;
    let mut options = BTreeMap::new();
    let mut body = Vec::new();
    let mut original = vec![lines[start].to_string()];
    let mut i = start + 1;
    let mut in_options = true;
    while i < lines.len() {
        let line = lines[i];
        original.push(line.to_string());
        if is_close(line, open.fence_char, open.fence_count, open.indent) {
            return Some(DirectiveFrame {
                open,
                options,
                body,
                original,
                start_line: line_no,
                end_line: line_no + (i - start) as u32,
            });
        }
        let content = strip_indent(line, open.indent);
        if in_options {
            if let Some((key, value)) = parse_myst_option(content.trim()) {
                options.insert(key, value);
                i += 1;
                continue;
            }
            in_options = false;
            if content.trim().is_empty() {
                i += 1;
                continue;
            }
        }
        body.push(content.to_string());
        i += 1;
    }
    Some(DirectiveFrame {
        open,
        options,
        body,
        original,
        start_line: line_no,
        end_line: line_no + (lines.len() - start).saturating_sub(1) as u32,
    })
}

#[must_use]
pub fn take_fenced_body(
    lines: &[&str],
    start: usize,
    fence_char: char,
    fence_count: usize,
    indent: usize,
) -> (Vec<String>, Vec<String>, usize) {
    let mut body = Vec::new();
    let mut original = vec![lines[start].to_string()];
    let mut i = start + 1;
    while i < lines.len() {
        original.push(lines[i].to_string());
        if is_close(lines[i], fence_char, fence_count, indent) {
            return (body, original, i);
        }
        body.push(lines[i].to_string());
        i += 1;
    }
    (body, original, lines.len().saturating_sub(1))
}

fn parse_braced_open(line: &str, fence_char: char) -> Option<DirectiveOpen> {
    let indent = count_indent(line);
    let trimmed = line.trim_start();
    let count = trimmed.chars().take_while(|c| *c == fence_char).count();
    if count < 3 {
        return None;
    }
    let rest = trimmed[count..].strip_prefix('{')?;
    let close = rest.find('}')?;
    let name = rest[..close].trim();
    if name.is_empty() {
        return None;
    }
    Some(DirectiveOpen {
        indent,
        fence_char,
        fence_count: count,
        name: name.to_string(),
        argument: rest[close + 1..].trim().to_string(),
    })
}

fn parse_myst_option(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(':')?;
    let (key, value) = rest.split_once(':')?;
    if key.is_empty() || key.chars().any(char::is_whitespace) {
        return None;
    }
    Some((key.to_string(), value.trim().to_string()))
}

fn strip_indent(line: &str, indent: usize) -> &str {
    let mut byte_idx = 0;
    for (removed, (idx, ch)) in line.char_indices().enumerate() {
        if removed >= indent || (ch != ' ' && ch != '\t') {
            byte_idx = idx;
            break;
        }
        byte_idx = idx + ch.len_utf8();
    }
    &line[byte_idx..]
}

fn count_indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_myst_options_until_body_starts() {
        let lines = [":::{figure} img.png", ":label: fig:x", "", "Caption", ":::"];
        let frame = take_myst_directive(&lines, 0, 1).unwrap();
        assert_eq!(
            frame.options.get("label").map(String::as_str),
            Some("fig:x")
        );
        assert_eq!(frame.body, vec!["Caption".to_string()]);
    }

    #[test]
    fn close_requires_at_least_opening_fence_count() {
        assert!(is_close("::::", ':', 3, 0));
        assert!(!is_close("::", ':', 3, 0));
    }

    #[test]
    fn close_more_indented_than_open_is_not_a_close() {
        let lines = [":::{note}", "  :::", ":::"];
        let frame = take_myst_directive(&lines, 0, 1).unwrap();
        assert_eq!(frame.body, vec!["  :::".to_string()]);
        assert_eq!(frame.end_line, 3);
    }
}
