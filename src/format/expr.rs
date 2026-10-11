//! Expressions printed into the space left for them: the standard formatter's width-budget
//! rewriting, ported. `rewrite(node, shape)` returns the
//! node's text in `shape`, or `None` when it cannot fit; callers try their layouts in order
//! and keep the first that works. Calls, method chains, struct literals, arrays and tuples
//! are laid out; other expressions print flat for now.

use super::shape::*;
use super::space;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
use crate::text::unicode::str_width;

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

/// Whether the last of `count` arguments may overflow: a closure or block always; a call,
/// struct literal, array or tuple when it is the only argument.
pub fn can_overflow(node: &SyntaxNode, count: usize) -> bool {
  match node.kind() {
    SyntaxKind::ClosureExpr | SyntaxKind::BlockExpr => true,
    SyntaxKind::RecordExpr
    | SyntaxKind::ArrayExpr
    | SyntaxKind::CallExpr
    | SyntaxKind::MethodCallExpr
    | SyntaxKind::TupleExpr => count == 1,
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
  let config = spacing.config();
  let tokens: Vec<SyntaxToken> = node
    .descendants_with_tokens()
    .into_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t),
      _ => None,
    })
    .collect();
  let mut out = String::new();
  for (i, token) in tokens.iter().enumerate() {
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
    SyntaxKind::RefExpr | SyntaxKind::FieldExpr | SyntaxKind::TryExpr | SyntaxKind::PrefixExpr => {
      node.children().first().is_some_and(is_simple)
    }
    SyntaxKind::IndexExpr => node.children().iter().all(is_simple),
    _ => false,
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

/// The expressions of a list: call arguments, array or tuple elements.
pub fn list_items(node: &SyntaxNode) -> Vec<SyntaxNode> {
  node.children()
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
    SyntaxKind::ArrayExpr if flat(node, shape.limits.spacing).contains(';') => {
      let text = flat(node, shape.limits.spacing);
      (text.width() <= shape.width).then_some(text)
    }
    SyntaxKind::ArrayExpr | SyntaxKind::SlicePat => {
      rewrite_list("", &list_items(node), shape, shape.limits.array, "[", "]")
    }
    SyntaxKind::TupleStructPat => {
      let children = node.children();
      let path = rewrite(children.first()?, shape)?;
      rewrite_list(&path, &children[1..], shape, shape.limits.fn_call, "(", ")")
    }
    SyntaxKind::TupleExpr | SyntaxKind::TuplePat => {
      let items = list_items(node);
      // `(a,)` is a one-element tuple; `(..)` is not.
      if items.len() == 1 && items[0].kind() != SyntaxKind::RestPat {
        let inner = rewrite(&items[0], shape.offset_left(1)?.sub_width(2)?)?;
        return Some(format!("({},)", inner));
      }
      rewrite_list("", &items, shape, shape.limits.fn_call, "(", ")")
    }
    SyntaxKind::ParenExpr => {
      let inner = rewrite(&node.children()[0], shape.offset_left(1)?.sub_width(1)?)?;
      Some(format!("({})", inner))
    }
    SyntaxKind::RefExpr | SyntaxKind::PrefixExpr => {
      let inner_node = node.children().into_iter().last()?;
      let whole = flat(node, shape.limits.spacing);
      let inner_flat = flat(&inner_node, shape.limits.spacing);
      let prefix = whole.strip_suffix(&inner_flat)?.to_string();
      let inner = rewrite(&inner_node, shape.offset_left(prefix.width())?)?;
      Some(format!("{}{}", prefix, inner))
    }
    // A literal is kept as written however long; the line runs over, as it does for the
    // standard formatter.
    SyntaxKind::Literal => Some(flat(node, shape.limits.spacing)),
    _ => {
      let text = flat(node, shape.limits.spacing);
      (text.width() <= shape.width).then_some(text)
    }
  }
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
  let last_shape = if all_in_one_line {
    shape
  } else if extendable {
    child_shape
  } else {
    child_shape.sub_width(shape.rhs_overhead())?
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
      let own_line = child_shape.sub_width(shape.rhs_overhead())?;
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
  let last_text = match last_text {
    Some(t) => t,
    None => rewrite_link(last, last_shape)?,
  };
  rewrites.push(last_text);
  let connector = if fits_single_line {
    String::new()
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

/// A chain link printed into `shape`, with its `?`s.
fn rewrite_link(link: &Link, shape: Shape) -> Option<String> {
  let tries = "?".repeat(link.tries);
  let shape = shape.sub_width(link.tries)?;
  let text = match &link.args {
    Some(args) => rewrite_list(&link.head, args, shape, shape.limits.fn_call, "(", ")")?,
    None => {
      let t = link.head.clone();
      if t.width() > shape.width {
        return None;
      }
      t
    }
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
  if !items.is_empty() {
    let last = items.len() - 1;
    let combine = items.len() == 1 && used < shape.limits.tab;
    let overflow = combine || can_overflow(&items[last], items.len());
    // A method chain overflowing a list it is not combined with keeps its links on one
    // line, though its last call's arguments may break (the standard formatter's
    // `force_one_line_chain`).
    let one_line_chain = !combine && is_method_call(&items[last]);
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
      last_item_shape(items, &texts, overflow_shape, max_width)
        .and_then(|s| rewrite(&items[last], s))
        .filter(|text| {
          !one_line_chain
            || !text.lines().skip(1).any(|l| {
              let l = l.trim_start();
              l.starts_with('.') && !l.starts_with("..")
            })
        })
    } else {
      None
    };
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
          if tactic == Tactic::Vertical
            && items.iter().all(is_simple)
            && texts.iter().all(|t| t.width() <= shape.limits.short_item)
          {
            tactic = Tactic::Mixed;
          }
        }
      }
    }
  }
  let trailing = if !shape.limits.attribute || items.last().is_some_and(comma_after) {
    ","
  } else {
    ""
  };
  let body = match tactic {
    // Attribute arguments keep a written trailing comma on one line too.
    Tactic::Horizontal if shape.limits.attribute => {
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
        let added = if line.is_empty() {
          text.width() + 1
        } else {
          text.width() + 2
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
  for element in list.children_with_tokens() {
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
          [name, value] => {
            let name = flat(name, shape.limits.spacing);
            let value = rewrite(value, v_shape.sub_width(1)?.offset_left(name.width() + 2)?)?;
            format!("{}: {}", name, value)
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
  let horizontal = h_width.is_some_and(|w| definitive(&all, w) == Tactic::Horizontal);
  let text = if horizontal {
    format!("{} {{ {} }}", path, all.join(", "))
  } else {
    let pad = " ".repeat(v_shape.indent);
    let mut body = String::new();
    for field in &fields {
      body.push_str(&format!("\n{}{},", pad, field));
    }
    if let Some(s) = &spread {
      body.push_str(&format!("\n{}{}", pad, s));
    }
    format!("{} {{{}\n{}}}", path, body, " ".repeat(shape.indent))
  };
  wrap_str(text, shape)
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
