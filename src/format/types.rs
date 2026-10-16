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
use super::shape::Shape;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
use crate::text::unicode::str_width;

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
      && text
        .lines()
        .skip(1)
        .all(|l| super::expr::code_width(l) <= shape.limits.max_width)
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
  let spacing = shape.limits.spacing;
  if param
    .descendants_with_tokens_iter()
    .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
  {
    return None;
  }
  // One attribute goes on the parameter's line (`#[case] value: T`).
  let attributes: Vec<SyntaxNode> = param
    .children_iter()
    .filter(|c| c.kind() == K::Attr)
    .collect();
  if !attributes.is_empty() {
    let [attribute] = attributes.as_slice() else {
      return None;
    };
    let attribute = format!("{} ", flat(attribute, spacing));
    let used = str_width(&attribute);
    let rest = rewrite_param_after(param, shape.offset_left(used)?)?;
    return Some(format!("{}{}", attribute, rest));
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
        let text = flat(&value, spacing);
        (str_width(&text) <= shape.width && !text.contains('\n')).then_some(text)
      }
      Some(value) => rewrite(&value, shape),
      None => {
        let text = flat(node, spacing);
        (str_width(&text) <= shape.width).then_some(text)
      }
    },
    K::AssocTypeArg => rewrite_constraint(node, shape),
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

/// `:` spaced as the settings ask.
fn colon_text(spacing: super::shape::Spacing) -> String {
  format!(
    "{}:{}",
    if spacing.space_before_colon { " " } else { "" },
    if spacing.space_after_colon { " " } else { "" }
  )
}

/// Whether a comment or attribute lies anywhere inside a node.
fn commented(node: &SyntaxNode) -> bool {
  node.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment | K::Attr))
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
  if params.is_empty() {
    return Some(format!("(){}", output));
  }
  let texts: Vec<String> = params
    .iter()
    .map(|p| match p.kind() {
      K::Param | K::SelfParam => rewrite_param(p, nested),
      _ => rewrite(p, nested),
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
    format!(
      "(\n{}{}{}\n{})",
      pad,
      texts.join(&format!(",\n{}", pad)),
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
  for (i, bound) in bounds.iter().enumerate() {
    let text = rewrite_bound(bound, shape)?;
    if i > 0 {
      let whitespace = if force_newline && (!previous_extendable || !in_order) {
        format!("\n{}", " ".repeat(shape.indent + shape.align))
      } else {
        " ".to_string()
      };
      if compressed {
        out.push('+');
      } else {
        out.push_str(&whitespace);
        out.push_str("+ ");
      }
    }
    previous_extendable = !is_trait(bound) || last_line_extendable(&text);
    out.push_str(&text);
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
  let spacing = shape.limits.spacing;
  if node.children_iter().any(|c| c.kind() == K::Attr) {
    return None;
  }
  let eq = if spacing.compressed_type_punctuation {
    "="
  } else {
    " = "
  };
  let mut out = String::new();
  let mut after_eq = false;
  for element in node.children_with_tokens_iter() {
    if element.kind().is_trivia() {
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
      // A const parameter's type the whole shape too; a default what is left.
      SyntaxElement::Node(n) if !after_eq => out.push_str(&rewrite(&n, shape)?),
      SyntaxElement::Node(n) => {
        let used = str_width(&out);
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
    }
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
  if items.is_empty() {
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
  if node
    .descendants_with_tokens_iter()
    .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
  {
    return None;
  }
  let ty = node.children_iter().next_back()?;
  if !is_type(ty.kind()) && ty.kind() != K::MacroType {
    return None;
  }
  let mut prefix = String::new();
  let attributes: Vec<SyntaxNode> = node
    .children_iter()
    .filter(|c| c.kind() == K::Attr)
    .collect();
  match attributes.as_slice() {
    [] => {}
    [attribute] => prefix.push_str(&format!("{} ", flat(attribute, spacing))),
    _ => return None,
  }
  // Its visibility and restrictions (`pub mut(crate)`): every token before the type.
  let at = ty.text_range().start;
  let tokens: Vec<SyntaxToken> = node
    .children_iter()
    .filter(|c| c.kind() != K::Attr && c.text_range().end <= at)
    .flat_map(|c| c.descendants_with_tokens())
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
  if node
    .children_with_tokens_iter()
    .any(|e| matches!(e, SyntaxElement::Token(ref t) if !t.kind().is_trivia()))
  {
    return None;
  }
  if let Some(text) = shape
    .offset_left(str_width(&prefix))
    .and_then(|s| rewrite(&ty, s))
    .filter(|t| !t.contains('\n'))
  {
    return Some(format!("{}{}", prefix, text));
  }
  // Across lines, as a right-hand side after the prefix and its space (the standard
  // formatter's `rewrite_struct_field`: `pub  Type<` with two spaces).
  if !attributes.is_empty() {
    return None;
  }
  if prefix.is_empty() {
    return rewrite(&ty, shape);
  }
  assign_rhs(&prefix, shape, |s| rewrite(&ty, s))
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
