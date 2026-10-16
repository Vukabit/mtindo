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
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, parse, parse_tokens};
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
  // Keywords are names in an attribute's arguments (`returns(ref)`), which do not read as
  // an expression: each stands in as the same word capitalized, put back after.
  let mut keywords: Vec<(String, String)> = Vec::new();
  let mut arguments = String::new();
  for (kind, text, _) in tree.token_texts() {
    // In a token tree a keyword may be an identifier still.
    let stands_in = (kind.name().ends_with("_KW") || kind == K::Ident)
      && crate::syntax::parser::is_keyword_in(text, Edition::E2021)
      && !matches!(text, "true" | "false" | "self" | "Self" | "super" | "crate");
    if stands_in {
      let mut word = text.to_string();
      word[..1].make_ascii_uppercase();
      if !keywords.iter().any(|(w, _)| *w == word) {
        keywords.push((word.clone(), text.to_string()));
      }
      arguments.push_str(&word);
    } else {
      arguments.push_str(text);
    }
  }
  if keywords
    .iter()
    .any(|(word, _)| tree.token_texts().any(|(_, text, _)| text == word))
  {
    return None;
  }
  let source = format!("fn f() {{\n{}{};\n}}", path, arguments);
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
    fn_call: config.attr_fn_like_width,
    attribute: true,
    comma_as_written: true,
    ..Limits::of(config)
  };
  // 1 = `]`
  let width = config
    .max_width
    .checked_sub(indent + open.len() + 1 + narrower)?;
  let shape = Shape {
    align: 0,
    indent,
    offset: open.len(),
    width,
    limits,
  };
  let text = rewrite(&call, shape)?;
  let text = if keywords.is_empty() {
    text
  } else {
    crate::syntax::parse_tokens(&text, Edition::E2021)
      .into_iter()
      .map(|(_, piece)| {
        keywords
          .iter()
          .find(|(word, _)| word == piece)
          .map_or(piece, |(_, keyword)| keyword.as_str())
      })
      .collect()
  };
  Some(format!("{}{}]", open, text))
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
      SyntaxElement::Token(t) => {
        let name = names.last_mut()?;
        // Two words written apart would fuse into one.
        let word = |c: char| c == '_' || c.is_alphanumeric();
        if name.ends_with(word) && t.text().starts_with(word) {
          return None;
        }
        name.push_str(t.text());
      }
      SyntaxElement::Node(_) => return None,
    }
  }
  // The tokens of a name are joined with nothing between, which keeps their meaning only
  // for a path: `A B` would become `AB`, an identifier and a string a reserved prefix
  // (`Par"x"`). Anything else is left to the general layout.
  if !names.iter().all(|n| n.is_empty() || is_path(n)) {
    return None;
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
  node
    .token_texts()
    .any(|(kind, _, _)| kind.is_trivia() && kind != K::Whitespace)
}

/// Whether a derive name, its tokens joined, is a path: `Name`, `a::b::Name`, `::a::Name`,
/// `r#Name` (no two words were fused joining it; the caller checks).
fn is_path(name: &str) -> bool {
  let rest = name.strip_prefix("::").unwrap_or(name);
  !rest.is_empty()
    && rest.split("::").all(|segment| {
      let ident = segment.strip_prefix("r#").unwrap_or(segment);
      ident
        .chars()
        .next()
        .is_some_and(|c| c == '_' || crate::text::unicode::is_ident_start(c))
        && ident.chars().all(crate::text::unicode::is_ident_continue)
    })
}

/// Whether `text` holds the attribute's tokens, commas aside.
fn same_tokens(node: &SyntaxNode, text: &str) -> bool {
  let code = |kind: SyntaxKind| !kind.is_trivia() && kind != K::Comma;
  // As written: nothing to compare.
  if super::stmt::written_as(node, node.text_range().start as usize, text) {
    return true;
  }
  let source = format!("{}\nfn f() {{}}", text);
  let ours = parse_tokens(&source, Edition::E2021);
  node
    .token_texts()
    .filter(|(kind, _, _)| code(*kind))
    .map(|(_, t, _)| t)
    .eq(
      ours
        .iter()
        .take_while(|(kind, _)| *kind != K::FnKw)
        .filter(|(kind, _)| code(*kind))
        .map(|(_, t)| *t),
    )
}

/// Whether a string in the attribute spans lines; the standard formatter leaves it as written.
fn spans_lines(node: &SyntaxNode) -> bool {
  node
    .token_texts()
    .any(|(kind, text, _)| kind.is_literal() && text.contains('\n'))
}

#[cfg(test)]
mod tests;
