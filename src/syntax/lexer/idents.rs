//! Identifiers, raw identifiers, lifetimes and labels, and the literal prefixes `r`, `b` and `c`.

use super::cursor::Cursor;
use super::literals;
use super::{LexError, Lexed};
use crate::syntax::{Edition, SyntaxKind};
use crate::text::unicode::{is_ident_continue, is_ident_start};

/// After an identifier-start character: an identifier, raw identifier or prefixed literal.
pub(super) fn ident_or_prefixed_literal(
  first: char,
  cursor: &mut Cursor,
  edition: Edition,
) -> Lexed {
  let c_strings = edition >= Edition::E2021;
  match (first, cursor.first(), cursor.nth(1)) {
    ('r', Some('#'), Some(c)) if is_ident_start(c) => {
      cursor.bump();
      cursor.eat_while(is_ident_continue);
      (SyntaxKind::RawIdent, None)
    }
    ('r', Some('#' | '"'), _) => literals::raw_string(cursor, SyntaxKind::RawStr),
    ('b', Some('\''), _) => literals::quoted_char(cursor, SyntaxKind::Byte),
    ('b', Some('"'), _) => {
      cursor.bump();
      literals::string(cursor, SyntaxKind::ByteStr)
    }
    ('b', Some('r'), Some('"' | '#')) => {
      cursor.bump();
      literals::raw_string(cursor, SyntaxKind::RawByteStr)
    }
    ('c', Some('"'), _) if c_strings => {
      cursor.bump();
      literals::string(cursor, SyntaxKind::CStr)
    }
    ('c', Some('r'), Some('"' | '#')) if c_strings => {
      cursor.bump();
      literals::raw_string(cursor, SyntaxKind::RawCStr)
    }
    _ => ident(cursor, edition),
  }
}

/// After `'`: a lifetime or label (`'a`), or a char literal (`'a'`).
pub(super) fn lifetime_or_char(cursor: &mut Cursor, edition: Edition) -> Lexed {
  let can_be_lifetime = cursor.nth(1) != Some('\'')
    && cursor
      .first()
      .is_some_and(|c| is_ident_start(c) || c.is_ascii_digit());
  if !can_be_lifetime {
    return literals::quoted_char_body(cursor, SyntaxKind::Char);
  }
  if edition >= Edition::E2021
    && cursor.first() == Some('r')
    && cursor.nth(1) == Some('#')
    && cursor.nth(2).is_some_and(is_ident_start)
  {
    cursor.bump();
    cursor.bump();
    cursor.eat_while(is_ident_continue);
    return (SyntaxKind::Lifetime, None);
  }
  let starts_with_number = cursor.first().is_some_and(|c| c.is_ascii_digit());
  cursor.bump();
  cursor.eat_while(is_ident_continue);
  if cursor.first() == Some('\'') {
    cursor.bump();
    literals::eat_suffix(cursor);
    return (SyntaxKind::Char, None);
  }
  if starts_with_number {
    (
      SyntaxKind::Lifetime,
      Some(LexError::LifetimeStartsWithNumber),
    )
  } else {
    (SyntaxKind::Lifetime, None)
  }
}

/// The rest of an identifier. From 2021 on, an identifier directly followed by `#`, `"` or `'`
/// is a reserved prefix, such as `k#x` or `f"..."`.
fn ident(cursor: &mut Cursor, edition: Edition) -> Lexed {
  let start = cursor.pos();
  cursor.eat_while(is_ident_continue);
  // `builtin#name(..)` is nightly built-in syntax, not a reserved prefix.
  let builtin_pound = cursor.first() == Some('#') && cursor.consumed_since(start) == "uiltin";
  if edition >= Edition::E2021 && matches!(cursor.first(), Some('#' | '"' | '\'')) && !builtin_pound
  {
    (SyntaxKind::Ident, Some(LexError::ReservedPrefix))
  } else {
    (SyntaxKind::Ident, None)
  }
}
