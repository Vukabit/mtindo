//! The verifier must never raise a false alarm or panic:
//! - a file checked against itself is clean;
//! - for a file that parses, moving its items around (header first, `macro_rules!` in their
//!   order, everything joined by blank lines in the file's own line endings) is clean;
//! - checking what layout writes never panics; its findings are not asserted on here.

use crate::syntax::ast::{Attr, Item, ItemKind};
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, parse};
use crate::verify::verify_layout;
use crate::{LayoutConfig, try_reorganize};

/// Checks one input; panics on any violation.
pub fn check(data: &[u8]) {
  let Ok(text) = std::str::from_utf8(data) else {
    return;
  };
  let edition = Edition::E2021;
  let same = verify_layout(text, text, edition);
  assert!(same.is_empty(), "a file against itself: {:?}", same);

  if let Some(moved) = moved_items(text, edition) {
    let found = verify_layout(text, &moved, edition);
    assert!(
      found.is_empty(),
      "moved items:\n{}\n---\n{}\n{:?}",
      text,
      moved,
      found
    );
  }

  if let Ok(output) = try_reorganize(text, &LayoutConfig::default(), false) {
    let _ = verify_layout(text, &output, edition);
  }
}

/// The file's top-level elements with the header first and the rest in reverse, except that
/// `macro_rules!` definitions keep their order among themselves. `None` when the file does
/// not parse cleanly or mixes line endings.
fn moved_items(text: &str, edition: Edition) -> Option<String> {
  let parsed = parse(text, edition);
  if !parsed.errors().is_empty() || text.contains('\r') {
    return None;
  }
  let tree = parsed.syntax_node();
  let mut header = Vec::new();
  let mut rest = Vec::new();
  let mut macros = Vec::new();
  for element in tree.children_with_tokens() {
    match element {
      SyntaxElement::Token(token) => match token.kind() {
        SyntaxKind::Whitespace => {}
        SyntaxKind::Bom
        | SyntaxKind::Shebang
        | SyntaxKind::Frontmatter
        | SyntaxKind::InnerLineDoc
        | SyntaxKind::InnerBlockDoc => header.push(token.text().to_string()),
        _ => rest.push(Some(token.text().to_string())),
      },
      SyntaxElement::Node(node) => {
        if Attr::cast(node.clone()).is_some_and(|a| a.is_inner()) {
          header.push(node.text());
        } else if Item::cast(node.clone()).is_some_and(|i| i.kind() == ItemKind::MacroRules) {
          macros.push(node.text());
          rest.push(None);
        } else {
          rest.push(Some(node.text()));
        }
      }
    }
  }
  rest.reverse();
  let mut macros = macros.into_iter();
  let body: Vec<String> = rest
    .into_iter()
    .map(|piece| piece.or_else(|| macros.next()).unwrap_or_default())
    .collect();
  let bom = if text.starts_with('\u{feff}') {
    "\u{feff}"
  } else {
    ""
  };
  let header: Vec<String> = header.into_iter().filter(|h| h != "\u{feff}").collect();
  // Line comments must end their line, so every piece is followed by a line break.
  let mut out = String::from(bom);
  for piece in header.iter() {
    out.push_str(piece);
    out.push('\n');
  }
  for piece in body {
    out.push('\n');
    out.push_str(&piece);
    out.push('\n');
  }
  Some(out)
}
