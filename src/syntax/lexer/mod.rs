//! Splits Rust source into tokens. Every byte of the input belongs to exactly one token,
//! including whitespace and comments, so the tokens always add back up to the input.
//!
//! The lexer never fails: malformed input becomes tokens carrying a [`LexError`].
//! Keywords are not recognised here; they are plain `Ident` tokens that the parser
//! interprets according to the edition.

mod comments;
mod cursor;
mod idents;
mod literals;

use cursor::Cursor;

use crate::syntax::{Edition, SyntaxKind};
use crate::text::unicode::is_ident_start;

/// A token's kind and any error found while lexing it.
type Lexed = (SyntaxKind, Option<LexError>);

/// Problems the lexer reports on a token. The token still covers the offending text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexError {
  EmptyChar,
  EmptyExponent,
  EmptyIntDigits,
  IndentedFrontmatter,
  InvalidRawStringDelimiter,
  LifetimeStartsWithNumber,
  ReservedGuardedString,
  ReservedPrefix,
  TooManyRawStringHashes,
  UnknownChar,
  UnterminatedBlockComment,
  UnterminatedFrontmatter,
  UnterminatedChar,
  UnterminatedRawString,
  UnterminatedString,
}

impl LexError {
  pub fn message(self) -> &'static str {
    match self {
      LexError::EmptyChar => "empty character literal",
      LexError::EmptyExponent => "float exponent has no digits",
      LexError::EmptyIntDigits => "number has a base prefix but no digits",
      LexError::IndentedFrontmatter => "frontmatter fence must start its line",
      LexError::InvalidRawStringDelimiter => "raw string is missing its opening quote",
      LexError::LifetimeStartsWithNumber => "lifetime starts with a number",
      LexError::ReservedGuardedString => "`#\"` and `##` are reserved in edition 2024",
      LexError::ReservedPrefix => "prefix is reserved from edition 2021",
      LexError::TooManyRawStringHashes => "raw string uses more than 255 `#`",
      LexError::UnknownChar => "character cannot start a token",
      LexError::UnterminatedBlockComment => "block comment is not closed",
      LexError::UnterminatedFrontmatter => "frontmatter is not closed",
      LexError::UnterminatedChar => "character literal is not closed",
      LexError::UnterminatedRawString => "raw string is not closed",
      LexError::UnterminatedString => "string is not closed",
    }
  }
}

/// One token: its kind, its length in bytes, and any error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
  pub kind: SyntaxKind,
  pub len: u32,
  pub error: Option<LexError>,
}

/// True for the characters Rust treats as whitespace (Unicode Pattern_White_Space).
pub fn is_whitespace(c: char) -> bool {
  matches!(
    c,
    '\t'
      | '\n'
      | '\u{000B}'
      | '\u{000C}'
      | '\r'
      | ' '
      | '\u{0085}'
      | '\u{200E}'
      | '\u{200F}'
      | '\u{2028}'
      | '\u{2029}'
  )
}

/// Splits floats that are really tuple-field chains. The lexer reads `x.0.1` as `x`, `.`,
/// `0.1`, and `x.0. await` as `x`, `.`, `0.`, `await`, like rustc's lexer; rustc's parser then
/// splits such floats. A float directly after a single `.` (not part of `..`) is always a
/// field chain, so this pass splits `0.1` into `0` `.` `1` and `0.` into `0` `.`. The token
/// texts still add up to the input.
pub fn split_tuple_field_floats(text: &str, tokens: Vec<Token>) -> Vec<Token> {
  let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
  let mut offset = 0;
  for token in tokens {
    let start = offset;
    offset += token.len as usize;
    let after_single_dot = out.last().is_some_and(|t| t.kind == SyntaxKind::Dot)
      && !(out.len() >= 2 && out[out.len() - 2].kind == SyntaxKind::Dot);
    let float_text = &text[start..offset];
    let splittable = token.kind == SyntaxKind::FloatNumber
      && token.error.is_none()
      && after_single_dot
      && float_text.split_once('.').is_some_and(|(whole, fraction)| {
        !whole.is_empty()
          && whole.bytes().all(|b| b.is_ascii_digit())
          && fraction.bytes().all(|b| b.is_ascii_digit())
      });
    if !splittable {
      out.push(token);
      continue;
    }
    let (whole, fraction) = float_text.split_once('.').unwrap();
    out.push(self::token(SyntaxKind::IntNumber, whole.len(), None));
    out.push(self::token(SyntaxKind::Dot, 1, None));
    if !fraction.is_empty() {
      out.push(self::token(SyntaxKind::IntNumber, fraction.len(), None));
    }
  }
  out
}

/// Splits `text` into tokens for the given edition.
pub fn tokenize(text: &str, edition: Edition) -> Vec<Token> {
  // Sized for the whole text at once: code runs about four or five bytes a token.
  let mut tokens = Vec::with_capacity(text.len() / 4);
  let mut offset = 0;
  if text.starts_with('\u{feff}') {
    let len = '\u{feff}'.len_utf8();
    tokens.push(token(SyntaxKind::Bom, len, None));
    offset = len;
  }
  if let Some(len) = shebang_len(&text[offset..], edition) {
    tokens.push(token(SyntaxKind::Shebang, len, None));
    offset += len;
  }
  offset += frontmatter(&text[offset..], &mut tokens);
  let mut cursor = Cursor::new(&text[offset..]);
  while !cursor.is_eof() {
    let start = cursor.pos();
    let (kind, error) = next_token(&mut cursor, edition);
    tokens.push(token(kind, cursor.pos() - start, error));
  }
  tokens
}

/// Cargo-script frontmatter: a `---` fenced block before any other token (after an optional
/// shebang and whitespace), closed by a line starting with the same number of dashes.
/// Pushes the leading whitespace and the frontmatter token; returns the bytes consumed.
fn frontmatter(text: &str, tokens: &mut Vec<Token>) -> usize {
  let ws_len = text.len() - text.trim_start_matches(is_whitespace).len();
  let rest = &text[ws_len..];
  if !rest.starts_with("---") {
    return 0;
  }
  let indented = ws_len > 0 && !text[..ws_len].ends_with('\n');
  let start = if indented { 0 } else { ws_len };
  if !indented && ws_len > 0 {
    tokens.push(token(SyntaxKind::Whitespace, ws_len, None));
  }
  let dashes = rest.len() - rest.trim_start_matches('-').len();
  let fence = format!("\n{}", "-".repeat(dashes));
  let line_end = |from: usize| rest[from..].find('\n').map_or(rest.len(), |i| from + i);
  let (end, mut error) = match rest.find(&fence) {
    Some(at) => (line_end(at + fence.len()), None),
    None => {
      let recovery = rest.find("\n---").map(|at| line_end(at + 1));
      (
        recovery.unwrap_or(rest.len()),
        Some(LexError::UnterminatedFrontmatter),
      )
    }
  };
  if indented {
    error = error.or(Some(LexError::IndentedFrontmatter));
  }
  let len = ws_len - start + end;
  tokens.push(token(SyntaxKind::Frontmatter, len, error));
  ws_len + end
}

fn next_token(cursor: &mut Cursor, edition: Edition) -> Lexed {
  let c = match cursor.bump() {
    Some(c) => c,
    None => return (SyntaxKind::Whitespace, None),
  };
  match c {
    c if is_whitespace(c) => {
      cursor.eat_while(is_whitespace);
      (SyntaxKind::Whitespace, None)
    }
    '/' if cursor.first() == Some('/') => {
      cursor.bump();
      (comments::line_comment(cursor), None)
    }
    '/' if cursor.first() == Some('*') => {
      cursor.bump();
      comments::block_comment(cursor)
    }
    c if is_ident_start(c) => idents::ident_or_prefixed_literal(c, cursor, edition),
    '0'..='9' => literals::number(c, cursor),
    '\'' => idents::lifetime_or_char(cursor, edition),
    '"' => literals::string(cursor, SyntaxKind::Str),
    '#' if edition >= Edition::E2024 && matches!(cursor.first(), Some('"' | '#')) => {
      (SyntaxKind::Pound, Some(LexError::ReservedGuardedString))
    }
    ';' => (SyntaxKind::Semi, None),
    ',' => (SyntaxKind::Comma, None),
    '.' => (SyntaxKind::Dot, None),
    '(' => (SyntaxKind::LParen, None),
    ')' => (SyntaxKind::RParen, None),
    '{' => (SyntaxKind::LBrace, None),
    '}' => (SyntaxKind::RBrace, None),
    '[' => (SyntaxKind::LBrack, None),
    ']' => (SyntaxKind::RBrack, None),
    '@' => (SyntaxKind::At, None),
    '#' => (SyntaxKind::Pound, None),
    '~' => (SyntaxKind::Tilde, None),
    '?' => (SyntaxKind::Question, None),
    ':' => (SyntaxKind::Colon, None),
    '$' => (SyntaxKind::Dollar, None),
    '=' => (SyntaxKind::Eq, None),
    '!' => (SyntaxKind::Bang, None),
    '<' => (SyntaxKind::Lt, None),
    '>' => (SyntaxKind::Gt, None),
    '-' => (SyntaxKind::Minus, None),
    '&' => (SyntaxKind::Amp, None),
    '|' => (SyntaxKind::Pipe, None),
    '+' => (SyntaxKind::Plus, None),
    '*' => (SyntaxKind::Star, None),
    '/' => (SyntaxKind::Slash, None),
    '^' => (SyntaxKind::Caret, None),
    '%' => (SyntaxKind::Percent, None),
    _ => (SyntaxKind::Unknown, Some(LexError::UnknownChar)),
  }
}

/// The length of a `#!` shebang line at the start of `text`. A `#!` followed (after whitespace
/// and plain comments) by `[` is an inner attribute instead.
fn shebang_len(text: &str, edition: Edition) -> Option<usize> {
  let tail = text.strip_prefix("#!")?;
  let mut cursor = Cursor::new(tail);
  while !cursor.is_eof() {
    let (kind, _) = next_token(&mut cursor, edition);
    match kind {
      SyntaxKind::Whitespace | SyntaxKind::LineComment | SyntaxKind::BlockComment => continue,
      SyntaxKind::LBrack => return None,
      _ => break,
    }
  }
  Some(2 + tail.lines().next().unwrap_or("").len())
}

fn token(kind: SyntaxKind, len: usize, error: Option<LexError>) -> Token {
  Token {
    kind,
    len: len as u32,
    error,
  }
}

#[cfg(test)]
mod tests;
