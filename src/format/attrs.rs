//! Attributes laid out as the Rust style lays them out: `name = value` on one line however
//! long; `#[derive(...)]` on one line when it ends four columns short of the line width, else
//! on one row inside or one name per line; any other argument list as a call, each list within
//! `attr_fn_like_width`, lines inside a broken one and a nested list overflowing its parent
//! ending a column short of the line width, a trailing comma kept only where written. Attributes holding a macro call or a
//! string spanning lines are left as written.
//!
//! Only attributes whose arguments read as names and values are taken (the caller checks);
//! one holding a comment is left as written.

use SyntaxKind as K;

use super::expr::{flat, rewrite};
use super::shape::{Limits, Shape, Spacing};
use crate::options::format::FormatConfig;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, parse};
use crate::text::unicode::str_width;

/// The attribute laid out, starting at column `indent` on a line of its own, its later lines
/// indented in full; `None` when it is not one this module takes or does not fit.
pub fn rewrite_attr(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<String> {
  if config.hard_tabs || config.tab_spaces == 0 || has_comment(node) || spans_lines(node) {
    return None;
  }
  let meta = node.children().into_iter().find(|c| c.kind() == K::Meta)?;
  let open = if node
    .children_with_tokens()
    .iter()
    .any(|e| e.kind() == K::Bang)
  {
    "#!["
  } else {
    "#["
  };
  let spacing = Spacing::of(config);
  let tree = meta
    .children()
    .into_iter()
    .find(|c| c.kind() == K::TokenTree);
  let text = match tree {
    // `#[name]` and `#[name = value]`: one line.
    None => format!("{}{}]", open, flat(&meta, spacing)),
    Some(tree) => {
      let path = flat(
        &meta.children().into_iter().find(|c| c.kind() == K::Path)?,
        spacing,
      );
      if path == "derive" {
        derive(&tree, open, indent, config)?
      } else {
        // A `use` declaration's attributes get four columns less (`use `), as the standard
        // formatter gives them (found by measuring it).
        let on_use = node.parent().is_some_and(|p| p.kind() == K::Use);
        call(
          &path,
          &tree,
          open,
          indent,
          if on_use { 4 } else { 0 },
          config,
        )?
      }
    }
  };
  same_tokens(node, &text).then_some(text)
}

/// `name(arguments)` read as a call and laid out as one, its argument lists within
/// `attr_fn_like_width`.
fn call(
  path: &str,
  tree: &SyntaxNode,
  open: &str,
  indent: usize,
  narrower: usize,
  config: &FormatConfig,
) -> Option<String> {
  let source = format!("fn f() {{\n{}{};\n}}", path, tree.text());
  let parsed = parse(&source, Edition::E2021);
  if !parsed.errors().is_empty() {
    return None;
  }
  let root = parsed.syntax_node();
  let call = root
    .descendants()
    .into_iter()
    .find(|n| n.kind() == K::ExprStmt)?
    .children()
    .into_iter()
    .next()
    .filter(|n| n.kind() == K::CallExpr)?;
  // The lines inside a broken attribute end a column short of the line width; on one line
  // it may reach it (found by measuring the standard formatter).
  let limits = Limits {
    max_width: config.max_width.checked_sub(1)?,
    tab: config.tab_spaces,
    fn_call: config.attr_fn_like_width,
    chain: config.chain_width,
    struct_lit: config.struct_lit_width,
    array: config.array_width,
    short_item: config.short_array_element_width_threshold,
    spacing: Spacing::of(config),
    attribute: true,
    comma_as_written: true,
    style_2024: config.style_edition >= 2024,
  };
  // 1 = `]`
  let width = config
    .max_width
    .checked_sub(indent + open.len() + 1 + narrower)?;
  let shape = Shape {
    indent,
    offset: open.len(),
    width,
    limits,
  };
  Some(format!("{}{}]", open, rewrite(&call, shape)?))
}

/// `#[derive(...)]`: one line when it ends four columns short of the line width, else the
/// names on one row inside when they fit before its comma, else one name per line, each with
/// its comma, as the standard formatter writes it (found by measuring it).
fn derive(tree: &SyntaxNode, open: &str, indent: usize, config: &FormatConfig) -> Option<String> {
  let mut names: Vec<String> = vec![String::new()];
  let tokens: Vec<_> = tree
    .children_with_tokens()
    .into_iter()
    .filter(|e| !e.kind().is_trivia())
    .collect();
  // Inside the parentheses: names of path tokens, separated by commas.
  for element in tokens.get(1..tokens.len().checked_sub(1)?)? {
    match element {
      SyntaxElement::Token(t) if t.kind() == K::Comma => names.push(String::new()),
      SyntaxElement::Token(t) => names.last_mut()?.push_str(t.text()),
      SyntaxElement::Node(_) => return None,
    }
  }
  // A written trailing comma leaves an empty last name; an empty one elsewhere
  // (`A,, B`) the standard formatter leaves as written.
  if names.last().is_some_and(String::is_empty) {
    names.pop();
  }
  if names.is_empty() || names.iter().any(String::is_empty) {
    return None;
  }
  let one_line = format!("{}derive({})]", open, names.join(", "));
  if indent + str_width(&one_line) + 4 <= config.max_width {
    return Some(one_line);
  }
  let pad = " ".repeat(indent + config.tab_spaces);
  // The names on one row inside, when they fit before its comma; else one per line.
  let row = names.join(", ");
  if indent + config.tab_spaces + str_width(&row) <= config.max_width {
    return Some(format!(
      "{}derive(\n{}{},\n{})]",
      open,
      pad,
      row,
      " ".repeat(indent)
    ));
  }
  let lines: String = names.iter().map(|n| format!("\n{}{},", pad, n)).collect();
  Some(format!(
    "{}derive({}\n{})]",
    open,
    lines,
    " ".repeat(indent)
  ))
}

fn has_comment(node: &SyntaxNode) -> bool {
  node.descendants_with_tokens().iter().any(
    |e| matches!(e, SyntaxElement::Token(t) if t.kind().is_trivia() && t.kind() != K::Whitespace),
  )
}

/// Whether `text` holds the attribute's tokens, commas aside.
fn same_tokens(node: &SyntaxNode, text: &str) -> bool {
  let code = |e: &SyntaxElement| match e {
    SyntaxElement::Token(t) if !t.kind().is_trivia() && t.kind() != K::Comma => {
      Some(t.text().to_string())
    }
    _ => None,
  };
  let theirs: Vec<String> = node
    .descendants_with_tokens()
    .iter()
    .filter_map(code)
    .collect();
  let parsed = parse(&format!("{}\nfn f() {{}}", text), Edition::E2021).syntax_node();
  let ours: Vec<String> = parsed
    .descendants_with_tokens()
    .iter()
    .take_while(|e| e.kind() != K::FnKw)
    .filter_map(code)
    .collect();
  theirs == ours
}

/// Whether a string in the attribute spans lines; the standard formatter leaves it as written.
fn spans_lines(node: &SyntaxNode) -> bool {
  node.descendants_with_tokens().iter().any(
    |e| matches!(e, SyntaxElement::Token(t) if t.kind().is_literal() && t.text().contains('\n')),
  )
}

#[cfg(test)]
mod tests;
