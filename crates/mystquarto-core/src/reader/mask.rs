//! Code-span masking for safe inline scans.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskedText {
    pub text: String,
    spans: Vec<(String, String)>,
}

impl MaskedText {
    #[must_use]
    pub fn unmask(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (token, original) in &self.spans {
            out = out.replace(token, original);
        }
        out
    }
}

/// Replaces CommonMark code spans with placeholders. Opening and closing
/// backtick runs must have the same length.
#[must_use]
pub fn mask_code_spans(input: &str) -> MaskedText {
    let bytes = input.as_bytes();
    let mut out = String::new();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            // Advance by a full UTF-8 character, not a single byte: `as
            // char` on an arbitrary byte reinterprets it as a Latin-1 code
            // point, corrupting every multi-byte character (accented
            // letters, em dash, smart quotes, CJK, ...) into mojibake.
            // Backtick (0x60) never appears as a continuation byte in valid
            // UTF-8, so scanning for backtick runs at the byte level above
            // and below is unaffected by this — only the plain-text advance
            // needed fixing.
            let ch_len = input[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            out.push_str(&input[i..i + ch_len]);
            i += ch_len;
            continue;
        }
        let run_start = i;
        while i < bytes.len() && bytes[i] == b'`' {
            i += 1;
        }
        let run_len = i - run_start;
        let mut j = i;
        let mut close = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let close_start = j;
                while j < bytes.len() && bytes[j] == b'`' {
                    j += 1;
                }
                if j - close_start == run_len {
                    close = Some(j);
                    break;
                }
                continue;
            }
            j += 1;
        }
        if let Some(end) = close {
            let original = &input[run_start..end];
            if is_executable_inline(original, run_len) || preceded_by_role_brace(input, run_start) {
                out.push_str(original);
            } else {
                let token = format!("\u{0}MQCODE{}\u{0}", spans.len());
                spans.push((token.clone(), original.to_string()));
                out.push_str(&token);
            }
            i = end;
        } else {
            out.push_str(&input[run_start..i]);
        }
    }
    MaskedText { text: out, spans }
}

/// `true` if the backtick run starting at `run_start` is immediately
/// preceded (no whitespace) by a bare `{name}` brace — i.e. this span is a
/// role invocation's argument (`{cite}`key`` — reference §5/§10's `{name}`
/// content`` forms), not a generic code span. Such spans must not be masked:
/// masking them makes their content invisible to
/// [`crate::reader::inline::read_legacy_role`], which looks for the literal
/// `}` `` sequence this function's caller would otherwise have erased.
///
/// Deliberately narrow — only a bare identifier (letters, digits, `:`, `-`,
/// `_`) between `{`/`}` counts, so ordinary prose like `See {this} `code``
/// (a brace that is not a role name) is unaffected.
fn preceded_by_role_brace(input: &str, run_start: usize) -> bool {
    let before = &input[..run_start];
    let Some(close_rel) = before.rfind('}') else {
        return false;
    };
    if close_rel + 1 != before.len() {
        return false;
    }
    let Some(open_rel) = before[..close_rel].rfind('{') else {
        return false;
    };
    let name = &before[open_rel + 1..close_rel];
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'))
}

fn is_executable_inline(span: &str, tick_count: usize) -> bool {
    let inner = &span[tick_count..span.len().saturating_sub(tick_count)];
    if inner.starts_with("r ") {
        return true;
    }
    let Some(rest) = inner.strip_prefix('{') else {
        return false;
    };
    let Some((engine, after)) = rest.split_once('}') else {
        return false;
    };
    matches!(engine, "python" | "r" | "julia" | "ojs") && after.starts_with(' ')
}

#[cfg(test)]
mod tests {
    use super::mask_code_spans;

    #[test]
    fn masks_matching_backtick_runs_only() {
        let masked = mask_code_spans("a `x` b ``code with ` inside`` c");
        assert!(!masked.text.contains("code with"));
        assert_eq!(
            masked.unmask(&masked.text),
            "a `x` b ``code with ` inside`` c"
        );
    }

    #[test]
    fn multi_byte_utf8_characters_survive_outside_code_spans() {
        // Regression test: the plain-text path used to advance byte-by-byte
        // and reinterpret each byte as a Latin-1 code point via `as char`,
        // which corrupts every multi-byte character. `rewrite_line`
        // (reader/inline.rs) reconstructs writer output from exactly this
        // masked text, so this bug was silent data corruption for any
        // non-ASCII prose.
        let input = "Café — naïve \u{201c}quotes\u{201d} 25 °C 日本語";
        let masked = mask_code_spans(input);
        assert_eq!(
            masked.text, input,
            "no code spans present, so masking must be a no-op"
        );
    }

    #[test]
    fn multi_byte_utf8_characters_survive_alongside_a_code_span() {
        let input = "Café `code` 日本語";
        let masked = mask_code_spans(input);
        assert!(!masked.text.contains("code"));
        assert_eq!(masked.unmask(&masked.text), input);
    }
}
