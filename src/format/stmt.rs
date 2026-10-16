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
  /// Whether a comma in the tail stays (after a match arm's body); else it goes, as the
  /// arguments it closed are on one line.
  pub keep_comma: bool,
  /// Where each link of a chain after the block's `})` starts (its `.`): each on a line of
  /// its own at the statement's indentation, as the standard formatter writes a chain whose
  /// root ends in a block.
  pub links: Vec<usize>,
}

/// Whether `node` is a block's tail: the expression ending it, with no `;`, laid out as a
/// statement is but for the `;`.
pub fn is_tail(node: &SyntaxNode) -> bool {
  (node.kind().name().ends_with("_EXPR") || node.kind() == K::Literal)
    && node.parent().is_some_and(|p| {
      p.kind() == K::BlockExpr && p.children_iter().next_back().as_ref() == Some(node)
    })
}

/// A control expression's head, up to its block's `{` (or a `match`'s): `if cond `, `while
/// cond `, `for pattern in iterator `, `loop `, `match scrutinee `, with its label, as the
/// standard formatter's `ControlFlow::rewrite_cond` lays it out. It starts at column
/// `column` on a line indented `indent` (after `} else ` for an `else if`); a condition that
/// breaks and does not end in closing delimiters puts the `{` on a line of its own.
pub fn rewrite_control_head(
  node: &SyntaxNode,
  indent: usize,
  column: usize,
  config: &FormatConfig,
) -> Option<Head> {
  if config.hard_tabs || config.tab_spaces == 0 {
    return None;
  }
  let children = node.children();
  let body = match node.kind() {
    K::IfExpr => children
      .get(1)
      .filter(|b| b.kind() == K::BlockExpr)?
      .clone(),
    K::WhileExpr | K::ForExpr | K::LoopExpr => children
      .iter()
      .rev()
      .find(|c| c.kind() == K::BlockExpr)?
      .clone(),
    K::MatchExpr => children
      .iter()
      .find(|c| c.kind() == K::MatchArmList)?
      .clone(),
    _ => return None,
  };
  let open = body
    .children_with_tokens_iter()
    .find(|e| e.kind() == K::LBrace)?
    .text_range()
    .start;
  let start = node.text_range().start;
  // Nothing in the head the layout code declines.
  let head: Vec<SyntaxToken> = node
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if t.text_range().start >= start && t.text_range().start < open => {
        Some(t)
      }
      _ => None,
    })
    .collect();
  if head.iter().any(|t| {
    (t.kind().is_trivia() && t.kind() != K::Whitespace)
      || super::items::is_doc(t.kind())
      || t.kind() == K::Pound
      || t.kind().is_literal() && t.text().contains('\n')
  }) {
    return None;
  }
  // The condition, pattern and iterator: nothing in them the layout code declines.
  if children
    .iter()
    .filter(|c| {
      !matches!(
        c.kind(),
        K::Label | K::BlockExpr | K::MatchArmList | K::IfExpr
      )
    })
    .any(|c| !clean_expression(c))
  {
    return None;
  }
  let shape = statement_shape(indent, config)?;
  let shape = Shape {
    offset: column.checked_sub(indent)?,
    width: config.max_width.checked_sub(column)?,
    ..shape
  };
  let spacing = Spacing::of(config);
  let label = children
    .iter()
    .find(|c| c.kind() == K::Label)
    .map(|l| format!("{} ", flat(l, spacing)))
    .unwrap_or_default();
  let keyword = head
    .iter()
    .find(|t| !t.kind().is_trivia() && t.parent() == *node)?
    .text()
    .to_string();
  let shape = shape.offset_left(str_width(&label))?;
  // 1 = the space after the keyword.
  let after = shape.offset_left(str_width(&keyword) + 1)?;
  let condition = match node.kind() {
    K::LoopExpr => String::new(),
    K::ForExpr => {
      let pattern = children.iter().find(|c| c.kind() != K::Label)?;
      let iterator = children
        .iter()
        .filter(|c| c.kind() != K::Label && c.kind() != K::BlockExpr)
        .nth(1)?;
      let pattern = rewrite(pattern, after.sub_width(3)?)?;
      super::expr::assign_value(&format!("{} in", pattern), iterator, after)?
    }
    _ => {
      let condition = children.iter().find(|c| c.kind() != K::Label)?;
      match rewrite(condition, after) {
        Some(text) => text,
        // Only `if` keeps its condition on its line.
        None if keyword != "if" => {
          let nested = shape.block_indent(config.tab_spaces);
          format!(
            "\n{}{}",
            " ".repeat(nested.indent),
            rewrite(condition, nested)?
          )
        }
        None => return None,
      }
    }
  };
  // A condition spanning lines puts it on a line of its own, unless its last line closes
  // delimiters at the head's own indentation (`) {`); so does ` {` past the line width.
  let last = condition.rsplit('\n').next().unwrap_or("");
  let closes_at_head = super::expr::last_line_extendable(&condition)
    && str_width(last) - str_width(last.trim_start()) == indent;
  let breaks_brace = (condition.contains('\n') || str_width(&condition) > shape.width)
    && !closes_at_head
    || !condition.contains('\n')
      && column + str_width(&label) + str_width(&keyword) + 1 + str_width(&condition) + 2
        > config.max_width;
  let mut text = format!("{}{}", label, keyword);
  if !condition.is_empty() {
    if !condition.starts_with('\n') {
      text.push(' ');
    }
    text.push_str(&condition);
  }
  if breaks_brace {
    text.push('\n');
    text.push_str(&" ".repeat(indent));
  } else {
    text.push(' ');
  }
  // The same tokens, as the lexer splits them, but for those [`compared`] passes over.
  let written: Vec<&str> = head
    .iter()
    .filter(|t| compared(t.kind()))
    .map(|t| t.text())
    .collect();
  let ours: Vec<String> = crate::syntax::parse_tokens(&text, Edition::E2021)
    .into_iter()
    .filter(|(kind, _)| compared(*kind))
    .map(|(_, t)| t.to_string())
    .collect();
  if written.concat() != ours.concat() {
    return None;
  }
  Some(Head {
    text,
    open: open as usize,
    tail: Vec::new(),
    keep_comma: false,
    links: Vec::new(),
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
  // The chain links after the call holding the block, when that call is the chain's root.
  let mut links: Vec<SyntaxNode> = Vec::new();
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
      // A plain call holding the block may root a chain: its links follow on lines.
      if call.kind() != K::MethodCallExpr {
        while let Some(up) = call.parent().filter(|p| {
          matches!(
            p.kind(),
            K::MethodCallExpr | K::FieldExpr | K::AwaitExpr | K::TryExpr
          ) && p.children_iter().next().as_ref() == Some(&call)
        }) {
          if up.kind() != K::TryExpr {
            links.push(up.clone());
          }
          call = up;
        }
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
  if statements == 0
    && (super::expr::lone_expression(&block).is_some()
      || !inside
        .iter()
        .any(|k| matches!(k, K::Use | K::MacroCall | K::MacroRules) || k.is_trivia()))
  {
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
  // The links' `.`s, and from the first to the last link's end what they hold: laid out
  // by the whole-file pass, on one line each.
  let dots: Vec<usize> = links
    .iter()
    .filter_map(|link| {
      link
        .children_with_tokens_iter()
        .find(|e| e.kind() == K::Dot)
        .map(|e| e.text_range().start as usize)
    })
    .rev()
    .collect();
  let links_range = match (dots.first(), links.first()) {
    (Some(&from), Some(outer)) => Some((from, outer.text_range().end as usize)),
    _ => None,
  };
  if let Some((from, to)) = links_range {
    let inner: Vec<SyntaxToken> = node
      .descendants_where(|kind| !kind.is_node())
      .filter_map(|e| match e {
        SyntaxElement::Token(t) => Some(t),
        SyntaxElement::Node(_) => None,
      })
      .filter(|t| (from..to).contains(&(t.text_range().start as usize)))
      .collect();
    if inner.iter().any(|t| {
      (t.kind().is_trivia() && t.kind() != K::Whitespace)
        || t.kind().is_literal() && t.text().contains('\n')
    }) || links.iter().any(|l| {
      l.children_iter()
        .skip(1)
        .any(|c| c.subtree_has(|kind| matches!(kind, K::BlockExpr | K::ClosureExpr)))
    }) {
      return None;
    }
    // Each link on a line of its own must fit there, the last with what follows it.
    let spacing = Spacing::of(config);
    let mut bounds: Vec<usize> = dots.clone();
    bounds.push(to);
    for (i, pair) in bounds.windows(2).enumerate() {
      let part: Vec<SyntaxToken> = inner
        .iter()
        .filter(|t| {
          let at = t.text_range().start as usize;
          at >= pair[0] && at < pair[1] && !t.kind().is_trivia()
        })
        .cloned()
        .collect();
      let after = if i + 2 == bounds.len() { 1 } else { 0 };
      if indent + str_width(&flat_tokens(&part, spacing)) + after > config.max_width {
        return None;
      }
    }
  }
  let in_links = |at: usize| links_range.is_some_and(|(from, to)| at >= from && at < to);
  // The tail: closing delimiters, `?`, `,` and `;` only (the links aside).
  let tail: Vec<(SyntaxKind, &str, usize)> = node
    .token_texts()
    .filter(|&(kind, _, range)| {
      range.start >= close.end && !kind.is_trivia() && !in_links(range.start as usize)
    })
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
  // Up to the links, which start lines of their own.
  let first_link = links_range.map_or(usize::MAX, |(from, _)| from);
  let written: Vec<&str> = tail
    .iter()
    .filter(|t| t.0 != K::Comma && t.2 < first_link)
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
  // The tail after the links (`;`) follows the last link.
  Some(Head {
    text,
    open: open.start as usize,
    tail: tail
      .iter()
      .filter(|t| t.2 < first_link)
      .map(|t| (t.2, t.0))
      .collect(),
    keep_comma: false,
    links: dots,
  })
}

/// A `let` whose value is a control expression spanning lines (`let x = if c {`, `let y =
/// match v {`): the `let`'s head, then the control expression's head on its line, as the
/// standard formatter keeps a value opening a block on the `let`'s line.
pub fn rewrite_let_control(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<Head> {
  if node.kind() != K::LetStmt || node.children_iter().any(|c| c.kind() == K::LetElse) {
    return None;
  }
  let value = expression_of(node)?;
  if !matches!(
    value.kind(),
    K::IfExpr | K::MatchExpr | K::LoopExpr | K::WhileExpr | K::ForExpr
  ) {
    return None;
  }
  // Nothing before the value the layout code declines.
  let start = header_start(node)?;
  if node.token_texts().any(|(kind, text, range)| {
    (range.start as usize) >= start
      && range.start < value.text_range().start
      && (kind.is_trivia() && kind != K::Whitespace
        || kind == K::Pound
        || kind.is_literal() && text.contains('\n'))
  }) {
    return None;
  }
  let shape = statement_shape(indent, config)?;
  let head = let_head(node, &value, shape)?;
  if head.contains('\n') {
    return None;
  }
  // On the `let`'s line, the head on one line within the line width; when it breaks, the
  // standard formatter weighs the next line by rules not followed yet: left as written.
  let column = indent + str_width(&head) + 1;
  let same = rewrite_control_head(&value, indent, column, config)?;
  if same.text.contains('\n') || column + str_width(&same.text) + 1 > config.max_width {
    return None;
  }
  Some(Head {
    text: format!("{} {}", head, same.text),
    open: same.open,
    tail: Vec::new(),
    keep_comma: false,
    links: Vec::new(),
  })
}

/// The statement laid out, starting at column `indent` on a line of its own, its later lines
/// indented in full; `None` when it is not one this module takes, or does not fit.
pub fn rewrite_statement(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<String> {
  let text = statement(node, indent, config)?;
  // A block's last `return`, `break` or `continue` gains a `;` where the block is on lines
  // (laid out in an expression, the block's layout adds it).
  let ends = is_tail(node)
    && matches!(node.kind(), K::ReturnExpr | K::BreakExpr | K::ContinueExpr)
    && node
      .parent()
      .is_some_and(|block| super::file::block_of_lines(&block));
  Some(if ends && config.trailing_semicolon {
    format!("{};", text)
  } else {
    text
  })
}

/// An assignment's left side, operator (`=`, `+=` and the rest) and right side.
pub(super) fn assignment(expression: &SyntaxNode) -> Option<(SyntaxNode, String, SyntaxNode)> {
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

/// Whether an expression (or a pattern) holds nothing laying it out would lose or misplace:
/// a comment, a block not laid out on one line, a `match`'s arms, a macro in braces, an
/// attribute, a string spanning lines, a tuple field written `x.0 .0`.
pub(super) fn clean_expression(node: &SyntaxNode) -> bool {
  if spaced_field(node) {
    return false;
  }
  laid_on_lines(node) || safe(&SyntaxElement::Node(node.clone())) && clean_inside(node)
}

/// Whether a token counts when comparing a layout's tokens with the written ones: not
/// trivia, a comma (a list laid out vertically gains a trailing one, one laid out on a line
/// loses it), a `;` (one follows a block's last `return`) or a brace (a closure's body gains
/// or loses its block).
pub(super) fn compared(kind: SyntaxKind) -> bool {
  !kind.is_trivia() && !matches!(kind, K::Comma | K::Semi | K::LBrace | K::RBrace)
}

/// Whether a node is laid out on lines of its own whatever its width, its insides checked
/// as they are laid out: a block of lines, a `match`'s arms, a macro call in braces (kept
/// as written, re-indented).
pub(super) fn laid_on_lines(node: &SyntaxNode) -> bool {
  match node.kind() {
    K::MatchArmList => true,
    K::BlockExpr => !super::expr::joins(node) && !super::expr::one_line_if_block(node),
    K::MacroCall => braced(node),
    _ => false,
  }
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

/// Whether `text` holds the statement's code tokens, but for those [`compared`] passes over.
pub(super) fn same_tokens(node: &SyntaxNode, text: &str) -> bool {
  // As `code_tokens` picks them, read straight from the tree.
  let start = header_start(node).unwrap_or(node.text_range().start as usize);
  if written_as(node, start, text) {
    return true;
  }
  let theirs = node
    .token_texts()
    .filter(|&(kind, _, range)| compared(kind) && range.start as usize >= start)
    .map(|(_, text, _)| text);
  // Inside a function body, where the statement stands, so that tokens split as they do
  // there (`>>` closing two generic lists).
  let wrapped = format!("fn f() {{\n{}\n}}", text);
  let tokens = parse_tokens(&wrapped, Edition::E2021);
  let ours = tokens
    .iter()
    .skip_while(|&&(kind, _)| kind != K::LBrace)
    .skip(1)
    .filter(|&&(kind, _)| compared(kind))
    .map(|&(_, text)| text);
  if !theirs.eq(ours) {
    return false;
  }
  // Every comment kept, each once.
  let comment = |kind: SyntaxKind| matches!(kind, K::LineComment | K::BlockComment);
  let mut theirs: Vec<&str> = node
    .token_texts()
    .filter(|&(kind, _, range)| comment(kind) && range.start as usize >= start)
    .map(|(_, text, _)| text.trim_end())
    .collect();
  let mut ours: Vec<&str> = tokens
    .iter()
    .filter(|&&(kind, _)| comment(kind))
    .map(|&(_, text)| text.trim_end())
    .collect();
  theirs.sort_unstable();
  ours.sort_unstable();
  theirs == ours
}

/// The shape a statement starting at column `indent` is laid out into.
pub(super) fn statement_shape(indent: usize, config: &FormatConfig) -> Option<Shape> {
  let limits = Limits::of(config);
  Some(Shape {
    align: 0,
    indent,
    offset: 0,
    width: config.max_width.checked_sub(indent)?,
    limits,
  })
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

/// Whether a string spanning lines is a call's argument: kept as written, its arguments laid
/// out around it as the standard formatter does.
fn argument_string(token: &SyntaxToken) -> bool {
  let literal = token.parent();
  // In a macro call's arguments, laid out as the call's are.
  if literal.kind() == K::TokenTree
    && literal
      .ancestors_iter()
      .any(|a| a.kind() == K::MacroCall && !braced(&a))
  {
    return true;
  }
  literal.kind() == K::Literal
    && literal.parent().is_some_and(|p| {
      // The value of a `let`, `const` or `static`.
      matches!(p.kind(), K::LetStmt | K::Const | K::Static)
        || p.kind() == K::ArgList
          && p
            .parent()
            .is_some_and(|call| matches!(call.kind(), K::CallExpr | K::MethodCallExpr))
    })
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
  // From after its attributes and the comments before it, which are laid out on their own.
  let Some(start) = header_start(node) else {
    return false;
  };
  if spaced_field(node) {
    return false;
  }
  // A tail that is a block itself (`unsafe { .. }`) only as one the layout code joins.
  (matches!(node.kind(), K::ExprStmt | K::LetStmt)
    || is_tail(node) && (node.kind() != K::BlockExpr || super::expr::joins(node)))
    && node
      .children_with_tokens_iter()
      .filter(|e| e.text_range().start as usize >= start)
      .all(|e| match &e {
        SyntaxElement::Node(n) => clean_expression(n),
        token => safe(token),
      })
}

/// Whether nothing inside `node` is one [`safe`] turns down, but for the blocks laid out on
/// lines and the arms of a `match`: their insides are checked as [`super::block`] lays them
/// out.
fn clean_inside(node: &SyntaxNode) -> bool {
  node
    .children_with_tokens_iter()
    .all(|element| match &element {
      SyntaxElement::Node(n) if laid_on_lines(n) => true,
      SyntaxElement::Node(n) => safe(&element) && clean_inside(n),
      SyntaxElement::Token(_) => safe(&element),
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
  let start = expression.text_range().start;
  let head: Vec<SyntaxToken> = code_tokens(node)
    .into_iter()
    .filter(|t| t.text_range().start < start)
    .collect();
  (!head.is_empty()).then(|| flat_tokens(&head, spacing).trim_end().to_string())
}

/// The statement's code on one line.
fn flat_code(node: &SyntaxNode, spacing: Spacing) -> String {
  flat_tokens(&code_tokens(node), spacing)
}

fn laid_out(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<String> {
  let limits = Limits::of(config);
  let shape = Shape {
    align: 0,
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
      match node.children_iter().find(|c| c.kind() == K::LetElse) {
        Some(otherwise) => let_else(&head, &expression, &otherwise, indent, shape),
        None => right_side(&head, &expression, indent, shape, tab),
      }
    }
    _ if is_tail(node) => rewrite(
      node,
      Shape {
        limits: Limits {
          block_tail: true,
          ..shape.limits
        },
        ..shape
      },
    ),
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

/// `let pattern = value else { .. };` as the standard formatter writes it: ` else {` on the
/// value's line when it fits there (after a value spanning lines, only when its last line
/// closes delimiters at the `let`'s indentation), else `else {` on a line of its own; the
/// block on that line (`{ return }`) when it is one expression and all of it fits within
/// `single_line_let_else_max_width`, else on lines.
fn let_else(
  head: &str,
  value: &SyntaxNode,
  otherwise: &SyntaxNode,
  indent: usize,
  shape: Shape,
) -> Option<String> {
  let block = otherwise
    .children_iter()
    .find(|c| c.kind() == K::BlockExpr)?;
  let assigned = right_side(head, value, indent, shape, shape.limits.tab)?;
  let assigned = assigned.strip_suffix(';')?;
  let same_line = if assigned.contains('\n') {
    let last = assigned.rsplit('\n').next().unwrap_or("");
    let at = str_width(last) - str_width(last.trim_start());
    at <= indent
      && !last.trim().is_empty()
      && last.trim().chars().all(|c| matches!(c, ')' | ']' | '}'))
  } else {
    // Measured in bytes, as the standard formatter measures it; 7 = ` else {`, 1 = `;`.
    shape.width.saturating_sub(1).saturating_sub(assigned.len()) >= 7
  };
  let pattern_lines = head.contains('\n');
  let newline = pattern_lines || !same_line;
  let mut text = assigned.to_string();
  if newline {
    text.push('\n');
    text.push_str(&" ".repeat(indent));
    text.push_str("else ");
  } else {
    text.push_str(" else ");
  }
  let available = shape
    .width
    .min(shape.limits.single_line_let_else)
    .saturating_sub(text.len());
  let statements = block
    .children_iter()
    .filter(|c| c.kind() != K::Label && c.kind() != K::Attr)
    .count();
  let one_line = (!newline && available > 0 && !text.contains('\n') && statements <= 1)
    .then(|| available.saturating_sub(1));
  let block_shape = Shape { offset: 0, ..shape };
  text.push_str(&super::block::else_block(&block, block_shape, one_line)?);
  text.push(';');
  Some(text)
}

/// `let pattern: Type =`, everything before the initializer: the pattern laid out as the
/// standard formatter lays it out (a struct pattern past `struct_lit_width` one field per
/// line), the type after it on the pattern's last line, laid out in what is left there
/// less ` =` (the standard formatter's `rewrite_let`); `None` when it does not fit.
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
  let ty = node
    .children_iter()
    .filter(|c| c.kind() != K::Attr)
    .nth(1)
    .filter(|t| t.text_range().end <= expression.text_range().start);
  let head = match ty {
    Some(ty) if after.ends_with(" =") || after.ends_with('=') => {
      let spacing = shape.limits.spacing;
      let colon = format!(
        "{}:{}",
        if spacing.space_before_colon { " " } else { "" },
        if spacing.space_after_colon { " " } else { "" }
      );
      let result = format!("let {}{}", pattern_text, colon);
      // A pattern across lines leaves the type the whole line after it.
      let base = if pattern_text.contains('\n') {
        Shape {
          width: shape.limits.max_width.saturating_sub(shape.indent),
          ..shape
        }
      } else {
        shape
      };
      // 2 = ` =`
      let ty_shape = base
        .offset_left(last_line_used(&result, shape.indent))?
        .sub_width(2)?;
      let ty_text = rewrite(&ty, ty_shape)?;
      format!("{}{} =", result, ty_text)
    }
    _ => format!("let {}{}", pattern_text, after),
  };
  (last_line_used(&head, shape.indent) <= shape.width).then_some(head)
}

/// A statement the layout code does not take whole, on one line when it fits there and
/// every call, chain, struct literal, array and tuple in it stays on one line by its own
/// width; else `None`.
fn on_one_line(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<String> {
  if !matches!(node.kind(), K::ExprStmt | K::LetStmt) && !is_tail(node) {
    return None;
  }
  // Blocks of lines, arms and a `let`'s `else` never go on one line.
  if node
    .descendants_iter()
    .any(|d| d.kind() == K::LetElse || laid_on_lines(&d))
  {
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
  let same_shape = shape.offset_left(head_width).and_then(|s| s.sub_width(1));
  let same_line = same_shape.and_then(|s| rewrite(value, s));
  // On one line where it is, when it fits there.
  if let (Some(same), Some(s)) = (&same_line, same_shape)
    && !same.contains('\n')
    && str_width(same) <= s.width
  {
    return Some(format!("{} {};", head, same));
  }
  let next_shape = shape.block_indent(tab).sub_width(1);
  let next_line = next_shape.and_then(|s| rewrite(value, s));
  let pad = " ".repeat(indent + tab);
  // The next line's first line within its shape, room for the `;` kept (the standard
  // formatter's `filtered_str_fits`).
  let next_fits = |next: &str| {
    next_shape.is_some_and(|s| {
      super::expr::first_line_width(next) <= s.width
        && next
          .lines()
          .skip(1)
          .all(|l| super::expr::code_width(l) <= s.limits.max_width)
    })
  };
  match (same_line, next_line) {
    (Some(same), Some(next)) if !next_fits(&next) => Some(format!("{} {};", head, same)),
    (Some(same), Some(next)) if prefer_next_line(&same, &next) => {
      Some(format!("{}\n{}{};", head, pad, next))
    }
    (Some(same), _) => Some(format!("{} {};", head, same)),
    (None, Some(next)) => Some(format!("{}\n{}{};", head, pad, next)),
    (None, None) => None,
  }
}

/// Whether one token or node is one [`clean_expression`] lets through.
fn safe(element: &SyntaxElement) -> bool {
  match element {
    // A comment between a chain's links, on a line of its own: the chain layout places it;
    // one among a call's arguments, which the list layout places.
    SyntaxElement::Token(t)
      if matches!(t.kind(), K::LineComment | K::BlockComment)
        && !t.text().contains('\n')
        && (super::expr::in_chain_gap(t)
          || matches!(t.parent().kind(), K::ArgList | K::ArrayExpr | K::TupleExpr)
          || t.parent().kind() == K::TokenTree
            && t
              .parent()
              .parent()
              .is_some_and(|p| p.kind() == K::MacroCall && !braced(&p))) =>
    {
      true
    }
    SyntaxElement::Token(t) => {
      !matches!(
        t.kind(),
        K::LineComment
          | K::BlockComment
          | K::OuterLineDoc
          | K::InnerLineDoc
          | K::OuterBlockDoc
          | K::InnerBlockDoc
      ) && !(t.kind().is_literal() && t.text().contains('\n') && !argument_string(t))
    }
    // A block of one expression the layout code puts on one line.
    SyntaxElement::Node(n) if n.kind() == K::BlockExpr => {
      super::expr::joins(n) || super::expr::one_line_if_block(n)
    }
    SyntaxElement::Node(n) => n.kind() != K::Attr,
  }
}

/// Whether `node` holds a tuple field written apart from the one before it (`x.0 .0`): the
/// standard formatter keeps the space a chain layout would drop.
fn spaced_field(node: &SyntaxNode) -> bool {
  let tokens: Vec<SyntaxKind> = node.token_texts().map(|(kind, _, _)| kind).collect();
  tokens
    .windows(4)
    .any(|w| w == [K::IntNumber, K::Whitespace, K::Dot, K::IntNumber])
}

fn statement(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<String> {
  if !clean(node, config) {
    return None;
  }
  if !takes(node) {
    return on_one_line(node, indent, config);
  }
  let text = laid_out(node, indent, config)?;
  same_tokens(node, &text).then_some(text)
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
        K::CallExpr
          | K::RecordExpr
          | K::ArrayExpr
          | K::TupleExpr
          | K::MacroExpr
          | K::BlockExpr
          | K::IfExpr
          | K::MatchExpr
          | K::LoopExpr
          | K::ClosureExpr
          | K::CastExpr
          | K::ParenExpr
      )
  };
  // An assignment, as a `let`: its two sides laid out as expressions are.
  if node.kind() == K::ExprStmt
    && let Some((left, _, right)) = expression_of(node).as_ref().and_then(assignment)
  {
    return laid(&left) || laid(&right);
  }
  // A `let` with a value, whatever it is: its pattern may break, and its value go on the
  // next line.
  if node.kind() == K::LetStmt && expression_of(node).is_some() {
    return true;
  }
  (matches!(node.kind(), K::ExprStmt | K::LetStmt) || is_tail(node))
    && expression_of(node).is_some_and(|expression| {
      laid(&expression)
        || matches!(expression.kind(), K::WhileExpr | K::ForExpr)
        // `return` with a value.
        || expression.kind() == K::ReturnExpr && !expression.children().is_empty()
    })
}

#[cfg(test)]
mod tests;
