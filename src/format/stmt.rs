//! Statements laid out anew with the width-budget code ([`super::expr`]): their lines broken
//! and joined as the Rust style lays them out, rather than kept where they were written.
//!
//! Only the statements the expression code covers are taken: a `let` with an initializer,
//! or an expression with a `;`, whose expression is a call, a method chain, a struct literal,
//! an array, a tuple or a macro call in parentheses or brackets; any other statement is put
//! on one line when it fits there. A statement holding a comment, a block, a macro in braces
//! or a string spanning lines is left to the whole-file rules, which keep its lines as
//! written; its attributes, and comments before it, are laid out on their own.

use SyntaxKind as K;

use super::expr::{flat, flat_tokens, is_chain, rewrite};
use super::items::header_start;
use super::shape::{Limits, Shape, Spacing};
use crate::options::format::FormatConfig;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, parse};
use crate::text::unicode::str_width;

/// The statement laid out, starting at column `indent` on a line of its own, its later lines
/// indented in full; `None` when it is not one this module takes, or does not fit.
pub fn rewrite_statement(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<String> {
  if !clean(node, config) {
    return None;
  }
  if !takes(node) {
    return on_one_line(node, indent, config);
  }
  // Types are printed flat for now: a statement whose generic arguments would have to
  // break is left as written.
  let generics = node
    .descendants()
    .iter()
    .any(|d| d.kind() == K::GenericArgList);
  if generics && indent + str_width(&flat_code(node, Spacing::of(config))) > config.max_width {
    return None;
  }
  let text = laid_out(node, indent, config)?;
  same_tokens(node, &text).then_some(text)
}

/// Whether every call, chain, struct literal, array, tuple and pattern in `node` stays on
/// one line by its own width, laid out from column `indent`.
pub(super) fn nested_fit(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> bool {
  let Some(shape) = statement_shape(indent, config) else {
    return false;
  };
  // The outermost laid-out expressions inside, each checked on its own.
  let laid = |n: &SyntaxNode| {
    is_chain(n)
      || matches!(
        n.kind(),
        K::CallExpr
          | K::MacroExpr
          | K::RecordExpr
          | K::ArrayExpr
          | K::TupleExpr
          | K::RecordPat
          | K::TupleStructPat
          | K::TuplePat
          | K::SlicePat
      )
  };
  node
    .descendants()
    .iter()
    .filter(|n| laid(n) && !n.ancestors().iter().skip(1).any(laid))
    .all(|n| rewrite(n, shape).is_some_and(|t| !t.contains('\n')))
}

/// Whether `text` holds the statement's code tokens, commas aside (a list laid out
/// vertically gains a trailing comma, one laid out on a line loses it).
pub(super) fn same_tokens(node: &SyntaxNode, text: &str) -> bool {
  let theirs = code_tokens(node)
    .into_iter()
    .filter(|t| t.kind() != K::Comma)
    .map(|t| t.text().to_string());
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

/// Whether a macro call's arguments are in braces: they are left as written, and so is a
/// statement holding them.
fn braced(call: &SyntaxNode) -> bool {
  call
    .children()
    .iter()
    .find(|c| c.kind() == K::TokenTree)
    .and_then(|tree| {
      tree
        .children_with_tokens()
        .into_iter()
        .find(|e| !e.kind().is_trivia())
    })
    .is_none_or(|first| first.kind() == K::LBrace)
}

/// Whether the statement holds nothing laying it out would lose or misplace: a comment, a
/// block, a macro, an attribute, a string spanning lines.
fn clean(node: &SyntaxNode, config: &FormatConfig) -> bool {
  // Indentation written with tabs is not laid out yet.
  if config.hard_tabs || config.tab_spaces == 0 {
    return false;
  }
  // `x.0 .0`: the standard formatter keeps the space a chain layout would drop.
  let tokens: Vec<SyntaxElement> = node
    .descendants_with_tokens()
    .into_iter()
    .filter(|e| matches!(e, SyntaxElement::Token(_)))
    .collect();
  let spaced_tuple_field = tokens.windows(4).any(|w| {
    w[0].kind() == K::IntNumber
      && w[1].kind() == K::Whitespace
      && w[2].kind() == K::Dot
      && w[3].kind() == K::IntNumber
  });
  // From after its attributes and the comments before it, which are laid out on their own.
  let Some(start) = header_start(node) else {
    return false;
  };
  matches!(node.kind(), K::ExprStmt | K::LetStmt)
    && !spaced_tuple_field
    && node
      .children_with_tokens()
      .iter()
      .filter(|e| e.text_range().start as usize >= start)
      .flat_map(|e| match e {
        SyntaxElement::Node(n) => n.descendants_with_tokens(),
        token => vec![token.clone()],
      })
      .all(|e| match &e {
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
        SyntaxElement::Node(n) => {
          !matches!(
            n.kind(),
            K::BlockExpr | K::MatchArmList | K::Attr | K::LetElse
          ) && !(n.kind() == K::MacroCall && braced(n))
        }
      })
}

/// The statement's code tokens, from after its attributes and the comments before it, which
/// are laid out on their own.
fn code_tokens(node: &SyntaxNode) -> Vec<SyntaxToken> {
  let start = header_start(node).unwrap_or(node.text_range().start as usize);
  node
    .descendants_with_tokens()
    .into_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t)
        if !t.kind().is_trivia() && t.text_range().start as usize >= start =>
      {
        Some(t)
      }
      _ => None,
    })
    .collect()
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
fn flat_before(node: &SyntaxNode, expression: &SyntaxNode, spacing: Spacing) -> Option<String> {
  let whole = flat_code(node, spacing);
  let rest = format!("{};", flat(expression, spacing));
  whole
    .strip_suffix(&rest)
    .map(|head| head.trim_end().to_string())
}

/// The statement's code on one line.
fn flat_code(node: &SyntaxNode, spacing: Spacing) -> String {
  flat_tokens(&code_tokens(node), spacing)
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
    spacing: Spacing::of(config),
    attribute: false,
    comma_as_written: false,
    style_2024: config.style_edition >= 2024,
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
  let pattern = node.children().into_iter().find(|c| c.kind() != K::Attr)?;
  let flat_head = flat_before(node, expression, shape.limits.spacing)?;
  let flat_pattern = flat(&pattern, shape.limits.spacing);
  // Only a plain `let`: `super let` and the like are left as written.
  let after = flat_head
    .strip_prefix("let ")?
    .strip_prefix(&flat_pattern)?;
  // 4 = `let `, 1 = `;`
  let pattern_text = rewrite(&pattern, shape.offset_left(4)?.sub_width(1)?)?;
  let head = format!("let {}{}", pattern_text, after);
  (last_line_used(&head, shape.indent) <= shape.width).then_some(head)
}

/// A statement the layout code does not take whole, on one line when it fits there and
/// every call, chain, struct literal, array and tuple in it stays on one line by its own
/// width; else `None`.
fn on_one_line(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<String> {
  if !matches!(node.kind(), K::ExprStmt | K::LetStmt) {
    return None;
  }
  let text = flat_code(node, Spacing::of(config));
  if indent + str_width(&text) > config.max_width {
    return None;
  }
  (nested_fit(node, indent, config) && same_tokens(node, &text)).then_some(text)
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

/// The shape a statement starting at column `indent` is laid out into.
fn statement_shape(indent: usize, config: &FormatConfig) -> Option<Shape> {
  let limits = Limits {
    max_width: config.max_width,
    tab: config.tab_spaces,
    fn_call: config.fn_call_width,
    chain: config.chain_width,
    struct_lit: config.struct_lit_width,
    array: config.array_width,
    short_item: config.short_array_element_width_threshold,
    spacing: Spacing::of(config),
    attribute: false,
    comma_as_written: false,
    style_2024: config.style_edition >= 2024,
  };
  Some(Shape {
    indent,
    offset: 0,
    width: config.max_width.checked_sub(indent)?,
    limits,
  })
}

/// Whether the layout code takes the statement whole: a `let` with an initializer, or an
/// expression statement, whose expression is a call, chain (`?` and `.await` too), struct
/// literal, array or tuple.
fn takes(node: &SyntaxNode) -> bool {
  matches!(node.kind(), K::ExprStmt | K::LetStmt)
    && expression_of(node).is_some_and(|expression| {
      is_chain(&expression)
        || matches!(
          expression.kind(),
          K::CallExpr | K::RecordExpr | K::ArrayExpr | K::TupleExpr | K::MacroExpr
        )
        // `return` with a value.
        || expression.kind() == K::ReturnExpr && !expression.children().is_empty()
    })
}

#[cfg(test)]
mod tests;
