//! Expressions printed into the space left for them: the standard formatter's width-budget
//! rewriting, ported. `rewrite(node, shape)` returns the
//! node's text in `shape`, or `None` when it cannot fit; callers try their layouts in order
//! and keep the first that works. Calls, method chains, struct literals, arrays and tuples
//! are laid out; other expressions print flat for now.

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
  /// In braces: its name, a space, and the braces as written.
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
  pub tries: usize,
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
  let same = shape.offset_left(used + 1).and_then(|s| rewrite(value, s));
  if let Some(same) = &same
    && !same.contains('\n')
  {
    return Some(format!("{} {}", head, same));
  }
  let nested = shape.block_indent(shape.limits.tab);
  let next = Shape {
    width: nested.width.checked_sub(shape.rhs_overhead())?,
    ..nested
  };
  let next = rewrite(value, next);
  let pad = " ".repeat(nested.indent);
  match (same, next) {
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

/// Whether generic arguments are ones [`rewrite_path`] breaks: paths (with generic
/// arguments of their own), lifetimes and constants, no tuple or array (the standard
/// formatter breaks those inside too).
pub fn breakable_generics(list: &SyntaxNode) -> bool {
  list.children().iter().all(|arg| {
    let text = flat(arg, Spacing::default());
    !text.contains(['(', '['])
  })
}

/// Whether the last of `count` arguments may overflow: a closure or block always; a call,
/// struct literal, array or tuple when it is the only argument.
pub fn can_overflow(node: &SyntaxNode, count: usize) -> bool {
  match node.kind() {
    SyntaxKind::ClosureExpr | SyntaxKind::BlockExpr => true,
    SyntaxKind::RecordExpr
    | SyntaxKind::ArrayExpr
    | SyntaxKind::CallExpr
    | SyntaxKind::MethodCallExpr
    | SyntaxKind::TupleExpr
    | SyntaxKind::MacroExpr
    // `combine_control_expr`.
    | SyntaxKind::IfExpr
    // Patterns, as the standard formatter's `can_be_overflowed_pat`.
    | SyntaxKind::RecordPat
    | SyntaxKind::TuplePat
    | SyntaxKind::TupleStructPat
    | SyntaxKind::PathPat => count == 1,
    SyntaxKind::RefPat | SyntaxKind::BoxPat => node
      .children()
      .first()
      .is_some_and(|c| can_overflow(c, count)),
    SyntaxKind::RefExpr | SyntaxKind::TryExpr | SyntaxKind::PrefixExpr => node
      .children()
      .first()
      .is_some_and(|c| can_overflow(c, count)),
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
            SyntaxElement::Node(n) => head.push_str(&flat(n, spacing)),
            SyntaxElement::Token(t) if !t.kind().is_trivia() => head.push_str(t.text()),
            _ => {}
          }
        }
        links.push(Link {
          head,
          args,
          tries: pending_tries,
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
  // An `async` block as an argument the standard formatter overflows in ways not followed
  // here: left as written.
  let asynchronous = block
    .children_with_tokens()
    .iter()
    .any(|e| e.kind() == SyntaxKind::AsyncKw);
  if asynchronous && parent.kind() == SyntaxKind::ArgList {
    return false;
  }
  match parent.kind() {
    SyntaxKind::ExprStmt | SyntaxKind::BlockExpr | SyntaxKind::MatchArm => prefixed,
    SyntaxKind::ClosureExpr
    | SyntaxKind::Fn
    | SyntaxKind::IfExpr
    | SyntaxKind::WhileExpr
    | SyntaxKind::ForExpr
    | SyntaxKind::LoopExpr => false,
    kind => {
      kind.name().ends_with("_EXPR")
        || matches!(
          kind,
          SyntaxKind::LetStmt | SyntaxKind::ArgList | SyntaxKind::RecordExprField
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

/// Whether a block is one of lines: it holds a statement, or it is a function's body holding
/// anything.
pub fn lines_block(node: &SyntaxNode) -> bool {
  node.children_iter().any(|c| is_statement(c.kind()))
    || node.parent().is_some_and(|p| p.kind() == SyntaxKind::Fn)
      && node.children_iter().any(|c| c.kind() != SyntaxKind::Label)
}

/// The expressions of a list: call arguments, array or tuple elements.
pub fn list_items(node: &SyntaxNode) -> Vec<SyntaxNode> {
  node.children()
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
        SyntaxKind::UnsafeKw | SyntaxKind::ConstKw | SyntaxKind::AsyncKw | SyntaxKind::MoveKw
          if !opened => {}
        _ => return None,
      },
      SyntaxElement::Node(n) if n.kind() == SyntaxKind::Label && !opened => {}
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

/// The standard formatter moves a right-hand side to the next line when it fits on one line
/// there but not on the first, when that saves more than one line, or when its first line
/// would otherwise end by opening a delimiter that the next line's does not.
pub fn prefer_next_line(same: &str, next: &str) -> bool {
  let lines = |text: &str| text.matches('\n').count();
  let opens = |text: &str, c: char| text.lines().next().is_some_and(|l| l.ends_with(c));
  lines(same) > 0
    && (lines(next) == 0
      || lines(same) > lines(next) + 1
      || ['(', '{', '[']
        .iter()
        .any(|&c| opens(same, c) && !opens(next, c)))
}

/// The node printed into `shape`, or `None` when it does not fit.
pub fn rewrite(node: &SyntaxNode, shape: Shape) -> Option<String> {
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
      (text.width() <= shape.width).then_some(text)
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
      let inner = rewrite(&node.children()[0], shape.offset_left(1)?.sub_width(1)?)?;
      Some(format!("({})", inner))
    }
    // `f(x)?` with no chain: the call laid out, then the `?`.
    SyntaxKind::TryExpr => {
      let inner = rewrite(&node.children()[0], shape.sub_width(1)?)?;
      Some(format!("{}?", inner))
    }
    // A closure's body on its line; one that breaks only if it is a struct literal (under
    // `&`, `?`, a unary operator or a cast too), as the standard formatter allows a
    // block-like body; any other makes the layout around it break instead.
    SyntaxKind::ClosureExpr => {
      let body = node.children().into_iter().last()?;
      // A struct pattern among its parameters past `struct_lit_width`, which the standard
      // formatter breaks a field per line, is not laid out here.
      let wide = |fields: &SyntaxNode| {
        flat(fields, shape.limits.spacing).width().saturating_sub(4) > shape.limits.struct_lit
      };
      if node
        .children()
        .iter()
        .filter(|c| c.kind() == SyntaxKind::ParamList)
        .flat_map(|c| c.descendants())
        .any(|d| d.kind() == SyntaxKind::RecordPatFieldList && wide(&d))
      {
        return None;
      }
      let whole = flat(node, shape.limits.spacing);
      let prefix = whole
        .strip_suffix(&flat(&body, shape.limits.spacing))?
        .to_string();
      let inner = rewrite(&body, shape.offset_left(prefix.width())?)?;
      (!inner.contains('\n') || multi_line_body(&body)).then(|| format!("{}{}", prefix, inner))
    }
    // A prefix (`&`, `!`, `return `), then what it applies to laid out; what follows
    // `return` gets a column less (found by measuring the standard formatter).
    SyntaxKind::RefExpr
    | SyntaxKind::PrefixExpr
    | SyntaxKind::ReturnExpr
    | SyntaxKind::RefPat
    | SyntaxKind::BoxPat => {
      let inner_node = node.children().into_iter().last()?;
      let whole = flat(node, shape.limits.spacing);
      let inner_flat = flat(&inner_node, shape.limits.spacing);
      let prefix = whole.strip_suffix(&inner_flat)?.to_string();
      let inner_shape = shape.offset_left(prefix.width())?;
      let inner_shape = if node.kind() == SyntaxKind::ReturnExpr {
        inner_shape.sub_width(1)?
      } else {
        inner_shape
      };
      let inner = rewrite(&inner_node, inner_shape)?;
      Some(format!("{}{}", prefix, inner))
    }
    SyntaxKind::MacroExpr | SyntaxKind::MacroCall => rewrite_macro(node, shape),
    // A literal is kept as written however long; the line runs over, as it does for the
    // standard formatter.
    SyntaxKind::Literal => Some(flat(node, shape.limits.spacing)),
    SyntaxKind::PathExpr => rewrite_path(node, shape),
    SyntaxKind::OrPat => rewrite_or_pattern(node, shape),
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
    // The tail of a match arm's or closure's block of it alone is not followed yet.
    SyntaxKind::IfExpr
      if node
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::BlockExpr && !lines_block(&p)) =>
    {
      None
    }
    SyntaxKind::IfExpr => {
      rewrite_if_one_line(node, shape).or_else(|| rewrite_if_lines(node, shape, false))
    }
    SyntaxKind::LetExpr => rewrite_let_condition(node, shape),
    SyntaxKind::BlockExpr if joins(node) => rewrite_block(node, shape),
    // Anything else on one line, unless something in it the standard formatter would break
    // for its own width: then left as written.
    _ => {
      let text = flat(node, shape.limits.spacing);
      (text.width() <= shape.width && !breaks_inside(node, shape.limits)).then_some(text)
    }
  }
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
  // `prefix` holds the space after the last `|`.
  let needed = if unmeasured {
    prefix
  } else {
    prefix + 2 + if rest { 4 } else { 0 }
  };
  needed <= shape.width
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
      _ => false,
    }
  })
}

/// A `?` around a chain is part of the chain; around anything else it is a suffix.
fn chain_with_links(node: &SyntaxNode) -> bool {
  node.kind() == SyntaxKind::TryExpr && !chain(node, Spacing::default()).2.is_empty()
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
  if lines.any(|l| l.width() > shape.limits.max_width)
    || last_line_width(&text) > shape.indent + shape.offset + shape.width
  {
    return None;
  }
  Some(text)
}

/// Whether an `if` stands where the standard formatter may write it on one line: not as a
/// statement, and as a block's tail only from style edition 2024 (before, a tail is a
/// statement too).
fn if_on_one_line(node: &SyntaxNode, style_2024: bool) -> bool {
  match node.parent() {
    Some(p) if p.kind() == SyntaxKind::ExprStmt => false,
    // As the tail of a block of lines; a match arm's or closure's block of it alone the
    // standard formatter writes on lines.
    Some(p) if p.kind() == SyntaxKind::BlockExpr => style_2024 && lines_block(&p),
    Some(_) => true,
    None => false,
  }
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

fn macro_arguments(node: &SyntaxNode, spacing: Spacing) -> Result<MacroArguments, Unread> {
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
  if tree
    .descendants_with_tokens()
    .iter()
    .any(|e| e.kind().is_trivia() && e.kind() != SyntaxKind::Whitespace)
  {
    return Err(Declined);
  }
  let path = call
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::Path)
    .ok_or(Declined)?;
  let name = format!("{}!", flat(&path, spacing));
  let inner = tree.text().to_string();
  let delimiter = tree
    .children_with_tokens()
    .into_iter()
    .find(|e| !e.kind().is_trivia())
    .ok_or(Declined)?
    .kind();
  let (source, list_kind, open, close, array) = match delimiter {
    // `vec!(...)` the standard formatter writes as `vec![...]`, which changes tokens.
    SyntaxKind::LParen if name != "vec!" => (
      format!("fn f() {{\n__mto_macro{};\n}}", inner),
      SyntaxKind::ArgList,
      "(",
      ")",
      false,
    ),
    SyntaxKind::LBrack => (
      format!("fn f() {{\n{};\n}}", inner),
      SyntaxKind::ArrayExpr,
      "[",
      "]",
      true,
    ),
    // `vec!(...)` the standard formatter writes with brackets, which changes tokens.
    SyntaxKind::LParen => return Err(Declined),
    SyntaxKind::LBrace => return Err(Unread::Braced(format!("{} {}", name, inner))),
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
  if list
    .descendant_nodes_where(|kind| kind == SyntaxKind::BinExpr)
    .any(|n| comparison(&n) && n.children_iter().any(|c| comparison(&c)))
    || list
      .descendant_nodes_where(|kind| kind == SyntaxKind::RangeExpr)
      .any(|n| {
        n.children_iter()
          .any(|c| c.kind() == SyntaxKind::RangeExpr && (has_start(&n) || has_start(&c)))
      })
  {
    return Err(AsWritten);
  }
  // `<-` written touching (the obsolete in-place arrow) and `builtin #` rustc rejects or
  // keeps apart; a `#` outside an attribute is left as written.
  // Kinds and ranges only: no handles.
  let tokens: Vec<(SyntaxKind, &str, TextRange)> = list
    .token_texts()
    .filter(|(kind, _, _)| !kind.is_trivia())
    .collect();
  if tokens
    .windows(2)
    .any(|w| w[0].0 == SyntaxKind::Lt && w[1].0 == SyntaxKind::Minus && w[0].2.end == w[1].2.start)
    || tokens.iter().any(|t| t.0 == SyntaxKind::Pound)
  {
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

/// Whether a closure's body may span lines: a struct literal, seen through `&`, `?`, a unary
/// operator and a cast (the standard formatter's `allow_multi_line`; its blocks, `match`es
/// and loops are not laid out here).
fn multi_line_body(body: &SyntaxNode) -> bool {
  match body.kind() {
    SyntaxKind::RecordExpr => true,
    SyntaxKind::RefExpr | SyntaxKind::TryExpr | SyntaxKind::PrefixExpr | SyntaxKind::CastExpr => {
      body.children().first().is_some_and(multi_line_body)
    }
    _ => false,
  }
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

/// A binary expression as the standard formatter's `rewrite_all_pairs` lays it out: the
/// operands joined by the same operator taken as a list (`a && b && c`), on one line when
/// it fits, else each operand after the first on a line of its own one level in, its
/// operator first; a first operand no wider than the indentation keeps the next on its
/// line. Failing that, the two sides as `rewrite_pair` places them.
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
      rewrite_pair(&children[0], &children[1], &operator, shape)
    })
}

/// A block of one expression ([`joins`]): `{ expr }` on one line when it fits there, else
/// the expression on a line of its own, one level in.
fn rewrite_block(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let inside = lone_expression(node)?;
  let whole = flat(node, shape.limits.spacing);
  let prefix = &whole[..whole.find('{')?];
  // 4 = `{ ` and ` }`
  if let Some(text) = shape
    .offset_left(prefix.width() + 2)
    .and_then(|s| s.sub_width(2))
    .and_then(|s| rewrite(&inside, s))
    .filter(|t| !t.contains('\n'))
  {
    return Some(format!("{}{{ {} }}", prefix, text));
  }
  if prefix.width() + 1 > shape.width {
    return None;
  }
  let nested = shape.block_indent(shape.limits.tab);
  let text = rewrite(&inside, nested)?;
  Some(format!(
    "{}{{\n{}{}\n{}}}",
    prefix,
    " ".repeat(nested.indent),
    text,
    " ".repeat(shape.indent)
  ))
}

/// A chain, as the standard formatter's block-indented chain formatter lays it out.
fn rewrite_chain(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let (root, root_tries, links) = chain(node, shape.limits.spacing);
  let mut root_text = format!("{}{}", rewrite(&root, shape)?, "?".repeat(root_tries));
  let child_count = links.len();
  let mut links: &[Link] = &links;
  // A short root takes the first links onto its line.
  let tab_width = shape.limits.tab.saturating_sub(shape.offset);
  let mut joined = false;
  while root_text.width() <= tab_width && !root_text.contains('\n') && !links.is_empty() {
    let s = shape.offset_left(root_text.width())?;
    match rewrite_link(&links[0], s) {
      Some(text) => {
        // `0. .f()`: a float before a method keeps its space.
        if space::glues(&root_text, &text) {
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
  let all_in_one_line = rewrites.iter().all(|r| !r.contains('\n')) && one_line_budget > 0;
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
  for r in &rewrites[1..] {
    result.push_str(&connector);
    result.push_str(r);
  }
  wrap_str(result, shape)
}

/// An `if` whose branches each hold one expression, on lines: `if cond {`, each branch's
/// expression one level in, `} else {` or `} else if cond {`, and `}`, as the standard
/// formatter writes it where it does not go on one line. The condition is laid out as a
/// head's is: one breaking or too wide puts `{` on a line of its own, unless its last line
/// closes delimiters at the `if`'s indentation. `nested` for an `else if`, which follows
/// `} else ` (7 columns).
fn rewrite_if_lines(node: &SyntaxNode, shape: Shape, nested: bool) -> Option<String> {
  let children = node.children();
  let (condition, then, otherwise) = match children.as_slice() {
    [condition, then] => (condition, then, None),
    [condition, then, otherwise] => (condition, then, Some(otherwise)),
    _ => return None,
  };
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
  let inner = shape.block_indent(shape.limits.tab);
  let inner_pad = " ".repeat(inner.indent);
  let mut text = format!("if {}", condition);
  if breaks_brace {
    text.push_str(&format!("\n{}{{", pad));
  } else {
    text.push_str(" {");
  }
  let body = rewrite(&lone_expression(then)?, inner)?;
  text.push_str(&format!("\n{}{}\n{}}}", inner_pad, body, pad));
  match otherwise {
    None => {}
    Some(next) if next.kind() == SyntaxKind::IfExpr => {
      let line = Shape {
        offset: 0,
        width: shape.limits.max_width.checked_sub(shape.indent)?,
        ..shape
      };
      text.push_str(" else ");
      text.push_str(&rewrite_if_lines(next, line, true)?);
    }
    Some(block) => {
      let body = rewrite(&lone_expression(block)?, inner)?;
      text.push_str(&format!(" else {{\n{}{}\n{}}}", inner_pad, body, pad));
    }
  }
  Some(text)
}

/// An `if` with an `else`, each holding one expression, on one line as the standard
/// formatter's `rewrite_single_line` writes it (`if c { a } else { b }`), within the shape
/// and `single_line_if_else_max_width`; never as a statement, where its blocks keep lines.
fn rewrite_if_one_line(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let (condition, then, otherwise) = if_parts(node)?;
  if !if_on_one_line(node, shape.limits.style_2024) || otherwise.kind() != SyntaxKind::BlockExpr {
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
  let text = match &link.args {
    Some(args) => rewrite_list(&link.head, args, shape, shape.limits.fn_call, "(", ")")?,
    // A field or `.await` is printed without a width check, as the standard formatter
    // prints it; whether the chain fits is decided around it.
    None => link.head.clone(),
  };
  Some(format!("{}{}", text, tries))
}

/// `ident(items)`: calls, arrays and tuples, with the standard formatter's tactics: on one
/// line within `max_width`, the last item overflowing, packed when every item is short and
/// simple, else one per line.
fn rewrite_list(
  ident: &str,
  items: &[SyntaxNode],
  shape: Shape,
  max_width: usize,
  open: &str,
  close: &str,
) -> Option<String> {
  if items.is_empty() {
    return wrap_str(format!("{}{}{}", ident, open, close), shape);
  }
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
  for item in items {
    texts.push(rewrite(item, nested)?);
  }
  let mut tactic = Tactic::Vertical;
  // The shape the last item failed to overflow into.
  let mut over_shape = None;
  if !items.is_empty() {
    let last = items.len() - 1;
    let combine = items.len() == 1 && used < shape.limits.tab;
    // A slice pattern's last element does not overflow; among several closures the last
    // does not either (`args_have_many_closure`).
    let slice = items[0]
      .parent()
      .is_some_and(|p| p.kind() == SyntaxKind::SlicePat);
    let closures = items
      .iter()
      .filter(|i| i.kind() == SyntaxKind::ClosureExpr)
      .count();
    let overflow = !slice && closures < 2 && (combine || can_overflow(&items[last], items.len()));
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
      over_shape = last_item_shape(items, &texts, overflow_shape, max_width);
      over_shape.and_then(|s| rewrite(&items[last], s))
    } else {
      None
    };
    if placeholder.is_some() {
      over_shape = None;
    }
    // `vec![x; n]` the standard formatter overflows across lines when it does not fit on
    // the line, which is not done here: the list fails.
    let repeat_macro = |item: &SyntaxNode| {
      matches!(item.kind(), SyntaxKind::MacroExpr | SyntaxKind::MacroCall)
        && macro_arguments(item, shape.limits.spacing).is_ok_and(|arguments| {
          arguments
            .list
            .children_with_tokens()
            .iter()
            .any(|e| e.kind() == SyntaxKind::Semi)
        })
    };
    if overflow && placeholder.is_none() && repeat_macro(&items[last]) {
      return None;
    }
    let mut probe = texts.clone();
    if let Some(over) = &placeholder {
      probe[last] = over.lines().next().unwrap_or("").to_string();
    }
    tactic = definitive(&probe, max_width.min(one_line_width));
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
        } else {
          tactic = definitive(&texts, max_width.min(one_line_width));
          let special = SPECIAL_MACROS
            .iter()
            .find(|(name, _)| *name == ident)
            .map(|&(_, before)| before);
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
  // A closure last in a list that cannot overflow as it is the standard formatter overflows
  // with its body in a block (`|x| {`) when that fits, which is not done here: the list
  // fails, so that what holds it is laid out another way or left as written. When that does
  // not fit either, or the body is one it puts in no block (a macro called with nothing),
  // the closure is an item on a line of its own.
  if tactic != Tactic::Horizontal
    && let Some(closure) = items.last().filter(|i| i.kind() == SyntaxKind::ClosureExpr)
    && let Some(body) = closure.children().into_iter().last()
    && !empty_macro(&body)
    && over_shape.is_some_and(|s: Shape| block_fits(closure, &body, s))
  {
    return None;
  }
  let trailing = if !shape.limits.comma_as_written || items.last().is_some_and(comma_after) {
    ","
  } else {
    ""
  };
  let body = match tactic {
    // Attribute and macro arguments keep a written trailing comma on one line too.
    Tactic::Horizontal if shape.limits.comma_as_written => {
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
      for text in &texts {
        // The item with its comma, and the space after the comma before it.
        let added = if line.is_empty() {
          text.width() + 1
        } else {
          text.width() + 3
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
  let single = tactic == Tactic::Horizontal && extend <= available;
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
  if single {
    wrap_str(text, shape)
  } else {
    text
      .lines()
      .skip(1)
      .all(|l| l.width() <= shape.limits.max_width)
      .then_some(text)
  }
}

/// A macro call laid out as a call (its arguments in parentheses) or an array (in brackets),
/// when the standard formatter reads its arguments as such; one it cannot read is left as
/// written, on one line where it fits.
fn rewrite_macro(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let MacroArguments {
    name,
    list,
    open,
    close,
    array,
  } = match macro_arguments(node, shape.limits.spacing) {
    Ok(arguments) => arguments,
    Err(Unread::AsWritten) => {
      let text = node.text().to_string();
      return (!text.contains('\n') && text.width() <= shape.width).then_some(text);
    }
    Err(Unread::Braced(text)) => {
      return (!text.contains('\n') && text.width() <= shape.width).then_some(text);
    }
    Err(Unread::Declined) => return None,
  };
  let width = if array {
    shape.limits.array
  } else {
    shape.limits.fn_call
  };

  // `vec![x; n]`: on one line when it fits there; the standard formatter's layout across
  // lines (`vec_with_semi`) is not followed here: left as written.
  if list
    .children_with_tokens()
    .iter()
    .any(|e| e.kind() == SyntaxKind::Semi)
  {
    let text = format!("{}{}", name, flat(&list, shape.limits.spacing));
    return (text.width() <= shape.width).then_some(text);
  }
  let items = list_items(&list);
  // Generic arguments that would have to break are not laid out yet (as in statements): an
  // argument holding them must fit on its own line with its comma when the list breaks.
  let own_line = shape.block_indent(shape.limits.tab).width.saturating_sub(1);
  if items.iter().any(|item| {
    item
      .descendants()
      .iter()
      .any(|d| d.kind() == SyntaxKind::GenericArgList)
      && flat(item, shape.limits.spacing).width() > own_line
  }) {
    return None;
  }
  let commas = list
    .children_with_tokens()
    .iter()
    .filter(|e| e.kind() == SyntaxKind::Comma)
    .count();
  if commas + 1 != items.len() && commas != items.len() {
    return None;
  }
  // An attribute among the arguments is not an argument of its own; blocks, `match` arms,
  // `let else` and macros in braces are not laid out yet: left as written, as a statement
  // holding them is.
  if list.descendants().iter().any(|d| {
    matches!(
      d.kind(),
      SyntaxKind::Attr | SyntaxKind::BlockExpr | SyntaxKind::MatchArmList | SyntaxKind::LetElse
    )
  }) || list.descendants_with_tokens().iter().any(
    |e| matches!(e, SyntaxElement::Token(t) if t.kind().is_literal() && t.text().contains('\n')),
  ) {
    return None;
  }
  let shape = Shape {
    limits: Limits {
      // `vec![...]` follows the array rule, unless it is inside another macro.
      comma_as_written: name != "vec!" || shape.limits.comma_as_written,
      ..shape.limits
    },
    ..shape
  };
  rewrite_list(&name, &list_items(&list), shape, width, open, close)
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

/// The two sides of a binary expression on one line when they fit, the right one after a
/// left one no wider than the indentation or opening a block even spanning lines; else the
/// right side on the next line one level in, after the operator (`rewrite_pair`).
fn rewrite_pair(
  left: &SyntaxNode,
  right: &SyntaxNode,
  operator: &str,
  shape: Shape,
) -> Option<String> {
  let used = shape.indent + shape.offset;
  let left_shape = Shape {
    width: shape.limits.max_width.saturating_sub(used),
    ..shape
  };
  let left_text = rewrite(left, left_shape)?;
  let infix = operator.width() + 2;
  if let Some(right_text) = shape
    .offset_left(last_line_width(&left_text) + infix)
    .and_then(|s| rewrite(right, s))
  {
    let same_line = left_text.width() <= shape.limits.tab
      || right_text.lines().next().is_some_and(|l| l.ends_with('{'));
    if (!right_text.contains('\n') || same_line)
      && last_line_width(&left_text) + infix + first_line_width(&right_text) <= shape.width
    {
      return Some(format!("{} {} {}", left_text, operator, right_text));
    }
  }
  let nested = shape.block_indent(shape.limits.tab);
  let right_shape = Shape {
    width: nested.width.checked_sub(shape.rhs_overhead())?,
    ..nested
  }
  .offset_left(operator.width() + 1)?;
  let right_text = rewrite(right, right_shape)?;
  Some(format!(
    "{}\n{}{} {}",
    left_text,
    " ".repeat(nested.indent),
    operator,
    right_text
  ))
}

/// A path on one line; one ending in generic arguments that the standard formatter finds
/// too wide (its `::<` and what follows, less the `<>`, past the shape) with its arguments
/// one per line, one level in, and `>` back at the path's indentation.
fn rewrite_path(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let spacing = shape.limits.spacing;
  let text = flat(node, spacing);
  let generics = node
    .descendants()
    .into_iter()
    .filter(|d| d.kind() == SyntaxKind::GenericArgList)
    .find(|d| d.text_range().end == node.text_range().end);
  let Some(generics) =
    generics.filter(|g| text.width().saturating_sub(2) > shape.width && breakable_generics(g))
  else {
    return (text.width() <= shape.width && !breaks_inside(node, shape.limits)).then_some(text);
  };
  // The list holds its `::` too: the head runs to its `<`.
  let inside = flat(&generics, spacing);
  let head = format!(
    "{}{}",
    text.strip_suffix(&inside)?,
    &inside[..inside.find('<')? + 1]
  );
  let nested = shape.block_indent(shape.limits.tab);
  let pad = " ".repeat(nested.indent);
  let mut out = head.clone();
  for arg in generics.children() {
    let arg = flat(&arg, spacing);
    // 1 = `,`
    if arg.width() + 1 > nested.width {
      return None;
    }
    out.push_str(&format!("\n{}{},", pad, arg));
  }
  out.push_str(&format!("\n{}>", " ".repeat(shape.indent)));
  (head.width() < shape.width).then_some(out)
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
  // Fields followed by a blank line after their comma, before another field or the rest:
  // laid out a field per line, the standard formatter keeps one there.
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
      // Not before `}`.
      _ => {
        if blank && element.kind() != SyntaxKind::RBrace {
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
        let parts = n.children();
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
                let next_shape = v_shape.block_indent(shape.limits.tab);
                let next = rewrite(value, next_shape)?;
                format!("{}:\n{}{}", name, " ".repeat(next_shape.indent), next)
              }
            }
          }
          _ => flat(n, shape.limits.spacing),
        };
        fields.push(text);
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
  wrap_str(text, shape)
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

/// Whether `text` fits: its first line in the shape's width, the rest within the line width.
fn wrap_str(text: String, shape: Shape) -> Option<String> {
  let mut lines = text.lines();
  if lines.next().is_some_and(|l| l.width() > shape.width) {
    return None;
  }
  if lines.any(|l| l.width() > shape.limits.max_width) {
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
