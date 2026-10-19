//! Expressions printed into the space left for them: the standard formatter's width-budget
//! rewriting, ported. `rewrite(node, shape)` returns the
//! node's text in `shape`, or `None` when it cannot fit; callers try their layouts in order
//! and keep the first that works. Calls, method chains, struct literals, arrays and tuples
//! are laid out; other expressions print flat for now.

use std::cell::RefCell;
use std::collections::HashMap;

use super::shape::*;
use super::space;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, TextRange};
use crate::text::unicode::str_width;

/// Format-like macros, with how many arguments come before the format string (the standard
/// formatter's `SPECIAL_CASE_MACROS`).
const SPECIAL_MACROS: &[(&str, usize)] = &[
  ("eprint!", 0),
  ("eprintln!", 0),
  ("format!", 0),
  ("format_args!", 0),
  ("print!", 0),
  ("println!", 0),
  ("panic!", 0),
  ("unreachable!", 0),
  ("debug!", 0),
  ("error!", 0),
  ("info!", 0),
  ("warn!", 0),
  ("assert!", 1),
  ("debug_assert!", 1),
  ("write!", 1),
  ("writeln!", 1),
  ("assert_eq!", 2),
  ("assert_ne!", 2),
  ("debug_assert_eq!", 2),
  ("debug_assert_ne!", 2),
];

/// A text's display width: wide characters take two columns, as the standard formatter
/// counts them.
trait Width {
  fn width(&self) -> usize;
}

/// The tactic for a list, as the standard formatter's `definitive_tactic` decides it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tactic {
  Horizontal,
  Vertical,
  Mixed,
  /// A format-like macro's arguments: those before the format string on the first line,
  /// the format string on a line of its own, the rest together on the next.
  Special(usize),
}

/// Why a macro's arguments are not laid out: the standard formatter cannot read them, and
/// leaves the call as written; or this formatter does not take them.
enum Unread {
  AsWritten,
  /// In braces: its name.
  Braced(String),
  Declined,
}

/// One link of a chain: `.name::<T>(args)`, `.field`, `.await`, each with the `?`s after it.
#[derive(Clone, Debug)]
pub struct Link {
  /// `.name`, `.name::<T>`, `.0` or `.await`.
  pub head: String,
  /// The arguments of a method call.
  pub args: Option<Vec<SyntaxNode>>,
  /// A method call's generic arguments (`::<T>`), each laid out in the link's shape, and
  /// the head before them.
  pub generics: Option<(String, SyntaxNode)>,
  pub tries: usize,
  /// The comments before it, after the code before it, each with whether it starts on
  /// that code's line.
  pub comments: Vec<(String, bool)>,
}

/// A macro call whose arguments read as a call's or an array's, laid out as one under the
/// macro's name (the standard formatter's `rewrite_macro_inner`): `name!(...)` as a call
/// within `fn_call_width`, `name![...]` as an array, each keeping a trailing comma only where
/// written (`vec![...]` as an array does). Arguments in braces, or that do not parse, or hold a
/// comment, make it `None`, left as written.
/// A macro call's arguments as the standard formatter reads them: parsed as a call's
/// arguments (in parentheses) or an array's elements (in brackets).
struct MacroArguments {
  name: String,
  list: SyntaxNode,
  open: &'static str,
  close: &'static str,
  /// Held to `array_width` rather than `fn_call_width`.
  array: bool,
}

/// The rewrites kept while a [`RewriteMemo`] is open, and the trees their nodes are of.
#[derive(Default)]
struct Memo {
  done: HashMap<((usize, u32), Shape), Option<String>>,
  trees: HashMap<usize, SyntaxNode>,
}

/// While alive, [`rewrite`] on this thread keeps what it returns for each node and shape,
/// and returns it again when asked again: layouts are tried in turn, each laying out the
/// same nodes in the same shapes, and a block inside an expression is laid out for each
/// layout of what holds it. Dropping it forgets them, and frees the trees they were made
/// from, which it keeps alive meanwhile so that no node's identity is reused.
pub struct RewriteMemo {
  /// Whether this guard opened the memo, rather than finding one open.
  opened: bool,
}

impl RewriteMemo {
  pub fn open() -> Self {
    let opened = MEMO.with(|memo| {
      let mut memo = memo.borrow_mut();
      let closed = memo.is_none();
      if closed {
        *memo = Some(Memo::default());
      }
      closed
    });
    RewriteMemo { opened }
  }
}

impl Drop for RewriteMemo {
  fn drop(&mut self) {
    if self.opened {
      // Taken out first, so the trees are not dropped while the memo is borrowed.
      let kept = MEMO.with(|memo| memo.borrow_mut().take());
      drop(kept);
    }
  }
}

impl Width for String {
  fn width(&self) -> usize {
    str_width(self)
  }
}

impl Width for str {
  fn width(&self) -> usize {
    str_width(self)
  }
}

/// `head` and then `value`: on the same line when it fits there, else on the next one, one
/// level in, as the standard formatter's `choose_rhs` decides; nothing after it.
pub fn assign_value(head: &str, value: &SyntaxNode, shape: Shape) -> Option<String> {
  let used = if head.contains('\n') {
    last_line_width(head).saturating_sub(shape.indent)
  } else {
    last_line_width(head)
  };
  let same = shape
    .offset_left(used + 1)
    .and_then(|s| rewrite_or_written(value, s));
  if let Some(same) = &same
    && !same.contains('\n')
    && same.width() + used < shape.width
  {
    return Some(format!("{} {}", head, same));
  }
  let nested = shape.block_indent(shape.limits.tab);
  let next = Shape {
    width: nested.width.checked_sub(shape.rhs_overhead())?,
    ..nested
  };
  let next_shape = next;
  let next = rewrite_or_written(value, next);
  let pad = " ".repeat(nested.indent);
  // The next line's first line within its shape (the standard formatter's
  // `filtered_str_fits`).
  let next_fits = |next: &str| {
    first_line_width(next) <= next_shape.width
      && later_lines(next).all(|(_, w)| w <= next_shape.limits.max_width)
  };
  match (same, next) {
    (Some(same), Some(next)) if !next_fits(&next) => Some(format!("{} {}", head, same)),
    (Some(same), Some(next)) if prefer_next_line(&same, &next) => {
      Some(format!("{}\n{}{}", head, pad, next))
    }
    (Some(same), _) => Some(format!("{} {}", head, same)),
    (None, Some(next)) => Some(format!("{}\n{}{}", head, pad, next)),
    (None, None) => None,
  }
}

/// A binary expression's operator (`+`, `&&`, `<=`), from its tokens between the operands;
/// `None` for an assignment, laid out as a statement is.
pub fn binary_operator(node: &SyntaxNode) -> Option<String> {
  if node.kind() != SyntaxKind::BinExpr {
    return None;
  }
  let operator: String = node
    .children_with_tokens()
    .iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t.text().to_string()),
      _ => None,
    })
    .collect();
  let assigns = operator == "="
    || operator.len() >= 2
      && operator.ends_with('=')
      && !matches!(operator.as_str(), "==" | "!=" | "<=" | ">=");
  (!operator.is_empty() && !assigns).then_some(operator)
}

/// Whether the last of `count` arguments may overflow: a closure or block always; a call,
/// struct literal, array or tuple when it is the only argument; an `if`, `for`, `loop` or
/// `while` then too, with `combine_control_expr` (`control`). Never one with attributes.
pub fn can_overflow(node: &SyntaxNode, count: usize, control: bool) -> bool {
  if element_attributes(node).is_none_or(|a| !a.is_empty()) {
    return false;
  }
  match node.kind() {
    SyntaxKind::ClosureExpr | SyntaxKind::BlockExpr => true,
    SyntaxKind::IfExpr | SyntaxKind::ForExpr | SyntaxKind::LoopExpr | SyntaxKind::WhileExpr => {
      control && count == 1
    }
    // `[x; n]` is not an array to the standard formatter (`ExprKind::Repeat`).
    SyntaxKind::ArrayExpr
      if node
        .children_with_tokens()
        .iter()
        .any(|e| e.kind() == SyntaxKind::Semi) =>
    {
      false
    }
    SyntaxKind::RecordExpr
    | SyntaxKind::ArrayExpr
    | SyntaxKind::CallExpr
    | SyntaxKind::MethodCallExpr
    | SyntaxKind::TupleExpr
    | SyntaxKind::MacroExpr
    | SyntaxKind::MatchExpr
    // Patterns, as the standard formatter's `can_be_overflowed_pat`.
    | SyntaxKind::RecordPat
    | SyntaxKind::TuplePat
    | SyntaxKind::TupleStructPat
    | SyntaxKind::PathPat => count == 1,
    SyntaxKind::RefPat | SyntaxKind::BoxPat => node
      .children()
      .first()
      .is_some_and(|c| can_overflow(c, count, control)),
    SyntaxKind::RefExpr | SyntaxKind::TryExpr | SyntaxKind::PrefixExpr | SyntaxKind::CastExpr => {
      node
        .children()
        .first()
        .is_some_and(|c| can_overflow(c, count, control))
    }
    _ => false,
  }
}

/// A chain cut into its root (with the `?`s after it) and links, innermost first.
pub fn chain(node: &SyntaxNode, spacing: Spacing) -> (SyntaxNode, usize, Vec<Link>) {
  let mut links: Vec<Link> = Vec::new();
  let root_tries;
  let mut current = node.clone();
  // Walking outermost first, `?`s are counted onto the link they follow.
  let mut pending_tries = 0;
  loop {
    match current.kind() {
      SyntaxKind::TryExpr => {
        pending_tries += 1;
        current = current.children()[0].clone();
      }
      SyntaxKind::MethodCallExpr | SyntaxKind::FieldExpr | SyntaxKind::AwaitExpr => {
        let children = current.children();
        let receiver = children[0].clone();
        let mut head = String::new();
        let mut args = None;
        let mut generics = None;
        let mut after_receiver = false;
        for element in current.children_with_tokens() {
          if let SyntaxElement::Node(n) = &element
            && n == &receiver
          {
            after_receiver = true;
            continue;
          }
          if !after_receiver {
            continue;
          }
          match &element {
            SyntaxElement::Node(n) if n.kind() == SyntaxKind::ArgList => {
              args = Some(list_items(n));
            }
            SyntaxElement::Node(n) if n.kind() == SyntaxKind::GenericArgList => {
              generics = Some((head.clone(), n.clone()));
              head.push_str(&flat(n, spacing));
            }
            SyntaxElement::Node(n) => head.push_str(&flat(n, spacing)),
            SyntaxElement::Token(t) if !t.kind().is_trivia() => head.push_str(t.text()),
            _ => {}
          }
        }
        links.push(Link {
          head,
          args,
          generics,
          tries: pending_tries,
          comments: chain_comments(&current, &receiver),
        });
        pending_tries = 0;
        current = receiver;
      }
      _ => {
        root_tries = pending_tries;
        break;
      }
    }
  }
  links.reverse();
  (current, root_tries, links)
}

/// The comments between a chain link's receiver's code and the link's `.`, each with
/// whether it starts on the receiver's last line.
pub fn chain_comments(link: &SyntaxNode, receiver: &SyntaxNode) -> Vec<(String, bool)> {
  let Some(from) = receiver
    .descendants_with_tokens()
    .iter()
    .rev()
    .find(|e| matches!(e, SyntaxElement::Token(t) if !t.kind().is_trivia()))
    .map(|e| e.text_range().end)
  else {
    return Vec::new();
  };
  let Some(to) = link
    .children_with_tokens_iter()
    .find(|e| matches!(e.kind(), SyntaxKind::Dot))
    .map(|e| e.text_range().start)
  else {
    return Vec::new();
  };
  let mut comments = Vec::new();
  let mut same_line = true;
  for (kind, text, range) in link.token_texts() {
    if range.start < from || range.start >= to {
      continue;
    }
    match kind {
      SyntaxKind::Whitespace if text.contains('\n') => same_line = false,
      SyntaxKind::LineComment | SyntaxKind::BlockComment => {
        comments.push((text.trim_end().to_string(), same_line));
      }
      _ => {}
    }
  }
  comments
}

/// A token the standard formatter drops: a [`leading_pipe`], or with
/// `remove_nested_parens` one of the parentheses directly around parentheses, no comment
/// in them (`rewrite_paren`).
pub fn dropped(token: &SyntaxToken, spacing: Spacing) -> bool {
  if leading_pipe(token)
    || root_of_visibility(token)
    || keyword_visibility_in(token)
    || empty_where(token)
    || empty_generics(token)
  {
    return true;
  }
  if !spacing.remove_nested_parens
    || !matches!(token.kind(), SyntaxKind::LParen | SyntaxKind::RParen)
  {
    return false;
  }
  let inner = token.parent();
  inner.kind() == SyntaxKind::ParenExpr && nested_paren(&inner)
}

/// Where each token within `node` that a line drops ([`dropped`]) starts.
pub fn dropped_tokens(node: &SyntaxNode, spacing: Spacing) -> Vec<usize> {
  node
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if dropped(&t, spacing) => Some(t.text_range().start as usize),
      _ => None,
    })
    .collect()
}

/// The `where` of an empty `where` clause, but a function's (which the standard formatter
/// keeps).
pub fn empty_where(token: &SyntaxToken) -> bool {
  token.kind() == SyntaxKind::WhereKw
    && token.parent().kind() == SyntaxKind::WhereClause
    && token.parent().children_iter().next().is_none()
}

/// The width of a text's first line.
pub fn first_line_width(text: &str) -> usize {
  text.lines().next().map_or(0, |l| l.width())
}

/// The node's tokens on one line, spaced as the Rust style spaces them ([`super::space`]);
/// a pair no rule covers keeps a space if it was written with one.
pub fn flat(node: &SyntaxNode, spacing: Spacing) -> String {
  // Only the code tokens are made, with the nodes on the way to them.
  let tokens: Vec<SyntaxToken> = node
    .descendants_where(|kind| !kind.is_node() && !kind.is_trivia())
    .filter_map(|e| match e {
      SyntaxElement::Token(t) => Some(t),
      SyntaxElement::Node(_) => None,
    })
    .collect();
  flat_tokens(&tokens, spacing)
}

/// Tokens on one line, spaced as [`flat`] spaces a node's.
pub fn flat_tokens(tokens: &[SyntaxToken], spacing: Spacing) -> String {
  let kept: Vec<SyntaxToken>;
  let tokens = if tokens.iter().any(|t| dropped(t, spacing)) {
    kept = tokens
      .iter()
      .filter(|t| !dropped(t, spacing))
      .cloned()
      .collect();
    &kept[..]
  } else {
    tokens
  };
  let config = spacing.config();
  let mut out = String::with_capacity(tokens.iter().map(|t| t.text().len() + 1).sum());
  // Inside a macro's token tree the tokens are not code yet (`::` is two colons, `,` may
  // follow nothing): their commas stay and their spacing is as written, any space one space.
  let in_tree = |t: &SyntaxToken| t.parent().kind() == SyntaxKind::TokenTree;
  for (i, token) in tokens.iter().enumerate() {
    if in_tree(token) {
      if i > 0 && tokens[i - 1].text_range().end < token.text_range().start {
        out.push(' ');
      }
      out.push_str(token.text());
      continue;
    }
    // On one line, a list keeps no trailing comma, except a one-element tuple's.
    if token.kind() == SyntaxKind::Comma
      && tokens.get(i + 1).is_some_and(|n| {
        matches!(
          n.kind(),
          SyntaxKind::RParen | SyntaxKind::RBrack | SyntaxKind::RBrace
        ) || (n.kind() == SyntaxKind::Gt && n.parent().kind() != SyntaxKind::BinExpr)
      })
      && !one_element_tuple(token)
    {
      continue;
    }
    if i > 0 {
      let previous = &tokens[i - 1];
      let written = previous.text_range().end < token.text_range().start;
      let spaced = if written {
        space::spacing(previous, token, Edition::E2021, &config).unwrap_or(true)
      } else {
        space::wants_space(previous, token, &config)
      };
      if spaced {
        out.push(' ');
      }
    }
    out.push_str(token.text());
  }
  out
}

/// An `if`'s condition, its block, and what follows its `else` (a block or another `if`).
pub fn if_parts(node: &SyntaxNode) -> Option<(SyntaxNode, SyntaxNode, SyntaxNode)> {
  if node.kind() != SyntaxKind::IfExpr {
    return None;
  }
  let children = node.children();
  match children.as_slice() {
    [condition, then, otherwise] if then.kind() == SyntaxKind::BlockExpr => {
      Some((condition.clone(), then.clone(), otherwise.clone()))
    }
    _ => None,
  }
}

/// Whether a comment sits between a chain link's receiver and its `.`, where the chain
/// layout places it.
pub fn in_chain_gap(token: &crate::syntax::SyntaxToken) -> bool {
  let at = token.text_range().start;
  token.parent().ancestors_iter().any(|link| {
    matches!(
      link.kind(),
      SyntaxKind::MethodCallExpr | SyntaxKind::FieldExpr | SyntaxKind::AwaitExpr
    ) && link.children_iter().next().is_some_and(|receiver| {
      let code_end = receiver
        .descendants_with_tokens()
        .iter()
        .rev()
        .find(|e| matches!(e, SyntaxElement::Token(t) if !t.kind().is_trivia()))
        .map(|e| e.text_range().end);
      let dot = link
        .children_with_tokens_iter()
        .find(|e| e.kind() == SyntaxKind::Dot)
        .map(|e| e.text_range().start);
      matches!((code_end, dot), (Some(from), Some(to)) if from <= at && at < to)
    })
  })
}

/// Whether `node` is a chain: a method call, field access, `?` or `.await` with a receiver.
pub fn is_chain(node: &SyntaxNode) -> bool {
  matches!(
    node.kind(),
    SyntaxKind::MethodCallExpr
      | SyntaxKind::FieldExpr
      | SyntaxKind::TryExpr
      | SyntaxKind::AwaitExpr
  )
}

/// A simple expression, in the standard formatter's sense: one that may share lines in a
/// packed list.
pub fn is_simple(node: &SyntaxNode) -> bool {
  match node.kind() {
    SyntaxKind::Literal => true,
    SyntaxKind::PathExpr => !flat(node, Spacing::default()).contains("::"),
    SyntaxKind::RefExpr
    | SyntaxKind::FieldExpr
    | SyntaxKind::TryExpr
    | SyntaxKind::PrefixExpr
    | SyntaxKind::CastExpr => node.children().first().is_some_and(is_simple),
    SyntaxKind::IndexExpr => node.children().iter().all(is_simple),
    _ => false,
  }
}

/// Whether a block's child is a statement, which makes the block one of lines.
pub fn is_statement(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    SyntaxKind::LetStmt
      | SyntaxKind::ExprStmt
      | SyntaxKind::Fn
      | SyntaxKind::Struct
      | SyntaxKind::Enum
      | SyntaxKind::Union
      | SyntaxKind::Use
      | SyntaxKind::Const
      | SyntaxKind::Static
      | SyntaxKind::Impl
      | SyntaxKind::Trait
      | SyntaxKind::TypeAlias
      | SyntaxKind::Module
      | SyntaxKind::MacroRules
      | SyntaxKind::MacroDef
      | SyntaxKind::ExternCrate
      | SyntaxKind::ExternBlock
      | SyntaxKind::MacroCall
  )
}

/// Whether the standard formatter puts the block on one line when it fits, as `{ expr }`: a
/// block of one expression where an expression goes (a `let`'s value, an argument, a chain's
/// start); as a statement or a block's tail only after `unsafe`, `const` or `async`; never
/// with outer attributes. A closure's body loses its braces instead, and the bodies of `if`,
/// loops, match arms and functions are laid out with them.
pub fn joins(block: &SyntaxNode) -> bool {
  if lone_expression(block).is_none() {
    return false;
  }
  let Some(parent) = block.parent() else {
    return false;
  };
  let prefixed = block.children_with_tokens().iter().any(|e| {
    matches!(
      e.kind(),
      SyntaxKind::UnsafeKw | SyntaxKind::ConstKw | SyntaxKind::AsyncKw
    )
  });
  // Outer attributes, on the block or the statement it is, keep it on lines.
  let attributes = |n: &SyntaxNode| n.children().iter().any(|c| c.kind() == SyntaxKind::Attr);
  if attributes(block) || parent.kind() == SyntaxKind::ExprStmt && attributes(&parent) {
    return false;
  }
  // A `try` block is put on one line wherever it stands (`rewrite_single_line_block`).
  if block
    .children_with_tokens()
    .iter()
    .any(|e| e.kind() == SyntaxKind::TryKw)
  {
    return true;
  }
  // A condition (`if unsafe { a } {`) is an expression.
  let condition = matches!(parent.kind(), SyntaxKind::IfExpr | SyntaxKind::WhileExpr)
    && parent.children().first() == Some(block);
  if condition {
    return true;
  }
  match parent.kind() {
    // A plain block ending a block, from style edition 2024 an expression: on one line
    // when what it holds is. Before, a statement, on lines.
    SyntaxKind::BlockExpr if plain_tail_block(block) => {
      super::file::style_2024()
        && lone_expression(block).is_some_and(|inside| {
          !matches!(
            inside.kind(),
            SyntaxKind::LoopExpr
              | SyntaxKind::WhileExpr
              | SyntaxKind::ForExpr
              | SyntaxKind::MatchExpr
          ) && !(inside.kind() == SyntaxKind::IfExpr
            && !inside
              .children_with_tokens_iter()
              .any(|e| e.kind() == SyntaxKind::ElseKw))
        })
    }
    SyntaxKind::ExprStmt
    | SyntaxKind::BlockExpr
    | SyntaxKind::MatchArm
    | SyntaxKind::ClosureExpr => prefixed,
    SyntaxKind::Fn
    | SyntaxKind::IfExpr
    | SyntaxKind::WhileExpr
    | SyntaxKind::ForExpr
    | SyntaxKind::LoopExpr => false,
    kind => {
      kind.name().ends_with("_EXPR")
        || matches!(
          kind,
          SyntaxKind::LetStmt
            | SyntaxKind::ArgList
            | SyntaxKind::RecordExprField
            | SyntaxKind::Const
            | SyntaxKind::Static
        )
    }
  }
}

/// Whether a text's last line holds only closing delimiters, so more can follow on it.
pub fn last_line_extendable(text: &str) -> bool {
  let last = text.rsplit('\n').next().unwrap_or("");
  !last.trim().is_empty()
    && text.contains('\n')
    && last
      .trim()
      .chars()
      .all(|c| matches!(c, ')' | ']' | '}' | '?' | '>'))
}

/// The width of a text's last line.
pub fn last_line_width(text: &str) -> usize {
  text.rsplit('\n').next().map_or(0, |l| l.width())
}

/// A `|` before a pattern's first alternative (`(| a, b)`, `| A =>`), which the standard
/// formatter drops.
pub fn leading_pipe(token: &SyntaxToken) -> bool {
  token.kind() == SyntaxKind::Pipe
    && matches!(
      token.parent().kind(),
      SyntaxKind::OrPat | SyntaxKind::MatchArm
    )
    && token
      .parent()
      .children_with_tokens_iter()
      .find(|e| !e.kind().is_trivia() && e.kind() != SyntaxKind::Attr)
      .is_some_and(|e| e.text_range() == token.text_range())
}

/// Whether a block is one of lines: it holds a statement, or it is a function's body holding
/// anything.
pub fn lines_block(node: &SyntaxNode) -> bool {
  node.children_iter().any(|c| is_statement(c.kind()))
    || node.parent().is_some_and(|p| p.kind() == SyntaxKind::Fn)
      && node.children_iter().any(|c| c.kind() != SyntaxKind::Label)
}

/// The expressions of a list: call arguments, array or tuple elements. Their attributes are
/// laid out with them ([`with_attributes`]).
pub fn list_items(node: &SyntaxNode) -> Vec<SyntaxNode> {
  node
    .children_iter()
    .filter(|c| c.kind() != SyntaxKind::Attr)
    .collect()
}

/// The expression a block holds when it holds nothing else: no statement, attribute or
/// comment, and no prefix but `unsafe`, `const`, `async`, `move` and a label.
pub fn lone_expression(block: &SyntaxNode) -> Option<SyntaxNode> {
  if block.kind() != SyntaxKind::BlockExpr {
    return None;
  }
  let mut inside = None;
  let mut opened = false;
  for element in block.children_with_tokens() {
    match &element {
      SyntaxElement::Token(t) => match t.kind() {
        SyntaxKind::LBrace => opened = true,
        SyntaxKind::RBrace | SyntaxKind::Whitespace => {}
        SyntaxKind::UnsafeKw
        | SyntaxKind::ConstKw
        | SyntaxKind::AsyncKw
        | SyntaxKind::MoveKw
        | SyntaxKind::TryKw
          if !opened => {}
        // `try bikeshed Type {`.
        SyntaxKind::Ident if !opened && t.text() == "bikeshed" => {}
        _ => return None,
      },
      SyntaxElement::Node(n) if n.kind() == SyntaxKind::Label && !opened => {}
      SyntaxElement::Node(n) if !opened && super::types::is_type(n.kind()) => {}
      SyntaxElement::Node(n)
        if inside.is_none()
          && opened
          && (n.kind().name().ends_with("_EXPR") || n.kind() == SyntaxKind::Literal) =>
      {
        inside = Some(n.clone());
      }
      SyntaxElement::Node(_) => return None,
    }
  }
  let inside = inside?;
  let quiet = !inside.descendants_with_tokens().iter().any(|e| {
    matches!(
      e.kind(),
      SyntaxKind::Attr
        | SyntaxKind::LineComment
        | SyntaxKind::BlockComment
        | SyntaxKind::OuterLineDoc
        | SyntaxKind::InnerLineDoc
        | SyntaxKind::OuterBlockDoc
        | SyntaxKind::InnerBlockDoc
    )
  });
  quiet.then_some(inside)
}

/// Whether a block is a branch of an `if` the layout code takes whole: every branch of the
/// `if`, and of the `else if`s after it, holds one expression ([`rewrite_if_lines`]).
pub fn one_line_if_block(block: &SyntaxNode) -> bool {
  let Some(mut node) = block.parent().filter(|p| p.kind() == SyntaxKind::IfExpr) else {
    return false;
  };
  while let Some(up) = node.parent().filter(|p| p.kind() == SyntaxKind::IfExpr) {
    node = up;
  }
  lone_branches(&node)
}

/// Whether `block` is a block of its own (no `unsafe`, `const`, `async` or label, though
/// attributes may stand before it) ending the block it stands in.
pub fn plain_tail_block(block: &SyntaxNode) -> bool {
  // One with attributes stands in a statement of its own, with no `;`.
  let standing = block.parent().filter(|p| {
    p.kind() == SyntaxKind::ExprStmt
      && !p
        .children_with_tokens_iter()
        .any(|e| e.kind() == SyntaxKind::Semi)
  });
  let holder = standing.unwrap_or_else(|| block.clone());
  block.kind() == SyntaxKind::BlockExpr
    && holder.parent().is_some_and(|p| {
      p.kind() == SyntaxKind::BlockExpr && p.children_iter().next_back().as_ref() == Some(&holder)
    })
    && block
      .children_with_tokens_iter()
      .take_while(|e| e.kind() != SyntaxKind::LBrace)
      .all(|e| e.kind().is_trivia())
}

/// The standard formatter moves a right-hand side that does not fit on one line where it is
/// to the next line when it is on one line there, when that saves more than one line, or
/// when its first line would otherwise end by opening a delimiter that the next line's does
/// not. Callers keep one fitting on one line where it is first.
pub fn prefer_next_line(same: &str, next: &str) -> bool {
  let lines = |text: &str| text.matches('\n').count();
  let opens = |text: &str, c: char| text.lines().next().is_some_and(|l| l.ends_with(c));
  lines(next) == 0
    || lines(same) > lines(next) + 1
    || ['(', '{', '[']
      .iter()
      .any(|&c| opens(same, c) && !opens(next, c))
}

/// The node printed into `shape`, or `None` when it does not fit.
pub fn rewrite(node: &SyntaxNode, shape: Shape) -> Option<String> {
  // Being a block's last expression concerns an `if` only, not what is inside another.
  let mut shape = shape;
  if node.kind() != SyntaxKind::IfExpr {
    shape.limits.block_tail = false;
  }
  let key = (node.identity(), shape);
  let kept = MEMO.with(|memo| {
    memo
      .borrow()
      .as_ref()
      .map(|memo| memo.done.get(&key).cloned())
  });
  let laid = |node: &SyntaxNode, shape: Shape| {
    let attributes = element_attributes(node)?;
    let text = if lost_field_comment(node) {
      node.text().to_string()
    } else {
      recover_comments(node, rewrite_node(node, shape)?)
    };
    with_attributes(&attributes, text, shape)
  };
  match kept {
    // Not open.
    None => laid(node, shape),
    Some(Some(done)) => done,
    Some(None) => {
      let done = laid(node, shape);
      MEMO.with(|memo| {
        if let Some(memo) = memo.borrow_mut().as_mut() {
          memo.trees.entry(key.0.0).or_insert_with(|| node.clone());
          memo.done.insert(key, done.clone());
        }
      });
      done
    }
  }
}

/// `node` laid out in `shape`; a macro call that cannot be is kept as written where that
/// fits, as the standard formatter's `format_expr` falls back to its text (`wrap_str`): for
/// a value on its line or the next, between which `prefer_next_line` chooses.
pub fn rewrite_or_written(node: &SyntaxNode, shape: Shape) -> Option<String> {
  rewrite(node, shape).or_else(|| {
    matches!(node.kind(), SyntaxKind::MacroExpr | SyntaxKind::MacroCall)
      .then(|| wrap_str(node.text().to_string().trim().to_string(), shape))
      .flatten()
  })
}

/// Whether a block's last expression gains a `;` on a line of its own: a `return`, `break`
/// or `continue`, with `trailing_semicolon`.
pub(super) fn ends_with_semi(node: &SyntaxNode, limits: Limits) -> bool {
  limits.trailing_semicolon
    && matches!(
      node.kind(),
      SyntaxKind::ReturnExpr | SyntaxKind::BreakExpr | SyntaxKind::ContinueExpr
    )
}

/// An `if` with an `else` on one line, wherever it stands (see [`rewrite_if_one_line`]).
pub(super) fn if_single_line(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let (condition, then, otherwise) = if_parts(node)?;
  if otherwise.kind() != SyntaxKind::BlockExpr {
    return None;
  }
  let (then, otherwise) = (lone_expression(&then)?, lone_expression(&otherwise)?);
  // 3 = `if `
  let condition = rewrite(&condition, shape.offset_left(3)?)?;
  if condition.contains('\n') {
    return None;
  }
  // `if` and `  {  } else {  }`
  let fixed = 2 + 16;
  let width = shape.width.checked_sub(condition.width() + fixed)?;
  let flat_shape = |width| Shape {
    align: 0,
    indent: 0,
    offset: 0,
    width,
    limits: shape.limits,
  };
  let then = rewrite(&then, flat_shape(width))?;
  let otherwise = rewrite(&otherwise, flat_shape(width.checked_sub(then.width())?))?;
  if then.contains('\n') || otherwise.contains('\n') {
    return None;
  }
  let text = format!("if {} {{ {} }} else {{ {} }}", condition, then, otherwise);
  // Measured in bytes, as the standard formatter measures it here.
  (text.len() <= shape.width && text.len() <= shape.limits.single_line_if_else).then_some(text)
}

/// Whether an expression is a macro call in braces: as a block's only statement, the
/// standard formatter reads it as a statement and keeps the block around it.
pub(super) fn is_macro(node: &SyntaxNode) -> bool {
  matches!(node.kind(), SyntaxKind::MacroExpr | SyntaxKind::MacroCall)
    && node
      .descendants()
      .into_iter()
      .find(|d| d.kind() == SyntaxKind::TokenTree)
      .and_then(|tree| tree.first_token())
      .is_some_and(|t| t.kind() == SyntaxKind::LBrace)
}

thread_local! {
  static MEMO: RefCell<Option<Memo>> = const { RefCell::new(None) };
}

/// Each line of `text` after the first, with its width as the standard formatter's
/// `filtered_str_fits` counts it (`filter_normal_code`): none for a line that is a comment,
/// or that ends in a line comment. Each line is read on its own, as the standard
/// formatter's char classes read a raw string's lines (found by measuring it); a `://` is a
/// link's, not a comment.
pub(super) fn later_lines(text: &str) -> impl Iterator<Item = (&str, usize)> {
  let inside = lines_in_block_comments(text);
  text.split('\n').zip(inside).skip(1).map(|(line, inside)| {
    let trimmed = line.trim_start();
    let comment = trimmed.starts_with("//") || trimmed.starts_with("/*");
    (
      line,
      if comment || inside || ends_in_line_comment(line) {
        0
      } else {
        line.width()
      },
    )
  })
}

/// Whether `node` is a struct literal with a comment between a field's attribute and the
/// field, which the standard formatter's layout loses and so keeps the literal as written
/// (`recover_comment_removed`).
pub(super) fn lost_field_comment(node: &SyntaxNode) -> bool {
  node.kind() == SyntaxKind::RecordExpr
    && node
      .descendant_nodes_where(|kind| kind == SyntaxKind::RecordExprField)
      .filter(|field| field.ancestors_iter().nth(2).as_ref() == Some(node))
      .any(|field| {
        let mut after_attribute = false;
        for element in field.children_with_tokens_iter() {
          match element.kind() {
            SyntaxKind::Attr => after_attribute = true,
            SyntaxKind::LineComment | SyntaxKind::BlockComment if after_attribute => return true,
            kind if kind.is_trivia() => {}
            _ => return false,
          }
        }
        false
      })
}

/// Whether a control expression's head spans lines laid out in `shape`.
pub(super) fn multi_line_condition(node: &SyntaxNode, shape: Shape) -> bool {
  if !matches!(
    node.kind(),
    SyntaxKind::IfExpr | SyntaxKind::MatchExpr | SyntaxKind::WhileExpr | SyntaxKind::ForExpr
  ) {
    return false;
  }
  // A `match`'s: its scrutinee alone, after `match ` and before ` {` (8 columns).
  if node.kind() == SyntaxKind::MatchExpr {
    return node
      .children_iter()
      .find(|c| c.kind() != SyntaxKind::Attr)
      .zip(shape.offset_left(8))
      .and_then(|(scrutinee, shape)| rewrite(&scrutinee, shape))
      .is_some_and(|text| text.contains('\n'));
  }
  super::stmt::rewrite_control_head(
    node,
    shape.indent,
    shape.indent + shape.offset,
    &shape.limits.config(),
  )
  .is_some_and(|head| head.text.trim_end().contains('\n'))
}

/// Whether a block is a plain one: no `unsafe`, `async`, `const` or label before its `{`.
pub(super) fn plain_block(block: &SyntaxNode) -> bool {
  block.kind() == SyntaxKind::BlockExpr
    && block
      .children_with_tokens()
      .iter()
      .take_while(|e| e.kind() != SyntaxKind::LBrace)
      .all(|e| e.kind().is_trivia())
}

/// An `if` whose branches each hold one expression, on lines: `if cond {`, each branch's
/// expression one level in, `} else {` or `} else if cond {`, and `}`, as the standard
/// formatter writes it where it does not go on one line. The condition is laid out as a
/// head's is: one breaking or too wide puts `{` on a line of its own, unless its last line
/// closes delimiters at the `if`'s indentation. `nested` for an `else if`, which follows
/// `} else ` (7 columns).
pub(super) fn rewrite_if_lines(node: &SyntaxNode, shape: Shape, nested: bool) -> Option<String> {
  let children = node.children();
  let (condition, then, otherwise) = match children.as_slice() {
    [condition, then] => (condition, then, None),
    [condition, then, otherwise] => (condition, then, Some(otherwise)),
    _ => return None,
  };
  // Nothing around the condition and branches that laying them out would lose, but comments
  // between the first branch and `else`: each on a line of its own, `else` on the next.
  let else_at = node
    .children_with_tokens_iter()
    .find(|e| e.kind() == SyntaxKind::ElseKw)
    .map(|e| e.text_range().start);
  let mut before_else: Vec<String> = Vec::new();
  for e in node.children_with_tokens_iter() {
    if !matches!(e.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment) {
      continue;
    }
    let at = e.text_range().start;
    let placed = e.kind() == SyntaxKind::LineComment
      && at >= then.text_range().end
      && else_at.is_some_and(|else_at| at < else_at);
    if !placed {
      return None;
    }
    if let SyntaxElement::Token(t) = &e {
      // A blank line written between two of them stays (as an empty entry).
      if !before_else.is_empty()
        && matches!(t.prev_sibling_or_token(), Some(SyntaxElement::Token(w))
          if w.kind() == SyntaxKind::Whitespace && w.text().matches('\n').count() > 1)
      {
        before_else.push(String::new());
      }
      before_else.push(t.text().trim_end().to_string());
    }
  }
  if !super::stmt::clean_expression(condition) {
    return None;
  }
  // From the `if`'s line, whatever came before it there.
  let fresh = Shape {
    width: shape
      .limits
      .max_width
      .checked_sub(shape.indent + shape.offset)?,
    ..shape
  };
  let fresh = if nested { fresh.offset_left(7)? } else { fresh };
  let condition = rewrite(condition, fresh.offset_left(3)?)?;
  let last = condition.rsplit('\n').next().unwrap_or("");
  let closes_at_head =
    last_line_extendable(&condition) && last.width() - last.trim_start().width() == shape.indent;
  let column = shape.indent + shape.offset + if nested { 7 } else { 0 };
  let breaks_brace = (condition.contains('\n') || condition.width() > fresh.width)
    && !closes_at_head
    || !condition.contains('\n') && column + 3 + condition.width() + 2 > shape.limits.max_width;
  let pad = " ".repeat(shape.indent);
  let inner = shape.block_inside();
  let inner_pad = " ".repeat(inner.indent);
  let mut text = format!("if {}", condition);
  if breaks_brace {
    text.push_str(&format!("\n{}", pad));
  } else {
    text.push(' ');
  }
  // A branch of one expression, or one of lines.
  let branch = |block: &SyntaxNode| match lone_expression(block) {
    Some(inside) => {
      let body = rewrite(&inside, inner)?;
      let semi = if ends_with_semi(&inside, shape.limits) {
        ";"
      } else {
        ""
      };
      Some(format!("{{\n{}{}{}\n{}}}", inner_pad, body, semi, pad))
    }
    None => super::block::block_body(block, shape.block_indent(0)),
  };
  // An empty branch of an `if` with an `else` keeps its braces apart.
  let apart = |body: String| {
    if body == "{}" {
      format!("{{\n{}}}", pad)
    } else {
      body
    }
  };
  let then_text = branch(then)?;
  // So does one after `else if`, one whose `{` starts a line after a condition on one line,
  // and one whose `{}` would pass the line width (the standard formatter's block width).
  let alone = breaks_brace && !condition.contains('\n');
  let past = !breaks_brace && column + 3 + condition.width() + 3 > shape.limits.max_width;
  text.push_str(&if otherwise.is_some() || nested || alone || past {
    apart(then_text)
  } else {
    then_text
  });
  let else_text = if before_else.is_empty() {
    " else ".to_string()
  } else {
    let mut lines = String::new();
    for comment in &before_else {
      if comment.is_empty() {
        lines.push('\n');
      } else {
        lines.push_str(&format!("\n{}{}", pad, comment));
      }
    }
    format!("{}\n{}else ", lines, pad)
  };
  match otherwise {
    None => {}
    Some(next) if next.kind() == SyntaxKind::IfExpr => {
      let line = Shape {
        offset: 0,
        width: shape.limits.max_width.checked_sub(shape.indent)?,
        ..shape
      };
      text.push_str(&else_text);
      text.push_str(&rewrite_if_lines(next, line, true)?);
    }
    Some(block) => {
      text.push_str(&else_text);
      text.push_str(&apart(branch(block)?));
    }
  }
  Some(text)
}

/// `ident(items)`: calls, arrays and tuples, with the standard formatter's tactics: on one
/// line within `max_width`, the last item overflowing, packed when every item is short and
/// simple, else one per line.
pub(super) fn rewrite_list(
  ident: &str,
  items: &[SyntaxNode],
  shape: Shape,
  max_width: usize,
  open: &str,
  close: &str,
) -> Option<String> {
  // With nothing in it, on the line when it fits there; else (a callee past its shape) the
  // closing delimiter on the next line, as the standard formatter's `wrap_items` leaves it
  // for the list holding the call to lay out again.
  if items.is_empty() {
    // An empty generic list says nothing (`impl<> Foo<>`).
    if open == "<" && !ident.ends_with("::") {
      return wrap_str(ident.to_string(), shape);
    }
    let text = format!("{}{}{}", ident, open, close);
    if wrap_str(text.clone(), shape).is_some() || open != "(" {
      return wrap_str(text, shape);
    }
    if last_line_width(ident) > shape.indent + shape.offset + shape.width || ident.contains('\n') {
      return None;
    }
    return Some(format!("{}(\n{})", ident, " ".repeat(shape.indent)));
  }
  // A type's list inside a macro's arguments (generic arguments) takes its trailing comma
  // as any does; the macro's own list of types keeps it as written.
  let comma_as_written = shape.limits.comma_as_written
    && (ident.ends_with('!') || !super::types::is_type(items[0].kind()));
  let outer_attribute = shape.limits.attribute
    && items
      .first()
      .and_then(|item| item.parent())
      .and_then(|list| list.parent())
      .and_then(|call| call.parent())
      .is_some_and(|statement| statement.kind() == SyntaxKind::ExprStmt);
  let used = last_line_width(ident);
  let one_line_width = shape.width.saturating_sub(used + 2);
  let one_line_shape = shape
    .offset_left(used + 1)
    .and_then(|s| s.sub_width(1))
    .unwrap_or(Shape { width: 0, ..shape });
  let nested = {
    let s = shape.block_indent(shape.limits.tab);
    Shape {
      width: s.width.saturating_sub(1),
      ..s
    }
  };
  let mut texts: Vec<String> = Vec::with_capacity(items.len());
  for item in &items[..items.len().saturating_sub(1)] {
    texts.push(rewrite(item, nested)?);
  }
  // The last item laid out on a line of its own is needed only when it does not overflow
  // (the standard formatter's items are rewritten lazily).
  let mut last_failed = false;
  if let Some(last) = items.last() {
    match rewrite(last, nested) {
      Some(text) => texts.push(text),
      None => {
        last_failed = true;
        texts.push(String::new());
      }
    }
  }
  // Comments among the items: one per line, the comments placed around them.
  if let Some(parent) = items.first().and_then(|i| i.parent())
    && let Some(comments) = super::lists::list_comments(&parent, items)
  {
    if last_failed {
      return None;
    }
    // Block comments only: on the line when it all fits there (`definitive_tactic` breaks
    // the list for line comments).
    // A comment holding a line comment (after a block one too), or spanning lines.
    let line_comment = |c: &Option<String>| {
      c.as_ref()
        .is_some_and(|c| c.contains('\n') || c.lines().any(|l| l.trim_start().starts_with("//")))
    };
    if !comments
      .iter()
      .any(|c| line_comment(&c.pre) || line_comment(&c.post))
      && texts.iter().all(|t| !t.contains('\n'))
    {
      let joined: Vec<String> = texts
        .iter()
        .zip(&comments)
        .map(|(t, c)| {
          let mut item = String::new();
          if let Some(pre) = &c.pre {
            item.push_str(pre.trim());
            item.push(' ');
          }
          item.push_str(t);
          if let Some(post) = &c.post {
            item.push(' ');
            item.push_str(post.trim());
          }
          item
        })
        .collect();
      let joined = joined.join(", ");
      // Each comment counted with six columns around it, one of them its space here
      // (`total_item_width`).
      let counted = comments
        .iter()
        .map(|c| usize::from(c.pre.is_some()) + usize::from(c.post.is_some()))
        .sum::<usize>()
        * 5;
      if joined.width() + counted <= max_width.min(one_line_width) {
        return wrap_str(format!("{}{}{}{}", ident, open, joined, close), shape);
      }
    }
    let trailing = !outer_attribute && (!comma_as_written || items.last().is_some_and(comma_after));
    // A format-like macro's arguments before and after its format string, each group on a
    // line, when the only comments are the format string's before it
    // (`SpecialMacro` after `maybe_get_args_offset`).
    let fits = |texts: &[String]| definitive(texts, nested.width) == Tactic::Horizontal;
    if let Some(before) = special_macro(ident, shape.limits.style_2024)
      && items.len() > before
      && let Some(pre) = &comments[before].pre
      && comments
        .iter()
        .enumerate()
        .all(|(i, c)| c.post.is_none() && (i == before || c.pre.is_none()))
      && items[..before].iter().all(is_simple)
      && items[before + 1..].iter().all(is_simple)
      && texts.iter().all(|t| !t.contains('\n'))
      && fits(&texts[..before])
      && fits(&texts[before + 1..])
    {
      let pad = " ".repeat(nested.indent);
      let mut lines = Vec::new();
      if before > 0 {
        lines.push(format!("{},", texts[..before].join(", ")));
      }
      lines.extend(pre.trim().lines().map(|l| l.trim().to_string()));
      lines.push(texts[before].clone());
      if items.len() > before + 1 {
        let last = lines.len() - 1;
        lines[last].push(',');
        lines.push(texts[before + 1..].join(", "));
      }
      let mut body = lines.join(&format!("\n{}", pad));
      if trailing {
        body.push(',');
      }
      return Some(format!(
        "{}{}\n{}{}\n{}{}",
        ident,
        open,
        pad,
        body,
        " ".repeat(shape.indent),
        close
      ));
    }
    // Short simple items packed, as without comments.
    let mixed = items.len() > 1
      && items.iter().all(is_simple)
      && texts.iter().all(|t| t.width() <= shape.limits.short_item)
      && special_macro(ident, shape.limits.style_2024).is_none();
    let body = super::lists::write_vertical(
      &texts,
      &comments,
      nested.indent,
      nested.width,
      shape.limits.max_width,
      trailing,
      mixed,
    )?;
    return Some(format!(
      "{}{}\n{}{}\n{}{}",
      ident,
      open,
      " ".repeat(nested.indent),
      body,
      " ".repeat(shape.indent),
      close
    ));
  }
  let mut tactic = Tactic::Vertical;
  if !items.is_empty() {
    let last = items.len() - 1;
    // A type is not an expression, which alone is combined with its callee.
    let combine = items.len() == 1
      && used < shape.limits.tab
      && !super::types::is_type(items[0].kind())
      && element_attributes(&items[0]).is_some_and(|a| a.is_empty());
    // A slice pattern's last element does not overflow; among several closures the last
    // does not either (`args_have_many_closure`).
    let slice = items[0]
      .parent()
      .is_some_and(|p| p.kind() == SyntaxKind::SlicePat);
    let closures = items
      .iter()
      .filter(|i| i.kind() == SyntaxKind::ClosureExpr)
      .count();
    let overflow = !slice
      && closures < 2
      && (combine
        || if super::types::is_type(items[last].kind()) {
          super::types::can_overflow_type(&items[last], items.len())
        } else {
          can_overflow(&items[last], items.len(), shape.limits.combine_control_expr)
        });
    // A method chain overflowing a list it is not combined with keeps its links on one
    // line, though its last call's arguments may break (the standard formatter's
    // `force_one_line_chain`).
    // For a macro's arguments only from style edition 2024, as the standard formatter has it.
    let one_line_chain = !combine
      && is_method_call(&items[last])
      && (!ident.ends_with('!') || shape.limits.style_2024);
    let placeholder = if overflow {
      // An attribute's nested list overflowing its parent stops a column short, as the
      // standard formatter writes it (found by measuring it).
      let overflow_shape = if shape.limits.attribute {
        one_line_shape.sub_width(1).unwrap_or(Shape {
          width: 0,
          ..one_line_shape
        })
      } else {
        one_line_shape
      };
      let overflow_shape = Shape {
        limits: Limits {
          one_line_chains: shape.limits.one_line_chains || one_line_chain,
          ..overflow_shape.limits
        },
        ..overflow_shape
      };
      last_item_shape(items, &texts, overflow_shape, max_width).and_then(|s| {
        match items[last].kind() {
          SyntaxKind::ClosureExpr => rewrite_last_closure(&items[last], s),
          // A control expression overflows unless its condition spans lines.
          _ if multi_line_condition(&items[last], s) => None,
          _ => rewrite(&items[last], s),
        }
      })
    } else {
      None
    };
    // `vec![x; n]` the standard formatter overflows across lines when it does not fit on
    // the line, which is not done here: the list fails.
    let repeat_macro = |item: &SyntaxNode| {
      matches!(item.kind(), SyntaxKind::MacroExpr | SyntaxKind::MacroCall)
        && macro_arguments(item, shape.limits.spacing, shape.limits.inside_macro).is_ok_and(
          |arguments| {
            arguments
              .list
              .children_with_tokens()
              .iter()
              .any(|e| e.kind() == SyntaxKind::Semi)
          },
        )
    };
    if overflow && placeholder.is_none() && repeat_macro(&items[last]) {
      return None;
    }
    if last_failed && placeholder.is_none() {
      return None;
    }
    let mut probe = texts.clone();
    if let Some(over) = &placeholder {
      probe[last] = over.lines().next().unwrap_or("").to_string();
    }
    tactic = definitive(&probe, max_width.min(one_line_width));
    if last_failed && !(overflow && tactic == Tactic::Horizontal) {
      return None;
    }
    match (overflow, tactic, placeholder) {
      (true, Tactic::Horizontal, Some(over)) if items.len() == 1 => {
        if over.matches('\n').count() == 1 {
          match rewrite(&items[last], nested) {
            Some(again) if !again.contains('\n') => texts[last] = again,
            _ => texts[last] = over,
          }
        } else {
          texts[last] = over;
        }
      }
      (true, Tactic::Horizontal, Some(over)) => texts[last] = over,
      _ => {
        if items.len() == 1
          && one_line_width != 0
          && !texts[0].contains('\n')
          && texts[0].width() <= one_line_width
        {
          tactic = Tactic::Horizontal;
        } else if shape.limits.attribute
          && items.len() == 1
          && items[0].kind() == SyntaxKind::BinExpr
          && texts[0].contains('\n')
          && first_line_width(&texts[0]) <= max_width.min(one_line_width)
        {
          // An attribute's one `name = value` across lines (a string written so) stays on
          // its line when its first line fits there (found by measuring the standard
          // formatter).
          tactic = Tactic::Horizontal;
        } else {
          tactic = definitive(&texts, max_width.min(one_line_width));
          let special = special_macro(ident, shape.limits.style_2024);
          if tactic == Tactic::Vertical {
            if let Some(before) = special {
              // The standard formatter's `maybe_get_args_offset`: the arguments around the
              // format string are simple, and each group fits on a line.
              let fits = |texts: &[String]| definitive(texts, nested.width) == Tactic::Horizontal;
              if items.len() > before
                && items[..before].iter().all(is_simple)
                && items[before + 1..].iter().all(is_simple)
                && fits(&texts[..before])
                && fits(&texts[before + 1..])
              {
                tactic = Tactic::Special(before);
              }
            } else if items.iter().all(is_simple)
              && texts.iter().all(|t| t.width() <= shape.limits.short_item)
            {
              tactic = Tactic::Mixed;
            }
          }
        }
      }
    }
  }
  // An attribute's own list takes no trailing comma (the standard formatter's
  // `SeparatorTactic::Never` there); a list nested in it keeps one as written.
  let trailing = if outer_attribute {
    ""
  } else if !comma_as_written || items.last().is_some_and(comma_after) {
    ","
  } else {
    ""
  };
  let body = match tactic {
    // Attribute and macro arguments keep a written trailing comma on one line too.
    Tactic::Horizontal if comma_as_written => {
      format!("{}{}", texts.join(", "), trailing)
    }
    Tactic::Horizontal => texts.join(", "),
    Tactic::Vertical => {
      let sep = format!(",\n{}", " ".repeat(nested.indent));
      format!("{}{}", texts.join(&sep), trailing)
    }
    Tactic::Mixed => {
      let mut lines: Vec<String> = Vec::new();
      let mut line = String::new();
      for (i, text) in texts.iter().enumerate() {
        // The item with its comma (the last's only when it takes one), and the space after
        // the comma before it.
        let comma = usize::from(i + 1 < texts.len() || !trailing.is_empty());
        let added = if line.is_empty() {
          text.width() + comma
        } else {
          text.width() + 2 + comma
        };
        if !line.is_empty() && line.width() + added > nested.width {
          lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
          line.push_str(", ");
        }
        line.push_str(text);
      }
      lines.push(line);
      let sep = format!(",\n{}", " ".repeat(nested.indent));
      format!("{}{}", lines.join(&sep), trailing)
    }
    Tactic::Special(before) => {
      let mut body = String::new();
      for (i, text) in texts.iter().enumerate() {
        if i > 0 {
          body.push(',');
          if i < before || i > before + 1 {
            body.push(' ');
          } else {
            body.push('\n');
            body.push_str(&" ".repeat(nested.indent));
          }
        }
        body.push_str(text);
      }
      format!("{}{}", body, trailing)
    }
  };
  // wrap_items: on the opening line when it is horizontal and its first line fits.
  let available = shape.width.saturating_sub(used);
  let extend = if body.is_empty() {
    2
  } else {
    first_line_width(&body) + 1
  };
  // Inside a macro's arguments, before style edition 2024, a list whose items went on one line
  // stays there when it fits, whatever its tactic (the standard formatter's `wrap_items`).
  let single = tactic == Tactic::Horizontal && extend <= available
    || shape.limits.inside_macro
      && !shape.limits.style_2024
      && !body.is_empty()
      && !body.contains('\n')
      && body.width() + 2 <= available;
  let text = if single {
    format!("{}{}{}{}", ident, open, body, close)
  } else if body.is_empty() {
    format!("{}{}{}", ident, open, close)
  } else {
    format!(
      "{}{}\n{}{}\n{}{}",
      ident,
      open,
      " ".repeat(nested.indent),
      body,
      " ".repeat(shape.indent),
      close
    )
  };
  // As the standard formatter's `rewrite_with_parens`: a list broken over lines is
  // returned whatever its opening line's width, for the caller to weigh against laying it
  // out elsewhere, though no later line may pass the line width; one kept on a line must
  // fit.
  // In an attribute a `name = literal` may run past it.
  // A raw string alone on its line is kept whatever its width (`format_expr`), and from
  // style edition 2024 a string too (`rewrite_string_lit`).
  let string_line = |line: &str| {
    let line = line.trim();
    let line = line.strip_suffix(',').unwrap_or(line);
    let raw = (line.starts_with("r\"") || line.starts_with("r#")) && line.ends_with(['"', '#']);
    // Or the first line of one continued with `\`, kept with it.
    let plain = shape.limits.style_2024 && line.starts_with('"') && line.ends_with(['"', '\\']);
    raw || plain
  };
  // A type's list is returned whatever its lines' widths (the standard formatter's callers
  // do not check them), as is one holding a macro call kept as written (the snippet its
  // `return_macro_parse_failure_fallback` returns, or a braced call's re-indented).
  let unread = |item: &SyntaxNode| {
    matches!(item.kind(), SyntaxKind::MacroExpr | SyntaxKind::MacroCall)
      && matches!(
        read_macro_arguments(item, shape.limits.spacing, shape.limits.inside_macro),
        Err(Unread::AsWritten | Unread::Braced(_))
      )
  };
  if single
    || shape.limits.attribute
    || super::types::is_type(items[0].kind())
    || items.iter().any(unread)
  {
    Some(text)
  } else {
    let fits = |inside: &[bool]| {
      later_lines(&text).enumerate().all(|(i, (l, w))| {
        w <= shape.limits.max_width || string_line(l) || inside.get(i + 1) == Some(&true)
      })
    };
    // A line inside a raw string spanning lines is not measured: kept as written
    // (`format_expr`), where a plain string's lines are (`rewrite_string_lit`).
    (fits(&[]) || fits(&in_raw_string(&text, shape.limits.style_2024))).then_some(text)
  }
}

/// Whether a macro call's arguments in parentheses or brackets cannot be read as code, so
/// the standard formatter keeps them as written: as with ones that are only comments.
pub(super) fn unread_macro(call: &SyntaxNode) -> bool {
  let comments_only = call
    .children_iter()
    .find(|c| c.kind() == SyntaxKind::TokenTree)
    .is_some_and(|tree| {
      let inside: Vec<SyntaxKind> = tree.children_with_tokens_iter().map(|e| e.kind()).collect();
      inside.len() > 2
        && inside[1..inside.len() - 1].iter().all(|k| k.is_trivia())
        && inside
          .iter()
          .any(|k| matches!(k, SyntaxKind::LineComment | SyntaxKind::BlockComment))
    });
  comments_only
    || matches!(
      read_macro_arguments(call, Spacing::default(), false),
      Err(Unread::AsWritten)
    )
}

/// A text after its outer attributes, each on a line of its own at the shape's indentation
/// (the standard formatter's `combine_strs_with_missing_comments`, never extending a line).
pub(super) fn with_attributes(
  attributes: &[SyntaxNode],
  text: String,
  shape: Shape,
) -> Option<String> {
  if attributes.is_empty() {
    return Some(text);
  }
  let config = shape.limits.config();
  let mut out = String::new();
  for attribute in attributes {
    out.push_str(&super::attrs::rewrite_attr(
      attribute,
      shape.indent,
      &config,
    )?);
    out.push('\n');
    out.push_str(&" ".repeat(shape.indent));
  }
  out.push_str(&text);
  Some(out)
}

/// The shape a closure's body starts in after its `|parameters|` and a space. With the
/// parameters across lines, the standard formatter takes their last line's whole width,
/// alignment included, off the closure's shape, so that this layout fails but near the
/// start of a line.
fn after_prefix(prefix: &str, shape: Shape) -> Option<Shape> {
  shape.offset_left(last_line_width(prefix) + 1)
}

/// One operand per line after the first, its operator first, one level in; one following an
/// operand no wider than the indentation joins its line when it fits
/// (`rewrite_pairs_multiline`).
fn binary_lines(
  operands: &[SyntaxNode],
  rewrites: &[Option<String>],
  operator: &str,
  shape: Shape,
) -> Option<String> {
  let nested = shape
    .block_indent(shape.limits.tab)
    .sub_width(shape.rhs_overhead())?;
  let used = shape.indent + shape.offset;
  let mut result = rewrites[0].clone()?;
  for (operand, text) in operands[1..].iter().zip(&rewrites[1..]) {
    let offset = if result.contains('\n') { 0 } else { used };
    if last_line_width(&result) + offset <= nested.indent + nested.offset
      && let Some(line) = shape.offset_left(
        operator.width() + 2 + last_line_width(&result).saturating_sub(leading_width(&result)),
      )
      && let Some(snug) = rewrite(operand, line)
    {
      result.push(' ');
      result.push_str(operator);
      result.push(' ');
      result.push_str(&snug);
      continue;
    }
    result.push('\n');
    result.push_str(&" ".repeat(nested.indent));
    result.push_str(operator);
    result.push(' ');
    result.push_str(text.as_ref()?);
  }
  Some(result)
}

/// The operands on one line; the last may span lines only after a prefix no wider than the
/// indentation, or when it opens with `{` (`rewrite_pairs_one_line`).
fn binary_one_line(
  operands: &[SyntaxNode],
  rewrites: &[Option<String>],
  operator: &str,
  shape: Shape,
) -> Option<String> {
  let mut result = String::new();
  for text in &rewrites[..rewrites.len() - 1] {
    let text = text.as_ref()?;
    if text.contains('\n') || result.width() > shape.width {
      return None;
    }
    result.push_str(text);
    result.push(' ');
    result.push_str(operator);
    result.push(' ');
  }
  let prefix = result.width();
  let last = rewrite(
    operands.last()?,
    shape.offset_left(last_line_width(&result))?,
  )?;
  result.push_str(&last);
  if result.contains('\n')
    && !last.starts_with('{')
    && (last.starts_with('(') || prefix > shape.limits.tab)
  {
    return None;
  }
  fits_shape(result, shape)
}

/// Whether the standard formatter would put the closure's body in a block to overflow it into
/// `shape`: its `|parameters|` on the line, with a column for the space before `{`. They are
/// held to the width less 3 (its parameter shape is 4 short, and 1 for the `|`), 4 more
/// with a struct pattern's `..`; a lone name, or none, is not measured.
fn block_fits(closure: &SyntaxNode, body: &SyntaxNode, shape: Shape) -> bool {
  let spacing = shape.limits.spacing;
  let prefix = flat(closure, spacing).width() - flat(body, spacing).width();
  let params: Vec<SyntaxNode> = closure
    .children()
    .into_iter()
    .filter(|c| c.kind() == SyntaxKind::ParamList)
    .flat_map(|list| list.children())
    .collect();
  let returns = closure
    .children()
    .iter()
    .any(|c| c.kind() == SyntaxKind::RetType);
  let lone_name = params.len() == 1
    && params[0]
      .children()
      .iter()
      .map(|c| c.kind())
      .eq([SyntaxKind::IdentPat]);
  let unmeasured = !returns && (params.is_empty() || lone_name);
  let rest = params.iter().any(|p| {
    p.descendants().iter().any(|d| {
      d.kind() == SyntaxKind::RecordPatField
        && d.children().iter().any(|c| c.kind() == SyntaxKind::RestPat)
    })
  });
  // `prefix` holds the space after the last `|`; with a return type, the space before it
  // is counted where the standard formatter counts one for it.
  let needed = if unmeasured {
    prefix
  } else {
    prefix + if returns { 1 } else { 2 } + if rest { 4 } else { 0 }
  };
  needed <= shape.width
}

/// Whether the standard formatter puts a closure's body in a block whatever its width
/// (`is_block_closure_forced`): an `if`, `while` or `for`, a `loop` from style edition 2024,
/// seen through `&`, `?`, a unary operator and a cast.
fn block_forced(body: &SyntaxNode, style_2024: bool) -> bool {
  match body.kind() {
    SyntaxKind::IfExpr | SyntaxKind::WhileExpr | SyntaxKind::ForExpr => true,
    SyntaxKind::LoopExpr => style_2024,
    SyntaxKind::RefExpr | SyntaxKind::TryExpr | SyntaxKind::PrefixExpr | SyntaxKind::CastExpr => {
      body
        .children()
        .first()
        .is_some_and(|inner| block_forced(inner, style_2024))
    }
    _ => false,
  }
}

/// A macro call in braces as the standard formatter writes it, whatever its width: its name,
/// a space, and its arguments as written, the lines after the first re-indented from the
/// shape's keeping their layout (`trim_left_preserve_layout`). Not one holding a string
/// spanning lines, nor one `rustfmt::skip::macros` names.
fn braced_macro(node: &SyntaxNode, name: &str, shape: Shape) -> Option<String> {
  let call = if node.kind() == SyntaxKind::MacroExpr {
    node
      .children()
      .into_iter()
      .find(|c| c.kind() == SyntaxKind::MacroCall)?
  } else {
    node.clone()
  };
  let tree = call
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::TokenTree)?;
  // A comment outside its braces (after them, in the statement it ends) would be lost.
  let (from, to) = (tree.text_range().start, tree.text_range().end);
  let comment_outside = node.token_texts().any(|(kind, _, range)| {
    matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment)
      && (range.start < from || range.start >= to)
  });
  if comment_outside || super::file::skipped_macro(&call) {
    return None;
  }
  if let Some(body) = super::macros::lazy_static_body(&call, shape.indent, &shape.limits.config()) {
    return Some(format!("{} {}", name, body));
  }
  let text = tree.text();
  let body = super::file::preserve_layout(&text, shape.indent, &shape.limits.config());
  Some(format!("{} {}", name, body))
}

/// Whether something inside `node` is past the width the standard formatter keeps it on one
/// line within: a chain of two links or more past `chain_width`, arguments past
/// `fn_call_width`, array elements past `array_width`, fields past `struct_lit_width`.
fn breaks_inside(node: &SyntaxNode, limits: Limits) -> bool {
  node.descendants().iter().any(|d| {
    let width = || flat(d, limits.spacing).width();
    match d.kind() {
      SyntaxKind::MethodCallExpr
      | SyntaxKind::FieldExpr
      | SyntaxKind::AwaitExpr
      | SyntaxKind::TryExpr => chain(d, limits.spacing).2.len() >= 2 && width() > limits.chain,
      SyntaxKind::ArgList => width().saturating_sub(2) > limits.fn_call,
      SyntaxKind::ArrayExpr => width().saturating_sub(2) > limits.array,
      // `{ ` and ` }` aside.
      SyntaxKind::RecordExprFieldList => width().saturating_sub(4) > limits.struct_lit,
      // Laid out on lines whatever their width.
      SyntaxKind::MatchArmList | SyntaxKind::LetElse => true,
      SyntaxKind::BlockExpr => !joins(d) && !one_line_if_block(d),
      _ => false,
    }
  })
}

/// A `?` around a chain is part of the chain; around anything else it is a suffix.
fn chain_with_links(node: &SyntaxNode) -> bool {
  node.kind() == SyntaxKind::TryExpr && !chain(node, Spacing::default()).2.is_empty()
}

/// A closure's body after its `|parameters|`, on their line: spanning lines only when it is
/// block-like (`rewrite_closure_expr`).
fn closure_expression(body: &SyntaxNode, prefix: &str, shape: Shape) -> Option<String> {
  let text = rewrite(body, shape)?;
  (!text.contains('\n') || multi_line_body(body) || shape.limits.inside_macro)
    .then(|| format!("{} {}", prefix, text))
}

/// A closure's `|parameters|` (with `move`, `async` and a return type): flat, when the
/// standard formatter keeps its parameters on one line.
fn closure_prefix(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let body = &node.children().into_iter().last()?;
  // A struct pattern among its parameters past `struct_lit_width`, which the standard
  // formatter breaks a field per line, its fields one level in from the column after the
  // `|`.
  let wide = |fields: &SyntaxNode| {
    flat(fields, shape.limits.spacing).width().saturating_sub(4) > shape.limits.struct_lit
  };
  let wide_pattern = node
    .children()
    .iter()
    .filter(|c| c.kind() == SyntaxKind::ParamList)
    .flat_map(|c| c.descendants())
    .any(|d| d.kind() == SyntaxKind::RecordPatFieldList && wide(&d));
  let whole = flat(node, shape.limits.spacing);
  let prefix = whole.strip_suffix(&flat(body, shape.limits.spacing))?;
  // A parameter whose type breaks over lines: the parameters one per line, lined up after
  // the `|` (each laid out in the standard formatter's visually indented `param_shape`).
  let params: Vec<SyntaxNode> = node
    .children()
    .into_iter()
    .filter(|c| c.kind() == SyntaxKind::ParamList)
    .flat_map(|list| list.children())
    .collect();
  let opening = prefix.find('|')?;
  // Room for `|| {` and a column more, however short the closure (the standard formatter's
  // `rewrite_closure_fn_decl`).
  shape
    .offset_left(prefix[..opening].width())?
    .sub_width(4)?
    .offset_left(1)?;
  // 4 = `|| {`
  let column = shape.indent + shape.offset + prefix[..opening].width() + 1;
  let param_shape = shape
    .offset_left(prefix[..opening].width() + 1)
    .and_then(|s| s.sub_width(4));
  // A struct pattern breaks from the column after the `|` (`block_indent` keeps the
  // visual alignment); a tuple pattern's list from the closure's indentation (`block`
  // drops it).
  let shape_of = |p: &SyntaxNode, s: Shape| {
    if p
      .children_iter()
      .next()
      .is_some_and(|c| c.kind() == SyntaxKind::RecordPat)
    {
      Shape {
        indent: column,
        offset: 0,
        ..s
      }
    } else {
      s
    }
  };
  if !params.is_empty()
    && let Some(param_shape) = param_shape
    && !node
      .children()
      .iter()
      .any(|c| c.kind() == SyntaxKind::RetType)
  {
    // A parameter's attributes, each on a line of its own, line up after the `|` too; what
    // breaks in the parameter itself stays at the closure's indentation.
    let texts: Option<Vec<String>> = params
      .iter()
      .map(|p| {
        let text = super::types::rewrite_param(p, shape_of(p, param_shape))?;
        let attributes = p
          .children_with_tokens_iter()
          .filter(|e| e.kind() == SyntaxKind::Attr || super::items::is_doc(e.kind()))
          .count();
        if attributes == 0 || !text.contains('\n') {
          return Some(text);
        }
        let old = format!("\n{}", " ".repeat(param_shape.indent));
        let new = format!("\n{}", " ".repeat(column));
        Some(text.replacen(&old, &new, attributes))
      })
      .collect();
    if let Some(texts) = texts
      && texts.iter().any(|t| t.contains('\n'))
    {
      let sep = format!(",\n{}", " ".repeat(column));
      return Some(format!("{}{}|", &prefix[..=opening], texts.join(&sep)));
    }
  }
  if wide_pattern {
    return None;
  }
  if block_fits(node, body, shape) {
    return Some(prefix.trim_end().to_string());
  }
  // Its parameters one per line, each after the first lined up after its `|` (the
  // standard formatter's vertical tactic, visually indented); not with a return type.
  let params: Vec<SyntaxNode> = node
    .children()
    .into_iter()
    .filter(|c| c.kind() == SyntaxKind::ParamList)
    .flat_map(|list| list.children())
    .collect();
  if params.len() < 2 {
    return None;
  }
  let ret = node
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::RetType);
  let opening = prefix.find('|')?;
  let column = shape.indent + shape.offset + prefix[..opening].width() + 1;
  let mut text = prefix[..=opening].to_string();
  for (i, param) in params.iter().enumerate() {
    let param = flat(param, shape.limits.spacing);
    // Room for its `,` or the closing `|` and ` {`.
    if column + param.width() + 4 > shape.limits.max_width {
      return None;
    }
    if i > 0 {
      text.push_str(",\n");
      // A blank line written before it, after the comma, stays.
      let blank = params[i]
        .prev_sibling_or_token()
        .is_some_and(|e| matches!(&e, SyntaxElement::Token(t) if t.kind() == SyntaxKind::Whitespace && t.text().matches('\n').count() > 1));
      if blank {
        text.push('\n');
      }
      text.push_str(&" ".repeat(column));
    }
    text.push_str(&param);
  }
  text.push('|');
  // A return type on a line of its own, at the closure's indentation and the columns up to
  // its `|` (`rewrite_closure_fn_decl`'s `argument_offset`).
  if let Some(ret) = ret {
    let at = shape.indent + prefix[..=opening].width();
    let ret = flat(&ret, shape.limits.spacing);
    if at + ret.width() + 2 > shape.limits.max_width {
      return None;
    }
    text.push_str(&format!("\n{}{}", " ".repeat(at), ret));
  }
  Some(text)
}

/// A closure's body put in a block of its own (`rewrite_closure_with_block`); not a call,
/// operation, cast, field, index, range or `?` starting with a block-like expression.
fn closure_with_block(body: &SyntaxNode, prefix: &str, shape: Shape) -> Option<String> {
  let mut left = body.clone();
  while let Some(first) = left.children().into_iter().next()
    && matches!(
      left.kind(),
      SyntaxKind::CallExpr
        | SyntaxKind::BinExpr
        | SyntaxKind::CastExpr
        | SyntaxKind::FieldExpr
        | SyntaxKind::IndexExpr
        | SyntaxKind::RangeExpr
        | SyntaxKind::TryExpr
        | SyntaxKind::MethodCallExpr
    )
  {
    left = first;
  }
  let vetoed = matches!(
    body.kind(),
    SyntaxKind::CallExpr
      | SyntaxKind::BinExpr
      | SyntaxKind::CastExpr
      | SyntaxKind::FieldExpr
      | SyntaxKind::IndexExpr
      | SyntaxKind::RangeExpr
      | SyntaxKind::TryExpr
  ) && matches!(
    left.kind(),
    SyntaxKind::IfExpr
      | SyntaxKind::MatchExpr
      | SyntaxKind::BlockExpr
      | SyntaxKind::LoopExpr
      | SyntaxKind::WhileExpr
      | SyntaxKind::ForExpr
  );
  // An `async` block, seen through `&` and unary operators, the standard formatter puts in
  // no block either (found by measuring it).
  let mut inner = body.clone();
  while matches!(inner.kind(), SyntaxKind::RefExpr | SyntaxKind::PrefixExpr)
    && let Some(operand) = inner.children().into_iter().next()
  {
    inner = operand;
  }
  let asynchronous = inner.kind() == SyntaxKind::BlockExpr
    && inner
      .children_with_tokens_iter()
      .any(|e| e.kind() == SyntaxKind::AsyncKw);
  // A macro called with nothing the standard formatter puts in no block (found by
  // measuring it).
  if vetoed || asynchronous || empty_macro(body) {
    return None;
  }
  let block = super::block::wrapped(body, shape)?;
  Some(format!("{} {}", prefix, block))
}

/// Whether a comma is written after `node`, comments and whitespace aside.
fn comma_after(node: &SyntaxNode) -> bool {
  let mut next = node.next_sibling_or_token();
  while let Some(SyntaxElement::Token(token)) = next {
    if !token.kind().is_trivia() {
      return token.kind() == SyntaxKind::Comma;
    }
    next = token.next_sibling_or_token();
  }
  false
}

fn definitive(items: &[String], limit: usize) -> Tactic {
  let total: usize =
    items.iter().map(|i| i.width()).sum::<usize>() + 2 * items.len().saturating_sub(1);
  if total <= limit && !items.iter().any(|i| i.contains('\n')) {
    Tactic::Horizontal
  } else {
    Tactic::Vertical
  }
}

/// The attributes written just before a list's element, which the parser makes siblings of
/// it; `None` when a comment stands among them.
fn element_attributes(node: &SyntaxNode) -> Option<Vec<SyntaxNode>> {
  let mut attributes = Vec::new();
  if !node.parent().is_some_and(|p| {
    matches!(
      p.kind(),
      SyntaxKind::ArrayExpr | SyntaxKind::ArgList | SyntaxKind::TupleExpr
    )
  }) {
    return Some(attributes);
  }
  let mut before = node.prev_sibling_or_token();
  let mut comment = false;
  while let Some(element) = before {
    match &element {
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Whitespace => {}
      // A comment before the element's attributes is the list's to place.
      SyntaxElement::Token(t) if t.kind().is_trivia() => comment = true,
      SyntaxElement::Node(n) if n.kind() == SyntaxKind::Attr && comment => return None,
      SyntaxElement::Node(n) if n.kind() == SyntaxKind::Attr => attributes.push(n.clone()),
      _ => break,
    }
    before = match &element {
      SyntaxElement::Token(t) => t.prev_sibling_or_token(),
      SyntaxElement::Node(n) => n.prev_sibling_or_token(),
    };
  }
  attributes.reverse();
  Some(attributes)
}

/// A `<` or `>` of an empty generic list (`impl<> Foo<> for Bar<>`), which says nothing: an
/// item's parameters or a path segment's arguments, not a binder's (`for<>` needs them).
fn empty_generics(token: &SyntaxToken) -> bool {
  let list = token.parent();
  matches!(token.kind(), SyntaxKind::Lt | SyntaxKind::Gt)
    && match list.kind() {
      SyntaxKind::GenericParamList => list
        .parent()
        .is_some_and(|owner| super::items::is_item(owner.kind())),
      SyntaxKind::GenericArgList => list
        .parent()
        .is_some_and(|owner| owner.kind() == SyntaxKind::PathSegment),
      _ => false,
    }
    && token.parent().children_iter().next().is_none()
    && !token.parent().children_with_tokens_iter().any(|e| {
      matches!(
        e.kind(),
        SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::Colon2
      )
    })
}

/// Whether `node` is a macro called with nothing (`vec![]`, `f!()`), seen through `&`, `?`,
/// a unary operator and a cast.
fn empty_macro(node: &SyntaxNode) -> bool {
  match node.kind() {
    SyntaxKind::MacroExpr | SyntaxKind::MacroCall => node
      .descendants()
      .into_iter()
      .find(|d| d.kind() == SyntaxKind::TokenTree)
      .is_some_and(|tree| {
        tree
          .children_with_tokens()
          .iter()
          .filter(|e| !e.kind().is_trivia())
          .count()
          == 2
      }),
    SyntaxKind::RefExpr | SyntaxKind::TryExpr | SyntaxKind::PrefixExpr | SyntaxKind::CastExpr => {
      node.children().first().is_some_and(empty_macro)
    }
    _ => false,
  }
}

/// Whether a line ends in a line comment: a `//` outside a string, a character or a block
/// comment, and not a link's `://`.
fn ends_in_line_comment(line: &str) -> bool {
  let bytes = line.as_bytes();
  // Inside a string: its closing quote, its `#`s and whether it is raw.
  let mut quote: Option<(u8, usize, bool)> = None;
  let mut block = false;
  let mut i = 0;
  while i < bytes.len() {
    let b = bytes[i];
    if block {
      if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
        block = false;
        i += 1;
      }
    } else if let Some((q, hashes, raw)) = quote {
      if !raw && b == b'\\' {
        i += 1;
      } else if b == q && bytes[i + 1..].iter().take_while(|&&h| h == b'#').count() >= hashes {
        quote = None;
        i += hashes;
      }
    } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') && (i == 0 || bytes[i - 1] != b':') {
      return true;
    } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
      block = true;
      i += 1;
    } else if b == b'"' {
      // A raw string's `r` and `#`s before it.
      let hashes = bytes[..i].iter().rev().take_while(|&&h| h == b'#').count();
      let raw = i > hashes && bytes[i - hashes - 1] == b'r';
      quote = Some((b'"', hashes, raw));
    } else if b == b'\'' && (bytes.get(i + 2) == Some(&b'\'') || bytes.get(i + 1) == Some(&b'\\')) {
      quote = Some((b'\'', 0, false));
    }
    i += 1;
  }
  false
}

/// Whether `text` fits as the standard formatter's `wrap_str` checks it: its first line in
/// the shape, the others within the line width, its last within the shape's end.
fn fits_shape(text: String, shape: Shape) -> Option<String> {
  let mut lines = text.lines();
  if lines.next().is_some_and(|l| l.width() > shape.width) {
    return None;
  }
  if !text.contains('\n') {
    return Some(text);
  }
  if later_lines(&text).any(|(_, w)| w > shape.limits.max_width)
    || last_line_width(&text) > shape.indent + shape.offset + shape.width
  {
    return None;
  }
  Some(text)
}

/// Whether an `if` stands where the standard formatter may write it on one line: not as a
/// statement, and as a block's tail only from style edition 2024 (before, a tail is a
/// statement too).
fn if_on_one_line(node: &SyntaxNode, limits: Limits) -> bool {
  match node.parent() {
    // A match arm's body is laid out as a statement (`ExprType::Statement`).
    Some(p) if matches!(p.kind(), SyntaxKind::ExprStmt | SyntaxKind::MatchArm) => false,
    // As a block's tail it is a statement before style edition 2024, an expression from
    // it; taken out of a closure's block it is an expression, out of a match arm's a
    // statement.
    Some(p) if p.kind() == SyntaxKind::BlockExpr => {
      let unwrapped = !limits.block_tail && plain_block(&p) && lone_expression(&p).is_some();
      match p.parent() {
        Some(owner) if owner.kind() == SyntaxKind::ClosureExpr && unwrapped => true,
        Some(owner) if owner.kind() == SyntaxKind::MatchArm && unwrapped => false,
        _ => limits.style_2024,
      }
    }
    Some(_) => true,
    None => false,
  }
}

/// For each line of `text`, whether it starts inside a raw string (or, from style edition
/// 2024, a string continued with `\`).
fn in_raw_string(text: &str, style_2024: bool) -> Vec<bool> {
  let mut starts = vec![false];
  for (kind, piece) in crate::syntax::parse_tokens(text, Edition::E2021) {
    // From style edition 2024 so are a string's whose lines but the last end with `\`.
    let raw = matches!(
      kind,
      SyntaxKind::RawStr | SyntaxKind::RawByteStr | SyntaxKind::RawCStr
    ) || style_2024
      && kind == SyntaxKind::Str
      && piece.lines().rev().skip(1).all(|line| line.ends_with('\\'));
    for _ in piece.matches('\n') {
      starts.push(raw);
    }
  }
  starts
}

/// Whether a chain's receiver, printed as `text`, ends like a block, so that the chain's
/// links stay at its indentation rather than one level in (the standard formatter's
/// `is_block_expr`): a call, array, struct literal, macro or block-like expression that
/// spans lines, seen through parentheses, binary operators, indexing, unary operators,
/// closures and `?`; a string spanning lines whose last line is short.
fn is_block_expr(node: &SyntaxNode, text: &str) -> bool {
  match node.kind() {
    SyntaxKind::MacroExpr
    | SyntaxKind::CallExpr
    | SyntaxKind::MethodCallExpr
    | SyntaxKind::ArrayExpr
    | SyntaxKind::RecordExpr
    | SyntaxKind::WhileExpr
    | SyntaxKind::IfExpr
    | SyntaxKind::BlockExpr
    | SyntaxKind::LoopExpr
    | SyntaxKind::ForExpr
    | SyntaxKind::MatchExpr => text.contains('\n'),
    SyntaxKind::ParenExpr
    | SyntaxKind::BinExpr
    | SyntaxKind::IndexExpr
    | SyntaxKind::PrefixExpr
    | SyntaxKind::ClosureExpr
    | SyntaxKind::TryExpr => node
      .children()
      .into_iter()
      .last()
      .is_some_and(|inner| is_block_expr(&inner, text)),
    SyntaxKind::Literal => {
      text.contains('\n')
        && text
          .rsplit('\n')
          .next()
          .is_some_and(|l| l.trim().width() <= 4)
    }
    _ => false,
  }
}

/// A method call, or one under `&`, `?`, a cast or a unary operator.
fn is_method_call(node: &SyntaxNode) -> bool {
  match node.kind() {
    SyntaxKind::MethodCallExpr => true,
    SyntaxKind::RefExpr | SyntaxKind::TryExpr | SyntaxKind::CastExpr | SyntaxKind::PrefixExpr => {
      node.children().first().is_some_and(is_method_call)
    }
    _ => false,
  }
}

/// A call or macro call, or one under `&`, `?`, a cast or a unary operator (the standard
/// formatter's `is_nested_call`; a method call is not one).
fn is_nested_call(node: &SyntaxNode) -> bool {
  match node.kind() {
    SyntaxKind::CallExpr | SyntaxKind::MacroExpr => true,
    SyntaxKind::RefExpr | SyntaxKind::TryExpr | SyntaxKind::CastExpr | SyntaxKind::PrefixExpr => {
      node.children().first().is_some_and(is_nested_call)
    }
    _ => false,
  }
}

/// The `in` of `pub(in crate)`, `pub(in super)` or `pub(in self)`, which the standard
/// formatter drops.
fn keyword_visibility_in(token: &SyntaxToken) -> bool {
  token.kind() == SyntaxKind::InKw
    && token.parent().kind() == SyntaxKind::Visibility
    && token
      .parent()
      .children_iter()
      .find(|c| c.kind() == SyntaxKind::Path)
      .is_some_and(|path| {
        let text: String = path.text().to_string().split_whitespace().collect();
        matches!(text.as_str(), "crate" | "super" | "self")
      })
}

/// The shape the overflowing last item gets: after the other items on the opening line.
fn last_item_shape(
  items: &[SyntaxNode],
  texts: &[String],
  shape: Shape,
  max: usize,
) -> Option<Shape> {
  // An attribute's nested list is not an expression, so not a nested call.
  if items.len() == 1 && (shape.limits.attribute || !is_nested_call(&items[0])) {
    return Some(shape);
  }
  let offset: usize = texts[..texts.len() - 1].iter().map(|t| 2 + t.len()).sum();
  Shape {
    width: max.min(shape.width),
    ..shape
  }
  .offset_left(offset)
}

/// The width of the indentation `text`'s last line starts with.
fn leading_width(text: &str) -> usize {
  let last = text.rsplit('\n').next().unwrap_or("");
  last.width() - last.trim_start().width()
}

/// For each line of `text`, whether it is a block comment's own: it starts inside one, and
/// that comment ends on it with nothing after, or goes on past it (the standard formatter's
/// `filter_normal_code` leaves such lines out). Strings and line comments are skipped.
fn lines_in_block_comments(text: &str) -> Vec<bool> {
  let mut out = Vec::new();
  let mut depth = 0usize;
  let mut in_string = false;
  for line in text.split('\n') {
    let starts_inside = depth > 0;
    let bytes = line.as_bytes();
    let mut i = 0;
    // Where the comment the line starts in closes.
    let mut closed_at = None;
    while i < bytes.len() {
      let pair = bytes.get(i..i + 2);
      if in_string {
        match bytes[i] {
          b'\\' => i += 1,
          b'"' => in_string = false,
          _ => {}
        }
      } else if depth > 0 {
        if pair == Some(b"*/") {
          depth -= 1;
          i += 1;
          if depth == 0 && starts_inside && closed_at.is_none() {
            closed_at = Some(i + 1);
          }
        } else if pair == Some(b"/*") {
          depth += 1;
          i += 1;
        }
      } else if pair == Some(b"//") {
        break;
      } else if pair == Some(b"/*") {
        depth += 1;
        i += 1;
      } else if bytes[i] == b'"'
        && !(i > 0 && bytes[i - 1] == b'\'' && bytes.get(i + 1) == Some(&b'\''))
      {
        in_string = true;
      }
      i += 1;
    }
    out.push(
      starts_inside
        && match closed_at {
          None => true,
          Some(at) => line[at.min(line.len())..].trim().is_empty(),
        },
    );
  }
  out
}

/// Whether every branch of an `if` (through its `else if`s) holds one expression.
fn lone_branches(node: &SyntaxNode) -> bool {
  let children = node.children();
  match children.as_slice() {
    [_, then] => lone_expression(then).is_some(),
    [_, then, otherwise] => {
      lone_expression(then).is_some()
        && if otherwise.kind() == SyntaxKind::IfExpr {
          lone_branches(otherwise)
        } else {
          lone_expression(otherwise).is_some()
        }
    }
    _ => false,
  }
}

fn macro_arguments(
  node: &SyntaxNode,
  spacing: Spacing,
  nested: bool,
) -> Result<MacroArguments, Unread> {
  let read = read_macro_arguments(node, spacing, nested);
  if matches!(read, Err(Unread::AsWritten)) && super::explain::active() {
    super::explain::note("MACRO_CALL", &node.text().to_string(), || {
      "macro arguments not read as expressions or a list, so kept as written".to_string()
    });
  }
  read
}

/// A macro call whose argument in parentheses or brackets is an item alone (one not holding
/// items), as the standard formatter reads an argument that is no expression, type or
/// pattern: the item formatted as a file's on a line of its own one level in, the closing
/// delimiter on the next.
fn macro_item_argument(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let call = if node.kind() == SyntaxKind::MacroExpr {
    node
      .children_iter()
      .find(|c| c.kind() == SyntaxKind::MacroCall)?
  } else {
    node.clone()
  };
  let tree = call
    .children_iter()
    .find(|c| c.kind() == SyntaxKind::TokenTree)?;
  let path = call
    .children_iter()
    .find(|c| c.kind() == SyntaxKind::Path)?;
  if tree
    .token_texts()
    .any(|(kind, _, _)| kind.is_trivia() && kind != SyntaxKind::Whitespace)
  {
    return None;
  }
  let vec = path.text().to_string().trim() == "vec";
  let (open, close) = match tree.first_token()?.kind() {
    // `vec!` in brackets whatever it is written in (`FORCED_BRACKET_MACROS`).
    SyntaxKind::LParen | SyntaxKind::LBrace if vec => ("[", "]"),
    SyntaxKind::LParen => ("(", ")"),
    SyntaxKind::LBrack => ("[", "]"),
    _ => return None,
  };
  let text = tree.text();
  let inner = text[1..text.len() - 1].trim();
  let parsed = crate::syntax::parse(inner, Edition::E2021);
  if !parsed.errors().is_empty() {
    return None;
  }
  let root = parsed.syntax_node();
  let items: Vec<SyntaxNode> = root.children();
  // One holding items (an `impl`, a trait, a module) the standard formatter keeps as
  // written (found by measuring it).
  if items.len() != 1
    || !(super::items::is_item(items[0].kind()) || items[0].kind() == SyntaxKind::Use)
    || matches!(
      items[0].kind(),
      SyntaxKind::Impl | SyntaxKind::Trait | SyntaxKind::Module | SyntaxKind::ExternBlock
    )
  {
    return None;
  }
  let nested = shape.indent + shape.limits.tab;
  let body = super::macros::format_item(inner, nested, &shape.limits.config())?;
  Some(format!(
    "{}!{}\n{}\n{}{}",
    flat(&path, shape.limits.spacing),
    open,
    body,
    " ".repeat(shape.indent),
    close
  ))
}

/// A macro call's arguments read one by one, as the standard formatter's
/// `parse_macro_args` reads them: each an expression, else a type (`?name`, a bare trait
/// object), else a pattern. The arguments as nodes of their own parses, and whether a comma
/// follows the last; `None` when one is none of these, or a comment is among them.
fn macro_mixed_arguments(
  node: &SyntaxNode,
  spacing: Spacing,
  nested: bool,
) -> Option<(String, Vec<SyntaxNode>, &'static str, &'static str, bool)> {
  let call = if node.kind() == SyntaxKind::MacroExpr {
    node
      .children_iter()
      .find(|c| c.kind() == SyntaxKind::MacroCall)?
  } else {
    node.clone()
  };
  let tree = call
    .children_iter()
    .find(|c| c.kind() == SyntaxKind::TokenTree)?;
  let path = call
    .children_iter()
    .find(|c| c.kind() == SyntaxKind::Path)?;
  let name = format!("{}!", flat(&path, spacing));
  if tree
    .token_texts()
    .any(|(kind, _, _)| kind.is_trivia() && kind != SyntaxKind::Whitespace)
  {
    return None;
  }
  let elements: Vec<SyntaxElement> = tree.children_with_tokens();
  let (open, close) = match elements.first()?.kind() {
    SyntaxKind::LParen if name != "vec!" || nested => ("(", ")"),
    SyntaxKind::LBrack => ("[", "]"),
    _ => return None,
  };
  // The text between the top-level commas.
  let mut pieces = vec![String::new()];
  for element in &elements[1..elements.len() - 1] {
    match element {
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Comma => pieces.push(String::new()),
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Semi => return None,
      SyntaxElement::Token(t) => pieces.last_mut()?.push_str(t.text()),
      SyntaxElement::Node(n) => pieces.last_mut()?.push_str(&n.text()),
    }
  }
  let trailing = pieces.len() > 1 && pieces.last()?.trim().is_empty();
  if trailing {
    pieces.pop();
  }
  let last = pieces.len() - 1;
  let mut items = Vec::with_capacity(pieces.len());
  for (i, piece) in pieces.iter().enumerate() {
    if piece.trim().is_empty() {
      return None;
    }
    let comma = if i == last && trailing { "," } else { "" };
    let parsed = |source: String, list: SyntaxKind| {
      let parsed = crate::syntax::parse(&source, Edition::E2021);
      if !parsed.errors().is_empty() {
        return None;
      }
      let list = parsed
        .syntax_node()
        .descendant_nodes_where(|kind| kind == list)
        .next()?;
      // A pattern's path is not one of its fields.
      let items: Vec<SyntaxNode> = list
        .children_iter()
        .filter(|c| c.kind() != SyntaxKind::Path)
        .collect();
      match items.as_slice() {
        [item] if !rustc_rejects(&list) => Some(item.clone()),
        _ => None,
      }
    };
    let expression = parsed(
      format!("fn f() {{\n__mto_macro({}{});\n}}", piece, comma),
      SyntaxKind::ArgList,
    )
    .filter(|e| {
      !e.subtree_has(|kind| kind == SyntaxKind::Attr)
        && !rustc_rejects(&e.parent().unwrap_or_else(|| e.clone()))
    });
    let item = expression
      .or_else(|| {
        parsed(
          format!("type __Mto = __mto<{}{}>;", piece, comma),
          SyntaxKind::GenericArgList,
        )
        .filter(|a| a.kind() == SyntaxKind::TypeArg)
      })
      .or_else(|| {
        parsed(
          format!(
            "fn f() {{\nmatch x {{\n__mto_macro({}{}) => {{}}\n}}\n}}",
            piece, comma
          ),
          SyntaxKind::TupleStructPat,
        )
        // Not a binding (`a @ ..`): its name reads as an expression, which the standard
        // formatter takes and then fails on the `@`.
        .filter(|p| {
          p.kind() != SyntaxKind::OrPat
            && !(p.kind() == SyntaxKind::IdentPat
              && p
                .children_with_tokens_iter()
                .any(|e| e.kind() == SyntaxKind::At))
        })
      })?;
    items.push(item);
  }
  Some((name, items, open, close, open == "["))
}

/// A macro call's arguments in parentheses read as types (`m!(Range<usize>, Vec<u8>)`), as
/// the standard formatter reads an argument that is no expression; every one must be a type.
fn macro_type_arguments(node: &SyntaxNode, spacing: Spacing) -> Option<MacroArguments> {
  let call = if node.kind() == SyntaxKind::MacroExpr {
    node
      .children()
      .into_iter()
      .find(|c| c.kind() == SyntaxKind::MacroCall)?
  } else {
    node.clone()
  };
  let tree = call
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::TokenTree)?;
  let tokens: Vec<SyntaxKind> = tree.token_texts().map(|(kind, _, _)| kind).collect();
  if tokens.first() != Some(&SyntaxKind::LParen)
    || tokens
      .iter()
      .any(|k| k.is_trivia() && *k != SyntaxKind::Whitespace)
  {
    return None;
  }
  let path = call
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::Path)?;
  let name = format!("{}!", flat(&path, spacing));
  let text = tree.text();
  let source = format!("type __Mto = __mto<{}>;", &text[1..text.len() - 1]);
  let parsed = crate::syntax::parse(&source, Edition::E2021);
  if !parsed.errors().is_empty() {
    return None;
  }
  let list = parsed
    .syntax_node()
    .descendants()
    .into_iter()
    .find(|n| n.kind() == SyntaxKind::GenericArgList)?;
  // One holding a comma (`name<A, B>`) the standard formatter reads as an expression up to
  // that comma first, and then fails on the rest: it is left as written. But where a
  // lifetime comes first (`Cow<'a, T>`), or anything else no expression reads comes before
  // the comma (`&F<dyn Fn() -> R<A, B>>`), it is read as a type.
  let no_expression = |arg: &SyntaxNode| {
    let text = flat(arg, spacing);
    let mut depth = 0i32;
    let comma = text.char_indices().find(|&(_, c)| {
      match c {
        '(' | '[' | '{' => depth += 1,
        ')' | ']' | '}' => depth -= 1,
        _ => {}
      }
      c == ',' && depth == 0
    });
    comma.is_some_and(|(at, _)| {
      let source = format!("fn f() {{\n{};\n}}", &text[..at]);
      !crate::syntax::parse(&source, Edition::E2021)
        .errors()
        .is_empty()
    })
  };
  let lifetime_first = |arg: &SyntaxNode| {
    arg
      .descendant_nodes_where(|kind| kind == SyntaxKind::GenericArgList)
      .next()
      .and_then(|args| args.children_iter().next())
      .is_some_and(|first| first.kind() == SyntaxKind::LifetimeArg)
  };
  list
    .children()
    .iter()
    .all(|arg| {
      arg.kind() == SyntaxKind::TypeArg
        && (!flat(arg, spacing).contains(',') || lifetime_first(arg) || no_expression(arg))
    })
    .then_some(())
    .filter(|()| !rustc_rejects(&list))
    .map(|()| MacroArguments {
      name,
      list,
      open: "(",
      close: ")",
      array: false,
    })
}

/// Whether a closure's body may span lines without a block of its own: a struct literal, a
/// block (not a `const` one), a `match` or a `loop`, seen through `&`, `?`, a unary
/// operator and a cast (the standard formatter's `allow_multi_line`).
fn multi_line_body(body: &SyntaxNode) -> bool {
  match body.kind() {
    SyntaxKind::BlockExpr => !body
      .children_with_tokens_iter()
      .any(|e| e.kind() == SyntaxKind::ConstKw),
    SyntaxKind::RecordExpr | SyntaxKind::MatchExpr | SyntaxKind::LoopExpr => true,
    SyntaxKind::RefExpr | SyntaxKind::TryExpr | SyntaxKind::PrefixExpr | SyntaxKind::CastExpr => {
      body.children().first().is_some_and(multi_line_body)
    }
    _ => false,
  }
}

/// Parentheses holding no comment and no attribute of their own around parenthesized
/// expression, which drop (`rewrite_paren`): `(((a)))` is `(a)`, `(/* c */((a)))` is
/// `(/* c */(a))`.
fn nested_paren(node: &SyntaxNode) -> bool {
  node.kind() == SyntaxKind::ParenExpr
    && node.children_with_tokens_iter().all(|e| {
      !matches!(
        e.kind(),
        SyntaxKind::LineComment | SyntaxKind::BlockComment | SyntaxKind::Attr
      )
    })
    && node
      .children_iter()
      .next()
      .is_some_and(|inner| inner.kind() == SyntaxKind::ParenExpr)
}

/// Whether a comma is a one-element tuple's, which it makes a tuple.
fn one_element_tuple(comma: &SyntaxToken) -> bool {
  let parent = comma.parent();
  matches!(
    parent.kind(),
    SyntaxKind::TupleExpr
      | SyntaxKind::TupleType
      | SyntaxKind::TuplePat
      | SyntaxKind::TupleStructPat
  ) && parent.children().len() == 1
}

/// [`macro_arguments`], without the note.
fn read_macro_arguments(
  node: &SyntaxNode,
  spacing: Spacing,
  nested: bool,
) -> Result<MacroArguments, Unread> {
  use Unread::{AsWritten, Declined};
  let call = if node.kind() == SyntaxKind::MacroExpr {
    node
      .children()
      .into_iter()
      .find(|c| c.kind() == SyntaxKind::MacroCall)
      .ok_or(Declined)?
  } else {
    node.clone()
  };
  let tree = call
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::TokenTree)
    .ok_or(Declined)?;
  let path = call
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::Path)
    .ok_or(Declined)?;
  let name = format!("{}!", flat(&path, spacing));
  // In braces, kept as written: comments in them too. But `vec! {..}`, which the standard
  // formatter writes as `vec![..]` (`FORCED_BRACKET_MACROS`).
  let braced = tree
    .first_token()
    .is_some_and(|t| t.kind() == SyntaxKind::LBrace);
  if braced && name != "vec!" {
    return Err(Unread::Braced(name));
  }
  // Comments on a line, which the arguments read again as code place (between the
  // arguments, in a block or another list); the statement's check makes sure each is kept.
  if tree.descendants_with_tokens().iter().any(|e| match e {
    SyntaxElement::Token(t) if t.kind().is_trivia() && t.kind() != SyntaxKind::Whitespace => {
      t.kind() == SyntaxKind::BlockComment && t.text().contains('\n')
    }
    _ => false,
  }) {
    return Err(Declined);
  }
  let mut inner = tree.text().to_string();
  let mut delimiter = tree
    .children_with_tokens()
    .into_iter()
    .find(|e| !e.kind().is_trivia())
    .ok_or(Declined)?
    .kind();
  // `vec!(..)` and `vec! {..}` are written `vec![..]`, but in another macro's arguments
  // (`is_nested_macro`).
  if name == "vec!" && nested && delimiter == SyntaxKind::LBrace {
    return Err(Unread::Braced(name));
  }
  if name == "vec!" && !nested && matches!(delimiter, SyntaxKind::LParen | SyntaxKind::LBrace) {
    if inner.len() < 2 {
      return Err(Declined);
    }
    inner = format!("[{}]", &inner[1..inner.len() - 1]);
    delimiter = SyntaxKind::LBrack;
  }
  let (source, list_kind, open, close, array) = match delimiter {
    SyntaxKind::LParen if name != "vec!" || nested => (
      format!("fn f() {{\n__mto_macro{};\n}}", inner),
      SyntaxKind::ArgList,
      "(",
      ")",
      false,
    ),
    // `[x; n]` only for `vec!` itself (`FORCED_BRACKET_MACROS`): `alloc::vec![0; k]` is
    // left as written.
    SyntaxKind::LBrack
      if name != "vec!"
        && tree
          .children_with_tokens()
          .iter()
          .any(|e| e.kind() == SyntaxKind::Semi) =>
    {
      return Err(AsWritten);
    }
    SyntaxKind::LBrack => (
      format!("fn f() {{\n{};\n}}", inner),
      SyntaxKind::ArrayExpr,
      "[",
      "]",
      true,
    ),
    _ => return Err(AsWritten),
  };
  let parsed = crate::syntax::parse(&source, Edition::E2021);
  if !parsed.errors().is_empty() {
    return Err(AsWritten);
  }
  let root = parsed.syntax_node();
  let statement = root
    .descendants()
    .into_iter()
    .find(|n| n.kind() == SyntaxKind::ExprStmt)
    .ok_or(AsWritten)?;
  let expression = statement.children().into_iter().next().ok_or(AsWritten)?;
  let list = match list_kind {
    SyntaxKind::ArgList => {
      if expression.kind() != SyntaxKind::CallExpr {
        return Err(AsWritten);
      }
      expression
        .children()
        .into_iter()
        .find(|c| c.kind() == SyntaxKind::ArgList)
        .ok_or(AsWritten)?
    }
    _ => {
      if expression.kind() != SyntaxKind::ArrayExpr {
        return Err(AsWritten);
      }
      expression
    }
  };
  if rustc_rejects(&list) {
    return Err(AsWritten);
  }
  Ok(MacroArguments {
    name,
    list,
    open,
    close,
    array,
  })
}

/// The text laid out for `node`, or the node as written when that lost a comment inside it:
/// the standard formatter's `recover_comment_removed`, which keeps an expression whose
/// comments it cannot place as written while laying out what is around it.
fn recover_comments(node: &SyntaxNode, text: String) -> String {
  // Only a binary expression's, where no comment is placed here; elsewhere one that would
  // be lost fails the layout, which the whole-file pass then places.
  if node.kind() != SyntaxKind::BinExpr
    || !node.subtree_has(|kind| matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment))
  {
    return text;
  }
  let mut needed: HashMap<&str, usize> = HashMap::new();
  for (kind, comment, _) in node.token_texts() {
    if matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment) {
      *needed.entry(comment.trim_end()).or_default() += 1;
    }
  }
  if needed
    .iter()
    .all(|(comment, &count)| text.matches(comment).count() >= count)
  {
    text
  } else {
    node.text().to_string()
  }
}

fn rewrite_binary(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let operator = binary_operator(node)?;
  // The operands, leftmost first, flattened through left operands with the same operator.
  let mut operands = Vec::new();
  let mut current = node.clone();
  loop {
    let children = current.children();
    let [left, right] = children.as_slice() else {
      return None;
    };
    operands.push(right.clone());
    if binary_operator(left).as_deref() == Some(operator.as_str()) {
      current = left.clone();
    } else {
      operands.push(left.clone());
      break;
    }
  }
  operands.reverse();
  let separator = operator.width();
  // `let` among the operands (a let chain) breaks them one per line, but for a lone name and
  // one `let` (`x && let Some(y) = z`).
  let lets = operands.iter().any(|o| o.kind() == SyntaxKind::LetExpr);
  // A name, maybe under a unary operator (the standard formatter's `is_ident`).
  fn is_ident(node: &SyntaxNode) -> bool {
    match node.kind() {
      SyntaxKind::PathExpr => !flat(node, Spacing::default()).contains("::"),
      SyntaxKind::PrefixExpr => node.children().last().is_some_and(is_ident),
      _ => false,
    }
  }
  let one_line_lets =
    operands.len() == 2 && is_ident(&operands[0]) && operands[1].kind() == SyntaxKind::LetExpr;
  // Each operand on its own: the first in the shape, the others one level in after the
  // operator and its space, with room for what follows the expression.
  let rewrites: Vec<Option<String>> = operands
    .iter()
    .enumerate()
    .map(|(i, operand)| {
      if i == 0 {
        return rewrite(operand, shape);
      }
      let nested = shape
        .block_indent(shape.limits.tab)
        .sub_width(shape.rhs_overhead())?
        .offset_left(separator + 1)?;
      rewrite(operand, nested)
    })
    .collect();
  if lets && !one_line_lets {
    return binary_lines(&operands, &rewrites, &operator, shape);
  }
  binary_one_line(&operands, &rewrites, &operator, shape)
    .or_else(|| binary_lines(&operands, &rewrites, &operator, shape))
    .or_else(|| {
      let children = node.children();
      rewrite_pair(
        &children[0],
        &children[1],
        &format!(" {} ", operator),
        shape,
      )
    })
}

/// A block of one expression ([`joins`]): `{ expr }` on one line when it fits there, else
/// the expression on a line of its own, one level in.
fn rewrite_block(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let inside = lone_expression(node)?;
  let whole = flat(node, shape.limits.spacing);
  let prefix = &whole[..whole.find('{')?];
  // 4 = `{ ` and ` }`; the whole within the width, which a string kept as written may not
  // be (`rewrite_single_line_block`).
  if let Some(text) = shape
    .offset_left(prefix.width() + 2)
    .and_then(|s| s.sub_width(2))
    .and_then(|s| rewrite(&inside, s))
    .filter(|t| !t.contains('\n') && prefix.width() + 4 + t.width() <= shape.width)
  {
    return Some(format!("{}{{ {} }}", prefix, text));
  }
  if prefix.width() + 1 > shape.width {
    return None;
  }
  let nested = shape.block_inside();
  // One the standard formatter gives up on is kept as written in the block (its visitor's
  // `push_rewrite` given nothing): on one line only, as one written across lines would
  // weigh the layouts around it by how it was written, deciding differently on a second
  // run.
  let text = rewrite(&inside, nested)
    .or_else(|| super::block::given_up(&inside, nested).filter(|t| !t.contains('\n')))?;
  let semi = if ends_with_semi(&inside, shape.limits) {
    ";"
  } else {
    ""
  };
  Some(format!(
    "{}{{\n{}{}{}\n{}}}",
    prefix,
    " ".repeat(nested.indent),
    text,
    semi,
    " ".repeat(shape.indent)
  ))
}

/// A binary expression as the standard formatter's `rewrite_all_pairs` lays it out: the
/// operands joined by the same operator taken as a list (`a && b && c`), on one line when
/// it fits, else each operand after the first on a line of its own one level in, its
/// operator first; a first operand no wider than the indentation keeps the next on its
/// line. Failing that, the two sides as `rewrite_pair` places them.
/// `expr as Type` as the standard formatter's `rewrite_pair` writes it: the expression laid
/// out with the line's budget, then ` as Type` on its last line when it fits there; else left
/// to the caller.
fn rewrite_cast(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let children = node.children();
  let [expression, ty] = children.as_slice() else {
    return None;
  };
  let used = shape.indent + shape.offset;
  let lhs_shape = Shape {
    width: shape.limits.max_width.saturating_sub(used),
    ..shape
  };
  let lhs = rewrite(expression, lhs_shape)?;
  // The last line's width counted from the line's start once the expression spans lines,
  // against the shape's width all the same (the standard formatter's `rewrite_pair`).
  let last = if lhs.contains('\n') {
    last_line_width(&lhs)
  } else {
    lhs.width()
  };
  // 4 = ` as `
  if let Some(text) = shape.offset_left(last + 4).and_then(|s| rewrite(ty, s)) {
    let same_line = lhs.width() <= shape.limits.tab
      || text.lines().next().is_some_and(|l| l.trim().ends_with('{'));
    if (!text.contains('\n') || same_line) && last + 4 + first_line_width(&text) <= shape.width {
      return Some(format!("{} as {}", lhs, text));
    }
  }
  // Else `as Type` on the next line, one level in.
  let nested = shape.block_indent(shape.limits.tab);
  let rhs_shape = Shape {
    width: nested.width.checked_sub(shape.rhs_overhead())?,
    ..nested
  };
  let text = rewrite(ty, rhs_shape.offset_left(3)?)?;
  Some(format!("{}\n{}as {}", lhs, " ".repeat(nested.indent), text))
}

/// A chain, as the standard formatter's block-indented chain formatter lays it out.
fn rewrite_chain(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let (root, root_tries, links) = chain(node, shape.limits.spacing);
  let mut root_text = format!("{}{}", rewrite(&root, shape)?, "?".repeat(root_tries));
  // A float ending in `.` before a link is put in parentheses (`(10.).method()`).
  if root.kind() == SyntaxKind::Literal
    && root_tries == 0
    && root_text.ends_with('.')
    && !links.is_empty()
  {
    root_text = format!("({})", root_text);
  }
  let child_count = links.len();
  let mut links: &[Link] = &links;
  // A short root takes the first links onto its line.
  let tab_width = shape.limits.tab.saturating_sub(shape.offset);
  let mut joined = false;
  let commented = links.iter().any(|l| !l.comments.is_empty());
  while root_text.width() <= tab_width
    && !root_text.contains('\n')
    && !links.is_empty()
    && links[0].comments.is_empty()
  {
    let s = shape.offset_left(root_text.width())?;
    match rewrite_link(&links[0], s) {
      Some(text) => {
        // `0. .f()`: a float before a method keeps its space; before style edition 2024
        // so does a tuple field after a tuple field (`x.0 .0`).
        let tuple_field = |t: &str| {
          t.strip_prefix('.')
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        };
        let previous_field = root_text
          .rfind('.')
          .is_some_and(|at| tuple_field(&root_text[at..]));
        if space::glues(&root_text, &text)
          || !shape.limits.style_2024 && previous_field && tuple_field(&text)
        {
          root_text.push(' ');
        }
        root_text.push_str(&text);
        joined = true;
      }
      None => break,
    }
    links = &links[1..];
  }
  if links.is_empty() {
    return wrap_str(root_text, shape);
  }
  // A root that took links onto its line ends as its last line does; otherwise as the
  // kind of expression it is.
  let block_end = if joined {
    last_line_extendable(&root_text)
  } else {
    is_block_expr(&root, &root_text)
  };
  let child_shape = if block_end {
    shape.block_indent(0)
  } else {
    shape.block_indent(shape.limits.tab)
  };
  let mut rewrites = vec![root_text];
  for link in &links[..links.len() - 1] {
    rewrites.push(rewrite_link(link, child_shape)?);
  }
  let last = &links[links.len() - 1];
  let extendable = last_line_extendable(&rewrites[0]);
  let almost_total = if extendable {
    last_line_width(&rewrites[0])
  } else {
    rewrites.iter().map(|r| r.width()).sum()
  } + last.tries;
  let one_line_budget = if child_count == 1 {
    shape.width
  } else {
    shape.width.min(shape.limits.chain)
  }
  .saturating_sub(almost_total);
  // Comments between the links put each on a line of its own.
  let all_in_one_line =
    !commented && rewrites.iter().all(|r| !r.contains('\n')) && one_line_budget > 0;
  let mut fits_single_line = false;
  // The last link's `?`s are taken off its shape here and again when it is printed, as the
  // standard formatter's `format_last_child` does.
  let last_shape = if all_in_one_line {
    shape.sub_width(last.tries)?
  } else if extendable {
    child_shape.sub_width(last.tries)?
  } else {
    child_shape.sub_width(shape.rhs_overhead() + last.tries)?
  };
  let mut last_text = None;
  if (all_in_one_line || extendable)
    && let Some(one_line_shape) = last_shape.offset_left(almost_total)
    && let Some(rw) = rewrite_link(last, one_line_shape)
  {
    let lines = rw.lines().count();
    let could_fit = first_line_width(&rw) <= one_line_budget;
    if could_fit && lines >= 5 {
      last_text = Some(rw);
      fits_single_line = all_in_one_line;
    } else {
      let own_line = child_shape.sub_width(shape.rhs_overhead() + last.tries)?;
      match rewrite_link(last, own_line) {
        Some(new) if !could_fit => last_text = Some(new),
        Some(new) if new.lines().count() >= lines => {
          last_text = Some(rw);
          fits_single_line = could_fit && all_in_one_line;
        }
        Some(new) => last_text = Some(new),
        None => {
          last_text = Some(rw);
          fits_single_line = could_fit && all_in_one_line;
        }
      }
    }
  }
  // When the last link cannot overflow (a closure whose body would break, which the standard
  // formatter breaks and then finds longer than the link on a line of its own), it takes a
  // line of its own. Where the standard formatter's overflowed link only runs past the line
  // (it checks its first line no further) it lays the link out on its own line, at the
  // links' indentation: tried first, so that a block in it is indented as theirs.
  let own_line = || {
    child_shape
      .sub_width(shape.rhs_overhead() + last.tries)
      .and_then(|s| rewrite_link(last, s))
  };
  let last_text = match last_text {
    Some(t) => t,
    None if all_in_one_line && let Some(t) = own_line().filter(|t| t.contains('\n')) => t,
    None => match rewrite_link(last, last_shape) {
      Some(t) => t,
      None => rewrite_link(
        last,
        child_shape.sub_width(shape.rhs_overhead() + last.tries)?,
      )?,
    },
  };
  rewrites.push(last_text);
  let connector = if fits_single_line {
    String::new()
  } else if shape.limits.one_line_chains {
    return None;
  } else {
    format!("\n{}", " ".repeat(child_shape.indent))
  };
  let mut result = rewrites[0].clone();
  for (r, link) in rewrites[1..].iter().zip(links) {
    for (comment, same_line) in &link.comments {
      if *same_line {
        result.push(' ');
      } else {
        result.push_str(&connector);
      }
      result.push_str(comment);
    }
    result.push_str(&connector);
    // `|| .. .f()`: a link joined to a root ending in dots keeps its space; before style
    // edition 2024 so does a tuple field after a tuple field (`x.0 .0`).
    let tuple_field = |t: &str| {
      t.strip_prefix('.')
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    };
    let previous_field = result
      .rfind('.')
      .is_some_and(|at| tuple_field(&result[at..]));
    if connector.is_empty()
      && (result.ends_with('.') && r.starts_with('.')
        || !shape.limits.style_2024 && previous_field && tuple_field(r))
    {
      result.push(' ');
    }
    result.push_str(r);
  }
  if commented {
    return Some(result);
  }
  wrap_str(result, shape)
}

/// A closure as the standard formatter's `rewrite_closure` writes it: a body of one
/// expression without its braces on the closure's line when it fits there (spanning lines
/// only if it is block-like); a block's statements on lines; an expression too long for the
/// line put in a block of its own.
fn rewrite_closure(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let body = node.children().into_iter().last()?;
  let prefix = closure_prefix(node, shape)?;
  let body_shape = after_prefix(&prefix, shape)?;
  // Inside a macro's arguments the body keeps or lacks its block as written.
  if shape.limits.inside_macro {
    return closure_expression(&body, &prefix, body_shape);
  }
  if body.kind() == SyntaxKind::BlockExpr
    && !body
      .children_with_tokens()
      .iter()
      .any(|e| matches!(e.kind(), SyntaxKind::AsyncKw | SyntaxKind::ConstKw))
  {
    let returns = node
      .children()
      .iter()
      .any(|c| c.kind() == SyntaxKind::RetType);
    let asynchronous = node
      .children_with_tokens()
      .iter()
      .any(|e| e.kind() == SyntaxKind::AsyncKw);
    let without = (!returns && !asynchronous)
      .then(|| {
        // A plain block of one expression is unwrapped, as deep as they go.
        let mut inner = body.clone();
        while !prefix.contains('\n')
          && plain_block(&inner)
          && let Some(expression) = lone_expression(&inner).filter(|e| !is_macro(e))
        {
          inner = expression;
        }
        if block_forced(&inner, shape.limits.style_2024) {
          closure_with_block(&inner, &prefix, shape)
        } else if prefix.contains('\n') && inner == body && plain_block(&inner) {
          // Its block kept after parameters across lines (`needs_block`), on one line when
          // it fits (`rewrite_block_inner`).
          rewrite_block(&inner, body_shape).map(|text| format!("{} {}", prefix, text))
        } else {
          closure_expression(&inner, &prefix, body_shape)
        }
      })
      .flatten();
    return without.or_else(|| {
      let block = match lone_expression(&body).filter(|e| !is_macro(e)) {
        // Its expression in a block of its own when no room is left on the line, as one
        // written without its block is (`closure_with_block`).
        Some(inside) => rewrite_block(&body, body_shape).or_else(|| {
          plain_block(&body)
            .then(|| super::block::wrapped(&inside, body_shape))
            .flatten()
        })?,
        None => super::block::rewrite_lines(&body, body_shape)?,
      };
      Some(format!("{} {}", prefix, block))
    });
  }
  closure_expression(&body, &prefix, body_shape)
    .or_else(|| closure_with_block(&body, &prefix, body_shape))
}

/// An `if` with an `else`, each holding one expression, on one line as the standard
/// formatter's `rewrite_single_line` writes it (`if c { a } else { b }`), within the shape
/// and `single_line_if_else_max_width`; never as a statement, where its blocks keep lines.
fn rewrite_if_one_line(node: &SyntaxNode, shape: Shape) -> Option<String> {
  if !if_on_one_line(node, shape.limits) {
    return None;
  }
  if_single_line(node, shape)
}

/// `a[i]` (`rewrite_index`): the index after what it indexes when it fits on one line there;
/// else on the next line one level in, `[` first, when it fits on one line there; else as it
/// lays out after what it indexes.
fn rewrite_index(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let children = node.children();
  let [base, index] = children.as_slice() else {
    let text = flat(node, shape.limits.spacing);
    return (text.width() <= shape.width).then_some(text);
  };
  let base_text = rewrite(base, shape)?;
  let offset = last_line_width(&base_text) + 1;
  let index_shape = if base_text.contains('\n') {
    Shape {
      offset: 0,
      width: shape.limits.max_width,
      ..shape
    }
    .offset_left(offset)
    .and_then(|s| s.sub_width(1 + shape.rhs_overhead()))
  } else {
    shape.offset_left(offset).and_then(|s| s.sub_width(1))
  };
  let first = index_shape.and_then(|s| rewrite(index, s));
  if let Some(text) = &first
    && !text.contains('\n')
  {
    return Some(format!("{}[{}]", base_text, text));
  }
  let next_shape = shape
    .block_indent(shape.limits.tab)
    .offset_left(1)?
    .sub_width(1 + shape.rhs_overhead())?;
  let next = rewrite(index, next_shape);
  let pad = " ".repeat(shape.indent + shape.limits.tab);
  match (first, next) {
    (_, Some(next)) if !next.contains('\n') => Some(format!("{}\n{}[{}]", base_text, pad, next)),
    (None, Some(next)) => Some(format!("{}\n{}[{}]", base_text, pad, next)),
    (Some(first), _) => Some(format!("{}[{}]", base_text, first)),
    _ => None,
  }
}

/// A closure last among the arguments, overflowing their list (the standard formatter's
/// `rewrite_last_closure`): a body forced into a block, or one whose condition spans lines,
/// put in one; any other laid out as a closure is.
fn rewrite_last_closure(node: &SyntaxNode, shape: Shape) -> Option<String> {
  if shape.limits.inside_macro {
    return rewrite_closure(node, shape);
  }
  let mut body = node.children().into_iter().last()?;
  if plain_block(&body)
    && let Some(expression) = lone_expression(&body).filter(|e| !is_macro(e))
  {
    body = expression;
  }
  let prefix = closure_prefix(node, shape)?;
  // Parameters across lines keep the closure from overflowing.
  if prefix.contains('\n') {
    return None;
  }
  let body_shape = shape.offset_left(prefix.width() + 1)?;
  if block_forced(&body, shape.limits.style_2024) {
    let returns = node
      .children()
      .iter()
      .any(|c| c.kind() == SyntaxKind::RetType);
    let block = closure_with_block(&body, &prefix, body_shape)?;
    if !returns
      && block.lines().count() <= 7
      && let Some(one) = closure_expression(&body, &prefix, shape).filter(|t| !t.contains('\n'))
    {
      return Some(one);
    }
    return Some(block);
  }
  if multi_line_condition(&body, body_shape) {
    return closure_with_block(&body, &prefix, body_shape);
  }
  rewrite_closure(node, shape)
}

/// `let pattern = value` in a condition: the pattern after `let `, the value after ` =` on
/// that line, or on the next one level in, as a `let` statement's is placed.
fn rewrite_let_condition(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let children = node.children();
  let [pattern, value] = children.as_slice() else {
    return None;
  };
  // 4 = `let `; 2 = ` =`, held back for a lone condition's but not in a let chain.
  let chained = node
    .parent()
    .is_some_and(|p| p.kind() == SyntaxKind::BinExpr);
  let pattern_shape = shape.offset_left(4)?;
  let pattern_shape = if chained {
    pattern_shape
  } else {
    pattern_shape.sub_width(2)?
  };
  let pattern = rewrite(pattern, pattern_shape)?;
  assign_value(&format!("let {} =", pattern), value, shape)
}

/// A chain link printed into `shape`, with its `?`s.
fn rewrite_link(link: &Link, shape: Shape) -> Option<String> {
  let tries = "?".repeat(link.tries);
  let shape = shape.sub_width(link.tries)?;
  // `::<A, B>`: each generic argument laid out in the link's shape, joined on the line (the
  // standard formatter's `rewrite_method_call`).
  let head = match &link.generics {
    Some((name, list)) => {
      let texts: Vec<String> = list
        .children()
        .iter()
        .map(|arg| rewrite(arg, shape))
        .collect::<Option<_>>()?;
      format!("{}::<{}>", name, texts.join(", "))
    }
    _ => link.head.clone(),
  };
  let text = match &link.args {
    Some(args) => rewrite_list(&head, args, shape, shape.limits.fn_call, "(", ")")?,
    // A field or `.await` is printed without a width check, as the standard formatter
    // prints it; whether the chain fits is decided around it.
    None => head,
  };
  Some(format!("{}{}", text, tries))
}

/// A macro call laid out as a call (its arguments in parentheses) or an array (in brackets),
/// when the standard formatter reads its arguments as such; one it cannot read is left as
/// written, on one line where it fits.
fn rewrite_macro(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let laid = rewrite_macro_call(node, shape);
  if laid.is_none() {
    super::macros::macro_failed();
    // Inside an expression, one that does not lay out is its written text where that fits
    // (the standard formatter's `format_expr`, `wrap_str` of the snippet); only one holding a
    // string across lines, whose lines are its own: another's written lines would differ
    // after a run, and so would a second run's layout.
    if node.kind() == SyntaxKind::MacroExpr
      && node
        .token_texts()
        .any(|(kind, text, _)| kind.is_literal() && text.contains('\n'))
      && node
        .parent()
        .is_some_and(|p| !matches!(p.kind(), SyntaxKind::ExprStmt | SyntaxKind::BlockExpr))
      && !super::file::skipped_macro(
        &node
          .children_iter()
          .find(|c| c.kind() == SyntaxKind::MacroCall)?,
      )
    {
      let text = node.text().to_string();
      return snippet_fits(&text, shape).then_some(text);
    }
  }
  laid
}

fn rewrite_macro_call(node: &SyntaxNode, shape: Shape) -> Option<String> {
  // One the settings or an attribute around it name: as written.
  let call = if node.kind() == SyntaxKind::MacroExpr {
    node
      .children_iter()
      .find(|c| c.kind() == SyntaxKind::MacroCall)?
  } else {
    node.clone()
  };
  if super::file::skipped_macro(&call) {
    return None;
  }
  let MacroArguments {
    name,
    list,
    open,
    close,
    array,
  } =
    match macro_arguments(node, shape.limits.spacing, shape.limits.inside_macro).or_else(|unread| {
      match unread {
        Unread::AsWritten => macro_type_arguments(node, shape.limits.spacing).ok_or(unread),
        unread => Err(unread),
      }
    }) {
      Ok(arguments) => arguments,
      // As written whatever its width, as the standard formatter's fallback for arguments it
      // cannot read returns it (`return_macro_parse_failure_fallback`): spanning lines, its
      // later lines re-indented keeping their layout when its last line only closes
      // delimiters, else where they were.
      // An item alone (`items!(const _: u8 = 0;)`), on a line of its own.
      Err(Unread::AsWritten) if let Some(text) = macro_item_argument(node, shape) => {
        return Some(text);
      }
      Err(Unread::AsWritten)
        if let Some((name, items, open, close, array)) =
          macro_mixed_arguments(node, shape.limits.spacing, shape.limits.inside_macro) =>
      {
        let width = if array {
          shape.limits.array
        } else {
          shape.limits.fn_call
        };
        let shape = Shape {
          limits: Limits {
            comma_as_written: true,
            inside_macro: true,
            ..shape.limits
          },
          ..shape
        };
        return rewrite_list(&name, &items, shape, width, open, close);
      }
      Err(Unread::AsWritten) => {
        super::macros::macro_failed();
        // From its name to its arguments' closing delimiter: the comments an item carries
        // before it and its `;` are not the call's.
        let whole = node.text();
        let from = node
          .descendant_nodes_where(|kind| kind == SyntaxKind::Path)
          .next()
          .map_or(0, |path| {
            (path.text_range().start - node.text_range().start) as usize
          });
        let end = node
          .descendant_nodes_where(|kind| kind == SyntaxKind::TokenTree)
          .next()
          .map_or(whole.len(), |tree| {
            (tree.text_range().end - node.text_range().start) as usize
          });
        let text = whole[from..end].to_string();
        if !text.contains('\n') {
          return Some(text);
        }
        let closes = text
          .lines()
          .last()
          .is_some_and(|l| l.trim().chars().all(|c| matches!(c, '}' | ')' | ']')));
        return Some(if closes {
          super::file::preserve_layout(&text, shape.indent, &shape.limits.config())
        } else {
          text
        });
      }
      Err(Unread::Braced(name)) => return braced_macro(node, &name, shape),
      Err(Unread::Declined) => return None,
    };
  let width = if array {
    shape.limits.array
  } else {
    shape.limits.fn_call
  };

  // `vec![x; n]` (`handle_vec_semi`): on one line when both fit there with 8 columns to
  // spare, else each on a line of its own one level in, `;` after the value.
  if list
    .children_with_tokens()
    .iter()
    .any(|e| e.kind() == SyntaxKind::Semi)
  {
    let parts = list.children();
    let [value, count] = parts.as_slice() else {
      return None;
    };
    // One level in from the call, as wide as what follows its name (`block_indent`).
    let nested = Shape {
      indent: shape.indent + shape.limits.tab,
      offset: 0,
      width: shape.width.checked_sub(name.width())?,
      limits: Limits {
        comma_as_written: true,
        inside_macro: true,
        ..shape.limits
      },
      ..shape
    };
    let value_text = rewrite(value, nested)?;
    let count_text = rewrite(count, nested)?;
    if !value_text.contains('\n')
      && !count_text.contains('\n')
      && value_text.width() + count_text.width() + 8 <= shape.width
    {
      return Some(format!(
        "{}{}{}; {}{}",
        name, open, value_text, count_text, close
      ));
    }
    let pad = " ".repeat(nested.indent);
    return Some(format!(
      "{}{}\n{}{};\n{}{}\n{}{}",
      name,
      open,
      pad,
      value_text,
      pad,
      count_text,
      " ".repeat(shape.indent),
      close
    ));
  }
  let items = list_items(&list);
  let commas = list
    .children_with_tokens()
    .iter()
    .filter(|e| e.kind() == SyntaxKind::Comma)
    .count();
  if commas + 1 != items.len() && commas != items.len() {
    return None;
  }
  // An attribute inside an argument: left as written, as a statement holding one is. One on
  // an argument goes on a line of its own before it.
  if list
    .descendants()
    .iter()
    .any(|d| d.kind() == SyntaxKind::Attr && d.parent().as_ref() != Some(&list))
  {
    return None;
  }
  let shape = Shape {
    limits: Limits {
      // `vec![...]` follows the array rule, unless it is inside another macro.
      comma_as_written: name != "vec!" || shape.limits.comma_as_written,
      inside_macro: true,
      ..shape.limits
    },
    ..shape
  };
  rewrite_list(&name, &list_items(&list), shape, width, open, close)
}

fn rewrite_node(node: &SyntaxNode, shape: Shape) -> Option<String> {
  match node.kind() {
    SyntaxKind::CallExpr => {
      let children = node.children();
      let callee = rewrite(&children[0], shape)?;
      let args = children
        .iter()
        .find(|c| c.kind() == SyntaxKind::ArgList)
        .map(list_items)
        .unwrap_or_default();
      rewrite_list(&callee, &args, shape, shape.limits.fn_call, "(", ")")
    }
    kind if is_chain(node) && kind != SyntaxKind::TryExpr || chain_with_links(node) => {
      rewrite_chain(node, shape)
    }
    SyntaxKind::RecordExpr | SyntaxKind::RecordPat => rewrite_struct(node, shape),
    // `[x; n]`: a `;` of its own, not one inside an element.
    SyntaxKind::ArrayExpr
      if node
        .children_with_tokens()
        .iter()
        .any(|e| e.kind() == SyntaxKind::Semi) =>
    {
      let text = flat(node, shape.limits.spacing);
      if text.width() <= shape.width && !breaks_inside(node, shape.limits) {
        return Some(text);
      }
      // As a pair inside the brackets (`rewrite_pair` with `[`, `; ` and `]`).
      let children = node.children();
      let [value, length] = children.as_slice() else {
        return None;
      };
      let inner = rewrite_pair(value, length, "; ", shape.offset_left(1)?.sub_width(1)?)?;
      Some(format!("[{}]", inner))
    }
    SyntaxKind::ArrayExpr | SyntaxKind::SlicePat => {
      rewrite_list("", &list_items(node), shape, shape.limits.array, "[", "]")
    }
    // Tuple patterns, with a path or without, are held to the line width only.
    SyntaxKind::TupleStructPat => {
      let children = node.children();
      let path = rewrite(children.first()?, shape)?;
      rewrite_list(
        &path,
        &children[1..],
        shape,
        shape.limits.max_width,
        "(",
        ")",
      )
    }
    SyntaxKind::TupleExpr | SyntaxKind::TuplePat => {
      let items = list_items(node);
      // `(a,)` is a one-element tuple; `(..)` is not.
      if items.len() == 1 && items[0].kind() != SyntaxKind::RestPat {
        let inner = rewrite(&items[0], shape.offset_left(1)?.sub_width(2)?)?;
        return Some(format!("({},)", inner));
      }
      let limit = if node.kind() == SyntaxKind::TuplePat {
        shape.limits.max_width
      } else {
        shape.limits.fn_call
      };
      rewrite_list("", &items, shape, limit, "(", ")")
    }
    SyntaxKind::ParenExpr => {
      // Parentheses directly around parentheses dropped.
      let mut outer = node.clone();
      while shape.limits.spacing.remove_nested_parens && nested_paren(&outer) {
        outer = outer.children().into_iter().next()?;
      }
      let inner = outer.children().into_iter().next()?;
      // Its block comments after `(` and before `)`, as written there but for the space
      // around them.
      let (from, to) = (inner.text_range().start, inner.text_range().end);
      let comments = |before: bool| -> Option<String> {
        let mut text = String::new();
        for element in outer.children_with_tokens_iter() {
          let SyntaxElement::Token(t) = element else {
            continue;
          };
          let at = t.text_range();
          if before && at.end > from || !before && at.start < to {
            continue;
          }
          match t.kind() {
            SyntaxKind::BlockComment if !t.text().contains('\n') => text.push_str(t.text()),
            SyntaxKind::Whitespace if !text.is_empty() => text.push(' '),
            SyntaxKind::Whitespace | SyntaxKind::LParen | SyntaxKind::RParen => {}
            _ => return None,
          }
        }
        Some(text.trim_end().to_string())
      };
      let (pre, post) = (comments(true)?, comments(false)?);
      let width = str_width(&pre) + str_width(&post);
      let inner = rewrite(&inner, shape.offset_left(1 + width)?.sub_width(1)?)?;
      Some(format!("({}{}{})", pre, inner, post))
    }
    // `f(x)?` with no chain: the call laid out, then the `?`.
    SyntaxKind::TryExpr => {
      let inner = rewrite(&node.children()[0], shape.sub_width(1)?)?;
      Some(format!("{}?", inner))
    }
    SyntaxKind::ClosureExpr => rewrite_closure(node, shape),
    SyntaxKind::IndexExpr => rewrite_index(node, shape),
    SyntaxKind::RangeExpr => rewrite_range(node, shape),
    // A prefix (`&`, `!`, `return `, `break 'label `), then what it applies to laid out; what follows
    // `return` gets a column less (found by measuring the standard formatter).
    SyntaxKind::RefExpr
    | SyntaxKind::PrefixExpr
    | SyntaxKind::ReturnExpr
    | SyntaxKind::BreakExpr
    | SyntaxKind::RefPat
    | SyntaxKind::BoxPat => {
      // `return` or `break` alone.
      let Some(inner_node) = node.children().into_iter().last() else {
        let text = flat(node, shape.limits.spacing);
        return (text.width() <= shape.width).then_some(text);
      };
      let whole = flat(node, shape.limits.spacing);
      let inner_flat = flat(&inner_node, shape.limits.spacing);
      let prefix = whole.strip_suffix(&inner_flat)?.to_string();
      let inner_shape = shape.offset_left(prefix.width())?;
      let inner_shape = if matches!(node.kind(), SyntaxKind::ReturnExpr | SyntaxKind::BreakExpr) {
        inner_shape.sub_width(1)?
      } else {
        inner_shape
      };
      let inner = rewrite(&inner_node, inner_shape)?;
      Some(format!("{}{}", prefix, inner))
    }
    SyntaxKind::MacroExpr | SyntaxKind::MacroCall => rewrite_macro(node, shape),
    // A literal must fit (the standard formatter's `wrap_str`): in a macro's arguments too,
    // the call then kept as written where that fits (`rewrite_or_written`).
    SyntaxKind::Literal => {
      let text = flat(node, shape.limits.spacing);
      // A raw string that does not fit is kept as written (the standard formatter's
      // `format_expr`), a raw byte or C string is not.
      let raw = node
        .first_token()
        .is_some_and(|t| t.kind() == SyntaxKind::RawStr);
      // From the 2024 style a string whose lines but the last end with `\` is kept however
      // wide (the standard formatter's `rewrite_string_lit`).
      let continued = shape.limits.style_2024
        && node
          .first_token()
          .is_some_and(|t| t.kind() == SyntaxKind::Str)
        && text.lines().rev().skip(1).all(|line| line.ends_with('\\'));
      if continued {
        Some(text)
      } else if raw {
        fits_shape(text.clone(), shape).or(Some(text))
      } else {
        fits_shape(text, shape)
      }
    }
    SyntaxKind::PathExpr => rewrite_path(node, shape),
    kind if super::types::is_type(kind) || kind == SyntaxKind::MacroType => {
      super::types::rewrite_type(node, shape)
    }
    SyntaxKind::OrPat => rewrite_or_pattern(node, shape),
    // `(pattern)`, the pattern laid out after the `(`.
    SyntaxKind::ParenPat if node.children().len() == 1 => {
      let inner = node.children().into_iter().next()?;
      let text = rewrite(&inner, shape.offset_left(1)?.sub_width(1)?)?;
      Some(format!("({})", text))
    }
    // `name @ pattern`: the pattern laid out after the binding.
    SyntaxKind::IdentPat
      if node
        .children()
        .last()
        .is_some_and(|c| c.kind().name().ends_with("_PAT")) =>
    {
      let inner = node.children().into_iter().last()?;
      let prefix = flat(node, shape.limits.spacing)
        .strip_suffix(&flat(&inner, shape.limits.spacing))?
        .to_string();
      // The standard formatter holds back the name and `@ `, not the space before `@`.
      let text = rewrite(&inner, shape.offset_left(prefix.width().saturating_sub(1))?)?;
      Some(format!("{}{}", prefix, text))
    }
    SyntaxKind::BinExpr if binary_operator(node).is_some() => rewrite_binary(node, shape),
    SyntaxKind::CastExpr => rewrite_cast(node, shape),
    // An attribute's `name = literal`, the literal whatever its width
    // (`MetaItemKind::NameValue`).
    SyntaxKind::BinExpr
      if shape.limits.attribute
        && let Some((left, operator, right)) = super::stmt::assignment(node)
        && operator == "="
        && right.kind() == SyntaxKind::Literal =>
    {
      let left = flat(&left, shape.limits.spacing);
      Some(format!("{} = {}", left, flat(&right, shape.limits.spacing)))
    }
    // An assignment, as a statement's: the left side, then the right as a `let`'s value is
    // placed (an attribute's `name = value` is not one).
    SyntaxKind::BinExpr if !shape.limits.attribute => {
      let (left, operator, right) = super::stmt::assignment(node)?;
      let left = rewrite(&left, shape.sub_width(operator.len() + 1)?)?;
      if left.contains('\n') {
        return None;
      }
      assign_value(&format!("{} {}", left, operator), &right, shape)
    }
    SyntaxKind::IfExpr => {
      rewrite_if_one_line(node, shape).or_else(|| rewrite_if_lines(node, shape, false))
    }
    SyntaxKind::LetExpr => rewrite_let_condition(node, shape),
    // A closure's block inside a macro's arguments stays, on one line when it fits there
    // (the standard formatter's `rewrite_single_line_block`).
    SyntaxKind::BlockExpr
      if joins(node)
        || shape.limits.inside_macro
          && lone_expression(node).is_some()
          && node
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::ClosureExpr)
          && !node.children().iter().any(|c| c.kind() == SyntaxKind::Attr) =>
    {
      rewrite_block(node, shape)
    }
    SyntaxKind::BlockExpr => super::block::rewrite_lines(node, shape),
    SyntaxKind::MatchExpr => super::block::rewrite_match(node, shape),
    SyntaxKind::ForExpr | SyntaxKind::WhileExpr | SyntaxKind::LoopExpr => {
      super::block::rewrite_loop(node, shape)
    }
    // Anything else on one line, unless something in it the standard formatter would break
    // for its own width: then left as written.
    _ => {
      let text = flat(node, shape.limits.spacing);
      (text.width() <= shape.width && !breaks_inside(node, shape.limits)).then_some(text)
    }
  }
}

/// Alternative patterns (`A | B`), as the standard formatter writes them: on one line when
/// they fit; when every one is short, packed onto lines; else one per line. A line after
/// the first starts with `| ` at the patterns' indentation.
fn rewrite_or_pattern(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let alternatives = node.children();
  let texts: Vec<String> = alternatives
    .iter()
    .map(|a| rewrite(a, shape))
    .collect::<Option<_>>()?;
  let short = alternatives
    .iter()
    .zip(&texts)
    .all(|(a, t)| t.len() <= 20 && !t.contains('\n') && short_pattern(a));
  let pad = " ".repeat(shape.indent);
  if short {
    // As `write_list` packs them: each after the first counted with its ` |`, and a space
    // between.
    let mut text = String::new();
    let mut line = 0;
    for (i, alternative) in texts.iter().enumerate() {
      let total = alternative.len() + if i > 0 { 2 } else { 0 };
      if line > 0 && line + 1 + total > shape.width {
        text.push_str(&format!("\n{}", pad));
        line = 0;
      } else if line > 0 {
        text.push(' ');
        line += 1;
      }
      if i > 0 {
        text.push_str("| ");
      }
      text.push_str(alternative);
      line += total;
    }
    return Some(text);
  }
  let total: usize = texts.iter().map(|t| t.width()).sum::<usize>() + 3 * (texts.len() - 1);
  if total <= shape.width && !texts.iter().any(|t| t.contains('\n')) {
    return Some(texts.join(" | "));
  }
  Some(texts.join(&format!("\n{}| ", pad)))
}

/// The two sides of a binary expression or a range on one line when they fit, the right one
/// after a left one no wider than the indentation or opening a block even spanning lines;
/// else the right side on the next line one level in, after the operator (`rewrite_pair`).
/// `infix` is the operator with the spaces written around it.
fn rewrite_pair(
  left: &SyntaxNode,
  right: &SyntaxNode,
  infix: &str,
  shape: Shape,
) -> Option<String> {
  let used = shape.indent + shape.offset;
  let left_shape = Shape {
    width: shape.limits.max_width.saturating_sub(used),
    ..shape
  };
  let left_text = rewrite(left, left_shape)?;
  if let Some(right_text) = shape
    .offset_left(last_line_width(&left_text) + infix.width())
    .and_then(|s| rewrite(right, s))
  {
    let same_line = left_text.width() <= shape.limits.tab
      || right_text.lines().next().is_some_and(|l| l.ends_with('{'));
    if (!right_text.contains('\n') || same_line)
      && last_line_width(&left_text) + infix.width() + first_line_width(&right_text) <= shape.width
    {
      return Some(format!("{}{}{}", left_text, infix, right_text));
    }
  }
  let nested = shape.block_indent(shape.limits.tab);
  // A `;` (`[x; n]`) ends the line it follows (`SeparatorPlace::Back`); an operator starts
  // the next.
  if infix.trim() == ";" {
    let right_shape = Shape {
      width: nested.width.checked_sub(shape.rhs_overhead())?,
      ..nested
    };
    let right_text = rewrite(right, right_shape)?;
    return Some(format!(
      "{};\n{}{}",
      left_text,
      " ".repeat(nested.indent),
      right_text
    ));
  }
  let infix = infix.trim_start();
  let right_shape = Shape {
    width: nested.width.checked_sub(shape.rhs_overhead())?,
    ..nested
  }
  .offset_left(infix.width())?;
  let right_text = rewrite(right, right_shape)?;
  Some(format!(
    "{}\n{}{}{}",
    left_text,
    " ".repeat(nested.indent),
    infix,
    right_text
  ))
}

/// A path, its generic arguments laid out as a type's are (the standard formatter's
/// `rewrite_path` in an expression): each segment's name must fit, but generic arguments
/// kept on the line may run past the shape, for what holds the path to weigh.
fn rewrite_path(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let path = node
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::Path)?;
  super::types::rewrite_path(&path, shape, true)
}

/// A range with both ends: as a binary expression is laid out, its `..` spaced as written
/// on one line.
fn rewrite_range(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let whole = flat(node, shape.limits.spacing);
  let children = node.children();
  let breaks = breaks_inside(node, shape.limits);
  if let [start, end] = children.as_slice() {
    if whole.width() <= shape.width && !breaks {
      return Some(whole);
    }
    let start_text = flat(start, shape.limits.spacing);
    let end_text = flat(end, shape.limits.spacing);
    let infix = whole.strip_prefix(&start_text)?.strip_suffix(&end_text)?;
    return rewrite_pair(start, end, infix, shape);
  }
  (whole.width() <= shape.width && !breaks).then_some(whole)
}

/// A struct literal or pattern: on one line within `struct_lit_width`, else one field per
/// line.
fn rewrite_struct(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let path = node
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::Path)
    .map(|p| flat(&p, shape.limits.spacing))
    .unwrap_or_default();
  let list = node.children().into_iter().find(|c| {
    matches!(
      c.kind(),
      SyntaxKind::RecordExprFieldList | SyntaxKind::RecordPatFieldList
    )
  })?;
  let v_shape = shape.block_indent(shape.limits.tab);
  let mut fields = Vec::new();
  let mut spread = None;
  let mut after_dots = false;
  // Fields followed by a blank line after their comma, before another field or a struct
  // literal's base: laid out a field per line, the standard formatter keeps one there.
  let mut blank_after = Vec::new();
  let mut comma = false;
  let mut blank = false;
  for element in list.children_with_tokens() {
    match &element {
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Comma => comma = true,
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Whitespace => {
        blank = comma && t.text().matches('\n').count() > 1;
      }
      SyntaxElement::Token(t) if t.kind().is_trivia() => {}
      // Not before `}`, nor before a pattern's `..`, which the standard formatter adds after
      // the list.
      _ => {
        let rest = matches!(&element, SyntaxElement::Node(n) if n.kind() == SyntaxKind::RecordPatField
          && n.children().first().is_some_and(|c| c.kind() == SyntaxKind::RestPat));
        if blank && element.kind() != SyntaxKind::RBrace && !rest {
          blank_after.push(fields.len());
        }
        (comma, blank) = (false, false);
      }
    }
    match &element {
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Dot2 => after_dots = true,
      SyntaxElement::Node(n) if after_dots => {
        spread = Some(format!("..{}", rewrite(n, v_shape.offset_left(2)?)?));
      }
      // A pattern's `..` is a field of its own, and ends the list.
      SyntaxElement::Node(n)
        if n.kind() == SyntaxKind::RecordPatField
          && n
            .children()
            .first()
            .is_some_and(|c| c.kind() == SyntaxKind::RestPat) =>
      {
        after_dots = true;
      }
      SyntaxElement::Node(n)
        if matches!(
          n.kind(),
          SyntaxKind::RecordExprField | SyntaxKind::RecordPatField
        ) =>
      {
        let (attributes, parts): (Vec<SyntaxNode>, Vec<SyntaxNode>) = n
          .children_iter()
          .partition(|c| c.kind() == SyntaxKind::Attr);
        let text = match parts.as_slice() {
          // The value after `name:`; on the next line one level in, with the whole line's
          // width, only when it does not fit there (the standard formatter's
          // `rewrite_field`).
          [name, value] => {
            let name = flat(name, shape.limits.spacing);
            let same = v_shape
              .sub_width(1)?
              .offset_left(name.width() + 2)
              .and_then(|s| rewrite(value, s));
            // A pattern's field keeps only a pattern on one line after its name; else the
            // pattern goes to the next line, one level in (the standard formatter's
            // `PatField`).
            let same = same.filter(|t| n.kind() != SyntaxKind::RecordPatField || !t.contains('\n'));
            match same {
              Some(same) => format!("{}: {}", name, same),
              // A value not laid out here (a binary expression the standard formatter breaks
              // at its operators) may fit on the line its way: left as written.
              None
                if n.kind() != SyntaxKind::RecordPatField
                  && !(is_chain(value)
                    || matches!(
                      value.kind(),
                      SyntaxKind::CallExpr
                        | SyntaxKind::RecordExpr
                        | SyntaxKind::ArrayExpr
                        | SyntaxKind::TupleExpr
                        | SyntaxKind::MacroExpr
                    )) =>
              {
                return None;
              }
              None => {
                // A pattern's keeps the field's width one level in (`PatField` takes
                // `block_indent` of the field's shape), so it may pass the line width.
                let next_shape = if n.kind() == SyntaxKind::RecordPatField {
                  Shape {
                    indent: v_shape.indent + shape.limits.tab,
                    align: 0,
                    offset: 0,
                    ..v_shape.sub_width(1)?
                  }
                } else {
                  v_shape.block_indent(shape.limits.tab)
                };
                let next = rewrite(value, next_shape)?;
                format!("{}:\n{}{}", name, " ".repeat(next_shape.indent), next)
              }
            }
          }
          _ if attributes.is_empty() => flat(n, shape.limits.spacing),
          [name] => flat(name, shape.limits.spacing),
          _ => return None,
        };
        // A comment among the attributes is left as written.
        if n
          .children_with_tokens_iter()
          .any(|e| e.kind().is_trivia() && e.kind() != SyntaxKind::Whitespace)
          && !attributes.is_empty()
        {
          return None;
        }
        // A pattern's field written `name: pattern` after its attributes stands one level
        // further in, as the standard formatter writes it.
        let text = if n.kind() == SyntaxKind::RecordPatField
          && !attributes.is_empty()
          && n
            .children_with_tokens_iter()
            .any(|e| e.kind() == SyntaxKind::Colon)
        {
          if text.contains('\n') {
            return None;
          }
          format!("{}{}", " ".repeat(shape.limits.tab), text)
        } else {
          text
        };
        fields.push(with_attributes(&attributes, text, v_shape)?);
      }
      _ => {}
    }
  }
  // `S { a, .. }`: a rest with nothing after it.
  if after_dots && spread.is_none() {
    spread = Some("..".to_string());
  }
  if fields.is_empty() && spread.is_none() {
    return wrap_str(format!("{} {{}}", path), shape);
  }
  let mut all: Vec<String> = fields.clone();
  if let Some(s) = &spread {
    all.push(s.clone());
  }
  let h_width = shape
    .width
    .checked_sub(path.width() + 3 + 2)
    .map(|w| w.min(shape.limits.struct_lit));
  let mut horizontal = h_width.is_some_and(|w| definitive(&all, w) == Tactic::Horizontal);
  // A pattern ending in `..` (the standard formatter's `rewrite_struct_pat`): its fields are
  // held to the width less ` { `, `, ..` and ` }`; fitting there they stay on one line with
  // the `..`, inside braces on lines of their own when that line is too wide.
  let mut rest_line = false;
  let h_rest = shape
    .width
    .checked_sub(path.width() + 3 + 6)
    .map(|w| w.min(shape.limits.struct_lit));
  // `Path { .. }` alone: on one line only where its `..` fits that width.
  if node.kind() == SyntaxKind::RecordPat && spread.as_deref() == Some("..") && fields.is_empty() {
    horizontal = h_rest.is_some_and(|w| w >= 2);
  }
  if node.kind() == SyntaxKind::RecordPat && spread.as_deref() == Some("..") && !fields.is_empty() {
    let fits = h_rest.is_some_and(|w| definitive(&fields, w) == Tactic::Horizontal);
    let line = fields.join(", ").width() + 4;
    horizontal = fits && h_rest.is_some_and(|w| line <= w);
    rest_line = fits && !horizontal;
  }
  // Comments among a literal's fields: one field per line, the comments placed around them
  // as a list's are; the `..base` an item of its own from its `..`.
  {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for field in list.children_iter() {
      if !matches!(
        field.kind(),
        SyntaxKind::RecordExprField | SyntaxKind::RecordPatField
      ) {
        continue;
      }
      let mut tokens = field
        .descendants_with_tokens_iter()
        .filter(|e| matches!(e, SyntaxElement::Token(t) if !t.kind().is_trivia()));
      let first = tokens.next()?.text_range();
      let last = tokens.last().map_or(first.end, |e| e.text_range().end);
      spans.push((first.start as usize, last as usize));
    }
    if let Some(dots) = list
      .children_with_tokens_iter()
      .find(|e| e.kind() == SyntaxKind::Dot2)
    {
      let end = list
        .children_iter()
        .find(|c| c.text_range().start >= dots.text_range().end)
        .map_or(dots.text_range().end, |n| n.text_range().end);
      spans.push((dots.text_range().start as usize, end as usize));
    }
    // A comma left among the comments after the last field (`b /* c */, /* d */`) the
    // standard formatter writes twice, breaking the code: left as written.
    let stray_comma = |c: &super::lists::ItemComments| {
      c.post.as_ref().is_some_and(|post| {
        let mut code = String::new();
        let mut rest = post.as_str();
        while !rest.is_empty() {
          if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |e| &after[e + 2..]);
          } else if rest.starts_with("//") {
            rest = rest.find('\n').map_or("", |e| &rest[e..]);
          } else {
            let c = rest.chars().next().unwrap_or(' ');
            code.push(c);
            rest = &rest[c.len_utf8()..];
          }
        }
        code.contains(',')
      })
    };
    if spans.len() == all.len()
      && let Some(comments) = super::lists::spanned_comments(&list, &spans)
    {
      // A pattern ending in `..` with comments the standard formatter leaves as written.
      if comments.iter().any(stray_comma)
        || node.kind() == SyntaxKind::RecordPat && spread.is_some()
      {
        return None;
      }
      // In a macro's arguments a trailing comma stays as written.
      let written_comma = list
        .children_with_tokens()
        .iter()
        .rev()
        .find(|e| !e.kind().is_trivia() && e.kind() != SyntaxKind::RBrace)
        .is_some_and(|e| e.kind() == SyntaxKind::Comma);
      let trailing = spread.is_none() && (!shape.limits.comma_as_written || written_comma);
      let body = super::lists::write_vertical_kept(
        &all,
        &comments,
        v_shape.indent,
        v_shape.width,
        shape.limits.max_width,
        trailing,
      )?;
      return Some(format!(
        "{} {{\n{}{}\n{}}}",
        path,
        " ".repeat(v_shape.indent),
        body,
        " ".repeat(shape.indent)
      ));
    }
  }
  let text = if horizontal {
    format!("{} {{ {} }}", path, all.join(", "))
  } else if rest_line {
    format!(
      "{} {{\n{}{}, ..\n{}}}",
      path,
      " ".repeat(v_shape.indent),
      fields.join(", "),
      " ".repeat(shape.indent)
    )
  } else {
    let pad = " ".repeat(v_shape.indent);
    let mut body = String::new();
    // Inside a macro the last field keeps a comma only where written.
    let last_written = list
      .children()
      .into_iter()
      .rev()
      .find(|c| c.kind() == SyntaxKind::RecordExprField)
      .is_some_and(|last| comma_after(&last));
    for (i, field) in fields.iter().enumerate() {
      let last = i + 1 == fields.len() && spread.is_none();
      let comma = if last && shape.limits.comma_as_written && !last_written {
        ""
      } else {
        ","
      };
      body.push_str(&format!("\n{}{}{}", pad, field, comma));
      if blank_after.contains(&(i + 1)) {
        body.push('\n');
      }
    }
    if let Some(s) = &spread {
      body.push_str(&format!("\n{}{}", pad, s));
    }
    format!("{} {{{}\n{}}}", path, body, " ".repeat(shape.indent))
  };
  // A pattern's later lines are not measured (`rewrite_struct_pat` returns them as laid
  // out, a field's pattern one level in maybe past the line).
  if node.kind() == SyntaxKind::RecordPat {
    return text
      .lines()
      .next()
      .is_some_and(|l| l.width() <= shape.width)
      .then_some(text);
  }
  wrap_str(text, shape)
}

/// The `::` starting a visibility's path (`pub(in ::a)`), which the standard formatter
/// drops (`format_visibility`).
fn root_of_visibility(token: &SyntaxToken) -> bool {
  token.kind() == SyntaxKind::Colon2
    && token
      .parent()
      .ancestors_iter()
      .any(|a| a.kind() == SyntaxKind::Visibility)
    && token
      .parent()
      .ancestors_iter()
      .find(|a| {
        a.kind() == SyntaxKind::Path
          && a
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::Visibility)
      })
      .and_then(|path| path.first_token())
      .is_some_and(|first| first.text_range() == token.text_range())
}

/// Whether macro arguments hold what rustc would not parse, and so the standard formatter
/// leaves as written, though this parser takes it.
fn rustc_rejects(list: &SyntaxNode) -> bool {
  // What rustc would not parse, and so the standard formatter leaves as written, though
  // this parser takes it: a comparison chained to another (`-a <"v"> "s"`), a comma after no
  // item (`[,]`, `(a,,b)`).
  let comparison = |n: &SyntaxNode| {
    n.kind() == SyntaxKind::BinExpr
      && n.children_with_tokens().iter().any(|e| {
        matches!(
          e.kind(),
          SyntaxKind::Lt
            | SyntaxKind::Gt
            | SyntaxKind::LtEq
            | SyntaxKind::GtEq
            | SyntaxKind::EqEq
            | SyntaxKind::Neq
        )
      })
  };
  // A range with a start is not chained either (`b'0' ..= b'9' | b'A' ..= b'Z'`, a pattern);
  // ranges with none nest (`.. .. ..`).
  let has_start = |range: &SyntaxNode| {
    range
      .children_with_tokens_iter()
      .find(|e| !e.kind().is_trivia())
      .is_some_and(|first| matches!(first, SyntaxElement::Node(_)))
  };
  // A qualified path with no name after it (`<T>`, not `<T>::f`).
  let bare_qualified = |path: &SyntaxNode| {
    path
      .children_iter()
      .rfind(|c| c.kind() == SyntaxKind::PathSegment)
      .and_then(|segment| segment.first_token())
      .is_some_and(|t| t.kind() == SyntaxKind::Lt)
  };
  // A `let` outside an `if`'s, a `while`'s or a guard's condition (`quote!(let x = y)`).
  let bare_let = |n: &SyntaxNode| {
    let mut up = n.parent();
    while let Some(p) = up.clone().filter(|p| p.kind() == SyntaxKind::BinExpr) {
      up = p.parent();
    }
    !up.is_some_and(|p| {
      matches!(
        p.kind(),
        SyntaxKind::IfExpr | SyntaxKind::WhileExpr | SyntaxKind::MatchGuard
      )
    })
  };
  if list
    .descendant_nodes_where(|kind| kind == SyntaxKind::LetExpr)
    .any(|n| bare_let(&n))
    || list
      .descendant_nodes_where(|kind| kind == SyntaxKind::BinExpr)
      .any(|n| comparison(&n) && n.children_iter().any(|c| comparison(&c)))
    || list
      .descendant_nodes_where(|kind| kind == SyntaxKind::Path)
      .any(|p| bare_qualified(&p))
    || list
      .descendant_nodes_where(|kind| kind == SyntaxKind::RangeExpr)
      .any(|n| {
        n.children_iter()
          .any(|c| c.kind() == SyntaxKind::RangeExpr && (has_start(&n) || has_start(&c)))
      })
  {
    return true;
  }
  // `<-` written touching (the obsolete in-place arrow) and `builtin #` rustc rejects or
  // keeps apart; a `#` outside an attribute is left as written, but in a macro call's own
  // arguments (`quote!(#x)`), which take any tokens.
  // Kinds and ranges only: no handles.
  let tokens: Vec<(SyntaxKind, &str, TextRange)> = list
    .token_texts()
    .filter(|(kind, _, _)| !kind.is_trivia())
    .collect();
  // Not the `<` opening generic arguments (`f::<-1>()`).
  let generic_opens: Vec<u32> = list
    .descendant_nodes_where(|kind| kind == SyntaxKind::GenericArgList)
    .filter_map(|n| {
      n.children_with_tokens_iter()
        .find(|e| e.kind() == SyntaxKind::Lt)
        .map(|e| e.text_range().start)
    })
    .collect();
  if tokens.windows(2).any(|w| {
    w[0].0 == SyntaxKind::Lt
      && w[1].0 == SyntaxKind::Minus
      && w[0].2.end == w[1].2.start
      && !generic_opens.contains(&w[0].2.start)
  })
    || list.descendants_with_tokens_iter().any(|e| {
      matches!(&e, SyntaxElement::Token(t) if t.kind() == SyntaxKind::Pound
        && !matches!(t.parent().kind(), SyntaxKind::TokenTree | SyntaxKind::Attr))
    })
    // A doc comment on an argument, which the standard formatter does not read there.
    || list.token_texts().any(|(kind, _, _)| super::items::is_doc(kind))
    // No binder after `?` (`?for<> Trait`).
    || tokens
      .windows(2)
      .any(|w| w[0].0 == SyntaxKind::Question && w[1].1 == "for")
  {
    return true;
  }
  false
}

/// Whether a pattern is short for packing alternatives: a literal, `_`, `..`, a name, a
/// tuple of at most one, a single-segment tuple struct of at most one, or one of those
/// behind `&`, `box` or parentheses.
fn short_pattern(node: &SyntaxNode) -> bool {
  match node.kind() {
    SyntaxKind::LiteralPat | SyntaxKind::WildcardPat | SyntaxKind::RestPat => true,
    SyntaxKind::IdentPat => !node
      .children()
      .iter()
      .any(|c| c.kind().name().ends_with("_PAT")),
    // A bare name is a binding to the standard formatter.
    SyntaxKind::PathPat => !flat(node, Spacing::default()).contains("::"),
    SyntaxKind::TuplePat => node.children().len() <= 1,
    SyntaxKind::TupleStructPat => {
      let children = node.children();
      children
        .first()
        .is_some_and(|p| !flat(p, Spacing::default()).contains("::"))
        && children.len() <= 2
    }
    SyntaxKind::RefPat | SyntaxKind::BoxPat | SyntaxKind::ParenPat => {
      node.children().first().is_some_and(short_pattern)
    }
    SyntaxKind::OrPat => node.children().iter().all(short_pattern),
    _ => false,
  }
}

/// Whether written text fits `shape` as the standard formatter's `filtered_str_fits` has
/// it: lines of comments not counted, the first line within the shape's width, the others
/// within the maximum width, the last within the shape's line.
fn snippet_fits(text: &str, shape: Shape) -> bool {
  let code: Vec<&str> = text
    .split('\n')
    .filter(|line| {
      let line = line.trim_start();
      !line.starts_with("//") && !line.starts_with("/*")
    })
    .collect();
  let Some(first) = code.first() else {
    return true;
  };
  if first.width() > shape.width {
    return false;
  }
  code.len() == 1
    || !code[1..]
      .iter()
      .any(|line| line.width() > shape.limits.max_width)
      && code
        .last()
        .is_some_and(|last| last.width() <= shape.indent + shape.offset + shape.width)
}

/// The arguments before a format-like macro's format string (`SPECIAL_CASE_MACROS`), and
/// from style edition 2024 `trace!`'s too.
fn special_macro(name: &str, style_2024: bool) -> Option<usize> {
  SPECIAL_MACROS
    .iter()
    .find(|(n, _)| *n == name)
    .map(|&(_, before)| before)
    .or_else(|| (style_2024 && name == "trace!").then_some(0))
}

/// Whether `text` fits: its first line in the shape's width, the rest within the line width.
fn wrap_str(text: String, shape: Shape) -> Option<String> {
  let mut lines = text.lines();
  if lines.next().is_some_and(|l| l.width() > shape.width) {
    return None;
  }
  if later_lines(&text).any(|(_, w)| w > shape.limits.max_width) {
    return None;
  }
  Some(text)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::syntax::{Edition, parse};

  /// The expression initializing `let x = ...;` in `source`, printed into its shape (block
  /// indent 4, after `    let x = `, a `;` reserved).
  fn laid_out(expression: &str) -> Option<String> {
    let source = format!("fn f() {{\n    let x = {};\n}}\n", expression);
    let tree = parse(&source, Edition::E2021).syntax_node();
    let node = tree
      .descendants()
      .into_iter()
      .find(|n| n.kind() == SyntaxKind::LetStmt)?
      .children()
      .into_iter()
      .last()?;
    let column = "    let x = ".len();
    rewrite(
      &node,
      Shape {
        align: 0,
        indent: 4,
        offset: column - 4,
        width: 100 - column - 1,
        limits: Limits::default(),
      },
    )
  }

  // Expected outputs are the standard formatter's for the same statements.
  #[test]
  fn test_calls() {
    assert_eq!(laid_out("foo(alpha, beta)").unwrap(), "foo(alpha, beta)");
    assert_eq!(
      laid_out("function_name(argument_number_one, argument_number_two, argument_number_three_x)")
        .unwrap(),
      "function_name(\n        argument_number_one,\n        argument_number_two,\n        argument_number_three_x,\n    )"
    );
    // A struct literal as the only argument overflows.
    assert_eq!(
      laid_out("foo(Bar { aaaa: 1, bbbb: 2, cccc: 3, dddd: 4 })").unwrap(),
      "foo(Bar {\n        aaaa: 1,\n        bbbb: 2,\n        cccc: 3,\n        dddd: 4,\n    })"
    );
  }

  #[test]
  fn test_chains() {
    assert_eq!(
      laid_out("Command::new(rustc).arg(\"--version\").output().ok()?").unwrap(),
      "Command::new(rustc).arg(\"--version\").output().ok()?"
    );
    assert_eq!(
      laid_out("rustc_wrapper.into_iter().chain(rustc_workspace_wrapper).chain(iter::once(rustc))")
        .unwrap(),
      "rustc_wrapper\n        .into_iter()\n        .chain(rustc_workspace_wrapper)\n        .chain(iter::once(rustc))"
    );
  }

  #[test]
  fn test_struct_literals() {
    assert_eq!(
      laid_out("Chain { state: ChainState::Linked { next: Some(head) } }").unwrap(),
      "Chain {\n        state: ChainState::Linked { next: Some(head) },\n    }"
    );
    assert_eq!(laid_out("Point { x, y }").unwrap(), "Point { x, y }");
  }
}
