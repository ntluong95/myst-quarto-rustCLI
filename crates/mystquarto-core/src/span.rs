//! Source location tracking for diagnostics (fixes D12: diagnostics that say
//! `article.md:55` instead of pointing at nothing).

/// A line range in a source file, 1-indexed to match how diagnostics report
/// `file:line`. Both ends are inclusive; a single-line construct has
/// `start_line == end_line`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// First line of the construct, 1-indexed.
    pub start_line: u32,
    /// Last line of the construct, 1-indexed, inclusive.
    pub end_line: u32,
}

impl Span {
    /// Builds a span covering `start_line..=end_line`.
    ///
    /// # Panics
    /// Panics if `end_line < start_line` or either is `0` (lines are
    /// 1-indexed).
    #[must_use]
    pub fn new(start_line: u32, end_line: u32) -> Self {
        assert!(
            start_line >= 1,
            "Span lines are 1-indexed, got start_line=0"
        );
        assert!(
            end_line >= start_line,
            "Span end_line ({end_line}) must be >= start_line ({start_line})"
        );
        Span {
            start_line,
            end_line,
        }
    }

    /// Builds a span covering a single line.
    #[must_use]
    pub fn single(line: u32) -> Self {
        Span::new(line, line)
    }

    /// Number of lines covered by this span.
    #[must_use]
    pub fn line_count(&self) -> u32 {
        self.end_line - self.start_line + 1
    }
}

#[cfg(test)]
mod tests {
    use super::Span;

    #[test]
    fn single_line_span_has_equal_bounds() {
        let s = Span::single(55);
        assert_eq!(s.start_line, 55);
        assert_eq!(s.end_line, 55);
        assert_eq!(s.line_count(), 1);
    }

    #[test]
    fn multi_line_span_counts_lines_inclusively() {
        let s = Span::new(10, 12);
        assert_eq!(s.line_count(), 3);
    }

    #[test]
    #[should_panic(expected = "1-indexed")]
    fn zero_start_line_panics() {
        let _ = Span::new(0, 1);
    }

    #[test]
    #[should_panic(expected = "must be >=")]
    fn end_before_start_panics() {
        let _ = Span::new(5, 4);
    }
}
