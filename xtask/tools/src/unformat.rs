//! The unformatter (S3-6): a file with the whitespace between its tokens scrambled, so the
//! compatibility harness tests mto on messy code, not only on code the standard formatter has
//! already tidied.
//!
//! Every run of whitespace between two tokens becomes another run, chosen from a fixed set
//! by a generator seeded from the file's own text, so the same file always scrambles the
//! same way. A run never becomes empty (tokens would merge), and a run after a line comment,
//! or after the shebang, keeps a line break (the comment would swallow the next line).
//! Tokens themselves, strings and comments included, are untouched, so the program means
//! what it meant.

use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::{Edition, SyntaxKind};

/// The runs that keep a line break, for where one is required.
const BREAKING: &[&str] = &["\n", "\n\n", "\n    ", "\n        ", "\n\t"];

/// What a run of whitespace may become. None puts spaces before a line break: the standard
/// formatter refuses to leave trailing whitespace in code it keeps as written, which would
/// drop those files from the set instead of testing them.
const RUNS: &[&str] = &[
  " ",
  "  ",
  "\n",
  "\n\n",
  "\t",
  "\n    ",
  "\n        ",
  "\n\t",
];

/// `text` with its whitespace scrambled; `None` when it has a carriage return (line endings
/// are left to their own tests).
pub fn unformat(text: &str, edition: Edition) -> Option<String> {
  if text.contains('\r') {
    return None;
  }
  let mut state = seed(text);
  let mut out = String::with_capacity(text.len() + text.len() / 4);
  let mut at = 0;
  let mut previous = None;
  for token in tokenize(text, edition) {
    let end = at + token.len as usize;
    let piece = &text[at..end];
    if token.kind == SyntaxKind::Whitespace {
      let needs_break = matches!(
        previous,
        Some(
          SyntaxKind::LineComment
            | SyntaxKind::OuterLineDoc
            | SyntaxKind::InnerLineDoc
            | SyntaxKind::Shebang
            | SyntaxKind::Frontmatter
        )
      );
      let choices = if needs_break { BREAKING } else { RUNS };
      state = next(state);
      out.push_str(choices[(state % choices.len() as u64) as usize]);
    } else {
      out.push_str(piece);
    }
    previous = Some(token.kind);
    at = end;
  }
  // A file ending without whitespace after a line comment still ends its line.
  if !out.ends_with('\n') {
    out.push('\n');
  }
  Some(out)
}

/// xorshift64*: a small, fixed generator, so scrambles repeat exactly.
fn next(mut x: u64) -> u64 {
  x ^= x >> 12;
  x ^= x << 25;
  x ^= x >> 27;
  x.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

/// A non-zero seed from the text (FNV-1a).
fn seed(text: &str) -> u64 {
  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  for byte in text.bytes() {
    hash ^= u64::from(byte);
    hash = hash.wrapping_mul(0x0100_0000_01b3);
  }
  hash | 1
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_tokens_survive_and_line_comments_keep_their_break() {
    let text = "fn main() { // note\n    let x = \"a  b\"; /* c */ x }\n";
    let messy = unformat(text, Edition::E2021).unwrap();
    let tokens = |t: &str| -> Vec<String> {
      let mut at = 0;
      tokenize(t, Edition::E2021)
        .into_iter()
        .filter_map(|tok| {
          let piece = t[at..at + tok.len as usize].to_string();
          at += tok.len as usize;
          (tok.kind != SyntaxKind::Whitespace).then_some(piece)
        })
        .collect()
    };
    assert_eq!(tokens(text), tokens(&messy));
    let after_comment = &messy[messy.find("// note").unwrap() + 7..];
    assert!(after_comment.starts_with([' ', '\n']) && after_comment.contains('\n'));
    // The same file always scrambles the same way.
    assert_eq!(unformat(text, Edition::E2021), Some(messy));
    assert_eq!(unformat("a\r\n", Edition::E2021), None);
  }
}
