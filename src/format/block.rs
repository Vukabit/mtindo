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
  // Laid out from the indentation alone (its insides at `block_inside`, its `}` at the
  // indentation), but for a comment's place, which weighs the width: kept by those, so the
  // layouts tried around a block lay it out once.
  let commented = block
    .children_with_tokens_iter()
    .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment));
  let key = Shape {
    offset: 0,
    align: 0,
    width: if commented { shape.width } else { 0 },
    ..shape
  };
  super::expr::remembered_block(block, key, || block_body_once(block, shape))
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
  // Flat up to its `{` alone.
  let head = super::expr::code_tokens_through(block, |t| t.kind() == K::LBrace);
  let whole = super::expr::flat_tokens(&head, shape.limits.spacing);
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
  // Its `async move` or `unsafe` however little room is left (the standard formatter's
  // budget for an `async` block saturates).
  let after = shape.offset_left(str_width(prefix)).unwrap_or(Shape {
    offset: shape.offset + str_width(prefix),
    width: 0,
    ..shape
  });
  let body = block_body(block, after)?;
  // A plain block is its body, not copied again.
  if prefix.is_empty() {
    return Some(body);
  }
  Some(format!("{}{}", prefix, body))
}

/// A `for`, `while` or `loop`: its head, then its block of lines.
pub fn rewrite_loop(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let config = shape.limits.config();
  let head = rewrite_control_head(node, shape.indent, shape.indent + shape.offset, &config)?;
  let body = node.children_iter().rfind(|c| c.kind() == K::BlockExpr)?;
  // What is left of the line after the head, for a block on it (`{ /* c */ }`).
  let last = head.text.rsplit('\n').next().unwrap_or("");
  let used = if head.text.contains('\n') {
    str_width(last)
  } else {
    shape.indent + shape.offset + str_width(last)
  };
  let text = block_body(
    &body,
    Shape {
      width: shape.limits.max_width.saturating_sub(used + 1),
      ..shape
    },
  )?;
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
  // `match scrutinee` within the shape's width too; ` {` may go past it (`rewrite_match`).
  if !head.text.contains('\n') && str_width(head.text.trim_end()) > shape.width {
    return None;
  }
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
    // Its attributes and doc comments each on a line of their own, in their order; none
    // with a comment among them, nor a doc comment across lines.
    let attributes: Vec<SyntaxNode> = arm
      .children_iter()
      .filter(|c| c.kind() == K::Attr)
      .collect();
    let code = super::items::header_start(arm).unwrap_or(0) as u32;
    let before: Vec<SyntaxElement> = arm
      .children_with_tokens_iter()
      .filter(|e| e.text_range().start < code)
      .collect();
    let documented = before.iter().any(|e| super::items::is_doc(e.kind()));
    if (!attributes.is_empty() || documented)
      && before
        .iter()
        .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
      || before.iter().any(|e| {
        matches!(e, SyntaxElement::Token(t) if super::items::is_doc(t.kind())
          && t.text().trim_end().contains('\n'))
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
    for element in &before {
      match element {
        SyntaxElement::Node(attribute) if attribute.kind() == K::Attr => {
          prefix.push_str(&super::attrs::rewrite_attr(
            attribute,
            inner.indent,
            &config,
          )?);
        }
        SyntaxElement::Token(doc) if super::items::is_doc(doc.kind()) => {
          prefix.push_str(doc.text().trim_end());
        }
        _ => continue,
      }
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
    // One the standard formatter gives up on is kept as written, as in any block
    // ([`given_up`]); on one line only, so a second run weighs the same text. Not one
    // holding a block, or a closure inside a closure, whose own blocks decide for them:
    // weighed here too, each level of closures nested in closures would weigh all those
    // inside it again.
    _ => rewrite(expression, inner).or_else(|| {
      let nested = expression
        .descendant_nodes_where(|kind| matches!(kind, K::ClosureExpr | K::BlockExpr))
        .any(|n| n.kind() == K::BlockExpr || &n != expression);
      (!nested)
        .then(|| given_up(expression, inner))
        .flatten()
        .filter(|t| !t.contains('\n'))
    })?,
  };
  Some(format!(
    "{{\n{}{}\n{}}}",
    " ".repeat(inner.indent),
    text,
    " ".repeat(shape.indent)
  ))
}

/// A statement in a block the standard formatter gives up on, as written (`push_rewrite`
/// given nothing): see [`gives_up`].
pub(super) fn given_up(node: &SyntaxNode, inner: Shape) -> Option<String> {
  let kept = gives_up(node, inner, true, true).then(|| node.text().to_string().trim().to_string());
  if kept.is_some() {
    super::file::note_kept();
  }
  kept
}

/// Whether the standard formatter gives up on a statement at `inner`'s indentation: its
/// layout with no line width still has a line past it, so that no layout fits, or it holds a
/// string spanning lines. Not one holding a comment or an attribute, nor one in a macro's
/// arguments, which the standard formatter keeps as written whole then. But for `any_string`,
/// not for a raw string spanning lines, nor from style edition 2024 one whose lines but the
/// last end with `\`, which the standard formatter keeps whatever their width; and but for
/// `string_lines`, those strings' later lines are not measured. What stands in a block
/// inside it does not count: the standard formatter lays a block's statements out one by
/// one, keeping one it gives up on as written there.
pub(super) fn gives_up(
  node: &SyntaxNode,
  inner: Shape,
  any_string: bool,
  string_lines: bool,
) -> bool {
  let commented =
    |n: &SyntaxNode| n.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment));
  super::expr::remembered_gives_up(node, inner, (any_string, string_lines), || {
    match hollowed(node) {
      None => gives_up_whole(node, inner, any_string, string_lines),
      // Comments only inside its blocks, which the standard formatter lays out apart: the
      // rest decides (a closure's head too wide gives up the statement whatever its body
      // holds).
      Some(bare) => {
        (gives_up_whole(node, inner, any_string, string_lines)
          || commented(node) && !commented(&bare))
          && gives_up_whole(&bare, inner, any_string, string_lines)
      }
    }
  })
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
  // A macro call standing as a statement (with a `;` or braces; else it is the block's
  // tail) and an item keep a blank line written after their attributes, which the standard
  // formatter visits apart from them.
  let macro_statement = node.kind() == K::ExprStmt
    && node
      .children_iter()
      .find(|c| c.kind() != K::Attr)
      .filter(|e| e.kind() == K::MacroExpr)
      .is_some_and(|call| {
        node
          .children_with_tokens_iter()
          .any(|e| e.kind() == K::Semi)
          || call
            .descendant_nodes_where(|kind| kind == K::TokenTree)
            .next()
            .and_then(|tree| tree.first_token())
            .is_some_and(|t| t.kind() == K::LBrace)
      });
  let item = super::items::is_item(node.kind()) && !matches!(node.kind(), K::Use | K::ExternCrate);
  let blank = (macro_statement || item)
    && config.blank_lines_upper_bound > 0
    && node.children_with_tokens_iter().any(|e| {
      matches!(&e, SyntaxElement::Token(t) if t.kind() == K::Whitespace
        && t.text_range().end as usize == start
        && t.text().matches('\n').count() > 1)
    });
  let mut prefix = String::new();
  for (i, attribute) in attributes.iter().enumerate() {
    prefix.push_str(&super::attrs::rewrite_attr(
      attribute,
      shape.indent,
      config,
    )?);
    prefix.push('\n');
    if blank && i + 1 == attributes.len() {
      prefix.push('\n');
    }
    prefix.push_str(&" ".repeat(shape.indent));
  }
  Some(format!("{}{}", prefix, bare()?))
}

fn block_body_once(block: &SyntaxNode, shape: Shape) -> Option<String> {
  // Only a comment on one line, not a line comment: on the braces' line (`{ /* c */ }`)
  // where it fits, wherever it was written (the standard formatter's `rewrite_empty_block`);
  // but a function's body, which the standard formatter visits, closes on the next line.
  let inside: Vec<SyntaxElement> = block
    .children_with_tokens_iter()
    .skip_while(|e| e.kind() != K::LBrace)
    .skip(1)
    .filter(|e| e.kind() != K::RBrace)
    .collect();
  if let [SyntaxElement::Token(comment)] = inside
    .iter()
    .filter(|e| e.kind() != K::Whitespace)
    .collect::<Vec<_>>()
    .as_slice()
    && comment.kind() == K::BlockComment
    && block.parent().is_none_or(|p| p.kind() != K::Fn)
    && !comment.text().contains('\n')
    && str_width(comment.text()) + 4 <= shape.width
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
  // Only comments, the first written on the `{`'s line: it stays there, the rest on lines
  // of their own.
  let on_brace_line = match significant.first() {
    Some(SyntaxElement::Token(first))
      if (first.kind() == K::LineComment
        || first.kind() == K::BlockComment && !first.text().contains('\n'))
        && significant
          .iter()
          .all(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
        && !inside
          .iter()
          .take_while(|e| e.kind() == K::Whitespace)
          .any(|e| matches!(e, SyntaxElement::Token(t) if t.text().contains('\n'))) =>
    {
      Some(first.text().trim_end().to_string())
    }
    _ => None,
  };
  let config = shape.limits.config();
  let lines = lines(block, inner, |node| match node.kind() {
    K::LetStmt => statement(node, inner, &config).or_else(|| given_up(node, inner)),
    K::ExprStmt => statement(node, inner, &config)
      .or_else(|| macro_as_written(node))
      .or_else(|| given_up(node, inner)),
    _ if is_tail(node) => statement(node, inner, &config)
      .or_else(|| macro_as_written(node))
      // Not a block or control expression, whose head and blocks the standard formatter
      // lays out apart.
      .or_else(|| {
        (!matches!(
          node.kind(),
          K::BlockExpr | K::IfExpr | K::MatchExpr | K::WhileExpr | K::ForExpr | K::LoopExpr
        ))
        .then(|| given_up(node, inner))
        .flatten()
      }),
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
    // Another item with nothing in its body (`struct S {}`, `trait T {}`) or none
    // (`struct S;`), its header on one line.
    K::Struct | K::Enum | K::Union | K::Trait | K::Impl | K::Module => {
      attributed(node, inner, &config, || {
        let (head, to) = super::items::rewrite_header(node, inner.indent, &config)?;
        if head.contains('\n') {
          return None;
        }
        let body = node.children_iter().find(|c| {
          matches!(
            c.kind(),
            K::RecordFieldList | K::VariantList | K::AssocItemList | K::ItemList
          )
        });
        match body {
          Some(body)
            if to == body.text_range().start as usize
              && body
                .children_with_tokens_iter()
                .all(|e| matches!(e.kind(), K::LBrace | K::RBrace | K::Whitespace)) =>
          {
            Some(format!("{}{{}}", head))
          }
          None if to == node.text_range().end as usize => Some(head),
          _ => None,
        }
      })
    }
    _ => None,
  })?;
  if let Some(first) = on_brace_line
    && lines.first().is_some_and(|line| line.trim() == first)
  {
    // Before style edition 2024 the comments right under it (up to a blank line) stand a
    // column further in.
    let mut lines = lines;
    if !shape.limits.style_2024 {
      for line in lines.iter_mut().skip(1) {
        if line.trim().is_empty() {
          break;
        }
        line.insert(0, ' ');
      }
    }
    let mut text = close(lines, shape);
    let end = text[2..].find('\n').map_or(text.len(), |i| i + 2);
    text.replace_range(1..end, &format!(" {}", first));
    return Some(text);
  }
  Some(close(lines, shape))
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
  if let Some(body) = super::macros::lazy_static_body(node, shape.indent, &shape.limits.config()) {
    return Some(format!("{}! {}", name, body));
  }
  let snippet = tree.text().to_string();
  if snippet.lines().any(|l| l.contains('"')) && snippet.contains('\n') {
    return None;
  }
  let body = super::file::preserve_layout(&snippet, shape.indent, &shape.limits.config());
  Some(format!("{}! {}", name, body))
}

/// The lines of a block, closed: `{}` when there are none. Written into one string made to
/// size: a block's lines hold all the blocks inside it, copied again at each level.
fn close(lines: Vec<String>, shape: Shape) -> String {
  if lines.is_empty() {
    return "{}".to_string();
  }
  let size = lines.iter().map(|line| line.len() + 1).sum::<usize>() + shape.indent + 3;
  let mut text = String::with_capacity(size);
  text.push('{');
  for line in &lines {
    text.push('\n');
    text.push_str(line);
  }
  text.push('\n');
  text.extend(std::iter::repeat_n(' ', shape.indent));
  text.push('}');
  text
}

/// [`gives_up`], for all the statement holds.
fn gives_up_whole(node: &SyntaxNode, inner: Shape, any_string: bool, string_lines: bool) -> bool {
  if inner.limits.inside_macro
    || node.subtree_has(|kind| {
      matches!(kind, K::Attr | K::LineComment | K::BlockComment) || super::items::is_doc(kind)
    })
  {
    return false;
  }
  let mut wide = inner.limits.config();
  wide.max_width = super::shape::UNLIMITED;
  let unlimited = Shape {
    width: wide.max_width - inner.indent,
    limits: super::shape::Limits::of(&wide),
    ..inner
  };
  // A statement, or an expression standing alone in a block made for it.
  let lay = |shape: Shape, config: &crate::options::format::FormatConfig| {
    statement(node, shape, config).or_else(|| rewrite(node, shape))
  };
  let Some(laid) = lay(unlimited, &wide) else {
    return false;
  };
  let max = inner.limits.max_width;
  // The lines inside a string kept whatever their width do not count.
  let kept = |kind: SyntaxKind, text: &str| {
    matches!(kind, K::RawStr | K::RawByteStr | K::RawCStr)
      || inner.limits.style_2024
        && kind == K::Str
        && text.lines().rev().skip(1).all(|line| line.ends_with('\\'))
  };
  let mut laid = laid;
  if !string_lines {
    for (kind, text, _) in node.token_texts() {
      if text.contains('\n') && kind.is_literal() && kept(kind, text) {
        laid = laid.replacen(text, text.lines().next().unwrap_or(""), 1);
      }
    }
  }
  let past_width = |text: &str| {
    text
      .split('\n')
      .enumerate()
      .any(|(i, line)| str_width(line) + if i == 0 { inner.indent } else { 0 } > max)
  };
  // Not one inside a macro call in braces, whose insides are kept as written.
  let braced: Vec<(u32, u32)> = node
    .descendant_nodes_where(|kind| kind == K::TokenTree)
    .filter(|tree| {
      tree.parent().is_some_and(|p| p.kind() == K::MacroCall)
        && tree.first_token().is_some_and(|t| t.kind() == K::LBrace)
    })
    .map(|tree| (tree.text_range().start, tree.text_range().end))
    .collect();
  let spanning = node.token_texts().any(|(kind, text, range)| {
    kind.is_literal()
      && text.contains('\n')
      && (any_string || !kept(kind, text))
      && !braced
        .iter()
        .any(|&(from, to)| from <= range.start && range.end <= to)
  });
  if spanning {
    return true;
  }
  // A closure whose `|parameters|` leave its body too little room, however short the body
  // (`rewrite_closure_fn_decl` fails).
  if node.kind() == K::ClosureExpr && super::expr::closure_prefix(node, inner).is_none() {
    return true;
  }
  // A line past the width with no width to keep to, unless the layout within the width
  // fits (what breaks only for want of room: parameters, generic arguments).
  past_width(&laid) && lay(inner, &inner.limits.config()).is_none_or(|text| past_width(&text))
}

/// The statement again with the insides of each outermost block in it taken out (`{}`),
/// parsed apart; `None` when it holds no such block. Not a block in a method chain, whose
/// text the standard formatter checks whole (`wrap_str` of the chain).
fn hollowed(node: &SyntaxNode) -> Option<SyntaxNode> {
  let blocks: Vec<SyntaxNode> = node
    .descendant_nodes_where(|kind| kind == K::BlockExpr)
    .filter(|b| {
      let between: Vec<SyntaxNode> = b
        .ancestors_iter()
        .skip(1)
        .take_while(|a| a != node)
        .collect();
      b != node
        && !between
          .iter()
          .any(|a| matches!(a.kind(), K::BlockExpr | K::TokenTree))
        // Not an arm's block holding one expression, which the standard formatter flattens
        // into the arm (`flatten_arm_body`): laid out with the `match`.
        && !(b.parent().is_some_and(|p| p.kind() == K::MatchArm)
          && super::expr::lone_expression(b).is_some())
        // The statement itself a chain too.
        && !between.iter().chain([node]).any(|a| {
          matches!(
            a.kind(),
            K::MethodCallExpr | K::FieldExpr | K::TryExpr | K::AwaitExpr | K::UseExpr
          )
        })
        && b.children_with_tokens_iter().any(|e| {
          !e.kind().is_trivia()
            && !matches!(
              e.kind(),
              K::LBrace | K::RBrace | K::Label | K::UnsafeKw | K::AsyncKw | K::ConstKw | K::MoveKw
            )
        })
    })
    .collect();
  if blocks.is_empty() {
    return None;
  }
  let text = node.text().to_string();
  let base = node.text_range().start as usize;
  let mut source = String::new();
  let mut from = 0;
  for block in &blocks {
    let brace = |kind: SyntaxKind| {
      block
        .children_with_tokens_iter()
        .find(|e| e.kind() == kind)
        .map(|e| e.text_range())
    };
    let (open, close) = (brace(K::LBrace)?, brace(K::RBrace)?);
    source.push_str(&text[from..open.end as usize - base]);
    from = close.start as usize - base;
  }
  source.push_str(&text[from..]);
  let wrapped = format!("fn __mto() {{\n{}\n}}", source);
  let parsed = crate::syntax::parse(&wrapped, crate::syntax::Edition::E2021);
  if !parsed.errors().is_empty() {
    return None;
  }
  let block = parsed
    .syntax_node()
    .descendant_nodes_where(|kind| kind == K::BlockExpr)
    .next()?;
  block.children_iter().find(|c| c.kind() == node.kind())
}

/// An item without a body standing in a block laid out here: a `use`, a `const`, a
/// `static`, a type alias or a macro call, its header laid out whole.
fn item(
  node: &SyntaxNode,
  shape: Shape,
  config: &crate::options::format::FormatConfig,
) -> Option<String> {
  let end = super::imports::code_end(node);
  // After its code on its line, only a line comment, one space after it (one it carries).
  let trailing = if end != node.text_range().end as usize {
    let rest: Vec<SyntaxElement> = node
      .children_with_tokens_iter()
      .filter(|e| e.text_range().start as usize >= end)
      .collect();
    match rest.as_slice() {
      [SyntaxElement::Token(space), SyntaxElement::Token(comment)]
        if space.kind() == K::Whitespace
          && !space.text().contains('\n')
          && comment.kind() == K::LineComment =>
      {
        format!(" {}", comment.text().trim_end())
      }
      _ => return None,
    }
  } else {
    String::new()
  };
  let laid = attributed(node, shape, config, || match node.kind() {
    K::Use => super::imports::rewrite_use(node, shape.indent, config),
    K::MacroCall => super::items::rewrite_macro_item(node, shape.indent, config),
    _ => {
      let (text, to) = super::items::rewrite_header(node, shape.indent, config)?;
      (to >= end).then_some(text)
    }
  })?;
  Some(format!("{}{}", laid, trailing))
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
          // Line comments on the lines after one ending the last arm's line: with it, the
          // arm's post-comment, lined up under it.
          if last
            && newlines > 0
            && t.kind() == K::LineComment
            && container.kind() == K::MatchArmList
            && let Some((line, comment)) = arm_comments.last_mut()
            && *line + 1 == lines.len()
            && comment.starts_with("//")
          {
            // A blank line among them stays.
            comment.push_str(if newlines > 1 { "\n\n" } else { "\n" });
            comment.push_str(text);
            newlines = 0;
            continue;
          }
          if last && newlines > 0 && !placed {
            return None;
          }
          // One across lines on lines of its own, its later lines placed as the whole-file
          // pass places them.
          if text.contains('\n') {
            if newlines == 0 && !lines.is_empty() {
              return None;
            }
            let config = inner.limits.config();
            blank(&mut lines, newlines);
            lines.push(format!(
              "{}{}",
              pad,
              super::file::reindent_block_comment(
                text,
                inner.indent,
                config.tab_spaces,
                &config,
                false
              )
            ));
            trailing = None;
            newlines = 0;
            continue;
          }
          if newlines == 0 && !lines.is_empty() {
            // After an entry, on its line; after an arm, aligned with those around it.
            if container.kind() == K::MatchArmList {
              arm_comments.push((lines.len() - 1, text.to_string()));
              newlines = 0;
              continue;
            }
            // Before the 2024 style, one after a block's last statement goes to the next line
            // when it would start past `comment_width` (80, an unstable setting at its
            // default).
            let line = lines.last()?;
            if last
              && container.kind() == K::BlockExpr
              && !inner.limits.style_2024
              && line.ends_with(';')
              && str_width(line.rsplit('\n').next().unwrap_or(line)) + 1 > 80
            {
              lines.push(format!("{}{}", pad, text));
              trailing = None;
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
        // One ending in the line comment it carries: the comments under it line up there.
        if let Some(comment) = node.last_token().filter(|t| t.kind() == K::LineComment)
          && let Some(last) = lines.last()
          && let Some(at) = last.rfind(comment.text().trim_end())
        {
          let before = &last[..at];
          trailing = Some(str_width(before.rsplit('\n').next().unwrap_or(before)));
        }
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
    // An arm spanning lines counts all its text, as the standard formatter measures it.
    let texts: Vec<String> = arm_lines
      .iter()
      .map(|&i| lines[i].strip_prefix(&pad).unwrap_or(&lines[i]).to_string())
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
      let column = str_width(lines[line].rsplit('\n').next().unwrap_or(""));
      let pad = " ".repeat(column);
      let comment: Vec<String> = comment
        .split('\n')
        .enumerate()
        .map(|(i, l)| {
          if i == 0 || l.is_empty() {
            l.to_string()
          } else {
            format!("{}{}", pad, l)
          }
        })
        .collect();
      lines[line].push_str(&comment.join("\n"));
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
