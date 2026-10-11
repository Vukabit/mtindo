//! Line and block comments, including doc comments. Block comments nest.

use super::LexError;
use super::cursor::Cursor;
use crate::syntax::SyntaxKind;

/// After `/*`: the comment up to its matching `*/`.
pub(super) fn block_comment(cursor: &mut Cursor) -> (SyntaxKind, Option<LexError>) {
  let rest = cursor.rest();
  let kind = if rest.starts_with('!') {
    SyntaxKind::InnerBlockDoc
  } else if rest.starts_with('*') && !rest.starts_with("**") && !rest.starts_with("*/") {
    SyntaxKind::OuterBlockDoc
  } else {
    SyntaxKind::BlockComment
  };
  let mut depth = 1;
  while let Some(c) = cursor.bump() {
    match c {
      '/' if cursor.first() == Some('*') => {
        cursor.bump();
        depth += 1;
      }
      '*' if cursor.first() == Some('/') => {
        cursor.bump();
        depth -= 1;
        if depth == 0 {
          return (kind, None);
        }
      }
      _ => {}
    }
  }
  (kind, Some(LexError::UnterminatedBlockComment))
}

/// After `//`: the comment up to, but not including, the end of the line.
pub(super) fn line_comment(cursor: &mut Cursor) -> SyntaxKind {
  let rest = cursor.rest();
  let kind = if rest.starts_with('!') {
    SyntaxKind::InnerLineDoc
  } else if rest.starts_with('/') && !rest.starts_with("//") {
    SyntaxKind::OuterLineDoc
  } else {
    SyntaxKind::LineComment
  };
  cursor.eat_while(|c| c != '\n');
  kind
}
