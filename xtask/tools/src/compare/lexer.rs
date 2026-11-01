//! The rustc-lexer answer key: mto's lexer held against rustc's own (`ra-ap-rustc_lexer`),
//! token for token. rustc's lexer does not know about editions, so mto is run for one
//! edition (default 2021) and the two must agree on every token boundary and on a coarse
//! token class; what only editions or nightly gates explain is an expected difference.

use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::{Edition, SyntaxKind};
use ra_ap_rustc_lexer as rustc;

struct Span {
  start: usize,
  end: usize,
  class: String,
}

pub fn differences(text: &str, edition: Edition) -> (Vec<String>, Vec<String>, usize) {
  let ours = mto_spans(text, edition);
  let theirs = rustc_spans(text);
  let (unexplained, expected) = compare(&ours, &theirs, text, edition);
  (unexplained, expected, ours.len())
}

/// The unexplained and the expected differences between the two lexers on one file, and
/// how many tokens mto produced.
/// The unexplained and the expected differences between the two lexers on one text, and
/// how many tokens rustc's lexer found: the measure `compare --source lexer` takes.
pub(crate) fn against_rustc(text: &str, edition: Edition) -> (Vec<String>, Vec<String>, usize) {
  let ours = mto_spans(text, edition);
  let theirs = rustc_spans(text);
  let (unexplained, expected) = compare(&ours, &theirs, text, edition);
  (unexplained, expected, theirs.len())
}

/// Every boundary or class mismatch, with the line it is on, split into unexplained
/// differences and expected ones (see [`expected_difference`]).
fn compare(
  ours: &[Span],
  theirs: &[Span],
  text: &str,
  edition: Edition,
) -> (Vec<String>, Vec<String>) {
  let mut differences = Vec::new();
  let mut expected = Vec::new();
  let mut last_was_expected = false;
  let (mut i, mut j) = (0, 0);
  while i < ours.len() && j < theirs.len() {
    let (a, b) = (&ours[i], &theirs[j]);
    if a.start == b.start && a.end == b.end && a.class == b.class {
      i += 1;
      j += 1;
      continue;
    }
    let line = text[..a.start.min(b.start)].matches('\n').count() + 1;
    let description = format!(
      "line {}: mto {} {:?}, rustc {} {:?}",
      line,
      a.class,
      &text[a.start..a.end],
      b.class,
      &text[b.start..b.end]
    );
    match expected_difference(&b.class, &text[b.start..b.end], edition) {
      Some(reason) => {
        expected.push(format!("{} (expected: {})", description, reason));
        last_was_expected = true;
      }
      None => {
        differences.push(description);
        last_was_expected = false;
      }
    }
    // Resynchronise where both lexers next start a token at the same byte, so one root
    // cause is reported once.
    i += 1;
    j += 1;
    while let (Some(x), Some(y)) = (ours.get(i), theirs.get(j)) {
      if x.start == y.start {
        break;
      }
      if x.start < y.start {
        i += 1;
      } else {
        j += 1;
      }
    }
  }
  if i < ours.len() || j < theirs.len() {
    let line = format!(
      "token counts differ at the end: mto {} left, rustc {} left",
      ours.len() - i,
      theirs.len() - j
    );
    // Running out of shared boundaries right after an expected difference is part of it.
    if last_was_expected {
      expected.push(format!("{} (follows the expected difference above)", line));
    } else {
      differences.push(line);
    }
  }
  (differences, expected)
}

/// Why rustc's edition-agnostic lexer is expected to differ here, if it is: it always produces
/// tokens that only exist in later editions or on nightly, and its parser splits them again.
fn expected_difference(
  rustc_class: &str,
  rustc_text: &str,
  edition: Edition,
) -> Option<&'static str> {
  match rustc_class {
    "forced-keyword" => Some("nightly raw keyword `k#name`; mto lexes a reserved prefix"),
    "guarded-string-prefix" if edition < Edition::E2024 => Some("`#\"` is only reserved from 2024"),
    "guarded-string-prefix" => Some("both reject `#\"`; mto marks the `#` and lexes the string"),
    "lifetime" if edition < Edition::E2021 && rustc_text.starts_with("'r#") => {
      Some("raw lifetimes exist from 2021")
    }
    "literal"
      if edition < Edition::E2021
        && (rustc_text.starts_with("c\"") || rustc_text.starts_with("cr")) =>
    {
      Some("C strings exist from 2021")
    }
    _ => None,
  }
}

fn mto_class(kind: SyntaxKind) -> String {
  match kind {
    SyntaxKind::Whitespace => "whitespace".to_string(),
    SyntaxKind::Frontmatter => "frontmatter".to_string(),
    SyntaxKind::LineComment | SyntaxKind::BlockComment => "comment".to_string(),
    SyntaxKind::OuterLineDoc
    | SyntaxKind::InnerLineDoc
    | SyntaxKind::OuterBlockDoc
    | SyntaxKind::InnerBlockDoc => "doc-comment".to_string(),
    SyntaxKind::Ident | SyntaxKind::RawIdent => "ident".to_string(),
    SyntaxKind::Lifetime => "lifetime".to_string(),
    SyntaxKind::Unknown => "unknown".to_string(),
    kind if kind.is_literal() => "literal".to_string(),
    kind => format!("punct {}", kind.name()),
  }
}

/// mto's tokens after the byte-order mark and shebang, which rustc strips before lexing.
fn mto_spans(text: &str, edition: Edition) -> Vec<Span> {
  let mut spans = Vec::new();
  let mut offset = 0;
  for token in tokenize(text, edition) {
    let end = offset + token.len as usize;
    if token.kind != SyntaxKind::Bom && token.kind != SyntaxKind::Shebang {
      spans.push(Span {
        start: offset,
        end,
        class: mto_class(token.kind),
      });
    }
    offset = end;
  }
  spans
}

fn rustc_class(kind: rustc::TokenKind) -> String {
  use rustc::TokenKind as K;
  let punct = |name: &str| format!("punct {}", name);
  match kind {
    K::Whitespace => "whitespace".to_string(),
    K::LineComment { doc_style: None }
    | K::BlockComment {
      doc_style: None, ..
    } => "comment".to_string(),
    K::LineComment { .. } | K::BlockComment { .. } => "doc-comment".to_string(),
    K::Ident | K::RawIdent | K::UnknownPrefix => "ident".to_string(),
    K::Lifetime { .. } | K::RawLifetime => "lifetime".to_string(),
    K::Literal { .. } => "literal".to_string(),
    K::InvalidIdent => "invalid-ident".to_string(),
    K::ForcedKeywordIdent => "forced-keyword".to_string(),
    K::UnknownPrefixLifetime => "unknown-prefix-lifetime".to_string(),
    K::GuardedStrPrefix => "guarded-string-prefix".to_string(),
    K::Frontmatter { .. } => "frontmatter".to_string(),
    K::Unknown => "unknown".to_string(),
    K::Eof => "eof".to_string(),
    K::Semi => punct("SEMICOLON"),
    K::Comma => punct("COMMA"),
    K::Dot => punct("DOT"),
    K::OpenParen => punct("L_PAREN"),
    K::CloseParen => punct("R_PAREN"),
    K::OpenBrace => punct("L_BRACE"),
    K::CloseBrace => punct("R_BRACE"),
    K::OpenBracket => punct("L_BRACK"),
    K::CloseBracket => punct("R_BRACK"),
    K::At => punct("AT"),
    K::Pound => punct("POUND"),
    K::Tilde => punct("TILDE"),
    K::Question => punct("QUESTION"),
    K::Colon => punct("COLON"),
    K::Dollar => punct("DOLLAR"),
    K::Eq => punct("EQ"),
    K::Bang => punct("BANG"),
    K::Lt => punct("L_ANGLE"),
    K::Gt => punct("R_ANGLE"),
    K::Minus => punct("MINUS"),
    K::And => punct("AMP"),
    K::Or => punct("PIPE"),
    K::Plus => punct("PLUS"),
    K::Star => punct("STAR"),
    K::Slash => punct("SLASH"),
    K::Caret => punct("CARET"),
    K::Percent => punct("PERCENT"),
  }
}

/// rustc's tokens, positioned in the original text: the byte-order mark and shebang are
/// skipped first, as rustc itself does.
fn rustc_spans(text: &str) -> Vec<Span> {
  let mut offset = if text.starts_with('\u{feff}') {
    '\u{feff}'.len_utf8()
  } else {
    0
  };
  offset += rustc::strip_shebang(&text[offset..]).unwrap_or(0);
  let mut spans = Vec::new();
  for token in rustc::tokenize(&text[offset..], rustc::FrontmatterAllowed::Yes) {
    let end = offset + token.len as usize;
    spans.push(Span {
      start: offset,
      end,
      class: rustc_class(token.kind),
    });
    offset = end;
  }
  spans
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_known_difference_is_expected_not_unexplained() {
    let (unexplained, expected, _) = differences("k#y\n", Edition::E2021);
    assert!(unexplained.is_empty(), "{:?}", unexplained);
    assert_eq!(expected.len(), 1, "{:?}", expected);
  }

  #[test]
  fn test_ordinary_code_lexes_the_same() {
    let text = "fn f<'a>(x: &'a str) -> u8 { b'x' + 0x1f_u8 } // done\n/* nested /* */ */\n";
    let (unexplained, expected, tokens) = differences(text, Edition::E2021);
    assert!(unexplained.is_empty(), "{:?}", unexplained);
    assert!(expected.is_empty(), "{:?}", expected);
    assert!(tokens > 10);
  }
}
