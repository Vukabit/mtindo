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
  // Only one comment, written on the `{`'s line: it stays there, `}` on the next line.
  let significant: Vec<&SyntaxElement> = inside
    .iter()
    .filter(|e| e.kind() != K::Whitespace)
    .collect();
  if let [SyntaxElement::Token(comment)] = significant.as_slice()
    && matches!(comment.kind(), K::LineComment | K::BlockComment)
    && !comment.text().contains('\n')
    && !inside
      .iter()
      .take_while(|e| e.kind() == K::Whitespace)
      .any(|e| matches!(e, SyntaxElement::Token(t) if t.text().contains('\n')))
  {
    return Some(format!(
      "{{ {}\n{}}}",
      comment.text().trim_end(),
      " ".repeat(shape.indent)
    ));
  }
  let inner = shape.block_inside();
  let config = shape.limits.config();
  let lines = lines(block, inner, |node| match node.kind() {
    K::LetStmt => statement(node, inner, &config),
    K::ExprStmt => statement(node, inner, &config).or_else(|| macro_as_written(node)),
    _ if is_tail(node) => statement(node, inner, &config).or_else(|| macro_as_written(node)),
    K::MacroCall => braced_macro(node, inner).or_else(|| item(node, inner, &config)),
    K::Use | K::Const | K::Static | K::TypeAlias | K::ExternCrate => item(node, inner, &config),
    // A function: its header, then its body laid out here; one written on its header's
    // line only when that header is on one line.
    K::Fn => attributed(node, inner, &config, || {
      let body = node.children_iter().find(|c| c.kind() == K::BlockExpr)?;
      let (head, to) = super::items::rewrite_header(node, inner.indent, &config)?;
      if to != body.text_range().start as usize {
        return None;
      }
      let text = block_body(&body, inner)?;
      if head.contains('\n') && text == "{}" {
        return Some(format!("{}{{\n{}}}", head, " ".repeat(inner.indent)));
      }
      Some(format!("{}{}", head, text))
    }),
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
  let text = block_body(&body, shape)?;
  // An empty body keeps its braces apart when its `{` starts a line after a head on one
  // line, or when `{}` would pass the line width.
  let alone = head.text.matches('\n').count() == 1
    && head
      .text
      .rsplit('\n')
      .next()
      .is_some_and(|l| l.trim().is_empty());
  let past = !head.text.contains('\n')
    && shape.indent + shape.offset + str_width(&head.text) + 2 > shape.limits.max_width;
  if text == "{}" && (alone || past) {
    return Some(format!("{}{{\n{}}}", head.text, " ".repeat(shape.indent)));
  }
  Some(format!("{}{}", head.text, text))
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
    if arm.kind() != K::MatchArm {
      return None;
    }
    // Its attributes each on a line of their own; none with a comment among them.
    let attributes: Vec<SyntaxNode> = arm
      .children_iter()
      .filter(|c| c.kind() == K::Attr)
      .collect();
    if !attributes.is_empty()
      && arm.children_with_tokens_iter().any(|e| {
        matches!(e.kind(), K::LineComment | K::BlockComment)
          && e.text_range().start < super::items::header_start(arm).unwrap_or(0) as u32
      })
    {
      return None;
    }
    // `#[rustfmt::skip]` keeps it as written: left to the whole-file pass.
    if attributes
      .iter()
      .filter_map(|a| crate::syntax::ast::Attr::cast(a.clone()))
      .any(|a| a.is_rustfmt_skip())
    {
      return None;
    }
    let mut prefix = String::new();
    for attribute in &attributes {
      prefix.push_str(&super::attrs::rewrite_attr(
        attribute,
        inner.indent,
        &config,
      )?);
      prefix.push('\n');
      prefix.push_str(&" ".repeat(inner.indent));
    }
    let text = match rewrite_arm_in(arm, inner.indent, &config, shape.limits.inside_macro)? {
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
    }?;
    Some(format!("{}{}", prefix, text))
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

/// A statement or item after its attributes, each on a line of their own; none skipped, and
/// no comment among them.
fn attributed(
  node: &SyntaxNode,
  shape: Shape,
  config: &crate::options::format::FormatConfig,
  bare: impl Fn() -> Option<String>,
) -> Option<String> {
  let attributes: Vec<SyntaxNode> = node
    .children_iter()
    .filter(|c| c.kind() == K::Attr)
    .collect();
  if attributes.is_empty() {
    return bare();
  }
  let start = super::items::header_start(node)?;
  let commented = node.children_with_tokens_iter().any(|e| {
    matches!(e.kind(), K::LineComment | K::BlockComment) && (e.text_range().start as usize) < start
  });
  let skipped = attributes
    .iter()
    .filter_map(|a| crate::syntax::ast::Attr::cast(a.clone()))
    .any(|a| a.is_rustfmt_skip());
  if commented || skipped {
    return None;
  }
  let mut prefix = String::new();
  for attribute in &attributes {
    prefix.push_str(&super::attrs::rewrite_attr(
      attribute,
      shape.indent,
      config,
    )?);
    prefix.push('\n');
    prefix.push_str(&" ".repeat(shape.indent));
  }
  Some(format!("{}{}", prefix, bare()?))
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

/// An item without a body standing in a block laid out here: a `use`, a `const`, a
/// `static`, a type alias or a macro call, its header laid out whole.
fn item(
  node: &SyntaxNode,
  shape: Shape,
  config: &crate::options::format::FormatConfig,
) -> Option<String> {
  let end = super::imports::code_end(node);
  // Nothing after its code on its line here, such as a comment.
  if end != node.text_range().end as usize {
    return None;
  }
  attributed(node, shape, config, || match node.kind() {
    K::Use => super::imports::rewrite_use(node, shape.indent, config),
    K::MacroCall => super::items::rewrite_macro_item(node, shape.indent, config),
    _ => {
      let (text, to) = super::items::rewrite_header(node, shape.indent, config)?;
      (to >= end).then_some(text)
    }
  })
}

/// A control expression standing as a statement, on lines, with its `;`.
fn laid_control(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let expression = if node.kind() == K::ExprStmt {
    node.children_iter().find(|c| c.kind() != K::Attr)?
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
  let comments_only = container.kind() == K::BlockExpr
    && container.children_iter().next().is_none()
    && container
      .children_with_tokens_iter()
      .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment));
  // After an arm on its line, a comment is aligned with those after the others, as a
  // list's are (`write_list`): placed once all are laid out. Each arm's line, and each
  // comment's.
  let mut arm_lines: Vec<usize> = Vec::new();
  let mut arm_comments: Vec<(usize, String)> = Vec::new();
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
    // Before an entry or comment: the blank lines kept, none after `{` but in a block of
    // comments alone.
    let blank = |lines: &mut Vec<String>, newlines: usize| {
      if !lines.is_empty() || comments_only {
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
          // to the whole-file pass; in a block not an `if`'s, on a line of its own one
          // level in.
          let last = last_entry > 0 && t.text_range().start >= last_entry;
          let placed = container.kind() == K::BlockExpr
            && container.parent().is_none_or(|p| p.kind() != K::IfExpr);
          if text.contains('\n') || last && newlines > 0 && !placed {
            return None;
          }
          if newlines == 0 && !lines.is_empty() {
            // After an entry, on its line; after an arm, aligned with those around it.
            if container.kind() == K::MatchArmList {
              arm_comments.push((lines.len() - 1, text.to_string()));
              newlines = 0;
              continue;
            }
            let last = lines.last_mut()?;
            last.push(' ');
            if t.kind() == K::LineComment {
              // An entry's text may span lines.
              trailing = Some(str_width(last.rsplit('\n').next().unwrap_or("")));
            }
            last.push_str(text);
          } else if let Some(column) =
            trailing.filter(|_| newlines == 1 && !inner.limits.style_2024)
          {
            // On the line after one ending a line of code, lined up under it however wide;
            // from the 2024 style at the block's indentation.
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
        // An arm follows the comment above it, its pre-comment, with no blank line.
        let after_comment = lines
          .last()
          .is_some_and(|l| l.trim_start().starts_with("//") || l.trim_start().starts_with("/*"));
        if !(container.kind() == K::MatchArmList && after_comment) {
          blank(&mut lines, newlines);
        }
        let text = entry(node)?;
        lines.push(format!("{}{}", pad, text));
        arm_lines.push(lines.len() - 1);
        newlines = 0;
      }
    }
  }
  if !arm_comments.is_empty() {
    let arms: Vec<SyntaxNode> = container
      .children_iter()
      .filter(|c| c.kind() == K::MatchArm)
      .collect();
    if arms.len() != arm_lines.len() {
      return None;
    }
    let comments = super::lists::list_comments(container, &arms)?;
    // An arm spanning lines starts a group of its own.
    let texts: Vec<String> = arm_lines
      .iter()
      .map(|&i| match lines[i].strip_prefix(&pad) {
        Some(text) if !text.contains('\n') => text.to_string(),
        _ => "\n".to_string(),
      })
      .collect();
    let spaces = super::lists::post_comment_spaces(
      &texts,
      &comments,
      inner.indent,
      inner.limits.max_width,
      false,
    );
    for (line, comment) in arm_comments {
      let arm = arm_lines.iter().position(|&i| i == line)?;
      let gap = spaces.get(arm).copied().flatten().unwrap_or(1);
      lines[line].push_str(&" ".repeat(gap));
      lines[line].push_str(&comment);
    }
  }
  Some(lines)
}

/// A macro call in parentheses or brackets standing as a statement, which could not be laid
/// out: as written, its later lines where they were, as the standard formatter keeps a
/// statement it cannot rewrite (`push_rewrite` given nothing). Not one with attributes or
/// comments.
fn macro_as_written(node: &SyntaxNode) -> Option<String> {
  let expression = if node.kind() == K::ExprStmt {
    node.children_iter().next()?
  } else {
    node.clone()
  };
  let call = match expression.kind() {
    K::MacroExpr => expression
      .children_iter()
      .find(|c| c.kind() == K::MacroCall)?,
    _ => return None,
  };
  let tree = call.children_iter().find(|c| c.kind() == K::TokenTree)?;
  if tree.first_token()?.kind() == K::LBrace
    || node.subtree_has(|kind| {
      matches!(kind, K::Attr | K::LineComment | K::BlockComment) || super::items::is_doc(kind)
    })
  {
    return None;
  }
  Some(node.text().to_string())
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
  attributed(node, shape, config, || {
    statement_without_attributes(node, shape, config)
  })
}

/// [`statement`] for what follows its attributes.
fn statement_without_attributes(
  node: &SyntaxNode,
  shape: Shape,
  config: &crate::options::format::FormatConfig,
) -> Option<String> {
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
