//! Maps byte offsets to lines and columns.

use crate::text::unicode::str_width;

/// A zero-based line and byte column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: usize,
    pub byte_column: usize,
}

/// The start offset of every line in a text, for fast offset-to-position lookups.
#[derive(Debug, Clone)]
pub struct LineIndex {
    line_starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (idx, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        LineIndex {
            line_starts,
            len: text.len(),
        }
    }

    /// The display column of `offset` within its line: the width of the text before it, with
    /// each tab advancing to the next multiple of `tab_width`. `text` must be the indexed text.
    pub fn display_column(&self, text: &str, offset: usize, tab_width: usize) -> usize {
        let position = self.line_col(offset);
        let line_start = self.line_starts[position.line];
        let prefix = &text[line_start..line_start + position.byte_column];
        let mut column = 0;
        for segment in prefix.split_inclusive('\t') {
            match segment.strip_suffix('\t') {
                Some(before_tab) => {
                    column += str_width(before_tab);
                    if let Some(stops) = column.checked_div(tab_width) {
                        column = (stops + 1) * tab_width;
                    }
                }
                None => column += str_width(segment),
            }
        }
        column
    }

    /// The line and byte column of `offset`. Offsets past the end clamp to the end.
    pub fn line_col(&self, offset: usize) -> LineCol {
        let offset = offset.min(self.len);
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next - 1,
        };
        LineCol {
            line,
            byte_column: offset - self.line_starts[line],
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// The byte offset where `line` starts, if the text has that many lines.
    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.line_starts.get(line).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_column_counts_wide_chars_and_tabs() {
        let text = "let 中 = 1;\n\tx\n";
        let index = LineIndex::new(text);
        let after_wide = text.find(" =").unwrap();
        assert_eq!(index.line_col(after_wide).byte_column, 7);
        assert_eq!(index.display_column(text, after_wide, 4), 6);
        let x = text.find('x').unwrap();
        assert_eq!(index.display_column(text, x, 4), 4);
        assert_eq!(index.display_column(text, x, 8), 8);
    }

    #[test]
    fn test_empty_text() {
        let index = LineIndex::new("");
        assert_eq!(index.line_count(), 1);
        assert_eq!(
            index.line_col(0),
            LineCol {
                line: 0,
                byte_column: 0
            }
        );
    }

    #[test]
    fn test_line_col_across_lines() {
        let text = "ab\ncd\n\nef";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 4);
        assert_eq!(
            index.line_col(0),
            LineCol {
                line: 0,
                byte_column: 0
            }
        );
        assert_eq!(
            index.line_col(2),
            LineCol {
                line: 0,
                byte_column: 2
            }
        );
        assert_eq!(
            index.line_col(3),
            LineCol {
                line: 1,
                byte_column: 0
            }
        );
        assert_eq!(
            index.line_col(6),
            LineCol {
                line: 2,
                byte_column: 0
            }
        );
        assert_eq!(
            index.line_col(8),
            LineCol {
                line: 3,
                byte_column: 1
            }
        );
        assert_eq!(
            index.line_col(99),
            LineCol {
                line: 3,
                byte_column: 2
            }
        );
        assert_eq!(index.line_start(1), Some(3));
        assert_eq!(index.line_start(4), None);
    }
}
