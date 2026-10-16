//! Macro metavariables renamed so that a `macro_rules!` transcriber reads as code.

use super::{Edition, SyntaxKind as K, parse_tokens};

/// `text` with each metavariable `$name` renamed `zname` (`$$name` to `$zname`), as the
/// standard formatter's `replace_names` does, and the renamings made; `None` when it holds
/// a repetition (`$(..)`). Strings and comments are passed over.
pub fn replace_names(text: &str, edition: Edition) -> Option<(String, Vec<(String, String)>)> {
  let mut out = String::with_capacity(text.len() + 64);
  let mut names: Vec<(String, String)> = Vec::new();
  let mut dollars = 0usize;
  let mut name = String::new();
  let mut register = |out: &mut String, name: &str, dollars: usize| {
    let new = format!("{}z{}", "$".repeat(dollars - 1), name);
    let old = format!("{}{}", "$".repeat(dollars), name);
    out.push_str(&new);
    if !names.iter().any(|(o, _)| *o == old) {
      names.push((old, new));
    }
  };
  for (kind, piece) in parse_tokens(text, edition) {
    let quoted = kind.is_literal() && (piece.contains('"') || piece.contains('\''));
    let comment = kind.is_trivia() && kind != K::Whitespace;
    if quoted || comment {
      out.push_str(piece);
      continue;
    }
    for c in piece.chars() {
      if c == '$' {
        dollars += 1;
      } else if dollars == 0 {
        out.push(c);
      } else if !c.is_alphanumeric() && !name.is_empty() {
        register(&mut out, &name, dollars);
        out.push(c);
        dollars = 0;
        name.clear();
      } else if c == '(' && name.is_empty() {
        return None;
      } else if c.is_alphanumeric() || c == '_' {
        name.push(c);
      }
    }
  }
  if !name.is_empty() {
    register(&mut out, &name, dollars);
  }
  Some((out, names))
}
