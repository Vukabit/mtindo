//! The bug report printed when the verifier stops a write, with the smallest input that
//! still shows the problem.

use super::Violation;
use crate::syntax::{Edition, SyntaxElement, parse};

/// The report for one file. `minimal` is the smallest input found that still fails.
pub fn bug_report(path: &str, violations: &[Violation], minimal: Option<&str>) -> String {
  let mut out = format!(
    "mto: internal error: the verifier stopped the write of {}; the file is unchanged.\nThis is a bug in mto. What it would have broken:\n",
    path
  );
  for violation in violations {
    out.push_str(&format!(
      "  {}: {}\n",
      violation.rule.name(),
      violation.detail
    ));
  }
  if let Some(minimal) = minimal {
    out.push_str("The smallest part of the file that still shows it:\n  ----\n");
    for line in minimal.lines() {
      out.push_str("  ");
      out.push_str(line);
      out.push('\n');
    }
    out.push_str("  ----\n");
  }
  out.push_str("Please report this, with the text above.\n");
  out
}

/// The smallest text, made by dropping whole top-level elements (items, comments, damaged
/// text) of `input`, for which `fails` is still true. Each pass tries dropping every
/// element once; passes repeat until none can go. `fails(input)` is assumed true.
pub fn minimize(input: &str, edition: Edition, fails: &dyn Fn(&str) -> bool) -> String {
  let mut current = input.to_string();
  loop {
    let pieces = top_level_pieces(&current, edition);
    let mut shrunk = false;
    let mut kept: Vec<String> = pieces.clone();
    let mut idx = 0;
    while idx < kept.len() {
      if kept[idx].trim().is_empty() {
        idx += 1;
        continue;
      }
      let candidate: String = kept
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != idx)
        .map(|(_, p)| p.as_str())
        .collect();
      if fails(&candidate) {
        kept.remove(idx);
        shrunk = true;
      } else {
        idx += 1;
      }
    }
    current = kept.concat();
    if !shrunk {
      return current;
    }
  }
}

/// The report for a file mto failed on (it panicked) while laying it out: `said` is what it
/// said and where, `minimal` the smallest input found that still fails.
pub fn panic_report(path: &str, said: &str, minimal: Option<&str>) -> String {
  let mut out = format!(
    "mto: internal error: mto failed while laying out {}; the file is unchanged.\nThis is a bug in mto. What failed:\n  {}\n",
    path, said
  );
  if let Some(minimal) = minimal {
    out.push_str("The smallest part of the file that still fails:\n  ----\n");
    for line in minimal.lines() {
      out.push_str("  ");
      out.push_str(line);
      out.push('\n');
    }
    out.push_str("  ----\n");
  }
  out.push_str("Please report this, with the text above.\n");
  out
}

/// The file's top-level elements as text, each with the whitespace that follows it, so
/// that concatenating them gives the file back.
fn top_level_pieces(text: &str, edition: Edition) -> Vec<String> {
  let tree = parse(text, edition).syntax_node();
  let mut pieces: Vec<String> = Vec::new();
  for element in tree.children_with_tokens_iter() {
    let (piece, whitespace) = match &element {
      SyntaxElement::Node(node) => (node.text().to_string(), false),
      SyntaxElement::Token(token) => (
        token.text().to_string(),
        token.kind() == crate::syntax::SyntaxKind::Whitespace,
      ),
    };
    match pieces.last_mut() {
      Some(last) if whitespace => last.push_str(&piece),
      _ => pieces.push(piece),
    }
  }
  pieces
}
