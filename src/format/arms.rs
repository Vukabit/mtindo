//! A `match`'s arms laid out as the standard formatter's `rewrite_match_arm` lays them out:
//! the patterns (alternatives on a line, packed when they are short, else one per line with
//! `|` first), the guard after them or on the next line, `=>`, and the body. A body that is
//! an expression goes on the arm's line when it fits there; a block's, a `match`'s or a
//! `loop`'s `{` follows `=>`, its insides left to the whole-file pass. A body the standard
//! formatter would put in a block of its own, or a block it would unwrap, changes tokens:
//! the arm is left as written.

use SyntaxKind as K;

use super::expr::{
  first_line_width, flat, joins, last_line_width, lone_expression, multi_line_condition,
  plain_block, prefer_next_line, rewrite,
};
use super::items::header_start;
use super::shape::{Shape, Spacing};
use super::stmt::{Head, clean_expression, compared, rewrite_control_head, statement_shape};
use crate::options::format::FormatConfig;
use crate::syntax::{Edition, SyntaxKind, SyntaxNode};
use crate::text::unicode::str_width;

/// An arm laid out.
pub enum Arm {
  /// The whole arm, its comma included.
  Whole(String),
  /// Up to its body's `{`, the rest left to the whole-file pass.
  Head(Head),
}

/// The arm starting at column `indent` on a line of its own, laid out; `None` for one left
/// as written. One with a leading `|` is left as written: laid out alone, its `match` may be
/// one the standard formatter keeps as written whole (a comment among an arm's patterns).
pub fn rewrite_arm(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<Arm> {
  let leading = node
    .descendants_where(|kind| kind == K::Pipe)
    .any(|e| matches!(&e, crate::syntax::SyntaxElement::Token(t) if super::expr::leading_pipe(t)));
  if leading {
    return None;
  }
  rewrite_arm_in(node, indent, config, false)
}

/// An arm as [`rewrite_arm`] lays it out; inside a macro's arguments (`inside_macro`) its
/// body keeps or lacks its block as written, and one that does not fit on the arm's line
/// goes on the next with no block around it, as the standard formatter will not add or
/// remove braces there.
pub fn rewrite_arm_in(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
  inside_macro: bool,
) -> Option<Arm> {
  if node.kind() != K::MatchArm || config.hard_tabs || config.tab_spaces == 0 {
    return None;
  }
  let start = header_start(node)?;
  let parts: Vec<SyntaxNode> = node
    .children_iter()
    .filter(|c| c.kind() != K::Attr)
    .collect();
  let pattern = parts.first()?;
  let guard = parts.iter().find(|c| c.kind() == K::MatchGuard);
  let body = parts
    .last()
    .filter(|b| *b != pattern && Some(*b) != guard)?;
  // Comments anywhere.
  if !clean_expression(pattern) {
    return None;
  }
  let condition = match guard {
    Some(guard) => Some(guard.children_iter().next()?),
    None => None,
  };
  // A condition of operators holding a comment, which their layout would lose: as written
  // (the standard formatter's `recover_comment_removed`).
  let written_guard = condition.as_ref().is_some_and(|c| {
    c.kind() == K::BinExpr && c.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment))
  });
  if !written_guard && condition.as_ref().is_some_and(|c| !clean_expression(c)) {
    return None;
  }
  let mut shape = statement_shape(indent, config)?;
  shape.limits.inside_macro = inside_macro;
  let tab = config.tab_spaces;
  // 5 = ` => {`
  let pattern_text = rewrite(pattern, shape.sub_width(5)?)?;
  let guard_text = match &condition {
    Some(condition) => rewrite_guard(condition, shape, &pattern_text, tab, written_guard)?,
    None => String::new(),
  };
  let lhs = format!("{}{}", pattern_text, guard_text);
  // A guard on a line of its own keeps the body off the arm's line, as do a block's inner
  // attributes (the body's attributes, in `rewrite_match_body`).
  let forbid_same_line = guard_text.contains('\n')
    || body.kind() == K::BlockExpr && body.children_iter().any(|c| c.kind() == K::Attr);
  // The columns `lhs`'s last line takes past the arm's indentation.
  let used = if lhs.contains('\n') {
    last_line_width(&lhs).saturating_sub(indent)
  } else {
    str_width(&lhs)
  };
  let written_comma = node
    .children_with_tokens_iter()
    .find(|e| e.kind() == K::Comma);
  let arrow = if forbid_same_line {
    format!(" =>\n{}", " ".repeat(indent))
  } else {
    " => ".to_string()
  };
  // A plain block of one expression the standard formatter takes the expression out of
  // (`flatten_arm_body`): not a macro call's, nor a control expression's whose condition
  // would span lines.
  let written_body = body.clone();
  let mut body = body.clone();
  while !inside_macro
    && body.kind() == K::BlockExpr
    && plain_block(&body)
    && !lone_macro(&body)
    && let Some(inner) = lone_expression(&body)
    && !shape
      .offset_left(used + 4)
      .is_some_and(|s| multi_line_condition(&inner, s))
  {
    body = inner;
  }
  let body = &body;
  let mut one_line_comment = false;
  let laid = match body.kind() {
    K::BlockExpr => {
      let prefixed = body
        .children_with_tokens_iter()
        .any(|e| matches!(e.kind(), K::UnsafeKw | K::ConstKw | K::AsyncKw));
      let opened = body
        .children_with_tokens_iter()
        .any(|e| e.kind() == K::LBrace);
      let empty = opened
        && !body
          .children_with_tokens_iter()
          .any(|e| !e.kind().is_trivia() && !matches!(e.kind(), K::LBrace | K::RBrace | K::Label))
        && !prefixed;
      // Comments alone in it: what is between the braces kept on the arm's line when it is
      // one line and no line comment, and fits there (the standard formatter's
      // `rewrite_empty_block`); else the block on lines.
      let inside = {
        let text = body.text();
        let from = text.find('{').map_or(0, |i| i + 1);
        let to = text.rfind('}').unwrap_or(text.len());
        text[from..to.max(from)].trim().to_string()
      };
      if empty && !inside.is_empty() {
        if !inside.contains('\n')
          && !inside.starts_with("//")
          && !forbid_same_line
          && indent + used + 4 + str_width(&inside) + 4 <= config.max_width
        {
          one_line_comment = true;
          Arm::Whole(format!("{} => {{ {} }}", lhs, inside))
        } else {
          let open = body
            .children_with_tokens_iter()
            .find(|e| e.kind() == K::LBrace)?
            .text_range()
            .start as usize;
          let tail = match &written_comma {
            Some(comma) => vec![(comma.text_range().start as usize, K::Comma)],
            None => Vec::new(),
          };
          Arm::Head(Head {
            text: format!("{}{}", lhs, arrow),
            open,
            tail,
            keep_comma: false,
            spaced: Vec::new(),
            links: Vec::new(),
          })
        }
      } else if empty {
        // `{}`, after the arrow even below a guard of its own line; its `}` on the next line
        // when `{}` runs past the line width.
        if indent + used + 4 + 2 > config.max_width {
          Arm::Whole(format!("{} => {{\n{}}}", lhs, " ".repeat(indent)))
        } else {
          Arm::Whole(format!("{} => {{}}", lhs))
        }
      } else {
        // On the arm's line when it fits there (`unsafe { f() }`), else its `{` after
        // `=>`; a block with a prefix keeps a comma after it, a plain one none.
        if prefixed
          && !forbid_same_line
          && joins_in_arm(body)
          && let Some(text) = shape
            .offset_left(used + 4)
            .and_then(|s| s.sub_width(1))
            .and_then(|s| rewrite(body, s))
            .filter(|t| !t.contains('\n'))
        {
          Arm::Whole(format!("{}{}{},", lhs, arrow, text))
        } else {
          let open = body
            .children_with_tokens_iter()
            .find(|e| e.kind() == K::LBrace)?
            .text_range()
            .start as usize;
          let prefix = flat_prefix(body, config)?;
          let tail = match (&written_comma, prefixed) {
            (Some(comma), _) => vec![(comma.text_range().start as usize, K::Comma)],
            (None, true) => return None,
            (None, false) => Vec::new(),
          };
          Arm::Head(Head {
            text: format!("{}{}{}", lhs, arrow, prefix),
            open,
            tail,
            keep_comma: prefixed,
            spaced: Vec::new(),
            links: Vec::new(),
          })
        }
      }
    }
    // Block-like, the arm's line keeps their head; a comma follows them.
    // Written so, with a comma after them (one taken out of its block is laid out whole
    // below: the pass would write the block's `}`).
    K::MatchExpr | K::LoopExpr
      if !forbid_same_line && written_comma.is_some() && *body == written_body =>
    {
      let comma = written_comma.as_ref()?;
      let head = rewrite_control_head(body, indent, indent + used + 4, config)?;
      Arm::Head(Head {
        text: format!("{}{}{}", lhs, arrow, head.text),
        open: head.open,
        tail: vec![(comma.text_range().start as usize, K::Comma)],
        keep_comma: true,
        spaced: Vec::new(),
        links: Vec::new(),
      })
    }
    _ => {
      if !clean_expression(body) {
        return None;
      }
      // 4 = ` => `, 1 = `,`
      let same_shape = shape.offset_left(used + 4).and_then(|s| s.sub_width(1));
      let same = if forbid_same_line {
        None
      } else {
        same_shape.and_then(|s| super::expr::rewrite_or_written(body, s))
      };
      let budget = same_shape.map_or(0, |s| s.width);
      let fits = |text: &String| !text.contains('\n') && str_width(text) <= budget;
      match same {
        Some(same) if fits(&same) => Arm::Whole(format!("{}{}{},", lhs, arrow, same)),
        same => {
          // On the next line, in a block of its own, when that looks better.
          let next = super::expr::rewrite_or_written(
            body,
            Shape {
              indent: indent + tab,
              offset: 0,
              width: config.max_width.checked_sub(indent + tab)?,
              ..shape
            },
          );
          // After a guard on a line of its own, `{` starts the next line.
          let open = if forbid_same_line {
            format!("\n{}{{", " ".repeat(indent))
          } else {
            " {".to_string()
          };
          let next_line = |next: String| {
            if inside_macro {
              return Arm::Whole(format!("{} =>\n{}{},", lhs, " ".repeat(indent + tab), next));
            }
            Arm::Whole(format!(
              "{} =>{}\n{}{}\n{}}}",
              lhs,
              open,
              " ".repeat(indent + tab),
              next,
              " ".repeat(indent)
            ))
          };
          let on_line = |same: String| Arm::Whole(format!("{}{}{},", lhs, arrow, same));
          let extend = extends(body);
          match (same, next) {
            (Some(same), Some(next)) if prefer_next_line(&same, &next) => next_line(next),
            (Some(same), _) if extend && first_line_width(&same) <= budget => on_line(same),
            (Some(same), Some(next)) if same.contains('\n') => next_line(next),
            (None, Some(next)) => next_line(next),
            (None, None) => return None,
            (Some(same), _) => on_line(same),
          }
        }
      }
    }
  };
  // The same tokens, commas aside.
  let laid_text = match &laid {
    Arm::Whole(text) => text.as_str(),
    Arm::Head(head) => head.text.as_str(),
  };
  let end = match &laid {
    Arm::Whole(_) => node.text_range().end as usize,
    Arm::Head(head) => head.open,
  };
  // No comment in what is laid out here: a block body's are its own, or kept as written.
  let comment_end = if one_line_comment {
    body.text_range().start as usize
  } else {
    end
  };
  // Or, laid out whole, ones its body's layout placed: each kept.
  let comments: Vec<String> = node
    .token_texts()
    .filter(|&(kind, _, range)| {
      let at = range.start as usize;
      at >= start && at < comment_end && matches!(kind, K::LineComment | K::BlockComment)
    })
    .map(|(_, text, _)| text.trim_end().to_string())
    .collect();
  // A guard's as written keeps its own.
  let in_guard = |at: usize| {
    written_guard
      && condition
        .as_ref()
        .is_some_and(|c| (c.text_range().start as usize..c.text_range().end as usize).contains(&at))
  };
  let placed = match &laid {
    Arm::Whole(text) => {
      let body_start = body.text_range().start as usize;
      node.token_texts().all(|(kind, _, range)| {
        let at = range.start as usize;
        !(at >= start
          && at < body_start
          && matches!(kind, K::LineComment | K::BlockComment)
          && !in_guard(at))
      }) && comments
        .iter()
        .all(|c| text.matches(c.as_str()).count() >= comments.iter().filter(|d| *d == c).count())
    }
    // A head holds no comment but its guard's.
    Arm::Head(_) => node.token_texts().all(|(kind, _, range)| {
      let at = range.start as usize;
      !(at >= start
        && at < comment_end
        && matches!(kind, K::LineComment | K::BlockComment)
        && !in_guard(at))
    }),
  };
  if !comments.is_empty() && !placed {
    return None;
  }
  // Less those a line drops (a leading `|`, nested parentheses).
  let dropped = super::expr::dropped_tokens(node, Spacing::of(config));
  // `vec!(..)` and `vec! {..}` are written `vec![..]`.
  let brackets = super::stmt::vec_delimiters(node);
  let written: String = node
    .token_texts()
    .filter_map(|(kind, text, range)| {
      let at = range.start as usize;
      if at < start || at >= end || dropped.contains(&at) {
        return None;
      }
      match brackets.iter().find(|b| b.0 == at) {
        Some(b) => Some(b.1),
        None => compared(kind).then_some(text),
      }
    })
    .collect();
  let ours: String = crate::syntax::parse_tokens(laid_text, Edition::E2021)
    .into_iter()
    .filter(|(kind, _)| compared(*kind))
    .map(|(_, text)| text)
    .collect();
  (written == ours).then_some(laid)
}

/// Whether the standard formatter keeps the body on the arm's line even spanning lines
/// (`can_flatten_block_around_this`): a call, chain, macro call, struct literal, array,
/// tuple, closure or block, seen through `&`, `?`, a unary operator, indexing and a cast.
fn extends(body: &SyntaxNode) -> bool {
  match body.kind() {
    K::IfExpr | K::ForExpr | K::WhileExpr => false,
    K::LoopExpr
    | K::MatchExpr
    | K::BlockExpr
    | K::ClosureExpr
    | K::ArrayExpr
    | K::CallExpr
    | K::MethodCallExpr
    | K::MacroExpr
    | K::RecordExpr
    | K::TupleExpr => true,
    K::RefExpr | K::TryExpr | K::PrefixExpr | K::IndexExpr | K::CastExpr => {
      body.children_iter().next().is_some_and(|c| extends(&c))
    }
    _ => false,
  }
}

/// The block's tokens before its `{` (`unsafe `, `'a: `), with a space after them.
fn flat_prefix(block: &SyntaxNode, config: &FormatConfig) -> Option<String> {
  let whole = flat(block, Spacing::of(config));
  let prefix = whole[..whole.find('{')?].trim_end();
  Some(if prefix.is_empty() {
    String::new()
  } else {
    format!("{} ", prefix)
  })
}

/// Whether a block of one expression after a prefix goes on the arm's line, as one does
/// where an expression goes.
fn joins_in_arm(block: &SyntaxNode) -> bool {
  joins(block)
}

/// Whether a block holds one macro call and nothing else (`{ m!() }`), which the standard
/// formatter keeps in its block.
fn lone_macro(block: &SyntaxNode) -> bool {
  lone_expression(block).is_some_and(|e| matches!(e.kind(), K::MacroExpr | K::MacroCall))
}

/// The guard after the patterns (` if cond`) when it fits there, a condition spanning lines
/// only after patterns no wider than the indentation; else on the next line, one level in
/// (the standard formatter's `rewrite_guard`).
fn rewrite_guard(
  condition: &SyntaxNode,
  shape: Shape,
  pattern: &str,
  tab: usize,
  written: bool,
) -> Option<String> {
  let laid = |s: Shape| {
    if written {
      Some(condition.text().to_string().trim().to_string())
    } else {
      rewrite(condition, s)
    }
  };
  let last = pattern.rsplit('\n').next().unwrap_or("");
  let pattern_width = str_width(last.trim());
  let multi_line_pattern = pattern.contains('\n') && pattern_width > tab;
  if !multi_line_pattern
    && let Some(same) = shape
      .offset_left(pattern_width + 4)
      .and_then(|s| s.sub_width(5))
      .and_then(laid)
    && (!same.contains('\n') || pattern_width <= tab)
  {
    return Some(format!(" if {}", same));
  }
  let nested = shape.block_indent(tab);
  let text = laid(nested.offset_left(3)?.sub_width(5)?)?;
  Some(format!("\n{}if {}", " ".repeat(nested.indent), text))
}
