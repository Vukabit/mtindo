//! mto's directives, written as line comments: `// mto: off` and `// mto: on` around
//! code mto leaves exactly as written, and `// mto: pin` above an item layout keeps in its
//! place.
//!
//! A directive is a line comment alone on its line whose whole text is the directive, with
//! nothing after it but whitespace, or ` -- ` and a reason (`// mto: off -- aligned by
//! hand`). It belongs to no item: comment attachment passes over it, so a comment after it
//! belongs to the code below. A region runs from `// mto: off` to the next `// mto: on` in
//! the same block, or to the block's end; one among a statement's or item's parts (list
//! items, match arms) leaves that whole statement or item as written.
//!
//! What looks meant as a directive and is not one (`//mto: off`, `// MTO: off`, `// mto:
//! of`, words after it without `--`, one after code on its line, in a block or doc comment)
//! is reported, never followed; so are markers without a partner.

use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

/// One of mto's directives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
  Off,
  On,
  Pin,
}

/// What a file's directives say: the regions left as written (byte ranges of the source),
/// where each `// mto: pin` stands, and what is wrong with any.
#[derive(Debug, Default)]
pub struct Directives {
  pub regions: Vec<(usize, usize)>,
  pub pins: Vec<usize>,
  /// Each problem, at the byte offset of the comment it is about.
  pub warnings: Vec<(usize, String)>,
}

/// The directive the line comment at `text[start..end]` is: one whose text is a directive,
/// alone on its line.
pub fn at(text: &str, start: usize, end: usize) -> Option<Marker> {
  let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
  if !text[line_start..start].trim().is_empty() {
    return None;
  }
  parse(&text[start..end])
}

/// The directive a line comment's text is, when it is one.
pub fn parse(comment: &str) -> Option<Marker> {
  let rest = comment.trim_end().strip_prefix("// mto: ")?;
  let word = match rest.split_once(" -- ") {
    Some((word, _)) => word.trim_end(),
    None => rest,
  };
  match word {
    "off" => Some(Marker::Off),
    "on" => Some(Marker::On),
    "pin" => Some(Marker::Pin),
    _ => None,
  }
}

/// Every directive in the tree `root` (whose text is `text`), read as a whole.
pub fn scan(root: &SyntaxNode, text: &str) -> Directives {
  let mut found = Directives::default();
  // Regions still open, with the block each started in.
  let mut open: Vec<(SyntaxNode, usize)> = Vec::new();
  for element in root.descendants_with_tokens_iter() {
    let SyntaxElement::Token(token) = element else {
      continue;
    };
    let (start, end) = (
      token.text_range().start as usize,
      token.text_range().end as usize,
    );
    let marker = if token.kind() == SyntaxKind::LineComment {
      at(text, start, end)
    } else {
      None
    };
    let Some(marker) = marker else {
      if let Some(problem) = near_miss(token.kind(), &text[start..end]) {
        found.warnings.push((start, problem));
      }
      continue;
    };
    let block = token.parent();
    let here = open.iter().position(|(b, _)| *b == block);
    match (marker, here) {
      (Marker::Pin, _) => found.pins.push(start),
      (Marker::Off, None) => open.push((block, start)),
      (Marker::Off, Some(_)) => found.warnings.push((
        start,
        "`// mto: off` inside a region already off: it does nothing".to_string(),
      )),
      (Marker::On, Some(at)) => {
        let (block, from) = open.remove(at);
        found.regions.push(widened(&block, from, end));
      }
      (Marker::On, None) => found.warnings.push((
        start,
        "`// mto: on` with no `// mto: off` before it in its block: it does nothing".to_string(),
      )),
    }
  }
  for (block, from) in open {
    found.warnings.push((
      from,
      "`// mto: off` with no `// mto: on` after it in its block: the region runs to the \
       block's end"
        .to_string(),
    ));
    let to = match block.last_token() {
      Some(last) if last.kind() == SyntaxKind::RBrace => last.text_range().start as usize,
      _ => block.text_range().end as usize,
    };
    found.regions.push(widened(&block, from, to));
  }
  found.regions.sort_unstable();
  found
}

/// The directive a comment's text seems meant as, written loosely: `mto` in any case, an
/// optional colon, then `off` (or `of`), `on` or `pin`, as a word of its own.
fn directive_word(body: &str) -> Option<&'static str> {
  let lower = body.to_lowercase();
  let rest = lower.strip_prefix("mto")?;
  let rest = rest.strip_prefix(':').unwrap_or(rest).trim_start();
  let word: String = rest
    .chars()
    .take_while(|c| c.is_ascii_alphanumeric())
    .collect();
  match word.as_str() {
    "off" | "of" => Some("off"),
    "on" => Some("on"),
    "pin" => Some("pin"),
    _ => None,
  }
}

/// A node whose children stand on lines of their own: items or statements.
fn holds_lines(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    SyntaxKind::SourceFile
      | SyntaxKind::ItemList
      | SyntaxKind::AssocItemList
      | SyntaxKind::ExternItemList
      | SyntaxKind::BlockExpr
  )
}

/// What is wrong with a comment that looks meant as a directive and is not one.
fn near_miss(kind: SyntaxKind, comment: &str) -> Option<String> {
  let (opener, body) = match kind {
    SyntaxKind::LineComment => ("//", comment.strip_prefix("//")?),
    SyntaxKind::BlockComment => ("/*", comment.strip_prefix("/*")?.trim_end_matches("*/")),
    SyntaxKind::OuterLineDoc => ("///", comment.strip_prefix("///")?),
    SyntaxKind::InnerLineDoc => ("//!", comment.strip_prefix("//!")?),
    _ => return None,
  };
  let body = body.trim();
  let word = directive_word(body)?;
  let right = format!("`// mto: {}`", word);
  Some(if kind != SyntaxKind::LineComment {
    format!(
      "a `{}` comment is not a directive, so this is not followed; write {} on a line of \
       its own",
      opener, right
    )
  } else if parse(comment).is_some() {
    format!(
      "a directive after code is not followed; put {} on a line of its own",
      right
    )
  } else if body.starts_with(&format!("mto: {} ", word)) {
    format!(
      "words after {} need ` -- ` before them (`// mto: {} -- why`), so this is not \
       followed",
      right, word
    )
  } else {
    format!("not followed: a directive is written {}", right)
  })
}

/// The region `from..to` in `block`, or, when `block` holds the parts of one statement or
/// item rather than lines of its own, that whole statement or item.
fn widened(block: &SyntaxNode, from: usize, to: usize) -> (usize, usize) {
  let mut whole = block.clone();
  while !holds_lines(whole.kind())
    && let Some(parent) = whole.parent()
  {
    if holds_lines(parent.kind()) {
      break;
    }
    whole = parent;
  }
  if holds_lines(whole.kind()) {
    return (from, to);
  }
  let range = whole.text_range();
  (from.min(range.start as usize), to.max(range.end as usize))
}

#[cfg(test)]
mod tests;
