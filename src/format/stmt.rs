//! Statements laid out anew with the width-budget code ([`super::expr`]): their lines broken
//! and joined as the Rust style lays them out, rather than kept where they were written.
//!
//! Only the statements the expression code covers are taken: a `let` with an initializer,
//! or an expression with a `;`, whose expression is a call, a method chain, a struct literal,
//! an array or a tuple. A statement holding a comment, a block, a macro, an attribute or a
//! string spanning lines is left to the whole-file rules, which keep its lines as written.

use SyntaxKind as K;

use super::expr::{flat, rewrite};
use super::shape::{Limits, Shape};
use crate::options::format::FormatConfig;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, parse};
use crate::text::unicode::str_width;

/// The statement laid out, starting at column `indent` on a line of its own, its later lines
/// indented in full; `None` when it is not one this module takes, or does not fit.
pub fn rewrite_statement(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<String> {
  if !takes(node, config) {
    return None;
  }
  // Types are printed flat for now: a statement whose generic arguments would have to
  // break is left as written.
  let generics = node
    .descendants()
    .iter()
    .any(|d| d.kind() == K::GenericArgList);
  if generics && indent + str_width(&flat(node)) > config.max_width {
    return None;
  }
  let text = laid_out(node, indent, config)?;
  same_tokens(node, &text).then_some(text)
}

/// The statement's expression: an expression statement's, or a `let`'s initializer.
fn expression_of(node: &SyntaxNode) -> Option<SyntaxNode> {
  let children = node.children();
  match node.kind() {
    K::ExprStmt => children.into_iter().next(),
    K::LetStmt => {
      // The initializer is the node after `=`.
      let mut after_eq = false;
      for element in node.children_with_tokens() {
        match element {
          SyntaxElement::Token(t) if t.kind() == K::Eq => after_eq = true,
          SyntaxElement::Node(n) if after_eq => return Some(n),
          _ => {}
        }
      }
      None
    }
    _ => None,
  }
}

/// The statement's tokens before `expression`, flat.
fn flat_before(node: &SyntaxNode, expression: &SyntaxNode) -> Option<String> {
  let whole = flat(node);
  let rest = format!("{};", flat(expression));
  whole
    .strip_suffix(&rest)
    .map(|head| head.trim_end().to_string())
}

fn laid_out(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<String> {
  let limits = Limits {
    max_width: config.max_width,
    tab: config.tab_spaces,
    fn_call: config.fn_call_width,
    chain: config.chain_width,
    struct_lit: config.struct_lit_width,
    array: config.array_width,
    short_item: config.short_array_element_width_threshold,
  };
  let shape = Shape {
    indent,
    offset: 0,
    width: config.max_width.checked_sub(indent)?,
    limits,
  };
  let tab = config.tab_spaces;
  match node.kind() {
    K::ExprStmt => {
      let expression = expression_of(node)?;
      let text = rewrite(&expression, shape.sub_width(1)?)?;
      Some(format!("{};", text))
    }
    K::LetStmt => {
      let expression = expression_of(node)?;
      let head = let_head(node, &expression, shape)?;
      let head_width = last_line_used(&head, indent) + 1;
      let same_line = shape
        .offset_left(head_width)
        .and_then(|s| s.sub_width(1))
        .and_then(|s| rewrite(&expression, s));
      let next_line = shape
        .block_indent(tab)
        .sub_width(1)
        .and_then(|s| rewrite(&expression, s));
      let pad = " ".repeat(indent + tab);
      match (same_line, next_line) {
        (Some(same), Some(next)) if prefer_next_line(&same, &next) => {
          Some(format!("{}\n{}{};", head, pad, next))
        }
        (Some(same), _) => Some(format!("{} {};", head, same)),
        (None, Some(next)) => Some(format!("{}\n{}{};", head, pad, next)),
        (None, None) => None,
      }
    }
    _ => None,
  }
}

/// The columns `text`'s last line takes past `indent`: a text starting at `indent` whose
/// later lines are indented in full.
fn last_line_used(text: &str, indent: usize) -> usize {
  match text.rsplit_once('\n') {
    Some((_, last)) => str_width(last).saturating_sub(indent),
    None => str_width(text),
  }
}

/// `let pattern: Type =`, everything before the initializer: the pattern laid out as the
/// standard formatter lays it out (a struct pattern past `struct_lit_width` one field per
/// line), the type on the pattern's last line; `None` when it does not fit, the type is
/// flat for now.
fn let_head(node: &SyntaxNode, expression: &SyntaxNode, shape: Shape) -> Option<String> {
  let pattern = node.children().into_iter().next()?;
  let flat_head = flat_before(node, expression)?;
  let flat_pattern = flat(&pattern);
  // Only a plain `let`: `super let` and the like are left as written.
  let after = flat_head
    .strip_prefix("let ")?
    .strip_prefix(&flat_pattern)?;
  // 4 = `let `, 1 = `;`
  let pattern_text = rewrite(&pattern, shape.offset_left(4)?.sub_width(1)?)?;
  let head = format!("let {}{}", pattern_text, after);
  (last_line_used(&head, shape.indent) <= shape.width).then_some(head)
}

/// The standard formatter moves a right-hand side to the next line when it fits on one line
/// there but not on the first, when that saves more than one line, or when its first line
/// would otherwise end by opening a delimiter that the next line's does not.
fn prefer_next_line(same: &str, next: &str) -> bool {
  let lines = |text: &str| text.matches('\n').count();
  let opens = |text: &str, c: char| text.lines().next().is_some_and(|l| l.ends_with(c));
  lines(same) > 0
    && (lines(next) == 0
      || lines(same) > lines(next) + 1
      || ['(', '{', '[']
        .iter()
        .any(|&c| opens(same, c) && !opens(next, c)))
}

/// Whether `text` holds the statement's tokens, commas aside (a list laid out vertically
/// gains a trailing comma, one laid out on a line loses it).
fn same_tokens(node: &SyntaxNode, text: &str) -> bool {
  let theirs = node
    .descendants_with_tokens()
    .into_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() && t.kind() != K::Comma => {
        Some(t.text().to_string())
      }
      _ => None,
    });
  // Inside a function body, where the statement stands, so that tokens split as they do
  // there (`>>` closing two generic lists).
  let parsed = parse(&format!("fn f() {{\n{}\n}}", text), Edition::E2021).syntax_node();
  let ours = parsed
    .descendants_with_tokens()
    .into_iter()
    .skip_while(|e| e.kind() != K::LBrace)
    .skip(1)
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() && t.kind() != K::Comma => {
        Some(t.text().to_string())
      }
      _ => None,
    });
  // The function's closing brace ends what was parsed.
  let mut ours: Vec<String> = ours.collect();
  ours.pop();
  theirs.eq(ours)
}

/// Whether the statement is one this module lays out.
fn takes(node: &SyntaxNode, config: &FormatConfig) -> bool {
  // Indentation written with tabs is not laid out yet.
  if config.hard_tabs || config.tab_spaces == 0 {
    return false;
  }
  if !matches!(node.kind(), K::ExprStmt | K::LetStmt) {
    return false;
  }
  let Some(expression) = expression_of(node) else {
    return false;
  };
  let laid_out = matches!(
    expression.kind(),
    K::CallExpr | K::MethodCallExpr | K::RecordExpr | K::ArrayExpr | K::TupleExpr
  );
  laid_out
    && node.descendants_with_tokens().iter().all(|e| match e {
      SyntaxElement::Token(t) => {
        !matches!(
          t.kind(),
          K::LineComment
            | K::BlockComment
            | K::OuterLineDoc
            | K::InnerLineDoc
            | K::OuterBlockDoc
            | K::InnerBlockDoc
        ) && !(t.kind().is_literal() && t.text().contains('\n'))
      }
      SyntaxElement::Node(n) => !matches!(
        n.kind(),
        K::BlockExpr
          | K::MatchArmList
          | K::MacroExpr
          | K::MacroCall
          | K::Attr
          | K::LetElse
          | K::ClosureExpr
      ),
    })
}

#[cfg(test)]
mod tests;
