//! Statements laid out anew with the width-budget code ([`super::expr`]): their lines broken
//! and joined as the Rust style lays them out, rather than kept where they were written.
//!
//! Only the statements the expression code covers are taken: a `let` with an initializer,
//! an expression with a `;`, or a block's tail (its last expression, with none), whose
//! expression is a call, a method chain, a struct literal, an array, a tuple, a binary
//! expression, a block of one expression or a macro call in parentheses or brackets; any
//! other statement is put on one line when it fits there. A statement holding a comment, a block, a macro in braces
//! or a string spanning lines is left to the whole-file rules, which keep its lines as
//! written; its attributes, and comments before it, are laid out on their own.

use SyntaxKind as K;

use super::expr::{flat, flat_tokens, is_chain, prefer_next_line, rewrite};
use super::items::header_start;
use super::shape::{Limits, Shape, Spacing};
use crate::options::format::FormatConfig;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, parse_tokens};
use crate::text::unicode::str_width;

/// A statement's head laid out by [`rewrite_head`].
pub struct Head {
  /// The head, with the space before `{`.
  pub text: String,
  /// Where the block's `{` starts.
  pub open: usize,
  /// Where each token of the tail after the block's `}` starts (`)`, `?`, `;`, `,`), and its
  /// kind: they follow the `}` on its line.
  pub tail: Vec<(usize, SyntaxKind)>,
}

/// Whether `node` is a block's tail: the expression ending it, with no `;`, laid out as a
/// statement is but for the `;`.
pub fn is_tail(node: &SyntaxNode) -> bool {
  (node.kind().name().ends_with("_EXPR") || node.kind() == K::Literal)
    && node.parent().is_some_and(|p| {
      p.kind() == K::BlockExpr && p.children_iter().next_back().as_ref() == Some(node)
    })
}

/// A statement holding one block (a closure's body, an `async` or `unsafe` block, a block
/// as an argument or a `let`'s value), laid out as an item's header is: its head on one
/// line up to the block's `{`, which the whole-file pass writes with the block's insides.
/// `None` for anything else, left to the rest of the formatter.
pub fn rewrite_head(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<Head> {
  if config.hard_tabs || config.tab_spaces == 0 || !matches!(node.kind(), K::ExprStmt | K::LetStmt)
  {
    return None;
  }
  let start = header_start(node)?;
  // The one outermost block.
  let mut blocks = node
    .descendant_nodes_where(|kind| kind == K::BlockExpr)
    .filter(|b| {
      !b.ancestors_iter()
        .skip(1)
        .take_while(|a| a != node)
        .any(|a| a.kind() == K::BlockExpr)
    });
  let block = blocks.next()?;
  if blocks.next().is_some() {
    return None;
  }
  // The block (or the closure it is the body of) is the `let`'s value, or the last argument
  // of the call the statement is, maybe with `?`. The standard formatter breaks the
  // arguments otherwise: a block deeper in, or after another closure among them.
  let mut value = block.clone();
  if let Some(closure) = block.parent().filter(|p| p.kind() == K::ClosureExpr) {
    value = closure;
  }
  let parent = value.parent()?;
  let mut arguments_from = None;
  let mut method = false;
  match parent.kind() {
    K::LetStmt => {}
    K::ArgList => {
      let arguments: Vec<SyntaxNode> = parent.children_iter().collect();
      if arguments.last() != Some(&value)
        || arguments[..arguments.len() - 1]
          .iter()
          .any(|a| a.subtree_has(|kind| kind == K::ClosureExpr))
      {
        return None;
      }
      let mut call = parent.parent()?;
      if !matches!(call.kind(), K::CallExpr | K::MethodCallExpr) {
        return None;
      }
      method = call.kind() == K::MethodCallExpr;
      while let Some(up) = call.parent().filter(|p| p.kind() == K::TryExpr) {
        call = up;
      }
      if !matches!(call.parent()?.kind(), K::ExprStmt | K::LetStmt) {
        return None;
      }
      arguments_from = Some(parent.text_range().start + 1);
    }
    _ => return None,
  }
  let brace = |kind: SyntaxKind| {
    block
      .child_spans()
      .find(|&(k, _, is_node)| !is_node && k == kind)
      .map(|(_, range, _)| range)
  };
  let (open, close) = (brace(K::LBrace)?, brace(K::RBrace)?);
  // What the standard formatter writes otherwise: an empty block, and a closure's body that
  // is one expression (its braces go when it fits).
  let inside: Vec<SyntaxKind> = block
    .child_spans()
    .filter(|&(k, range, is_node)| {
      is_node && range.start > open.start && range.end <= close.start
        || k.is_trivia() && k != K::Whitespace
    })
    .map(|(k, _, _)| k)
    .collect();
  let statements = inside
    .iter()
    .filter(|k| matches!(k, K::LetStmt | K::ExprStmt) || super::items::is_item(**k))
    .count();
  // A block of one expression and nothing else the standard formatter writes on one line
  // when it fits (`unsafe { f() }`), a closure's without its braces.
  if statements == 0 {
    return None;
  }
  // The head: nothing in it the layout code declines.
  let head: Vec<SyntaxToken> = node
    .descendants_where(|kind| !kind.is_node())
    .filter_map(|e| match e {
      SyntaxElement::Token(t) => Some(t),
      SyntaxElement::Node(_) => None,
    })
    .filter(|t| {
      let at = t.text_range().start;
      at as usize >= start && at < open.start
    })
    .collect();
  if head.iter().any(|t| {
    matches!(
      t.kind(),
      K::LineComment
        | K::BlockComment
        | K::OuterLineDoc
        | K::InnerLineDoc
        | K::OuterBlockDoc
        | K::InnerBlockDoc
        | K::Pound
    ) || t.kind().is_literal() && t.text().contains('\n')
  }) {
    return None;
  }
  let code: Vec<SyntaxToken> = head.into_iter().filter(|t| !t.kind().is_trivia()).collect();
  // The tail: closing delimiters, `?`, `,` and `;` only.
  let tail: Vec<(SyntaxKind, &str, usize)> = node
    .token_texts()
    .filter(|&(kind, _, range)| range.start >= close.end && !kind.is_trivia())
    .map(|(kind, text, range)| (kind, text, range.start as usize))
    .collect();
  let mut i = 0;
  while i < tail.len() {
    match tail[i].0 {
      K::RParen | K::RBrack | K::Question | K::Semi | K::Comma => i += 1,
      _ => return None,
    }
  }
  let spacing = Spacing::of(config);
  let flat = flat_tokens(&code, spacing);
  // A chain before the block is held to `chain_width` from its first token to the `{`, past
  // which the standard formatter breaks it a link per line (not done yet).
  let closure = value.kind() == K::ClosureExpr;
  // A struct pattern among a closure's parameters past `struct_lit_width` the standard
  // formatter breaks a field per line.
  let wide = |fields: SyntaxNode| {
    let inside: Vec<SyntaxToken> = code_tokens(&fields)
      .into_iter()
      .filter(|t| !matches!(t.kind(), K::LBrace | K::RBrace))
      .collect();
    str_width(&flat_tokens(&inside, Spacing::of(config))) > config.struct_lit_width
  };
  if closure
    && value
      .children_iter()
      .filter(|c| c.kind() == K::ParamList)
      .flat_map(|c| c.descendant_nodes_where(|kind| kind == K::RecordPatFieldList))
      .any(wide)
  {
    return None;
  }
  // From `from` to the `{`, laid flat, is held to `limit` with room for `})`; a closure's
  // parameters in a call keep one column more.
  let within = |from: u32, limit: usize, extra: usize| {
    let part: Vec<SyntaxToken> = code
      .iter()
      .filter(|t| t.text_range().start >= from)
      .cloned()
      .collect();
    str_width(&flat_tokens(&part, spacing)) + 2 + 2 + extra <= limit
  };
  if arguments_from.is_some_and(|from| !within(from, config.fn_call_width, usize::from(closure))) {
    return None;
  }
  if let Some(chain) = node
    .descendant_nodes_where(|kind| kind == K::MethodCallExpr)
    .find(|c| c.text_range().start as usize >= start && c.text_range().start < open.start)
    && !within(chain.text_range().start, config.chain_width, 0)
  {
    return None;
  }
  let last = code.last()?;
  let open_token = block.children_with_tokens_iter().find_map(|e| match e {
    SyntaxElement::Token(t) if t.kind() == K::LBrace => Some(t),
    _ => None,
  })?;
  let space = if super::space::wants_space(last, &open_token, config)
    || last.text_range().end < open_token.text_range().start
      && super::space::spacing(last, &open_token, Edition::E2021, config).unwrap_or(true)
  {
    " "
  } else {
    ""
  };
  let text = format!("{}{}", flat, space);
  // `{` must fit with room for the tail after `}` (`);`), a closure's with one more column,
  // and a method call's with its `?`s counted twice (the standard formatter does).
  let written: Vec<&str> = tail
    .iter()
    .filter(|t| t.0 != K::Comma)
    .map(|t| t.1)
    .collect();
  let closing = 1 + str_width(&written.concat());
  let tries = if method {
    tail.iter().filter(|t| t.0 == K::Question).count()
  } else {
    0
  };
  if indent + str_width(&text) + closing + usize::from(closure) + tries > config.max_width {
    return None;
  }
  Some(Head {
    text,
    open: open.start as usize,
    tail: tail.iter().map(|t| (t.2, t.0)).collect(),
  })
}

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
  let generics = node.subtree_has(|kind| kind == K::GenericArgList);
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
  // Outermost within the statement: a statement inside a block inside a chain (`match x
  // { ... }?`) is checked whole.
  node
    .descendants_iter()
    .filter(|n| {
      laid(n)
        && !n
          .ancestors_iter()
          .skip(1)
          .take_while(|a| a != node)
          .any(|a| laid(&a))
    })
    .all(|n| rewrite(&n, shape).is_some_and(|t| !t.contains('\n')))
}

/// Whether `text` holds the statement's code tokens, commas aside (a list laid out
/// vertically gains a trailing comma, one laid out on a line loses it).
pub(super) fn same_tokens(node: &SyntaxNode, text: &str) -> bool {
  // As `code_tokens` picks them, read straight from the tree.
  let start = header_start(node).unwrap_or(node.text_range().start as usize);
  if written_as(node, start, text) {
    return true;
  }
  let theirs = node
    .token_texts()
    .filter(|&(kind, _, range)| {
      !kind.is_trivia() && range.start as usize >= start && kind != K::Comma
    })
    .map(|(_, text, _)| text);
  // Inside a function body, where the statement stands, so that tokens split as they do
  // there (`>>` closing two generic lists).
  let wrapped = format!("fn f() {{\n{}\n}}", text);
  let ours = parse_tokens(&wrapped, Edition::E2021)
    .into_iter()
    .skip_while(|&(kind, _)| kind != K::LBrace)
    .skip(1)
    .filter(|&(kind, _)| !kind.is_trivia() && kind != K::Comma)
    .map(|(_, text)| text);
  // The function's closing brace ends what was parsed.
  let mut ours: Vec<&str> = ours.collect();
  ours.pop();
  theirs.eq(ours)
}

/// Whether `text` is the node's text from `from` to its end exactly: laid out as written,
/// so holding the same tokens without parsing it to compare.
pub(super) fn written_as(node: &SyntaxNode, from: usize, text: &str) -> bool {
  let mut rest = text;
  let same = node
    .token_texts()
    .filter(|&(_, _, range)| range.start as usize >= from)
    .all(|(_, token, _)| match rest.strip_prefix(token) {
      Some(after) => {
        rest = after;
        true
      }
      None => false,
    });
  same && rest.is_empty()
}

/// An assignment's left side, operator (`=`, `+=` and the rest) and right side.
fn assignment(expression: &SyntaxNode) -> Option<(SyntaxNode, String, SyntaxNode)> {
  if expression.kind() != K::BinExpr {
    return None;
  }
  let mut nodes = expression.children_iter();
  let (left, right) = (nodes.next()?, nodes.next()?);
  let operator = expression
    .children_with_tokens_iter()
    .find_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t),
      _ => None,
    })?;
  matches!(
    operator.kind(),
    K::Eq
      | K::PlusEq
      | K::MinusEq
      | K::StarEq
      | K::SlashEq
      | K::PercentEq
      | K::CaretEq
      | K::AmpEq
      | K::PipeEq
      | K::ShlEq
      | K::ShrEq
  )
  .then(|| (left, operator.text().to_string(), right))
}

/// Whether a macro call's arguments are in braces: they are left as written, and so is a
/// statement holding them.
fn braced(call: &SyntaxNode) -> bool {
  call
    .children_iter()
    .find(|c| c.kind() == K::TokenTree)
    .and_then(|tree| {
      tree
        .children_with_tokens_iter()
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
  let tokens: Vec<SyntaxKind> = node.token_texts().map(|(kind, _, _)| kind).collect();
  let spaced_tuple_field = tokens
    .windows(4)
    .any(|w| w == [K::IntNumber, K::Whitespace, K::Dot, K::IntNumber]);
  // From after its attributes and the comments before it, which are laid out on their own.
  let Some(start) = header_start(node) else {
    return false;
  };
  let safe = |e: &SyntaxElement| match e {
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
    // A block of one expression the layout code puts on one line.
    SyntaxElement::Node(n) if n.kind() == K::BlockExpr => super::expr::joins(n),
    SyntaxElement::Node(n) => {
      !matches!(n.kind(), K::MatchArmList | K::Attr | K::LetElse)
        && !(n.kind() == K::MacroCall && braced(n))
    }
  };
  // Every kind `safe` can turn down: the walk below makes nothing else.
  let suspect = |kind: SyntaxKind| {
    kind.is_literal()
      || matches!(
        kind,
        K::LineComment
          | K::BlockComment
          | K::OuterLineDoc
          | K::InnerLineDoc
          | K::OuterBlockDoc
          | K::InnerBlockDoc
          | K::BlockExpr
          | K::MatchArmList
          | K::Attr
          | K::LetElse
          | K::MacroCall
      )
  };
  // A tail that is a block itself (`unsafe { .. }`) only as one the layout code joins.
  (matches!(node.kind(), K::ExprStmt | K::LetStmt)
    || is_tail(node) && (node.kind() != K::BlockExpr || super::expr::joins(node)))
    && !spaced_tuple_field
    && node
      .children_with_tokens_iter()
      .filter(|e| e.text_range().start as usize >= start)
      .all(|e| match &e {
        SyntaxElement::Node(n) => n.descendants_where(suspect).all(|d| safe(&d)),
        token => safe(token),
      })
}

/// The statement's code tokens, from after its attributes and the comments before it, which
/// are laid out on their own.
fn code_tokens(node: &SyntaxNode) -> Vec<SyntaxToken> {
  let start = header_start(node).unwrap_or(node.text_range().start as usize);
  node
    .descendants_with_tokens_iter()
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
    // After its attributes (`#[cfg(x)] a.b();`).
    K::ExprStmt => children.into_iter().find(|c| c.kind() != K::Attr),
    K::LetStmt => {
      // The initializer is the node after `=`.
      let mut after_eq = false;
      for element in node.children_with_tokens_iter() {
        match element {
          SyntaxElement::Token(t) if t.kind() == K::Eq => after_eq = true,
          SyntaxElement::Node(n) if after_eq => return Some(n),
          _ => {}
        }
      }
      None
    }
    _ if is_tail(node) => Some(node.clone()),
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
    one_line_chains: false,
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
      if let Some((left, operator, right)) = assignment(&expression) {
        // As the standard formatter's `rewrite_assignment`: the left side, then the right
        // as a `let`'s value is placed.
        let left = rewrite(&left, shape.sub_width(operator.len() + 1)?)?;
        let head = format!("{} {}", left, operator);
        return right_side(&head, &right, indent, shape, tab);
      }
      // A block-like statement may end without its `;`.
      let semi = node
        .children_with_tokens_iter()
        .any(|e| e.kind() == K::Semi);
      if !semi {
        return rewrite(&expression, shape);
      }
      let text = rewrite(&expression, shape.sub_width(1)?)?;
      Some(format!("{};", text))
    }
    K::LetStmt => {
      let expression = expression_of(node)?;
      let head = let_head(node, &expression, shape)?;
      right_side(&head, &expression, indent, shape, tab)
    }
    _ if is_tail(node) => rewrite(node, shape),
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
  let pattern = node.children_iter().find(|c| c.kind() != K::Attr)?;
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
  if !matches!(node.kind(), K::ExprStmt | K::LetStmt) && !is_tail(node) {
    return None;
  }
  let text = flat_code(node, Spacing::of(config));
  if indent + str_width(&text) > config.max_width {
    return None;
  }
  (nested_fit(node, indent, config) && same_tokens(node, &text)).then_some(text)
}

/// `head` (`let x =`, `a.b +=`) and then `value`, with its `;`: on the same line when it
/// fits there, else on the next one, one level in, as the standard formatter's
/// `choose_rhs` decides.
fn right_side(
  head: &str,
  value: &SyntaxNode,
  indent: usize,
  shape: Shape,
  tab: usize,
) -> Option<String> {
  // `last_line_used` counts a later line's columns past `indent`, as a `let` pattern
  // spanning lines leaves them.
  let head_width = last_line_used(head, indent) + 1;
  let same_line = shape
    .offset_left(head_width)
    .and_then(|s| s.sub_width(1))
    .and_then(|s| rewrite(value, s));
  let next_line = shape
    .block_indent(tab)
    .sub_width(1)
    .and_then(|s| rewrite(value, s));
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
    one_line_chains: false,
  };
  Some(Shape {
    indent,
    offset: 0,
    width: config.max_width.checked_sub(indent)?,
    limits,
  })
}

/// Whether the layout code takes the statement whole: a `let` with an initializer, an
/// expression statement or a block's tail, whose expression is a call, chain (`?` and
/// `.await` too), struct literal, array, tuple, binary expression or block.
fn takes(node: &SyntaxNode) -> bool {
  let laid = |expression: &SyntaxNode| {
    // Through `*`, `&`, `!` and `-`: `*a.b().c() += 1` is a chain's.
    let mut expression = expression.clone();
    while matches!(expression.kind(), K::PrefixExpr | K::RefExpr)
      && let Some(inner) = expression.children_iter().next_back()
    {
      expression = inner;
    }
    let expression = &expression;
    is_chain(expression)
      || super::expr::binary_operator(expression).is_some()
      || matches!(
        expression.kind(),
        K::CallExpr | K::RecordExpr | K::ArrayExpr | K::TupleExpr | K::MacroExpr | K::BlockExpr
      )
  };
  // An assignment, as a `let`: its two sides laid out as expressions are.
  if node.kind() == K::ExprStmt
    && let Some((left, _, right)) = expression_of(node).as_ref().and_then(assignment)
  {
    return laid(&left) || laid(&right);
  }
  (matches!(node.kind(), K::ExprStmt | K::LetStmt) || is_tail(node))
    && expression_of(node).is_some_and(|expression| {
      is_chain(&expression)
        || matches!(
          expression.kind(),
          K::CallExpr | K::RecordExpr | K::ArrayExpr | K::TupleExpr | K::MacroExpr | K::BlockExpr
        )
        // `return` with a value.
        || expression.kind() == K::ReturnExpr && !expression.children().is_empty()
        || super::expr::binary_operator(&expression).is_some()
    })
}

#[cfg(test)]
mod tests;
