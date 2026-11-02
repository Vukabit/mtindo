//! Rust in color, for the code mto prints to a terminal (`--emit stdout`, so `mto -`):
//! keywords, strings, numbers, comments, attributes, lifetimes and macro names. It reads
//! tokens only, so code that does not parse is colored as far as it lexes. Each colored
//! piece is closed at every line end, so a line stands alone in a pager. Removing the color
//! codes gives back `text` exactly.

use crate::syntax::lexer::tokenize;
use crate::syntax::parser::is_keyword_in;
use crate::syntax::{Edition, SyntaxKind as K};

const COMMENT: &str = "90";
const KEYWORD: &str = "35";
const STRING: &str = "32";
const NUMBER: &str = "33";
const ATTRIBUTE: &str = "36";
const LIFETIME: &str = "36";
const MACRO: &str = "1;34";

/// `text` with color codes around its tokens.
pub fn highlight(text: &str, edition: Edition) -> String {
  let tokens = tokenize(text, edition);
  let mut out = Painter {
    out: String::with_capacity(text.len() * 2),
    open: None,
  };
  let mut at = 0;
  // Inside an attribute: how deep in its brackets.
  let mut attribute: Option<usize> = None;
  for (idx, token) in tokens.iter().enumerate() {
    let piece = &text[at..at + token.len as usize];
    at += token.len as usize;
    let next = |n: usize| tokens.get(idx + n).map(|t| t.kind);
    if token.kind == K::Pound
      && attribute.is_none()
      && (next(1) == Some(K::LBrack) || (next(1) == Some(K::Bang) && next(2) == Some(K::LBrack)))
    {
      attribute = Some(0);
    }
    if let Some(depth) = attribute.as_mut() {
      match token.kind {
        K::LBrack => *depth += 1,
        K::RBrack => *depth -= 1,
        _ => {}
      }
      let done = token.kind == K::RBrack && *depth == 0;
      let color = match token.kind {
        K::LineComment | K::BlockComment => COMMENT,
        kind if kind.is_literal() => STRING,
        _ => ATTRIBUTE,
      };
      out.push(Some(color), piece);
      if done {
        attribute = None;
      }
      continue;
    }
    let color = match token.kind {
      K::LineComment
      | K::BlockComment
      | K::OuterLineDoc
      | K::InnerLineDoc
      | K::OuterBlockDoc
      | K::InnerBlockDoc
      | K::Shebang
      | K::Frontmatter => Some(COMMENT),
      K::IntNumber | K::FloatNumber => Some(NUMBER),
      kind if kind.is_literal() => Some(STRING),
      K::Lifetime => Some(LIFETIME),
      // `name!`, but not `name != other` written tight.
      K::Ident if next(1) == Some(K::Bang) && next(2) != Some(K::Eq) => Some(MACRO),
      K::Bang if idx > 0 && tokens[idx - 1].kind == K::Ident && next(1) != Some(K::Eq) => {
        let name_at = at - piece.len() - tokens[idx - 1].len as usize;
        let name = &text[name_at..at - piece.len()];
        (!is_keyword_in(name, edition)).then_some(MACRO)
      }
      K::Ident if is_keyword_in(piece, edition) => Some(KEYWORD),
      _ => None,
    };
    let color = match (token.kind, color) {
      // A keyword before `!` is not a macro: `if !done`.
      (K::Ident, Some(MACRO)) if is_keyword_in(piece, edition) => Some(KEYWORD),
      _ => color,
    };
    out.push(color, piece);
  }
  out.close();
  out.out
}

/// Colored text, with neighbours of one color in one span.
struct Painter {
  out: String,
  /// The color the text written last is in, still open.
  open: Option<&'static str>,
}

impl Painter {
  /// `piece` in `color`, or plain; a span is closed at each line end.
  fn push(&mut self, color: Option<&'static str>, piece: &str) {
    for (idx, line) in piece.split('\n').enumerate() {
      if idx > 0 {
        self.close();
        self.out.push('\n');
      }
      if line.is_empty() {
        continue;
      }
      // Spaces take the color around them rather than end it.
      let color = if line.trim().is_empty() && self.open.is_some() {
        self.open
      } else {
        color
      };
      if color != self.open {
        self.close();
        if let Some(color) = color {
          self.out.push_str(&format!("\x1b[{}m", color));
        }
        self.open = color;
      }
      self.out.push_str(line);
    }
  }

  fn close(&mut self) {
    if self.open.take().is_some() {
      self.out.push_str("\x1b[0m");
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// `text` without its color codes.
  fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("\x1b[") {
      out.push_str(&rest[..at]);
      let end = rest[at..].find('m').expect("a code ends in m");
      rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    out
  }

  #[test]
  fn test_each_kind_of_token_gets_its_color() {
    let text = "#[derive(Debug)]\nfn f<'a>(x: &'a str) -> u8 {\n  // note\n  if !x.is_empty() { println!(\"{}\", 1) }\n  a != b\n}\n";
    let colored = highlight(text, Edition::E2021);
    let expect = [
      "\x1b[36m#[derive(Debug)]\x1b[0m\n",
      "\x1b[35mfn \x1b[0mf<\x1b[36m'a\x1b[0m>",
      "  \x1b[90m// note\x1b[0m\n",
      "\x1b[35mif \x1b[0m!x",
      "\x1b[1;34mprintln!\x1b[0m(\x1b[32m\"{}\"\x1b[0m, \x1b[33m1\x1b[0m)",
      "  a != b\n",
    ];
    for piece in expect {
      assert!(colored.contains(piece), "{:?} not in {:?}", piece, colored);
    }
    assert_eq!(plain(&colored), text);
  }

  #[test]
  fn test_a_token_across_lines_is_closed_at_each_line_end() {
    let colored = highlight("/* one\ntwo */\n", Edition::E2021);
    assert_eq!(colored, "\x1b[90m/* one\x1b[0m\n\x1b[90mtwo */\x1b[0m\n");
  }

  #[test]
  fn test_every_fixture_comes_back_as_it_was() {
    fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
      for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
          walk(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
          files.push(path);
        }
      }
    }
    let mut files = Vec::new();
    walk(
      &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
      &mut files,
    );
    assert!(files.len() > 50);
    for file in files {
      let text = std::fs::read_to_string(&file).unwrap();
      assert_eq!(
        plain(&highlight(&text, Edition::E2024)),
        text,
        "{}",
        file.display()
      );
    }
  }
}
