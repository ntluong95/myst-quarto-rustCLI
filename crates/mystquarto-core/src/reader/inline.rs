//! Single-pass inline recognizer used by readers for type-aware decisions.

use crate::reader::mask::mask_code_spans;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineEvent {
    Citation(String),
    CrossReference(String),
    LegacyRole { role: String, target: String },
    JupyterEval { engine: String, expr: String },
    KnitrEval(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineScan {
    pub events: Vec<InlineEvent>,
}

/// Scans a line once for citation, cross-reference, legacy role, and
/// executable inline-code forms. Code spans are masked first, so examples
/// that discuss syntax are left alone by higher layers.
#[must_use]
pub fn scan_line(line: &str, known_labels: &[String]) -> InlineScan {
    let masked = mask_code_spans(line);
    let text = masked.text.as_str();
    let mut events = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if let Some((len, mut group_events)) = read_bracket_citation(rest) {
            events.append(&mut group_events);
            i += len;
        } else if let Some((len, event)) = read_legacy_role(rest)
            .or_else(|| read_braced_eval(rest))
            .or_else(|| read_knitr_eval(rest))
            .or_else(|| read_at_reference(rest, known_labels))
        {
            events.push(event);
            i += len;
        } else {
            i += text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    InlineScan { events }
}

fn read_legacy_role(rest: &str) -> Option<(usize, InlineEvent)> {
    let tail = rest.strip_prefix('{')?;
    let close = tail.find("}`")?;
    let role = &tail[..close];
    let after = close + 2;
    let end = tail[after..].find('`')? + after;
    Some((
        end + 2,
        InlineEvent::LegacyRole {
            role: role.to_string(),
            target: tail[after..end].to_string(),
        },
    ))
}

fn read_braced_eval(rest: &str) -> Option<(usize, InlineEvent)> {
    let tail = rest.strip_prefix("`{")?;
    let close = tail.find("}")?;
    let engine = &tail[..close];
    let body_start = close + 1;
    let end = tail[body_start..].find('`')? + body_start;
    Some((
        end + 3,
        InlineEvent::JupyterEval {
            engine: engine.to_string(),
            expr: tail[body_start..end].trim().to_string(),
        },
    ))
}

fn read_knitr_eval(rest: &str) -> Option<(usize, InlineEvent)> {
    let tail = rest.strip_prefix("`r ")?;
    let end = tail.find('`')?;
    Some((end + 4, InlineEvent::KnitrEval(tail[..end].to_string())))
}

fn read_bracket_citation(rest: &str) -> Option<(usize, Vec<InlineEvent>)> {
    let tail = rest
        .strip_prefix("[@")
        .or_else(|| rest.strip_prefix("[-@"))?;
    let end = tail.find(']')?;
    let events: Vec<InlineEvent> = tail[..end]
        .split(';')
        .enumerate()
        .filter_map(|(idx, part)| {
            if idx == 0 {
                citation_key_from_first_part(part)
            } else {
                citation_key_from_part(part)
            }
        })
        .map(InlineEvent::Citation)
        .collect();
    (!events.is_empty()).then(|| (end + rest.len() - tail.len() + 1, events))
}

fn citation_key_from_first_part(part: &str) -> Option<String> {
    citation_key_from_candidate(part.trim_start().trim_start_matches('-'))
}

fn citation_key_from_part(part: &str) -> Option<String> {
    let key_start = part.find('@')? + 1;
    citation_key_from_candidate(part[key_start..].trim_start())
}

fn citation_key_from_candidate(candidate: &str) -> Option<String> {
    let len = candidate
        .char_indices()
        .take_while(|(_, c)| c.is_alphanumeric() || matches!(c, ':' | '-' | '_' | '.' | '/'))
        .last()
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    let key = candidate[..len].trim_end_matches(['.', ',', ';', ':']);
    (!key.is_empty()).then(|| key.to_string())
}

fn read_at_reference(rest: &str, known_labels: &[String]) -> Option<(usize, InlineEvent)> {
    let tail = rest.strip_prefix('@')?;
    let len = tail
        .char_indices()
        .take_while(|(_, c)| c.is_alphanumeric() || matches!(c, ':' | '-' | '_' | '.' | '/'))
        .last()
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    if len == 0 {
        return None;
    }
    let mut key = tail[..len]
        .trim_end_matches(['.', ',', ';', ':'])
        .to_string();
    while key.ends_with('/') {
        key.pop();
    }
    if known_labels.iter().any(|l| l == &key) || has_crossref_prefix(&key) {
        Some((1 + key.len(), InlineEvent::CrossReference(key)))
    } else {
        Some((1 + key.len(), InlineEvent::Citation(key)))
    }
}

fn has_crossref_prefix(key: &str) -> bool {
    matches!(
        key.split_once(':').map(|(prefix, _)| prefix),
        Some("fig" | "tbl" | "tab" | "eq" | "sec" | "nb" | "thm")
    )
}

/// Rewrites `line` for a specific writer target, leaving three things
/// untouched: code spans (masked, same as [`scan_line`]), modern citation
/// syntax (`[@key]`/`[-@key]`/bare `@key`-as-citation — reference §4:
/// Quarto and modern MyST share this syntax exactly, so there is nothing to
/// rewrite), and any text `render` declines to replace (returns `None`,
/// meaning "this event's original spelling is already correct for the
/// target dialect").
///
/// Shares [`scan_line`]'s exact matcher functions and priority order (bracket
/// citations first, then legacy-role / braced-eval / knitr-eval / at-reference)
/// so detection and rewriting can never disagree about what a span is or
/// where it ends — the two were written from the same phase spec section and
/// diverging would silently reintroduce a defect like D15.
///
/// `render` is only invoked for the four event kinds a writer can legitimately
/// need to change: [`InlineEvent::CrossReference`] (label normalization),
/// [`InlineEvent::LegacyRole`] (every `{name}`content`` form — legacy and
/// modern MyST roles alike, since [`read_legacy_role`] does not distinguish
/// them syntactically), [`InlineEvent::JupyterEval`], and
/// [`InlineEvent::KnitrEval`].
#[must_use]
pub fn rewrite_line(
    line: &str,
    known_labels: &[String],
    mut render: impl FnMut(InlineEvent) -> Option<String>,
) -> String {
    let masked = mask_code_spans(line);
    let text = masked.text.as_str();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if let Some((len, _)) = read_bracket_citation(rest) {
            out.push_str(&rest[..len]);
            i += len;
            continue;
        }
        if let Some((len, event)) = read_legacy_role(rest)
            .or_else(|| read_braced_eval(rest))
            .or_else(|| read_knitr_eval(rest))
        {
            out.push_str(&render(event).unwrap_or_else(|| rest[..len].to_string()));
            i += len;
            continue;
        }
        if let Some((len, event)) = read_at_reference(rest, known_labels) {
            match event {
                InlineEvent::CrossReference(_) => {
                    out.push_str(&render(event).unwrap_or_else(|| rest[..len].to_string()));
                }
                _ => out.push_str(&rest[..len]),
            }
            i += len;
            continue;
        }
        let ch_len = text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    masked.unmask(&out)
}

#[cfg(test)]
mod tests {
    use super::{rewrite_line, scan_line, InlineEvent};

    #[test]
    fn keeps_doi_key_and_excludes_sentence_period() {
        let scan = scan_line("See [@10.1038/nmeth.1974] and @numpy.", &[]);
        assert_eq!(
            scan.events,
            vec![
                InlineEvent::Citation("10.1038/nmeth.1974".to_string()),
                InlineEvent::Citation("numpy".to_string())
            ]
        );
    }

    #[test]
    fn grouped_citations_emit_every_key() {
        let scan = scan_line("See [@10.1038/nmeth.1974; @smith2020].", &[]);
        assert_eq!(
            scan.events,
            vec![
                InlineEvent::Citation("10.1038/nmeth.1974".to_string()),
                InlineEvent::Citation("smith2020".to_string())
            ]
        );
    }

    #[test]
    fn ignores_role_syntax_inside_code_spans() {
        let scan = scan_line("Use `{cite}`key`` as text.", &[]);
        assert!(scan.events.is_empty());
    }

    #[test]
    fn detects_knitr_and_jupyter_inline_engines() {
        let scan = scan_line("`r x + 1` and `{python} x`", &[]);
        assert!(scan
            .events
            .contains(&InlineEvent::KnitrEval("x + 1".to_string())));
        assert!(matches!(
            scan.events.last(),
            Some(InlineEvent::JupyterEval { engine, .. }) if engine == "python"
        ));
    }

    #[test]
    fn inline_eval_match_consumes_the_closing_backtick() {
        let out = rewrite_line("`r x + 1` and `{python} x`.", &[], |event| match event {
            InlineEvent::KnitrEval(expr) => Some(format!("{{eval}}`{expr}`")),
            InlineEvent::JupyterEval { expr, .. } => Some(format!("`{{python}} {expr}`")),
            _ => None,
        });
        assert_eq!(out, "{eval}`x + 1` and `{python} x`.");
    }

    #[test]
    fn known_myst_reference_prefixes_are_crossrefs_even_without_definitions() {
        let scan = scan_line("See @fig:samples and @numpy.", &[]);
        assert_eq!(
            scan.events,
            vec![
                InlineEvent::CrossReference("fig:samples".to_string()),
                InlineEvent::Citation("numpy".to_string())
            ]
        );
    }

    #[test]
    fn rewrite_passes_through_modern_citations_and_code_spans_unchanged() {
        let line = "See [@10.1038/nmeth.1974; @smith2020] and `[@not-a-citation]`.";
        let out = rewrite_line(line, &[], |_| panic!("no event should need rendering"));
        assert_eq!(out, line);
    }

    #[test]
    fn rewrite_replaces_only_the_matched_span() {
        let out = rewrite_line("before {cite}`k` after", &[], |event| match event {
            InlineEvent::LegacyRole { role, target } if role == "cite" => {
                Some(format!("[@{target}]"))
            }
            _ => None,
        });
        assert_eq!(out, "before [@k] after");
    }

    #[test]
    fn rewrite_normalizes_cross_reference_but_leaves_citation_at_sign_alone() {
        let known = vec!["fig-samples".to_string()];
        let out = rewrite_line(
            "See @fig-samples and @numpy.",
            &known,
            |event| match event {
                InlineEvent::CrossReference(key) => Some(format!("[[{key}]]")),
                _ => None,
            },
        );
        assert_eq!(out, "See [[fig-samples]] and @numpy.");
    }

    #[test]
    fn rewrite_declining_a_replacement_keeps_the_original_span() {
        let out = rewrite_line("`{python} x + 1`", &[], |_| None);
        assert_eq!(out, "`{python} x + 1`");
    }
}
