//! Types printed into the space left for them, as the standard formatter's `types.rs` lays
//! them out: a path's generic arguments as a list in angle brackets (on the line when they
//! fit, else one per line one level in, the last overflowing when it may), a function type's
//! parameters as a list in parentheses with its return type after them, a tuple's elements
//! as a call's arguments, bounds joined by `+` and broken before each `+` when they do not
//! fit, and a reference, pointer, slice or `dyn`/`impl` before what it applies to.
//! `rewrite_type` returns `None` when the type does not fit, or holds anything laying it
//! out would lose (a comment, an attribute).

use SyntaxKind as K;

use super::expr::{first_line_width, flat, last_line_extendable, last_line_width, rewrite};
use super::shape::{Limits, Shape};
use crate::options::format::FormatConfig;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
use crate::text::unicode::str_width;

/// Whether a comment is alone in a function type's parentheses (`Fn(/* c */)`), on a line,
/// which its layout keeps.
pub fn alone_in_function_type(comment: &SyntaxToken) -> bool {
  matches!(comment.kind(), K::LineComment | K::BlockComment)
    && !comment.text().contains('\n')
    && comment.parent().kind() == K::ParenthesizedArgList
    && (comment.parent().children_iter().next().is_none() || before_parameter(comment))
}

/// `head` and then a right-hand side `rhs` lays out into a shape: after a space on the
/// head's last line when it fits there on one line, else on the next line one level in when
/// that is better (the standard formatter's `rewrite_assign_rhs` and `choose_rhs`).
pub fn assign_rhs(
  head: &str,
  shape: Shape,
  rhs: impl Fn(Shape) -> Option<String>,
) -> Option<String> {
  let used = if head.contains('\n') {
    last_line_width(head).saturating_sub(shape.indent + shape.align)
  } else {
    last_line_width(head)
  };
  let same_shape = shape.offset_left(used + 1).unwrap_or(Shape {
    width: 0,
    offset: shape.offset + used + 1,
    ..shape
  });
  let same = rhs(same_shape);
  if let Some(same) = &same
    && !same.contains('\n')
    && str_width(same) <= same_shape.width
  {
    return Some(format!("{} {}", head, same));
  }
  let nested = shape.block_indent(shape.limits.tab);
  let next_shape = Shape {
    width: nested.width.checked_sub(shape.rhs_overhead())?,
    ..nested
  };
  let next = rhs(next_shape);
  let pad = format!("\n{}", " ".repeat(nested.indent));
  // The next line's text must fit there (`filtered_str_fits`).
  let fits = |text: &str| {
    first_line_width(text) <= next_shape.width
      && !super::expr::later_line_past(text, shape.limits.max_width)
  };
  match (same, next) {
    (Some(same), Some(next)) if !fits(&next) => Some(format!("{} {}", head, same)),
    (Some(same), Some(next)) if super::expr::prefer_next_line(&same, &next) => {
      Some(format!("{}{}{}", head, pad, next))
    }
    (None, Some(next)) => Some(format!("{}{}{}", head, pad, next)),
    (None, None) => None,
    (Some(same), _) => Some(format!("{} {}", head, same)),
  }
}

/// Whether a comment on one line stands between two bounds, which [`join_bounds_inner`]
/// places.
pub fn between_bounds(comment: &SyntaxToken) -> bool {
  is_comment(comment.kind())
    && !comment.text().contains('\n')
    && comment.parent().kind() == K::TypeBoundList
    // Not one in a generic argument's (`Trait<Assoc: A // c + B>`), which the standard
    // formatter writes oddly.
    && !comment
      .parent()
      .ancestors_iter()
      .any(|a| a.kind() == K::GenericArgList)
}

/// Whether the last of `count` generic arguments may overflow its list: a one-element tuple,
/// or a reference or pointer to one (the standard formatter's `can_be_overflowed_type`).
pub fn can_overflow_type(node: &SyntaxNode, count: usize) -> bool {
  match node.kind() {
    K::TupleType => count == 1,
    K::TypeArg | K::RefType | K::PtrType => node
      .children_iter()
      .next_back()
      .is_some_and(|c| can_overflow_type(&c, count)),
    _ => false,
  }
}

/// Whether a type parameter of a function's generics, laid out a parameter per line one
/// level in from `indent`, leaves its default no room: the standard formatter then fails
/// the generics, and gives the function's header up.
pub fn default_without_room(function: &SyntaxNode, indent: usize, config: &FormatConfig) -> bool {
  let Some(list) = function
    .children_iter()
    .find(|c| c.kind() == K::GenericParamList)
  else {
    return false;
  };
  // 1 = the comma.
  let Some(width) = config.max_width.checked_sub(indent + config.tab_spaces + 1) else {
    return false;
  };
  let shape = Shape {
    indent: indent + config.tab_spaces,
    align: 0,
    offset: 0,
    width,
    limits: Limits::of(config),
  };
  list
    .children_iter()
    .filter(|p| p.kind() == K::TypeParam && p.children_iter().all(|c| c.kind() != K::Attr))
    .any(|p| {
      before_default(&p, shape)
        .is_some_and(|(text, default)| default.is_some() && text.len() > width)
    })
}

/// Whether a comment on one line stands among a reference type's parts (`& // c`), which
/// [`reference_with_comments`] places.
pub fn in_reference(comment: &SyntaxToken) -> bool {
  is_comment(comment.kind())
    && !comment.text().contains('\n')
    && comment.parent().kind() == K::RefType
}

/// Whether a node is a type, a generic argument or a generic parameter: what
/// [`rewrite_type`] lays out.
pub fn is_type(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    K::PathType
      | K::RefType
      | K::PtrType
      | K::SliceType
      | K::ArrayType
      | K::TupleType
      | K::ParenType
      | K::NeverType
      | K::InferType
      | K::FnPtrType
      | K::DynTraitType
      | K::ImplTraitType
      | K::UnsafeBinderType
      | K::TypeArg
      | K::LifetimeArg
      | K::ConstArg
      | K::AssocTypeArg
      | K::LifetimeParam
      | K::TypeParam
      | K::ConstParam
      | K::TupleField
  )
}

/// Bounds joined by ` + `; when they span lines or pass the shape, each after the first on
/// a line of its own led by `+` (one level in when `indent`), unless the one before ends in
/// a closing delimiter (the standard formatter's `join_bounds`).
pub fn join_bounds(list: &SyntaxNode, shape: Shape, indent: bool) -> Option<String> {
  let bounds: Vec<SyntaxNode> = list
    .children_iter()
    .filter(|c| c.kind() == K::TypeBound)
    .collect();
  if bounds.is_empty() || commented(list) {
    return None;
  }
  let joined = join_bounds_inner(&bounds, shape, indent, false)?;
  let retry = if !shape.limits.style_2024 {
    bounds.len() > 1 && (joined.contains('\n') || str_width(&joined) > shape.width)
  } else if !joined.contains('\n') && str_width(&joined) <= shape.width {
    false
  } else {
    bounds.len() > 1 || multi_item_bound(&bounds[0])
  };
  if retry {
    join_bounds_inner(&bounds, shape, indent, true)
  } else {
    Some(joined)
  }
}

/// The shape `n` columns further in, later lines lined up there (the standard formatter's
/// `Shape::legacy` from the indentation plus `n`).
pub fn legacy(shape: Shape, n: usize) -> Option<Shape> {
  Some(Shape {
    align: shape.align + n,
    offset: shape.offset + n,
    width: shape.width.checked_sub(n)?,
    ..shape
  })
}

/// A parameter (`name: Type`, a type alone, `self`) printed into `shape`, its type in what
/// the name leaves of it (the standard formatter's `Param` rewrite).
pub fn rewrite_param(param: &SyntaxNode, shape: Shape) -> Option<String> {
  if param.descendants_with_tokens_iter().any(|e| match e {
    SyntaxElement::Token(t) => {
      matches!(t.kind(), K::LineComment | K::BlockComment)
        && !alone_in_function_type(&t)
        && !in_reference(&t)
        // A block comment on a line before a function type's argument, written before it.
        && !(t.kind() == K::BlockComment
          && !t.text().contains('\n')
          && t.parent().kind() == K::ParenthesizedArgList)
        // Among a struct pattern's fields, placed by its list layout.
        && t.parent().kind() != K::RecordPatFieldList
        // On a line among a tuple type's elements, placed by its list layout.
        && !(t.parent().kind() == K::TupleType && !t.text().contains('\n'))
    }
    SyntaxElement::Node(_) => false,
  }) {
    return None;
  }
  // Its attributes laid out at its column, each on a line of its own; one on a line goes on
  // the parameter's line when both fit there (`#[case] value: T`), as the standard
  // formatter's `combine_strs_with_missing_comments` puts them.
  let attributes: Vec<SyntaxNode> = param
    .children_iter()
    .filter(|c| c.kind() == K::Attr)
    .collect();
  // Its doc comments too, in their order among the attributes, each on a line of its own.
  let docs = param
    .children_with_tokens_iter()
    .any(|e| super::items::is_doc(e.kind()));
  if !attributes.is_empty() || docs {
    let config = shape.limits.config();
    let pad = " ".repeat(shape.indent);
    let texts = param
      .children_with_tokens_iter()
      .filter_map(|e| match e {
        // One too wide to lay out stays as written, when on one line.
        SyntaxElement::Node(a) if a.kind() == K::Attr => Some(
          super::attrs::rewrite_attr(&a, shape.indent, &config).or_else(|| {
            let written = a.text().to_string();
            (!written.contains('\n')).then_some(written)
          }),
        ),
        SyntaxElement::Token(t) if super::items::is_doc(t.kind()) => {
          Some(Some(t.text().trim_end().to_string()))
        }
        _ => None,
      })
      .collect::<Option<Vec<String>>>()?;
    let joined = texts.join(&format!("\n{}", pad));
    if !docs
      && !joined.contains('\n')
      && let Some(rest) = shape
        .offset_left(str_width(&joined) + 1)
        .and_then(|s| rewrite_param_after(param, s))
      && str_width(&joined) + 1 + first_line_width(&rest) <= shape.width
    {
      return Some(format!("{} {}", joined, rest));
    }
    // One attribute that does not fit with the parameter, whose arguments pass
    // `attr_fn_like_width` (which the standard formatter would break, failing the
    // signature): left as written (found by measuring it). One wider than the line on its
    // own goes on a line of its own, as does a bare name's (`#[name]`) in a function's.
    let first = attributes.first();
    let wide_arguments = joined
      .find('(')
      .zip(joined.rfind(')'))
      .is_some_and(|(open, close)| {
        close > open && str_width(&joined[open + 1..close]) > config.attr_fn_like_width
      });
    let in_fn = param
      .parent()
      .and_then(|list| list.parent())
      .is_some_and(|f| f.kind() == K::Fn);
    let bare = first.is_some_and(|a| {
      a.descendant_nodes_where(|kind| kind == K::TokenTree)
        .next()
        .is_none()
        && !a.children_with_tokens_iter().any(|e| e.kind() == K::Eq)
        && !a
          .descendant_nodes_where(|kind| kind == K::Meta)
          .any(|m| m.children_with_tokens_iter().any(|e| e.kind() == K::Eq))
    });
    if !docs
      && texts.len() == 1
      && !joined.contains('\n')
      && wide_arguments
      && !(in_fn && bare)
      && str_width(&joined) <= shape.width
    {
      return None;
    }
    let rest = rewrite_param_after(
      param,
      Shape {
        offset: 0,
        width: shape.width + shape.offset,
        ..shape
      },
    )?;
    return Some(format!("{}\n{}{}", joined, pad, rest));
  }
  rewrite_param_after(param, shape)
}

/// A path, its segments joined by `::`, each segment's generic arguments laid out after its
/// name (the standard formatter's `rewrite_path`); `expression` for a path in an expression,
/// whose generic arguments follow `::`.
pub fn rewrite_path(path: &SyntaxNode, shape: Shape, expression: bool) -> Option<String> {
  let spacing = shape.limits.spacing;
  let mut out = String::new();
  for element in path.children_with_tokens_iter() {
    match element {
      SyntaxElement::Token(t) if t.kind() == K::Colon2 => out.push_str("::"),
      SyntaxElement::Token(t) if t.kind().is_trivia() => {}
      SyntaxElement::Token(t) => out.push_str(t.text()),
      SyntaxElement::Node(segment) if segment.kind() == K::PathSegment => {
        // What the segment starts after, on the path's last line.
        let used = if out.contains('\n') {
          last_line_width(&out).saturating_sub(shape.indent + shape.offset)
        } else {
          str_width(&out)
        };
        let segment_shape = shape.offset_left(used)?;
        out.push_str(&rewrite_segment(&segment, segment_shape, expression)?);
      }
      // A path nested in a path (`a::b` before `::c`).
      SyntaxElement::Node(inner) if inner.kind() == K::Path => {
        out.push_str(&rewrite_path(&inner, shape, expression)?);
      }
      SyntaxElement::Node(other) => out.push_str(&flat(&other, spacing)),
    }
  }
  Some(out)
}

/// The type, generic argument or generic parameter printed into `shape`, or `None`.
pub fn rewrite_type(node: &SyntaxNode, shape: Shape) -> Option<String> {
  if node.kind() == K::TupleField {
    return rewrite_tuple_field(node, shape);
  }
  // A generic parameter's attribute it lays out itself.
  if matches!(node.kind(), K::LifetimeParam | K::TypeParam | K::ConstParam)
    && !node.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment))
  {
    return rewrite_generic_param(node, shape);
  }
  if commented(node) {
    return None;
  }
  let spacing = shape.limits.spacing;
  match node.kind() {
    K::PathType => {
      let path = node.children_iter().find(|c| c.kind() == K::Path)?;
      rewrite_path(&path, shape, false)
    }
    K::TypeArg => rewrite(&node.children_iter().next()?, shape),
    K::LifetimeArg | K::NeverType => Some(flat(node, spacing)),
    K::InferType => (shape.width >= 1).then(|| "_".to_string()),
    K::ConstArg => match node.children_iter().next() {
      // A block on one line when it fits there (`{ N - 1 }`).
      Some(value) if value.kind() == K::BlockExpr => {
        // One holding a macro call, whose arguments are laid out, as an expression is.
        let text = if value.subtree_has(|kind| kind == K::MacroCall) {
          let lone = crate::format::expr::lone_expression(&value)?;
          // 4 = `{ ` and ` }`
          let inner = crate::format::expr::rewrite(&lone, shape.offset_left(2)?.sub_width(2)?)?;
          format!("{{ {} }}", inner)
        } else {
          flat(&value, spacing)
        };
        if str_width(&text) <= shape.width && !text.contains('\n') {
          return Some(text);
        }
        // Else on lines: its expression one level in, `}` at the line's indentation.
        let lone = crate::format::expr::lone_expression(&value)?;
        let inner = shape.block_indent(shape.limits.tab);
        let inside = crate::format::expr::rewrite(&lone, inner)?;
        Some(format!(
          "{{\n{}{}\n{}}}",
          " ".repeat(inner.indent),
          inside,
          " ".repeat(shape.indent)
        ))
      }
      Some(value) => rewrite(&value, shape),
      None => {
        let text = flat(node, spacing);
        (str_width(&text) <= shape.width).then_some(text)
      }
    },
    K::AssocTypeArg => rewrite_constraint(node, shape),
    K::RefType
      if node
        .children_with_tokens_iter()
        .any(|e| is_comment(e.kind())) =>
    {
      reference_with_comments(node, shape)
    }
    K::RefType | K::PtrType => {
      let inner = node.children_iter().next_back()?;
      let prefix = prefix_before(node, &inner, shape)?;
      // A reference's type lines up after the prefix; a pointer's only starts there.
      let inner_shape = if node.kind() == K::RefType {
        legacy(shape, str_width(&prefix))?
      } else {
        shape.offset_left(str_width(&prefix))?
      };
      let text = rewrite(&inner, inner_shape)?;
      Some(format!("{}{}", prefix, text))
    }
    K::SliceType => {
      let inner = node.children_iter().next()?;
      // 4 = `[]` and the standard formatter's two columns more.
      let inner_shape = legacy(shape, 1)?.sub_width(3)?;
      Some(format!("[{}]", rewrite(&inner, inner_shape)?))
    }
    K::ArrayType => {
      let text = flat(node, spacing);
      (str_width(&text) <= shape.width).then_some(text)
    }
    K::ParenType => rewrite_paren(node, shape),
    K::TupleType => {
      let items: Vec<SyntaxNode> = node.children_iter().collect();
      if items.is_empty() {
        return (shape.width >= 2).then(|| "()".to_string());
      }
      // A one-element tuple keeps its comma.
      if items.len() == 1 {
        let inner = rewrite(&items[0], shape.offset_left(1)?.sub_width(2)?)?;
        return (!inner.contains('\n')).then(|| format!("({},)", inner));
      }
      super::expr::rewrite_list("", &items, shape, shape.limits.fn_call, "(", ")")
    }
    K::FnPtrType => {
      let params = node.children_iter().find(|c| c.kind() == K::ParamList)?;
      let prefix = prefix_before(node, &params, shape)?;
      let ret = node.children_iter().find(|c| c.kind() == K::RetType);
      let text = function_type(
        &params,
        ret.as_ref(),
        shape.offset_left(str_width(&prefix))?,
      )?;
      Some(format!("{}{}", prefix, text))
    }
    K::DynTraitType => {
      let Some(bounds) = node
        .children_iter()
        .find(|c| c.kind() == K::TypeBoundList && c.children_iter().next().is_some())
      else {
        return Some(flat(node, spacing));
      };
      let prefix = prefix_before(node, &bounds, shape)?;
      let text = join_bounds(&bounds, shape.offset_left(str_width(&prefix))?, true)?;
      Some(format!("{}{}", prefix, text))
    }
    K::ImplTraitType => {
      let Some(bounds) = node
        .children_iter()
        .find(|c| c.kind() == K::TypeBoundList && c.children_iter().next().is_some())
      else {
        return Some(flat(node, spacing));
      };
      let prefix = prefix_before(node, &bounds, shape)?;
      // Before style edition 2024 the bounds take the whole shape, `impl ` not counted.
      let text = if shape.limits.style_2024 {
        join_bounds(&bounds, shape, false)?
      } else {
        join_bounds(&bounds, shape, true)?
      };
      Some(format!("{}{}", prefix, text))
    }
    K::LifetimeParam | K::TypeParam | K::ConstParam => rewrite_generic_param(node, shape),
    // A macro call standing as a type, laid out as one in an expression is.
    K::MacroType => {
      let call = node.children_iter().find(|c| c.kind() == K::MacroCall)?;
      crate::format::expr::rewrite(&call, shape)
    }
    _ => {
      let text = flat(node, spacing);
      (str_width(&text) <= shape.width).then_some(text)
    }
  }
}

/// A `where` predicate: `for<'a> Type: Bounds` with the bounds placed as a right-hand side,
/// or `'a: 'b + 'c` (the standard formatter's `WherePredicate` rewrite).
pub fn rewrite_where_predicate(predicate: &SyntaxNode, shape: Shape) -> Option<String> {
  let spacing = shape.limits.spacing;
  if commented(predicate) {
    return None;
  }
  let bounds = predicate
    .children_iter()
    .find(|c| c.kind() == K::TypeBoundList);
  let colon = colon_text(spacing);
  let Some(ty) = predicate
    .children_iter()
    .find(|c| c.kind() != K::ForBinder && c.kind() != K::TypeBoundList)
  else {
    // A lifetime's: its bounds joined after it.
    let lifetime = predicate
      .children_with_tokens_iter()
      .find(|e| e.kind() == K::Lifetime)?;
    let has_colon = predicate
      .children_with_tokens_iter()
      .any(|e| e.kind() == K::Colon);
    if !has_colon {
      return Some(lifetime_text(&lifetime));
    }
    let head = format!("{}{}", lifetime_text(&lifetime), colon);
    let Some(bounds) = bounds.filter(|b| b.children_iter().next().is_some()) else {
      return Some(head.trim_end().to_string());
    };
    let rest = join_bounds(&bounds, shape.sub_width(str_width(&head))?, true)?;
    return Some(format!("{}{}", head, rest));
  };
  let binder = predicate
    .children_iter()
    .find(|c| c.kind() == K::ForBinder)
    .map(|b| format!("{} ", flat(&b, spacing)))
    .unwrap_or_default();
  let ty_text = rewrite(&ty, shape)?;
  // A predicate written without its `:` (`where T`) keeps none.
  if !predicate
    .children_with_tokens_iter()
    .any(|e| e.kind() == K::Colon)
  {
    return Some(format!("{}{}", binder, ty_text));
  }
  let head = format!("{}{}{}", binder, ty_text, colon.trim_end());
  let Some(bounds) = bounds.filter(|b| b.children_iter().next().is_some()) else {
    return Some(head);
  };
  assign_rhs(&head, shape, |s| join_bounds(&bounds, s, true))
}

/// One level in: a block level when nothing is lined up, else that much past the
/// alignment, the width up to the line's (`block_indent` then `with_max_width`).
fn aligned_block_indent(shape: Shape) -> Shape {
  let tab = shape.limits.tab;
  if shape.align == 0 {
    shape.block_indent(tab)
  } else {
    Shape {
      align: shape.align + tab,
      offset: shape.align + tab,
      width: shape
        .limits
        .max_width
        .saturating_sub(shape.indent + shape.align + tab),
      ..shape
    }
  }
}

/// A generic parameter up to its default, with the ` = ` before it, and the default.
fn before_default(node: &SyntaxNode, shape: Shape) -> Option<(String, Option<SyntaxNode>)> {
  let spacing = shape.limits.spacing;
  let eq = if spacing.compressed_type_punctuation {
    "="
  } else {
    " = "
  };
  let mut out = String::new();
  let mut after_eq = false;
  for element in node.children_with_tokens_iter() {
    if element.kind().is_trivia() || element.kind() == K::Attr {
      continue;
    }
    match element {
      SyntaxElement::Token(t) if t.kind() == K::Eq => {
        out.push_str(eq);
        after_eq = true;
      }
      SyntaxElement::Token(t) if t.kind() == K::Colon => out.push_str(&colon_text(spacing)),
      SyntaxElement::Token(t) if t.kind() == K::ConstKw => out.push_str("const "),
      SyntaxElement::Token(t) => out.push_str(t.text()),
      SyntaxElement::Node(n) if n.kind() == K::Name => out.push_str(&flat(&n, spacing)),
      // The bounds take the parameter's whole shape.
      SyntaxElement::Node(n) if n.kind() == K::TypeBoundList => {
        out.push_str(&join_bounds(&n, shape, true)?);
      }
      // A const parameter's type the whole shape too.
      SyntaxElement::Node(n) if !after_eq => out.push_str(&rewrite(&n, shape)?),
      SyntaxElement::Node(n) => return Some((out, Some(n))),
    }
  }
  Some((out, None))
}

/// Whether a block comment on one line stands right before a function type's parameter,
/// on its line (`Fn(/* name: */ T)`): written before it.
fn before_parameter(comment: &SyntaxToken) -> bool {
  if comment.kind() != K::BlockComment || comment.text().contains('\n') {
    return false;
  }
  let mut next = comment.next_sibling_or_token();
  while let Some(SyntaxElement::Token(t)) = &next {
    match t.kind() {
      K::Whitespace if !t.text().contains('\n') => next = t.next_sibling_or_token(),
      K::BlockComment if !t.text().contains('\n') => next = t.next_sibling_or_token(),
      _ => return false,
    }
  }
  matches!(next, Some(SyntaxElement::Node(n)) if n.kind() != K::Attr)
}

/// Whether a blank line is written between two items of a list.
fn blank_between(before: &SyntaxNode, after: &SyntaxNode) -> bool {
  let mut next = before.next_sibling_or_token();
  while let Some(element) = next {
    if element.text_range().start >= after.text_range().start {
      break;
    }
    if let SyntaxElement::Token(t) = &element
      && t.kind() == K::Whitespace
      && t.text().matches('\n').count() > 1
    {
      return true;
    }
    next = match &element {
      SyntaxElement::Token(t) => t.next_sibling_or_token(),
      SyntaxElement::Node(n) => n.next_sibling_or_token(),
    };
  }
  false
}

/// `:` spaced as the settings ask.
fn colon_text(spacing: super::shape::Spacing) -> String {
  format!(
    "{}:{}",
    if spacing.space_before_colon { " " } else { "" },
    if spacing.space_after_colon { " " } else { "" }
  )
}

/// Whether a comment or attribute lies anywhere inside a node, but a comment alone in a
/// function type's parentheses, which its layout keeps.
fn commented(node: &SyntaxNode) -> bool {
  node.subtree_has(|kind| kind == K::Attr)
    || node.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment))
      && node.descendants_with_tokens_iter().any(|e| match e {
        SyntaxElement::Token(t) => {
          matches!(t.kind(), K::LineComment | K::BlockComment)
            && !alone_in_function_type(&t)
            && !in_reference(&t)
            && !between_bounds(&t)
            // Among a tuple type's elements, placed by the list layout.
            && t.parent().kind() != K::TupleType
            // A block comment on a line before a function type's argument, written before it.
            && !(t.kind() == K::BlockComment
              && !t.text().contains('\n')
              && t.parent().kind() == K::ParenthesizedArgList)
        }
        SyntaxElement::Node(_) => false,
      })
}

/// The block comments [`before_parameter`] right before `param`, each followed by a space.
fn comments_before(param: &SyntaxNode) -> String {
  let mut comments = Vec::new();
  let mut previous = param.prev_sibling_or_token();
  while let Some(SyntaxElement::Token(t)) = &previous {
    match t.kind() {
      K::Whitespace => {}
      K::BlockComment => comments.push(format!("{} ", t.text())),
      _ => break,
    }
    previous = t.prev_sibling_or_token();
  }
  comments.reverse();
  comments.concat()
}

/// The comments on one line each between two bounds, before their `+` (`before`) or after
/// it, and whether no line ended before the first of them.
fn comments_between_plus(
  first: &SyntaxNode,
  second: &SyntaxNode,
  before: bool,
) -> (Vec<String>, bool) {
  let mut comments = Vec::new();
  let mut same_line = true;
  let mut past_plus = false;
  let mut element = first.next_sibling_or_token();
  while let Some(e) = element {
    if e.text_range().start >= second.text_range().start {
      break;
    }
    match &e {
      SyntaxElement::Token(t) if t.kind() == K::Plus => past_plus = true,
      SyntaxElement::Token(t) if past_plus != before => {
        if t.kind() == K::Whitespace && comments.is_empty() && t.text().contains('\n') {
          same_line = false;
        }
        if is_comment(t.kind()) {
          comments.push(t.text().trim_end().to_string());
        }
      }
      _ => {}
    }
    element = match &e {
      SyntaxElement::Token(t) => t.next_sibling_or_token(),
      SyntaxElement::Node(n) => n.next_sibling_or_token(),
    };
  }
  (comments, same_line)
}

/// A function type's parameters and return type (`(A, B) -> C`), as the standard
/// formatter's `format_function_type`: the parameters on the line when they fit before the
/// return type, else one per line one level in; the return type after `)`, or on the next
/// line at the parameters' indentation when it does not fit there.
fn function_type(list: &SyntaxNode, ret: Option<&SyntaxNode>, shape: Shape) -> Option<String> {
  let output = match ret {
    Some(ret) => {
      let ty = ret.children_iter().next_back()?;
      // 4 = ` -> `
      format!(" -> {}", rewrite(&ty, shape.offset_left(4)?)?)
    }
    None => String::new(),
  };
  // `(..)`, return type notation, as written.
  if list
    .children_with_tokens_iter()
    .any(|e| e.kind() == K::Dot2)
  {
    return Some(format!("{}{}", flat(list, shape.limits.spacing), output));
  }
  let params: Vec<SyntaxNode> = list
    .children_iter()
    .filter(|c| c.kind() != K::Attr)
    .collect();
  if params.len() != list.children_iter().count() {
    return None;
  }
  let nested = shape.block_indent(shape.limits.tab);
  // Nothing but comments inside: a line comment on a line of its own one level in, `)` on
  // the next; others as written.
  if params.is_empty() {
    let text = list.text().to_string();
    let inside = text
      .strip_prefix('(')
      .and_then(|t| t.strip_suffix(')'))?
      .trim();
    let args = if inside.starts_with("//") {
      format!(
        "(\n{}{}\n{})",
        " ".repeat(nested.indent),
        inside,
        " ".repeat(shape.indent)
      )
    } else {
      format!("({})", inside)
    };
    return Some(format!("{}{}", args, output));
  }
  let texts: Vec<String> = params
    .iter()
    .map(|p| {
      let comments = comments_before(p);
      let text = match p.kind() {
        K::Param | K::SelfParam => rewrite_param(p, nested),
        _ => rewrite(p, nested),
      }?;
      Some(format!("{}{}", comments, text))
    })
    .collect::<Option<_>>()?;
  let variadic = texts.last().is_some_and(|t| t.ends_with("..."));
  let total = texts.iter().map(|t| str_width(t)).sum::<usize>() + 2 * (texts.len() - 1);
  let horizontal = !output.contains('\n')
    && !texts.iter().any(|t| t.contains('\n'))
    && total <= shape.width.saturating_sub(2 + output.len());
  let args = if horizontal {
    format!("({})", texts.join(", "))
  } else {
    let pad = " ".repeat(nested.indent);
    let comma = if variadic { "" } else { "," };
    // A blank line written between two parameters stays (`preserve_newline`).
    let mut joined = texts[0].clone();
    for (i, text) in texts.iter().enumerate().skip(1) {
      let blank = blank_between(&params[i - 1], &params[i]);
      joined.push_str(&format!(
        ",\n{}{}{}",
        if blank { "\n" } else { "" },
        pad,
        text
      ));
    }
    format!(
      "(\n{}{}{}\n{})",
      pad,
      joined,
      comma,
      " ".repeat(shape.indent)
    )
  };
  if output.is_empty() || last_line_width(&args) + first_line_width(&output) <= shape.width {
    Some(format!("{}{}", args, output))
  } else {
    Some(format!(
      "{}\n{}{}",
      args,
      " ".repeat(nested.indent),
      output.trim_start()
    ))
  }
}

fn is_comment(kind: SyntaxKind) -> bool {
  matches!(kind, K::LineComment | K::BlockComment)
}

fn join_bounds_inner(
  bounds: &[SyntaxNode],
  shape: Shape,
  indent: bool,
  force_newline: bool,
) -> Option<String> {
  let compressed = shape.limits.spacing.compressed_type_punctuation;
  let shape = if indent && force_newline {
    aligned_block_indent(shape)
  } else {
    shape
  };
  // Lifetimes after every trait.
  let is_trait = |b: &SyntaxNode| b.children_iter().next().is_some();
  let in_order = match (
    bounds.iter().rposition(is_trait),
    bounds.iter().position(|b| !is_trait(b)),
  ) {
    (Some(last_trait), Some(first_lifetime)) => last_trait < first_lifetime,
    _ => true,
  };
  let mut out = String::new();
  let mut previous_extendable = false;
  let pad = shape.indent + shape.align;
  // The comments after the bound before (up to its `+`), with whether they were written on
  // its line.
  let mut trailing: (Vec<String>, bool) = (Vec::new(), true);
  for (i, bound) in bounds.iter().enumerate() {
    let text = rewrite_bound(bound, shape)?;
    if i == 0 {
      out.push_str(&text);
    } else {
      let whitespace = if force_newline && (!previous_extendable || !in_order) {
        format!("\n{}", " ".repeat(pad))
      } else {
        " ".to_string()
      };
      let leading = comments_between_plus(&bounds[i - 1], bound, false);
      let mut joiner = if compressed {
        "+".to_string()
      } else {
        format!("{}+ ", whitespace)
      };
      if !leading.0.is_empty() {
        joiner = joiner.trim_end().to_string();
      }
      if !trailing.0.is_empty() {
        joiner = joiner.trim_start().to_string();
      }
      // The bound after its `+` with the comments between them (`combine_strs_with_missing
      // _comments`); then after what is before with the comments after that.
      let after = if leading.0.is_empty() {
        format!("{}{}", joiner, text)
      } else {
        super::items::combine_with_comments(&joiner, &text, &leading.0, leading.1, pad, shape.width)
      };
      if trailing.0.is_empty() {
        out.push_str(&after);
      } else {
        out = super::items::combine_with_comments(
          &out,
          &after,
          &trailing.0,
          trailing.1,
          pad,
          shape.width,
        );
      }
    }
    trailing = match bounds.get(i + 1) {
      Some(next) => comments_between_plus(bound, next, true),
      None => (Vec::new(), true),
    };
    previous_extendable = !is_trait(bound) || last_line_extendable(&text);
  }
  Some(out)
}

fn lifetime_text(element: &SyntaxElement) -> String {
  match element {
    SyntaxElement::Token(t) => t.text().to_string(),
    SyntaxElement::Node(n) => n.text().to_string(),
  }
}

/// Whether a bound is a trait whose path has several segments, or one segment with more
/// than one generic argument (the standard formatter's `is_item_with_multi_items_array`).
fn multi_item_bound(bound: &SyntaxNode) -> bool {
  let Some(path) = bound.descendant_nodes_where(|kind| kind == K::Path).next() else {
    return bound
      .children_iter()
      .any(|c| c.kind() == K::UseBoundGenericArgs && c.children_iter().count() > 1);
  };
  let segments: Vec<SyntaxNode> = path
    .descendant_nodes_where(|kind| kind == K::PathSegment)
    .filter(|s| s.ancestors_iter().find(|a| a.kind() == K::Path).as_ref() == Some(&path))
    .collect();
  segments.len() > 1
    || segments.first().is_some_and(|s| {
      s.children_iter()
        .find(|c| c.kind() == K::GenericArgList)
        .is_some_and(|g| g.children_iter().count() > 1)
    })
}

/// The tokens of `node` before `inner`, spaced: `&'a mut `, `*const `, `dyn `,
/// `for<'a> unsafe extern "C" fn`.
fn prefix_before(node: &SyntaxNode, inner: &SyntaxNode, shape: Shape) -> Option<String> {
  let at = inner.text_range().start;
  let tokens: Vec<SyntaxToken> = node
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() && t.text_range().start < at => Some(t),
      _ => None,
    })
    .collect();
  let text = super::expr::flat_tokens(&tokens, shape.limits.spacing);
  // A function pointer's `fn` meets its `(`; the others take a space before what follows,
  // but `&` alone.
  Some(match node.kind() {
    K::FnPtrType => text,
    // A trait object without `dyn`.
    _ if text.is_empty() => text,
    K::RefType if text == "&" => text,
    _ => format!("{} ", text),
  })
}

/// A reference type holding comments among its `&`, lifetime, `mut` and type, as the
/// standard formatter writes one: each part joined to what is before it with the comments
/// between them (`combine_strs_with_missing_comments`), on lines at the shape's indentation.
fn reference_with_comments(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let spacing = shape.limits.spacing;
  let mut result = String::new();
  let mut comments: Vec<String> = Vec::new();
  // Whether a line ended before the first of `comments`.
  let mut broke = false;
  let mut seen_amp = false;
  let join = |result: &mut String, part: &str, comments: &mut Vec<String>, broke: bool| {
    if comments.is_empty() {
      result.push_str(part);
    } else {
      *result = super::items::combine_with_comments(
        result.trim_end(),
        part,
        comments,
        !broke,
        shape.indent,
        shape.width,
      );
      comments.clear();
    }
  };
  for element in node.children_with_tokens_iter() {
    match &element {
      SyntaxElement::Token(t) if t.kind() == K::Amp && !seen_amp => {
        result.push('&');
        seen_amp = true;
      }
      SyntaxElement::Token(t) if t.kind() == K::Whitespace => {
        if comments.is_empty() && t.text().contains('\n') {
          broke = true;
        }
      }
      SyntaxElement::Token(t) if is_comment(t.kind()) => {
        if t.text().contains('\n') {
          return None;
        }
        comments.push(t.text().trim_end().to_string());
      }
      SyntaxElement::Token(t) if t.kind() == K::MutKw => {
        join(&mut result, "mut ", &mut comments, broke);
        broke = false;
      }
      SyntaxElement::Node(n) if n.kind() == K::Lifetime => {
        let text = format!("{} ", flat(n, spacing));
        join(&mut result, &text, &mut comments, broke);
        broke = false;
      }
      SyntaxElement::Node(n) => {
        let text = rewrite(n, shape)?;
        join(&mut result, &text, &mut comments, broke);
        broke = false;
      }
      SyntaxElement::Token(t) if t.kind() == K::Lifetime => {
        let text = format!("{} ", t.text());
        join(&mut result, &text, &mut comments, broke);
        broke = false;
      }
      _ => return None,
    }
  }
  comments.is_empty().then_some(result)
}

/// One bound: a lifetime, `use<..>`, or a trait with its `for<..>`, `?`, `const` or
/// `async` before it, in parentheses as written.
fn rewrite_bound(bound: &SyntaxNode, shape: Shape) -> Option<String> {
  let spacing = shape.limits.spacing;
  if let Some(args) = bound
    .children_iter()
    .find(|c| c.kind() == K::UseBoundGenericArgs)
  {
    return use_bound(&args, shape);
  }
  let Some(path) = bound.children_iter().find(|c| c.kind() == K::PathType) else {
    return Some(flat(bound, spacing));
  };
  let tokens: Vec<SyntaxToken> = bound
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t),
      _ => None,
    })
    .collect();
  let start = path.text_range().start;
  let end = path.text_range().end;
  let before: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| t.text_range().start < start)
    .cloned()
    .collect();
  let after: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| t.text_range().start >= end)
    .cloned()
    .collect();
  let paren = before.last().is_some_and(|t| t.kind() == K::LParen);
  let before: Vec<SyntaxToken> = if paren {
    before[..before.len() - 1].to_vec()
  } else {
    before
  };
  let mut prefix = super::expr::flat_tokens(&before, spacing);
  if !prefix.is_empty() {
    prefix.push(' ');
  }
  // `?` and `!` take no space after them.
  if prefix.ends_with("? ") || prefix.ends_with("! ") {
    prefix.pop();
  }
  // `async` is not counted (the standard formatter's `PolyTraitRef` rewrite).
  let counted = str_width(&prefix) - if prefix.contains("async ") { 6 } else { 0 };
  let text = rewrite(&path, shape.offset_left(counted)?)?;
  let (open, close) = if paren { ("(", ")") } else { ("", "") };
  let _ = after;
  Some(format!("{}{}{}{}", open, prefix, text, close))
}

/// `Name = Type`, `Name<T> = Type` or `Name: Bounds` among generic arguments.
fn rewrite_constraint(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let spacing = shape.limits.spacing;
  let mut out = String::new();
  let mut seen_infix = false;
  for element in node.children_with_tokens_iter() {
    if element.kind().is_trivia() {
      continue;
    }
    match element {
      SyntaxElement::Token(t) if t.kind() == K::Eq => {
        out.push_str(if spacing.compressed_type_punctuation {
          "="
        } else {
          " = "
        });
        seen_infix = true;
      }
      SyntaxElement::Token(t) if t.kind() == K::Colon => {
        out.push_str(": ");
        seen_infix = true;
      }
      SyntaxElement::Token(t) => out.push_str(t.text()),
      SyntaxElement::Node(n) if n.kind() == K::NameRef => out.push_str(&flat(&n, spacing)),
      SyntaxElement::Node(n) if n.kind() == K::GenericArgList && !seen_infix => {
        let items: Vec<SyntaxNode> = n.children_iter().collect();
        let list_shape = shape.offset_left(str_width(&out))?;
        out.push_str(&super::expr::rewrite_list(
          "",
          &items,
          list_shape,
          shape.limits.max_width,
          "<",
          ">",
        )?);
      }
      SyntaxElement::Node(n) if n.kind() == K::TypeBoundList => {
        let bounds_shape = legacy(shape, last_line_width(&out))?;
        out.push_str(&join_bounds(&n, bounds_shape, true)?);
      }
      SyntaxElement::Node(n) => {
        let used = last_line_width(&out);
        out.push_str(&rewrite(&n, legacy(shape, used)?)?);
      }
    }
  }
  Some(out)
}

/// A generic parameter: its name (`const N: T` for a const one), then its bounds, then its
/// default (the standard formatter's `GenericParam` rewrite).
fn rewrite_generic_param(node: &SyntaxNode, shape: Shape) -> Option<String> {
  // One attribute goes before it on its line (`#[may_dangle] T`); more are not laid out.
  let attributes: Vec<SyntaxNode> = node
    .children_iter()
    .filter(|c| c.kind() == K::Attr)
    .collect();
  if let [attribute] = attributes.as_slice() {
    let config = shape.limits.config();
    let attribute = super::attrs::rewrite_attr(attribute, shape.indent, &config)?;
    if attribute.contains('\n') {
      return None;
    }
    let rest = rewrite_generic_param_after(node, shape.offset_left(str_width(&attribute) + 1)?)?;
    return Some(format!("{} {}", attribute, rest));
  }
  if !attributes.is_empty() {
    return None;
  }
  rewrite_generic_param_after(node, shape)
}

/// [`rewrite_generic_param`] for what follows its attributes.
fn rewrite_generic_param_after(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let (mut out, default) = before_default(node, shape)?;
  if let Some(n) = default {
    // Its room counted in bytes from all the text before it, line breaks and indentation
    // too when the bounds break (`param.len()`).
    let used = out.len();
    let default_shape = Shape {
      width: shape.width.checked_sub(used)?,
      ..shape
    };
    // A type parameter's default lines up after the parameter.
    let default_shape = if node.kind() == K::ConstParam {
      default_shape
    } else {
      Shape {
        offset: shape.offset + used,
        align: shape.align + used,
        ..default_shape
      }
    };
    out.push_str(&rewrite(&n, default_shape)?);
  }
  Some(out)
}

/// A parameter after its attributes.
fn rewrite_param_after(param: &SyntaxNode, shape: Shape) -> Option<String> {
  let spacing = shape.limits.spacing;
  if param.kind() == K::SelfParam || param.children_iter().all(|c| c.kind() == K::Attr) {
    let tokens: Vec<SyntaxToken> = param
      .descendants_with_tokens_iter()
      .filter_map(|e| match e {
        SyntaxElement::Token(t)
          if !t.kind().is_trivia() && !t.parent().ancestors_iter().any(|a| a.kind() == K::Attr) =>
        {
          Some(t)
        }
        _ => None,
      })
      .collect();
    return Some(super::expr::flat_tokens(&tokens, spacing));
  }
  let colon = param
    .children_with_tokens_iter()
    .find(|e| e.kind() == K::Colon);
  let Some(colon) = colon else {
    // A type alone, or `...`.
    return match param.children_iter().rfind(|c| c.kind() != K::Attr) {
      Some(ty) => rewrite(&ty, shape),
      None => Some(flat(param, spacing)),
    };
  };
  let at = colon.text_range().start;
  let pattern = param
    .children_iter()
    .find(|c| c.text_range().end <= at && c.kind() != K::Attr)?;
  let mut out = rewrite(&pattern, shape)?;
  out.push_str(&colon_text(spacing));
  // `name: ...`, a C variadic's.
  let Some(ty) = param.children_iter().find(|c| c.text_range().start > at) else {
    if param
      .children_with_tokens_iter()
      .any(|e| e.kind() == K::Dot3)
    {
      out.push_str("...");
      return Some(out);
    }
    return None;
  };
  let used = last_line_width(&out);
  let ty_shape = Shape {
    width: shape.width.checked_sub(used)?,
    offset: shape.offset + used,
    ..shape
  };
  out.push_str(&rewrite(&ty, ty_shape)?);
  Some(out)
}

/// `(T)`: before style edition 2024 the type one column in; from it on one line when it
/// fits there, else on a line of its own one level in.
fn rewrite_paren(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let inner = node.children_iter().next()?;
  if !shape.limits.style_2024 {
    let text = rewrite(&inner, legacy(shape, 1)?.sub_width(1)?)?;
    return Some(format!("({})", text));
  }
  if let Some(text) = shape.sub_width(2).and_then(|s| rewrite(&inner, s))
    && !text.contains('\n')
  {
    return Some(format!("({})", text));
  }
  let nested = aligned_block_indent(shape);
  let text = rewrite(&inner, nested)?;
  Some(format!(
    "(\n{}{}\n{})",
    " ".repeat(nested.indent + nested.align),
    text,
    " ".repeat(shape.indent + shape.align)
  ))
}

/// A path segment: its name, then its generic arguments or a function type's parameters and
/// return type; `<T as Trait>` its type and trait (the standard formatter's `qself`).
fn rewrite_segment(segment: &SyntaxNode, shape: Shape, expression: bool) -> Option<String> {
  let spacing = shape.limits.spacing;
  let elements: Vec<SyntaxElement> = segment
    .children_with_tokens_iter()
    .filter(|e| !e.kind().is_trivia())
    .collect();
  // `<T as Trait>`: the type in the shape, then the trait after ` as `, with `>::` kept.
  if elements.first().is_some_and(|e| e.kind() == K::Lt) {
    let mut out = String::from("<");
    let mut after_as = false;
    for element in &elements[1..] {
      match element {
        SyntaxElement::Token(t) if t.kind() == K::AsKw => {
          out.push_str(" as ");
          after_as = true;
        }
        SyntaxElement::Token(t) => out.push_str(t.text()),
        SyntaxElement::Node(n) if after_as => {
          // 3 = `>::`
          let used = last_line_width(&out);
          let trait_shape = shape.sub_width(3)?.offset_left(used)?;
          out.push_str(&rewrite(n, trait_shape)?);
        }
        SyntaxElement::Node(n) => out.push_str(&rewrite(n, shape)?),
      }
    }
    return Some(out);
  }
  let args = elements
    .iter()
    .position(|e| matches!(e.kind(), K::GenericArgList | K::ParenthesizedArgList));
  let name_end = args.unwrap_or(elements.len());
  let name: String = elements[..name_end]
    .iter()
    .map(|e| match e {
      SyntaxElement::Token(t) => t.text().to_string(),
      SyntaxElement::Node(n) => flat(n, spacing),
    })
    .collect();
  let Some(at) = args else {
    return (str_width(&name) <= shape.width).then_some(name);
  };
  let after_name = shape.offset_left(str_width(&name))?;
  let SyntaxElement::Node(list) = &elements[at] else {
    return None;
  };
  if list.kind() == K::ParenthesizedArgList {
    let ret = elements[at + 1..].iter().find_map(|e| match e {
      SyntaxElement::Node(n) if n.kind() == K::RetType => Some(n.clone()),
      _ => None,
    });
    let text = function_type(list, ret.as_ref(), after_name)?;
    return Some(format!("{}{}", name, text));
  }
  let separator = if list
    .children_with_tokens_iter()
    .any(|e| e.kind() == K::Colon2)
  {
    "::"
  } else {
    ""
  };
  let _ = expression;
  let items: Vec<SyntaxNode> = list.children_iter().collect();
  // An empty list says nothing (`Foo<>` is `Foo`), but after `::` or with a comment.
  if items.is_empty() {
    if separator.is_empty()
      && !list
        .children_with_tokens_iter()
        .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
    {
      return (str_width(&name) <= shape.width).then_some(name);
    }
    return Some(format!("{}{}<>", name, separator));
  }
  // The `::` is not counted (the standard formatter lays the arguments out first).
  let args_shape = after_name;
  let text = super::expr::rewrite_list("", &items, args_shape, shape.limits.max_width, "<", ">")?;
  // A qualified path first among them keeps a space written before it (`< <T as A>::B>`),
  // as the standard formatter keeps the snippet before its first item.
  let spaced = list
    .children_with_tokens_iter()
    .skip_while(|e| e.kind() != K::Lt)
    .nth(1)
    .is_some_and(|e| e.kind() == K::Whitespace);
  let text = match text.strip_prefix("<<") {
    Some(rest) if spaced => format!("< <{}", rest),
    _ => text,
  };
  Some(format!("{}{}{}", name, separator, text))
}

/// A tuple struct's field: one attribute on its line, its visibility, then its type.
fn rewrite_tuple_field(node: &SyntaxNode, shape: Shape) -> Option<String> {
  let spacing = shape.limits.spacing;
  let ty = node.children_iter().next_back()?;
  // A line comment among its attributes, before its type, starts a line of its own; any
  // other would be lost.
  let leading = |e: &SyntaxElement| {
    e.kind() == K::LineComment
      && e.text_range().end <= ty.text_range().start
      && matches!(e, SyntaxElement::Token(t) if t.parent() == *node)
  };
  // One on a line among a tuple type's elements is placed by the tuple's list.
  let in_tuple = |e: &SyntaxElement| matches!(e, SyntaxElement::Token(t) if t.parent().kind() == K::TupleType && !t.text().contains('\n'));
  if node
    .descendants_with_tokens_iter()
    .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment) && !leading(&e) && !in_tuple(&e))
  {
    return None;
  }
  if !is_type(ty.kind()) && ty.kind() != K::MacroType {
    return None;
  }
  let mut prefix = String::new();
  let mut attribute_lines: Option<String> = None;
  // Its attributes and doc comments, in order.
  let attributes: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(|e| e.kind() == K::Attr || super::items::is_doc(e.kind()) || leading(e))
    .collect();
  let docs = attributes
    .iter()
    .any(|e| super::items::is_doc(e.kind()) || e.kind() == K::LineComment);
  // Its attributes each on a line of their own (one the standard formatter leaves as
  // written keeps its lines), and the field on their line when they are one line
  // (`is_attributes_extendable`), else on the next; a doc comment ends its line.
  if !attributes.is_empty() {
    let config = shape.limits.config();
    let pad = " ".repeat(shape.indent);
    let mut texts: Vec<String> = Vec::new();
    for e in &attributes {
      let text = match e {
        SyntaxElement::Token(doc) => doc.text().trim_end().to_string(),
        SyntaxElement::Node(a) if super::file::meta_like(a) => {
          super::attrs::rewrite_attr(a, shape.indent, &config)?
        }
        SyntaxElement::Node(a) => a.text().to_string(),
      };
      // A line comment written on the line before stays at its end.
      let same_line = e.kind() == K::LineComment
        && !texts.is_empty()
        && matches!(e, SyntaxElement::Token(t) if t
          .prev_sibling_or_token()
          .is_some_and(|w| w.kind() != K::Whitespace || !matches!(&w, SyntaxElement::Token(w) if w.text().contains('\n'))));
      match texts.last_mut() {
        Some(last) if same_line => {
          last.push(' ');
          last.push_str(&text);
        }
        _ => texts.push(text),
      }
    }
    let joined = texts.join(&format!("\n{}", pad));
    if joined.contains('\n') || docs {
      prefix.push_str(&format!("{}\n{}", joined, pad));
    } else {
      prefix.push_str(&format!("{} ", joined));
    }
    attribute_lines = Some(joined);
  }
  // Its visibility and restrictions (`pub mut(crate)`): every token before the type.
  let at = ty.text_range().start;
  let tokens: Vec<SyntaxToken> = node
    .children_iter()
    .filter(|c| c.kind() != K::Attr && c.text_range().end <= at)
    .flat_map(|c| c.descendants_where(|kind| !kind.is_node() && !kind.is_trivia()))
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t),
      _ => None,
    })
    .collect();
  let lead = super::expr::flat_tokens(&tokens, spacing);
  if !lead.is_empty() {
    prefix.push_str(&format!("{} ", lead));
  }
  // Tokens directly in the field before the type would be lost.
  if node.children_with_tokens_iter().any(
    |e| matches!(e, SyntaxElement::Token(ref t) if !t.kind().is_trivia() && !super::items::is_doc(t.kind())),
  ) {
    return None;
  }
  // After attributes on lines of their own, the type starts a line at the field's column.
  let type_shape = match prefix.rsplit_once('\n') {
    Some((_, last)) => Shape {
      offset: 0,
      width: shape.width + shape.offset,
      ..shape
    }
    .offset_left(str_width(last).saturating_sub(shape.indent)),
    None => shape.offset_left(str_width(&prefix)),
  };
  if let Some(text) = type_shape
    .and_then(|s| rewrite(&ty, s))
    .filter(|t| !t.contains('\n'))
  {
    return Some(format!("{}{}", prefix, text));
  }
  // Across lines, as a right-hand side after the prefix and its space (the standard
  // formatter's `rewrite_struct_field`: `pub  Type<` with two spaces); after attributes, on
  // a line of its own below them.
  let field = |prefix: &str| {
    if prefix.is_empty() {
      rewrite(&ty, shape)
    } else {
      assign_rhs(prefix, shape, |s| rewrite(&ty, s))
    }
  };
  match attribute_lines {
    Some(lines) => Some(format!(
      "{}\n{}{}",
      lines,
      " ".repeat(shape.indent),
      field(&lead)?
    )),
    None => field(&prefix),
  }
}

/// `use<'a, T>`, its arguments as a list in angle brackets: on the line when they fit,
/// else one per line one level in.
fn use_bound(args: &SyntaxNode, shape: Shape) -> Option<String> {
  let items: Vec<String> = args
    .children_with_tokens_iter()
    .filter(|e| !e.kind().is_trivia() && !matches!(e.kind(), K::Lt | K::Gt | K::Comma))
    .map(|e| match e {
      SyntaxElement::Token(t) => t.text().to_string(),
      SyntaxElement::Node(n) => flat(&n, shape.limits.spacing),
    })
    .collect();
  let joined = items.join(", ");
  // 5 = `use<>`
  if str_width(&joined) + 5 <= shape.width {
    return Some(format!("use<{}>", joined));
  }
  let pad = " ".repeat(shape.indent + shape.limits.tab);
  let lines: String = items.iter().map(|i| format!("\n{}{},", pad, i)).collect();
  Some(format!("use<{}\n{}>", lines, " ".repeat(shape.indent)))
}
