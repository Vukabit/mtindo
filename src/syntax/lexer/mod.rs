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

/// Problems the lexer reports on a token. The token still covers the offending text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexError {
    EmptyChar,
    EmptyExponent,
    EmptyIntDigits,
    InvalidRawStringDelimiter,
    LifetimeStartsWithNumber,
    ReservedGuardedString,
    ReservedPrefix,
    TooManyRawStringHashes,
    UnknownChar,
    UnterminatedBlockComment,
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
            LexError::InvalidRawStringDelimiter => "raw string is missing its opening quote",
            LexError::LifetimeStartsWithNumber => "lifetime starts with a number",
            LexError::ReservedGuardedString => "`#\"` and `##` are reserved in edition 2024",
            LexError::ReservedPrefix => "prefix is reserved from edition 2021",
            LexError::TooManyRawStringHashes => "raw string uses more than 255 `#`",
            LexError::UnknownChar => "character cannot start a token",
            LexError::UnterminatedBlockComment => "block comment is not closed",
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
        '\t' | '\n'
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

/// Splits `text` into tokens for the given edition.
pub fn tokenize(text: &str, edition: Edition) -> Vec<Token> {
    let mut tokens = Vec::new();
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
    let mut cursor = Cursor::new(&text[offset..]);
    while !cursor.is_eof() {
        let start = cursor.pos();
        let (kind, error) = next_token(&mut cursor, edition);
        tokens.push(token(kind, cursor.pos() - start, error));
    }
    tokens
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

/// A token's kind and any error found while lexing it.
type Lexed = (SyntaxKind, Option<LexError>);

#[cfg(test)]
mod tests;
