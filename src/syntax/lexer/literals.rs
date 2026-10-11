//! Number, character and string literals. Every literal token includes its suffix.

use super::cursor::Cursor;
use super::{LexError, Lexed};
use crate::syntax::SyntaxKind;
use crate::text::unicode::{is_ident_continue, is_ident_start};

/// The most `#` a raw string may use.
const MAX_RAW_STRING_HASHES: usize = 255;

/// Consumes a literal suffix such as `u8` in `1u8`, if one follows.
pub(super) fn eat_suffix(cursor: &mut Cursor) {
  if cursor.first().is_some_and(is_ident_start) {
    cursor.bump();
    cursor.eat_while(is_ident_continue);
  }
}

/// After the first digit: an integer or float literal.
pub(super) fn number(first_digit: char, cursor: &mut Cursor) -> Lexed {
  let mut error = None;
  if first_digit == '0' {
    match cursor.first() {
      Some('b' | 'o') => {
        cursor.bump();
        if !eat_decimal_digits(cursor) {
          error = Some(LexError::EmptyIntDigits);
        }
      }
      Some('x') => {
        cursor.bump();
        if !eat_hexadecimal_digits(cursor) {
          error = Some(LexError::EmptyIntDigits);
        }
      }
      Some('0'..='9' | '_') => {
        eat_decimal_digits(cursor);
      }
      Some('.' | 'e' | 'E') => {}
      _ => {
        eat_suffix(cursor);
        return (SyntaxKind::IntNumber, None);
      }
    }
  } else {
    eat_decimal_digits(cursor);
  }
  let kind = match cursor.first() {
    // `1.` is a float, but `1..2` is a range and `1.foo()` is a method call.
    Some('.') if cursor.nth(1) != Some('.') && !cursor.nth(1).is_some_and(is_ident_start) => {
      cursor.bump();
      if cursor.first().is_some_and(|c| c.is_ascii_digit()) {
        eat_decimal_digits(cursor);
        if matches!(cursor.first(), Some('e' | 'E')) {
          cursor.bump();
          if !eat_exponent(cursor) {
            error = error.or(Some(LexError::EmptyExponent));
          }
        }
      }
      SyntaxKind::FloatNumber
    }
    Some('e' | 'E') => {
      cursor.bump();
      if !eat_exponent(cursor) {
        error = error.or(Some(LexError::EmptyExponent));
      }
      SyntaxKind::FloatNumber
    }
    _ => SyntaxKind::IntNumber,
  };
  eat_suffix(cursor);
  (kind, error)
}

/// After the opening `'` of a char or byte literal.
pub(super) fn quoted_char(cursor: &mut Cursor, kind: SyntaxKind) -> Lexed {
  cursor.bump();
  quoted_char_body(cursor, kind)
}

/// The body and closing `'` of a char or byte literal, the opening `'` already consumed.
pub(super) fn quoted_char_body(cursor: &mut Cursor, kind: SyntaxKind) -> Lexed {
  let start = cursor.pos();
  let terminated = single_quoted(cursor);
  let empty = terminated && cursor.pos() - start == 1;
  eat_suffix(cursor);
  let error = if !terminated {
    Some(LexError::UnterminatedChar)
  } else if empty {
    Some(LexError::EmptyChar)
  } else {
    None
  };
  (kind, error)
}

/// After `r`, `br` or `cr`: `#`s, a quote, the contents, a quote and the same number of `#`s.
pub(super) fn raw_string(cursor: &mut Cursor, kind: SyntaxKind) -> Lexed {
  let mut hashes = 0;
  while cursor.first() == Some('#') {
    cursor.bump();
    hashes += 1;
  }
  // Like rustc, the character that should have been the opening quote belongs to the token.
  if cursor.bump() != Some('"') {
    return (kind, Some(LexError::InvalidRawStringDelimiter));
  }
  loop {
    match cursor.bump() {
      None => return (kind, Some(LexError::UnterminatedRawString)),
      Some('"') => {
        let mut closing = 0;
        while closing < hashes && cursor.first() == Some('#') {
          cursor.bump();
          closing += 1;
        }
        if closing == hashes {
          break;
        }
      }
      Some(_) => {}
    }
  }
  eat_suffix(cursor);
  if hashes > MAX_RAW_STRING_HASHES {
    (kind, Some(LexError::TooManyRawStringHashes))
  } else {
    (kind, None)
  }
}

/// After the opening `"` of a string, byte string or C string.
pub(super) fn string(cursor: &mut Cursor, kind: SyntaxKind) -> Lexed {
  loop {
    match cursor.bump() {
      None => return (kind, Some(LexError::UnterminatedString)),
      Some('"') => break,
      Some('\\') => {
        if matches!(cursor.first(), Some('\\' | '"')) {
          cursor.bump();
        }
      }
      Some(_) => {}
    }
  }
  eat_suffix(cursor);
  (kind, None)
}

fn eat_decimal_digits(cursor: &mut Cursor) -> bool {
  let mut has_digits = false;
  while let Some(c) = cursor.first() {
    match c {
      '_' => {}
      '0'..='9' => has_digits = true,
      _ => break,
    }
    cursor.bump();
  }
  has_digits
}

/// After `e` or `E`: an optional sign and the exponent digits.
fn eat_exponent(cursor: &mut Cursor) -> bool {
  if matches!(cursor.first(), Some('-' | '+')) {
    cursor.bump();
  }
  eat_decimal_digits(cursor)
}

fn eat_hexadecimal_digits(cursor: &mut Cursor) -> bool {
  let mut has_digits = false;
  while let Some(c) = cursor.first() {
    match c {
      '_' => {}
      c if c.is_ascii_hexdigit() => has_digits = true,
      _ => break,
    }
    cursor.bump();
  }
  has_digits
}

/// The body of a char literal up to its closing `'`. Returns whether the closing quote was found.
/// An unclosed literal stops before a `/` (likely a comment) or at a line break.
fn single_quoted(cursor: &mut Cursor) -> bool {
  if cursor.nth(1) == Some('\'') && cursor.first() != Some('\\') {
    cursor.bump();
    cursor.bump();
    return true;
  }
  loop {
    match cursor.first() {
      Some('\'') => {
        cursor.bump();
        return true;
      }
      Some('/') | None => return false,
      Some('\n') if cursor.nth(1) != Some('\'') => return false,
      Some('\\') => {
        cursor.bump();
        cursor.bump();
      }
      Some(_) => {
        cursor.bump();
      }
    }
  }
}
