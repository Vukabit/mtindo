//! Blocks laid out inside an expression: a closure's body, an `async` block, a `match`, a
//! loop, an `if`'s branches, a `let`'s `else`. The standard formatter writes their insides
//! with the same visitor as a function's body, so each statement is laid out here as the
//! whole-file pass lays it out at that indentation ([`super::stmt`], [`super::arms`]), its
//! blank lines kept within bounds and the comments among them on their lines. Anything not
//! followed yet inside (an item, an attribute, a comment inside a statement) makes the block
//! `None`, so that what holds it is left to the whole-file pass.

use SyntaxKind as K;

use super::arms::{Arm, rewrite_arm_in};
use super::expr::{ends_with_semi, flat, joins, lone_expression, rewrite};
use super::shape::Shape;
use super::stmt::{is_tail, rewrite_control_head, rewrite_statement};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};
use crate::text::unicode::str_width;

/// A block's braces and what is between them, laid out as [`rewrite_lines`] does.
pub fn block_body(block: &SyntaxNode, shape: Shape) -> Option<String> {
  // Only a comment written on the braces' line stays there (`{ /* c */ }`), the gap kept as
  // written.
  let inside: Vec<SyntaxElement> = block
    .children_with_tokens_iter()
    .skip_while(|e| e.kind() != K::LBrace)
    .skip(1)
    .filter(|e| e.kind() != K::RBrace)
    .collect();
  if let [
    SyntaxElement::Token(before),
    SyntaxElement::Token(comment),
    SyntaxElement::Token(after),
  ] = inside.as_slice()
    && comment.kind() == K::BlockComment
    && before.kind() == K::Whitespace
    && after.kind() == K::Whitespace
    && !before.text().contains('\n')
    && !after.text().contains('\n')
    && !comment.text().contains('\n')
  {
    return Some(format!("{{ {} }}", comment.text()));
  }
  let inner = shape.block_inside();
  let config = shape.limits.config();
  let lines = lines(block, inner, |node| match node.kind() {
    K::LetStmt | K::ExprStmt => statement(node, inner, &config),
    _ if is_tail(node) => statement(node, inner, &config),
    K::MacroCall => braced_macro(node, inner),
    _ => None,
  })?;
  Some(close(lines, shape))
}

/// A block's statements for a `let`'s `else`: on one line (`{ return }`) when `one_line`
/// and it is a block of one expression that fits in `width`, else on lines.
pub fn else_block(block: &SyntaxNode, shape: Shape, one_line: Option<usize>) -> Option<String> {
  if let Some(width) = one_line
    && let Some(expression) = lone_expression(block)
    && let Some(text) = shape
      .offset_left(2)
      .and_then(|s| rewrite(&expression, s))
      .filter(|t| !t.contains('\n'))
  {
    let text = format!("{{ {} }}", text);
    if text.len() <= width {
      return Some(text);
    }
  }
  block_body(block, shape)
}

/// A block of lines in `shape`: its prefix (`unsafe `, `async move `, a label), `{`, each
/// statement one level in on lines of its own, and `}` at the shape's indentation; `{}` when
/// it is empty.
pub fn rewrite_lines(block: &SyntaxNode, shape: Shape) -> Option<String> {
  // Nothing before the `{` that laying it out flat would lose.
  if block
    .children_with_tokens_iter()
    .take_while(|e| e.kind() != K::LBrace)
    .any(|e| matches!(e.kind(), K::Attr | K::LineComment | K::BlockComment))
  {
    return None;
  }
  let whole = flat(block, shape.limits.spacing);
  let prefix = &whole[..whole.find('{')?];
  // Only a comment on one line, not a line comment: on the block's line (`{ /* c */ }`),
  // as an expression's block is (a control expression's is not).
  let inside: Vec<SyntaxElement> = block
    .children_with_tokens_iter()
    .skip_while(|e| e.kind() != K::LBrace)
    .skip(1)
    .filter(|e| !matches!(e.kind(), K::Whitespace | K::RBrace))
    .collect();
  if let [SyntaxElement::Token(comment)] = inside.as_slice()
    && comment.kind() == K::BlockComment
    && !comment.text().contains('\n')
    && comment.text().len() + 4 <= shape.width
  {
    return Some(format!("{}{{ {} }}", prefix, comment.text()));
  }
  let body = block_body(block, shape.offset_left(str_width(prefix))?)?;
  Some(format!("{}{}", prefix, body))
}

/// A `for`, `while` or `loop`: its head, then its block of lines.
pub fn rewrite_loop(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let config = shape.limits.config();
  let head = rewrite_control_head(node, shape.indent, shape.indent + shape.offset, &config)?;
  let body = node.children_iter().rfind(|c| c.kind() == K::BlockExpr)?;
  Some(format!("{}{}", head.text, block_body(&body, shape)?))
}

/// A `match`: its head as a control expression's, then each arm on lines of its own one
/// level in (at the `match`'s own indentation without `match_arm_indent`), and `}`.
pub fn rewrite_match(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let config = shape.limits.config();
  let head = rewrite_control_head(node, shape.indent, shape.indent + shape.offset, &config)?;
  let arms = node.children_iter().find(|c| c.kind() == K::MatchArmList)?;
  if arms.children_iter().any(|c| c.kind() == K::Attr) {
    return None;
  }
  let inner = if shape.limits.match_arm_indent {
    shape.block_indent(shape.limits.tab)
  } else {
    shape.block_indent(0)
  };
  let lines = lines(&arms, inner, |arm| {
    if arm.kind() != K::MatchArm || arm.children_iter().any(|c| c.kind() == K::Attr) {
      return None;
    }
    match rewrite_arm_in(arm, inner.indent, &config, shape.limits.inside_macro)? {
      Arm::Whole(text) => Some(text),
      Arm::Head(head) => {
        let body = arm
          .children_iter()
          .next_back()
          .filter(|b| b.text_range().end as usize > head.open)?;
        let rest = match body.kind() {
          K::BlockExpr => block_body(&body, inner)?,
          K::MatchExpr => match_body(&body, inner)?,
          K::LoopExpr => block_body(&body.children_iter().next_back()?, inner)?,
          _ => return None,
        };
        let comma = if head.keep_comma && !head.tail.is_empty() {
          ","
        } else {
          ""
        };
        Some(format!("{}{}{}", head.text, rest, comma))
      }
    }
  })?;
  Some(format!("{}{}", head.text, close(lines, shape)))
}

/// The expression in a block of its own: `{`, the expression one level in, `}` (the
/// standard formatter's `rewrite_closure_with_block`).
pub fn wrapped(expression: &SyntaxNode, shape: Shape) -> Option<String> {
  let inner = shape.block_inside();
  // As a block's tail: an `if` on lines.
  // From style edition 2024 the tail is an expression, an `if` on one line when it fits.
  let text = match expression.kind() {
    K::IfExpr => inner
      .limits
      .style_2024
      .then(|| super::expr::if_single_line(expression, inner))
      .flatten()
      .or_else(|| super::expr::rewrite_if_lines(expression, inner, false))?,
    _ => rewrite(expression, inner)?,
  };
  Some(format!(
    "{{\n{}{}\n{}}}",
    " ".repeat(inner.indent),
    text,
    " ".repeat(shape.indent)
  ))
}

/// A braced macro call standing as a statement: its name, a space and its arguments, their
/// lines kept as written but re-indented from the call's.
fn braced_macro(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let tree = node.children_iter().find(|c| c.kind() == K::TokenTree)?;
  if tree.first_token()?.kind() != K::LBrace {
    return None;
  }
  let path = node.children_iter().find(|c| c.kind() == K::Path)?;
  let name = flat(&path, shape.limits.spacing);
  let snippet = tree.text().to_string();
  if snippet.lines().any(|l| l.contains('"')) && snippet.contains('\n') {
    return None;
  }
  let body = super::file::preserve_layout(&snippet, shape.indent, &shape.limits.config());
  Some(format!("{}! {}", name, body))
}

/// The lines of a block, closed: `{}` when there are none.
fn close(lines: Vec<String>, shape: Shape) -> String {
  if lines.is_empty() {
    return "{}".to_string();
  }
  format!("{{\n{}\n{}}}", lines.join("\n"), " ".repeat(shape.indent))
}

/// A control expression standing as a statement, on lines, with its `;`.
fn laid_control(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let expression = if node.kind() == K::ExprStmt {
    node.children_iter().next()?
  } else if node.kind() == K::LetStmt {
    return None;
  } else {
    node.clone()
  };
  if !matches!(
    expression.kind(),
    K::IfExpr | K::MatchExpr | K::ForExpr | K::WhileExpr | K::LoopExpr | K::BlockExpr
  ) || expression.kind() == K::BlockExpr && joins(&expression)
  {
    return None;
  }
  let semi = node
    .children_with_tokens_iter()
    .any(|e| e.kind() == K::Semi);
  let text = match expression.kind() {
    K::IfExpr => super::expr::rewrite_if_lines(&expression, shape, false)?,
    K::BlockExpr => rewrite_lines(&expression, shape)?,
    _ => rewrite(&expression, shape)?,
  };
  Some(if semi { format!("{};", text) } else { text })
}

/// The lines between a container's braces, each entry laid out by `entry` from the inner
/// shape's indentation, the blank lines between them kept within `blank_lines`, and the
/// comments among them on lines of their own or after the entry they follow on its line.
fn lines(
  container: &SyntaxNode,
  inner: Shape,
  entry: impl Fn(&SyntaxNode) -> Option<String>,
) -> Option<Vec<String>> {
  let pad = " ".repeat(inner.indent);
  let (lower, upper) = inner.limits.blank_lines;
  let mut lines: Vec<String> = Vec::new();
  let mut opened = false;
  let mut newlines = 0;
  // The column of a line comment ending the last line of code.
  let mut trailing: Option<usize> = None;
  // Where the last entry ends.
  let last_entry = container
    .children_iter()
    .next_back()
    .map_or(0, |n| n.text_range().end);
  for element in container.children_with_tokens_iter() {
    if !opened {
      if element.kind() == K::LBrace {
        opened = true;
      }
      continue;
    }
    // Before an entry or comment: the blank lines kept, none after `{`.
    let blank = |lines: &mut Vec<String>, newlines: usize| {
      if !lines.is_empty() {
        let keep = newlines.saturating_sub(1).clamp(lower, upper.max(lower));
        lines.extend(std::iter::repeat_n(String::new(), keep));
      }
    };
    match &element {
      SyntaxElement::Token(t) => match t.kind() {
        K::Whitespace => newlines += t.text().matches('\n').count(),
        K::RBrace => break,
        K::LineComment | K::BlockComment => {
          let text = t.text().trim_end();
          // One just before the `}` keeps its place from the container, as written: left
          // to the whole-file pass.
          let last = t.text_range().start >= last_entry;
          if text.contains('\n') || last && newlines > 0 {
            return None;
          }
          if newlines == 0 && !lines.is_empty() {
            // After an entry, on its line; after an arm, aligned with those around it as
            // a list's are: left to the whole-file pass.
            if container.kind() == K::MatchArmList {
              return None;
            }
            let last = lines.last_mut()?;
            last.push(' ');
            if t.kind() == K::LineComment {
              trailing = Some(str_width(last));
            }
            last.push_str(text);
          } else if let Some(column) = trailing
            .filter(|&column| newlines == 1 && column + str_width(text) <= inner.limits.max_width)
            && t.kind() == K::LineComment
          {
            // On the line after one ending a line of code, lined up under it when it fits
            // there.
            lines.push(format!("{}{}", " ".repeat(column), text));
          } else {
            // On a line of its own; one after `{` goes to the next line.
            blank(&mut lines, newlines);
            lines.push(format!("{}{}", pad, text));
            trailing = None;
          }
          newlines = 0;
        }
        _ => return None,
      },
      SyntaxElement::Node(node) => {
        trailing = None;
        blank(&mut lines, newlines);
        let text = entry(node)?;
        lines.push(format!("{}{}", pad, text));
        newlines = 0;
      }
    }
  }
  Some(lines)
}

/// The `match`'s arms in their braces, after its head.
fn match_body(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let text = rewrite_match(node, shape)?;
  Some(text[text.find('{')?..].to_string())
}

/// A statement inside a block laid out from `shape`'s indentation, as the whole-file pass
/// lays it out: whole, or a control expression on lines with its `;`.
fn statement(
  node: &SyntaxNode,
  shape: Shape,
  config: &crate::options::format::FormatConfig,
) -> Option<String> {
  if node.children_iter().any(|c| c.kind() == K::Attr) {
    return None;
  }
  // A block's last `return`, `break` or `continue` gains a `;`.
  let semi = |text: String| {
    if is_tail(node) && ends_with_semi(node, shape.limits) && !text.ends_with(';') {
      format!("{};", text)
    } else {
      text
    }
  };
  if let Some(text) = rewrite_statement(node, shape.indent, config) {
    return Some(semi(text));
  }
  laid_control(node, shape)
}
