//! Item headers laid out as the Rust style lays them out: everything before an item's body
//! (`pub fn name<T>(a: T) -> R`, `impl<T> Trait for Type`, `extern "C"`, `pub struct Name`)
//! with the body's `{`, or a whole item ending in `;`. The standard formatter's item
//! layouts are ported: a function's signature (`rewrite_fn_base`), an `impl`'s
//! (`format_impl_ref_and_type`), a trait's and a trait alias's, a struct's, enum's or
//! union's generics (`format_generics`), a type alias (`rewrite_ty`), a const's or static's
//! type and value, a tuple struct's fields as a call's arguments, and a field's type. Types,
//! generic parameters and bounds in them break as [`super::types`] breaks them. A `where`
//! clause follows on lines of its own, a predicate per line (an `impl` with one predicate
//! and no items stays on one line when it fits). The body itself is left to the whole-file
//! rules.
//!
//! Left as written: a header holding a comment (but line comments among a function's
//! parameters), a macro, a parameter with more than one attribute, a const's or static's
//! `where` or generics, a type alias's `where` before its `=`, and a header whose layout
//! would not keep its tokens.

use SyntaxKind as K;

use super::expr::flat_tokens;
use super::shape::Spacing;
use crate::options::format::FormatConfig;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
use crate::text::unicode::str_width;

/// Where an item's own text starts: after its attributes and the comments and doc comments
/// before it.
pub fn header_start(node: &SyntaxNode) -> Option<usize> {
  node
    .children_with_tokens_iter()
    .find(|e| e.kind() != K::Attr && !e.kind().is_trivia() && !is_doc(e.kind()))
    .map(|e| e.text_range().start as usize)
}

/// Whether a token is a doc comment, which is an attribute, not trivia.
pub fn is_doc(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    K::OuterLineDoc | K::InnerLineDoc | K::OuterBlockDoc | K::InnerBlockDoc
  )
}

/// Whether a node is a field of a struct or union definition, or an enum's variant: each
/// starts a line of its own.
pub fn is_field(node: &SyntaxNode) -> bool {
  match node.kind() {
    K::Variant => true,
    K::RecordField => node
      .parent()
      .and_then(|list| list.parent())
      .is_some_and(|owner| {
        matches!(owner.kind(), K::Struct | K::Union) || variant_fields_on_lines(&owner)
      }),
    _ => false,
  }
}

/// The items whose header this module lays out.
pub fn is_item(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    K::Fn
      | K::Struct
      | K::Enum
      | K::Union
      | K::Trait
      | K::Impl
      | K::ExternBlock
      | K::Module
      | K::TypeAlias
      | K::Const
      | K::Static
      | K::ExternCrate
  )
}

/// A field or variant on one line from column `indent`, with its comma (added when the last
/// one has none), and where in the source it ends. A struct-like variant stays on one line
/// while its fields fit in `struct_variant_width`; a wider one is left as written, as is one
/// holding a comment or a macro, or one that does not fit.
pub fn rewrite_field(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<(String, usize)> {
  if config.hard_tabs || config.tab_spaces == 0 || !is_field(node) {
    return None;
  }
  // A field's default value (`f: T = 0`) the standard formatter leaves as written.
  if node.kind() == K::RecordField && node.children_with_tokens_iter().any(|e| e.kind() == K::Eq) {
    return None;
  }
  let start = header_start(node)?;
  let elements: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(|e| e.text_range().start as usize >= start)
    .collect();
  let unwanted = |e: &SyntaxElement| match e {
    SyntaxElement::Token(t) => {
      (t.kind().is_trivia() && t.kind() != K::Whitespace) || is_doc(t.kind())
    }
    SyntaxElement::Node(n) => {
      n.subtree_has(|kind| {
        (kind.is_trivia() && kind != K::Whitespace)
          || is_doc(kind)
          || matches!(kind, K::MacroCall | K::MacroType | K::MacroExpr | K::BlockExpr)
      })
        // A tuple field's attribute is written before its type on its line; with two, they
        // go on lines of their own.
        || n.descendant_nodes_where(|kind| kind == K::Attr).any(|a| {
          a.parent().is_none_or(|p| {
            p.kind() != K::TupleField || p.children_of_kind(K::Attr).count() > 1
          })
        })
    }
  };
  let spacing = Spacing::of(config);
  // A variant whose fields are only a block comment on one line: the comment between the
  // delimiters, touching them (`V {/* c */}`, `V(/* c */) = 1`).
  if node.kind() == K::Variant
    && let Some(fields) = node
      .children_iter()
      .find(|c| matches!(c.kind(), K::RecordFieldList | K::TupleFieldList))
    && fields.children_iter().next().is_none()
    && let [comment] = fields
      .children_with_tokens_iter()
      .filter(|e| {
        !matches!(
          e.kind(),
          K::Whitespace | K::LBrace | K::RBrace | K::LParen | K::RParen
        )
      })
      .collect::<Vec<_>>()
      .as_slice()
    && comment.kind() == K::BlockComment
    && let SyntaxElement::Token(comment) = comment
    && !comment.text().contains('\n')
    && !elements
      .iter()
      .filter(|e| e.text_range() != fields.text_range())
      .any(|e| {
        (e.kind().is_trivia() && e.kind() != K::Whitespace)
          || matches!(e.kind(), K::Attr)
          || is_doc(e.kind())
      })
  {
    let (from, to) = (fields.text_range().start, fields.text_range().end);
    let head: Vec<SyntaxElement> = elements
      .iter()
      .filter(|e| e.text_range().start < from)
      .cloned()
      .collect();
    let tail: Vec<SyntaxElement> = elements
      .iter()
      .filter(|e| e.text_range().start >= to)
      .cloned()
      .collect();
    let (open, close) = if fields.kind() == K::RecordFieldList {
      (" {", "}")
    } else {
      ("(", ")")
    };
    let tail = flat_tokens(&code_tokens(&tail), spacing);
    let tail = if tail.is_empty() {
      String::new()
    } else {
      format!(" {}", tail)
    };
    let comma = if comma_follows(node) { "" } else { "," };
    let text = format!(
      "{}{}{}{}{}{}",
      flat_tokens(&code_tokens(&head), spacing),
      open,
      comment.text(),
      close,
      tail,
      comma
    );
    if indent + str_width(&text) > config.max_width {
      return None;
    }
    return Some((text, node.text_range().end as usize));
  }
  // A struct-like variant whose fields hold doc comments, attributes or comments: its head
  // and `{` here, its fields laid out one per line by the whole-file pass as a struct's are,
  // its `},` after them.
  if node.kind() == K::Variant
    && let Some(fields) = node
      .children_iter()
      .find(|c| c.kind() == K::RecordFieldList)
    && elements.iter().any(unwanted)
    && !elements
      .iter()
      .filter(|e| e.text_range() != fields.text_range())
      .any(unwanted)
    && comma_follows(node)
    && !fields.children().is_empty()
  {
    let head: Vec<SyntaxElement> = elements
      .iter()
      .filter(|e| e.text_range().start < fields.text_range().start)
      .cloned()
      .collect();
    let text = format!("{} ", flat_tokens(&code_tokens(&head), spacing));
    if indent + str_width(&text) + 1 > config.max_width {
      return None;
    }
    return Some((text, fields.text_range().start as usize));
  }
  if elements.iter().any(unwanted) {
    return None;
  }
  // A field's type laid out after its name (the standard formatter's
  // `rewrite_struct_field`): on the line when it fits there on one line, else as a
  // right-hand side.
  if node.kind() == K::RecordField
    && let Some(colon) = elements.iter().find(|e| e.kind() == K::Colon)
    && let Some(ty) = node
      .children_iter()
      .find(|c| c.text_range().start >= colon.text_range().end)
    && elements
      .iter()
      .all(|e| e.text_range().end <= ty.text_range().end)
  {
    let before: Vec<SyntaxElement> = elements
      .iter()
      .filter(|e| e.text_range().end <= colon.text_range().start)
      .cloned()
      .collect();
    let prefix = format!(
      "{}{}:",
      flat_tokens(&code_tokens(&before), spacing),
      if config.space_before_colon { " " } else { "" }
    );
    let gap = if config.space_after_colon { " " } else { "" };
    let shape = super::shape::Shape {
      indent,
      align: 0,
      offset: 0,
      width: config.max_width.checked_sub(indent + 1)?,
      limits: super::shape::Limits::of(config),
    };
    let one_line = shape
      .offset_left(str_width(&prefix) + gap.len())
      .and_then(|s| super::expr::rewrite(&ty, s))
      .filter(|t| !t.contains('\n'));
    let text = match one_line {
      Some(t) => format!("{}{}{}", prefix, gap, t),
      None => super::types::assign_rhs(&prefix, shape, |s| super::expr::rewrite(&ty, s))?,
    };
    let comma = if comma_follows(node) || config.trailing_comma_never && is_last(node) {
      ""
    } else {
      ","
    };
    if comma.is_empty() && !comma_follows(node) {
      return None;
    }
    return Some((format!("{}{}", text, comma), node.text_range().end as usize));
  }
  // A tuple variant whose fields hold attributes: its fields as a list (`V(#[a] T)`), as a
  // tuple struct's.
  if node.kind() == K::Variant
    && let Some(fields) = node.children_iter().find(|c| c.kind() == K::TupleFieldList)
    && fields.subtree_has(|kind| kind == K::Attr)
  {
    let items: Vec<SyntaxNode> = fields
      .children_iter()
      .filter(|c| c.kind() == K::TupleField)
      .collect();
    let head: Vec<SyntaxElement> = elements
      .iter()
      .filter(|e| e.text_range().start < fields.text_range().start)
      .cloned()
      .collect();
    if elements.iter().any(|e| {
      e.text_range().start >= fields.text_range().end
        && e.kind() != K::Comma
        && !e.kind().is_trivia()
    }) {
      return None;
    }
    let lead = flat_tokens(&code_tokens(&head), spacing);
    // 1 = `,`
    let shape = super::shape::Shape {
      indent,
      align: 0,
      offset: 0,
      width: config.max_width.checked_sub(indent + 1)?,
      limits: super::shape::Limits::of(config),
    };
    let text = super::expr::rewrite_list(&lead, &items, shape, config.fn_call_width, "(", ")")?;
    let comma = if comma_follows(node) { "" } else { "," };
    return Some((format!("{}{}", text, comma), node.text_range().end as usize));
  }
  let listed = |kind| matches!(kind, K::TupleType | K::TupleFieldList | K::ArrayType);
  let too_wide = node.descendant_nodes_where(listed).any(|d| {
    let limit = match d.kind() {
      K::TupleType | K::TupleFieldList => config.fn_call_width,
      K::ArrayType => config.array_width,
      _ => return false,
    };
    str_width(&flat_tokens(
      &code_tokens(&[SyntaxElement::Node(d.clone())]),
      spacing,
    )) > limit
  });
  if too_wide {
    return None;
  }
  // With `trailing_comma = "Never"` the last one takes no comma; one written stays as
  // written.
  let comma = if comma_follows(node) {
    ""
  } else if config.trailing_comma_never {
    if node
      .parent()
      .and_then(|list| list.children_iter().next_back())
      .is_some_and(|last| &last == node)
    {
      return None;
    }
    ","
  } else {
    ","
  };
  let fields = node
    .children_iter()
    .find(|c| c.kind() == K::RecordFieldList);
  // A struct-like variant whose fields pass `struct_variant_width` is written one field per
  // line, and so are all of an enum's when it has a variant on several lines (one with
  // attributes or doc comments is) and one on a single line, as the standard formatter's
  // `format_variant_list` does.
  let wide = |f: &SyntaxNode| {
    str_width(&flat_tokens(
      &code_tokens(&[SyntaxElement::Node(f.clone())]),
      spacing,
    )) > config.struct_variant_width + 4
  };
  if let Some(fields) = &fields
    && !fields.children().is_empty()
    && (wide(fields)
      || node
        .parent()
        .is_some_and(|list| mixed_variants(&list, indent, config)))
  {
    let head: Vec<SyntaxElement> = elements
      .iter()
      .filter(|e| e.text_range().start < fields.text_range().start)
      .cloned()
      .collect();
    let tail: Vec<SyntaxElement> = elements
      .iter()
      .filter(|e| e.text_range().start >= fields.text_range().end)
      .cloned()
      .collect();
    let pad = " ".repeat(indent + config.tab_spaces);
    let (lower, upper) = super::shape::Limits::of(config).blank_lines;
    let mut lines = String::new();
    for (i, field) in fields.children_iter().enumerate() {
      // The blank lines written before a field after the first kept within the bounds.
      let mut newlines = 0;
      let mut before = field.prev_sibling_or_token();
      while let Some(SyntaxElement::Token(t)) = &before
        && t.kind() == K::Whitespace
      {
        newlines += t.text().matches('\n').count();
        before = t.prev_sibling_or_token();
      }
      if i > 0 {
        let blank = newlines.saturating_sub(1).clamp(lower, upper.max(lower));
        lines.push_str(&"\n".repeat(blank));
      }
      let text = flat_tokens(&code_tokens(&[SyntaxElement::Node(field)]), spacing);
      if indent + config.tab_spaces + str_width(&text) + 1 > config.max_width {
        return None;
      }
      lines.push_str(&format!("\n{}{},", pad, text));
    }
    let tail = flat_tokens(&code_tokens(&tail), spacing);
    let tail = if tail.is_empty() {
      String::new()
    } else {
      format!(" {}", tail)
    };
    return Some((
      format!(
        "{} {{{}\n{}}}{}{}",
        flat_tokens(&code_tokens(&head), spacing),
        lines,
        " ".repeat(indent),
        tail,
        comma
      ),
      node.text_range().end as usize,
    ));
  }
  let text = flat_tokens(&code_tokens(&elements), spacing);
  // 1 = `,`
  if indent + str_width(&text) + 1 > config.max_width {
    return None;
  }
  Some((format!("{}{}", text, comma), node.text_range().end as usize))
}

/// The header laid out from column `indent`, and where in the source its text ends: before
/// the body's `{` (the text then ends with the space before it), or at the item's end.
/// `None` too for one whose layout would not keep its tokens.
pub fn rewrite_header(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<(String, usize)> {
  let (text, end) = laid_header(node, indent, config)?;
  let start = header_start(node)?;
  let from = node.text_range().start as usize;
  let written = node.text().to_string();
  let written = written.get(start.checked_sub(from)?..end.checked_sub(from)?)?;
  same_header_tokens(written, &text).then_some((text, end))
}

/// A macro call standing as an item or statement, its arguments in parentheses or brackets,
/// laid out as one in an expression is (the standard formatter's `rewrite_macro` in item
/// position), with its `;`. `None` for one holding a comment, or that does not fit.
pub fn rewrite_macro_item(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<String> {
  if config.hard_tabs
    || config.tab_spaces == 0
    || node.kind() != K::MacroCall
    || super::file::skipped_macro(node)
  {
    return None;
  }
  let start = header_start(node)?;
  // A comment after its `;` is not its own.
  let end = super::imports::code_end(node);
  let commented = node.token_texts().any(|(kind, _, range)| {
    range.start as usize >= start
      && (range.start as usize) < end
      && kind.is_trivia()
      && kind != K::Whitespace
  });
  if commented {
    return None;
  }
  let semi = node
    .children_with_tokens_iter()
    .any(|e| e.kind() == K::Semi);
  let shape = super::stmt::statement_shape(indent, config)?;
  let shape = if semi { shape.sub_width(1)? } else { shape };
  let text = super::expr::rewrite(node, shape)?;
  let text = if semi { format!("{};", text) } else { text };
  super::stmt::same_tokens_until(node, &text, end).then_some(text)
}

/// Whether a struct-like variant's fields hold doc comments, attributes or comments: they
/// are laid out one per line by the whole-file pass, as a struct's are ([`rewrite_field`]).
pub fn variant_fields_on_lines(node: &SyntaxNode) -> bool {
  node.kind() == K::Variant
    && comma_follows(node)
    && node
      .children_iter()
      .find(|c| c.kind() == K::RecordFieldList)
      .is_some_and(|fields| {
        !fields.children().is_empty()
          && fields.subtree_has(|kind| {
            (kind.is_trivia() && kind != K::Whitespace) || is_doc(kind) || kind == K::Attr
          })
      })
}

/// The code tokens of some elements, in order.
fn code_tokens(elements: &[SyntaxElement]) -> Vec<SyntaxToken> {
  elements
    .iter()
    .flat_map(|e| match e {
      SyntaxElement::Node(n) => n.descendants_with_tokens(),
      token => vec![token.clone()],
    })
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t),
      _ => None,
    })
    .collect()
}

/// Whether a comma follows a node, whitespace and comments aside.
fn comma_follows(node: &SyntaxNode) -> bool {
  let mut next = node.next_sibling_or_token();
  while let Some(SyntaxElement::Token(token)) = next {
    if !token.kind().is_trivia() {
      return token.kind() == K::Comma;
    }
    next = token.next_sibling_or_token();
  }
  false
}

/// A const or static with a value, as the standard formatter's `rewrite_static` lays it out:
/// everything up to `=` on one line, then the value placed as a `let`'s is (on that line, or
/// on the next one level in), then `;`. `None` for one with a `where` clause, or anything
/// before the value or in it that laying it out would lose.
fn const_value(
  node: &SyntaxNode,
  start: usize,
  indent: usize,
  config: &FormatConfig,
) -> Option<String> {
  let eq = node
    .children_with_tokens_iter()
    .find(|e| e.kind() == K::Eq)?
    .text_range();
  let value = node
    .children_iter()
    .find(|c| c.text_range().start >= eq.end)?;
  // A comment outside the value (after `=`, after `;`) would be lost; the value's own are
  // checked as it is laid out.
  let inside = value.text_range();
  // A comment after the `;` is not the item's.
  let end = super::imports::code_end(node);
  let commented = node.token_texts().any(|(kind, _, range)| {
    (kind.is_trivia() && kind != K::Whitespace)
      && range.start as usize >= start
      && (range.start as usize) < end
      && !(inside.start <= range.start && range.start < inside.end)
  });
  if commented
    || node.children_iter().any(|c| c.kind() == K::WhereClause)
    || !super::stmt::clean_expression(&value)
  {
    return None;
  }
  let head: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(|e| {
      let at = e.text_range().start as usize;
      start <= at && at < eq.end as usize
    })
    .collect();
  // Nothing in the head that printing it flat would lose.
  if head.iter().any(|e| match e {
    SyntaxElement::Token(t) => t.kind().is_trivia() && t.kind() != K::Whitespace,
    SyntaxElement::Node(n) => n.subtree_has(|kind| {
      (kind.is_trivia() && kind != K::Whitespace) || matches!(kind, K::Attr | K::MacroCall)
    }),
  }) {
    return None;
  }
  // The type laid out after the name in what the line leaves less ` =`, else on the next
  // line one level in (the standard formatter's `rewrite_static`).
  let spacing = Spacing::of(config);
  let ty = node
    .children_iter()
    .find(|c| super::types::is_type(c.kind()) && c.text_range().end <= eq.start);
  let head = match ty {
    Some(ty) => {
      let at = ty.text_range().start;
      let before: Vec<SyntaxElement> = head
        .iter()
        .filter(|e| e.text_range().end <= at)
        .cloned()
        .collect();
      let prefix = format!("{} ", flat_tokens(&code_tokens(&before), spacing));
      let limits = super::shape::Limits::of(config);
      let indented = |at: usize| super::shape::Shape {
        indent: at,
        align: 0,
        offset: 0,
        width: config.max_width.saturating_sub(at),
        limits,
      };
      // 2 = ` =`
      let ty_text = indented(indent)
        .offset_left(str_width(&prefix) + 2)
        .and_then(|s| super::expr::rewrite(&ty, s));
      let ty_text = match ty_text {
        Some(text) => format!("{}{}", prefix, text),
        None => {
          let inner = indent + config.tab_spaces;
          let text = super::expr::rewrite(&ty, indented(inner))?;
          format!("{}\n{}{}", prefix.trim_end(), " ".repeat(inner), text)
        }
      };
      format!("{} =", ty_text)
    }
    None => {
      let head = flat_tokens(&code_tokens(&head), spacing);
      if indent + str_width(&head) + 2 > config.max_width {
        return None;
      }
      head
    }
  };
  // 1 = `;`
  let shape = super::stmt::statement_shape(indent, config)?.sub_width(1)?;
  let text = format!("{};", super::expr::assign_value(&head, &value, shape)?);
  super::stmt::same_tokens_until(node, &text, end).then_some(text)
}

/// Whether a node is the last of its list.
fn is_last(node: &SyntaxNode) -> bool {
  node
    .parent()
    .and_then(|list| list.children_iter().next_back())
    .is_some_and(|last| &last == node)
}

/// [`rewrite_header`] before its tokens are checked.
fn laid_header(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<(String, usize)> {
  if config.hard_tabs || config.tab_spaces == 0 || !is_item(node.kind()) {
    return None;
  }
  let start = header_start(node)?;
  // A generic const the standard formatter leaves as written.
  if matches!(node.kind(), K::Const | K::Static)
    && node
      .children_iter()
      .any(|c| c.kind() == K::GenericParamList)
  {
    return None;
  }
  if matches!(node.kind(), K::Const | K::Static)
    && let Some(text) = const_value(node, start, indent, config)
  {
    return Some((text, super::imports::code_end(node)));
  }
  let body = node.children_iter().find(|c| {
    matches!(
      c.kind(),
      K::BlockExpr
        | K::RecordFieldList
        | K::VariantList
        | K::AssocItemList
        | K::ExternItemList
        | K::ItemList
    )
  });
  // Without a body, up to its last code token: a comment after its `;` is not its own.
  let end = match &body {
    Some(body) => body.text_range().start as usize,
    None => node
      .children_with_tokens_iter()
      .rfind(|e| !e.kind().is_trivia())
      .map_or(node.text_range().end as usize, |e| {
        e.text_range().end as usize
      }),
  };
  let header: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(|e| {
      let at = e.text_range().start as usize;
      start <= at && at < end
    })
    .collect();
  // A parameter may have one attribute, on its line (`#[case] value: T`); more go on lines
  // of their own, not laid out here.
  let parameter_attributes = |n: &SyntaxNode| {
    let attributes: Vec<SyntaxNode> = n.descendant_nodes_where(|kind| kind == K::Attr).collect();
    attributes.iter().all(|a| {
      a.parent().is_some_and(|p| {
        matches!(p.kind(), K::Param | K::SelfParam | K::TupleField)
          || matches!(
            p.kind(),
            K::TypeParam | K::LifetimeParam | K::ConstParam
          ) && p.children_iter().filter(|c| c.kind() == K::Attr).count() == 1
      }) && !a
        .token_texts()
        .any(|(kind, text, _)| kind.is_literal() && text.contains('\n'))
        // One the standard formatter leaves as written keeps its lines, a tuple field's
        // laid out on lines of their own.
        && (super::file::meta_like(a)
          || !a.text().to_string().contains('\n')
          || a.parent().is_some_and(|p| p.kind() == K::TupleField))
    })
  };
  // A comment among a function's parameters, a tuple struct's fields or generic
  // parameters, placed by the list layout.
  let parameter_comment = |t: &SyntaxToken| {
    matches!(t.kind(), K::LineComment | K::BlockComment)
      && !t.text().contains('\n')
      && matches!(
        t.parent().kind(),
        K::ParamList | K::TupleFieldList | K::GenericParamList
      )
  };
  let commented = |n: &SyntaxNode| {
    n.descendants_with_tokens_iter().any(|e| match e {
      SyntaxElement::Token(t) => {
        (t.kind().is_trivia() && t.kind() != K::Whitespace && !parameter_comment(&t))
          || is_doc(t.kind())
      }
      SyntaxElement::Node(_) => false,
    })
  };
  let unwanted = |e: &SyntaxElement| match e {
    SyntaxElement::Token(t) => {
      (t.kind().is_trivia() && t.kind() != K::Whitespace) || is_doc(t.kind())
    }
    SyntaxElement::Node(n) if matches!(n.kind(), K::ParamList | K::TupleFieldList) => {
      commented(n)
        || n.subtree_has(|kind| {
          matches!(
            kind,
            K::BlockExpr | K::MatchArmList | K::MacroCall | K::MacroType | K::MacroExpr
          )
        })
        || n.subtree_has(|kind| kind == K::Attr) && !parameter_attributes(n)
    }
    SyntaxElement::Node(n) => {
      matches!(n.kind(), K::MacroCall | K::Attr)
        // A block as a const generic argument (`B<{ N + 1 }>`) is one.
        || n
          .descendant_nodes_where(|kind| matches!(kind, K::BlockExpr | K::MatchArmList))
          .any(|b| !b.parent().is_some_and(|p| p.kind() == K::ConstArg))
        || commented(n)
        || n.subtree_has(|kind| matches!(kind, K::MacroCall | K::MacroType | K::MacroExpr))
        || n.subtree_has(|kind| kind == K::Attr) && !parameter_attributes(n)
    }
  };
  if header.iter().any(unwanted) {
    return None;
  }
  let spacing = Spacing::of(config);
  let tokens = code_tokens(&header);
  // A `where` clause goes on lines of its own after the rest of the header: `where`, then
  // a predicate per line one level in, each with a comma; then the body's `{` on a line of
  // its own, or the item's `;` after the last predicate (in its comma's place), or a type
  // alias's `= Type;` after them.
  let where_clause = header.iter().find_map(|e| match e {
    SyntaxElement::Node(n) if n.kind() == K::WhereClause => Some(n.clone()),
    _ => None,
  });
  let mut where_block = None;
  let mut tokens = tokens;
  // An empty `where` says nothing: dropped, but a function's.
  tokens.retain(|t| !super::expr::empty_where(t));
  // A type alias's `where` (an associated type's, before or after its value), a const's and
  // a static's have layouts of their own: left as written.
  if where_clause.is_some() && matches!(node.kind(), K::Const | K::Static) {
    return None;
  }
  // An empty `where` says nothing: dropped (its token, by the line's layout), but a
  // function's.
  let where_clause =
    where_clause.filter(|c| c.children_iter().next().is_some() || node.kind() == K::Fn);
  if let Some(clause) = &where_clause {
    let (from, to) = (
      clause.text_range().start as usize,
      clause.text_range().end as usize,
    );
    let tail: Vec<SyntaxToken> = tokens
      .iter()
      .filter(|t| t.text_range().start as usize >= to)
      .cloned()
      .collect();
    tokens.retain(|t| (t.text_range().start as usize) < from);
    let inner = indent + config.tab_spaces;
    // 1 = `,`
    let clause_shape = super::shape::Shape {
      indent: inner,
      align: 0,
      offset: 0,
      width: config.max_width.checked_sub(inner + 1)?,
      limits: super::shape::Limits::of(config),
    };
    let predicates: Vec<String> = clause
      .children_iter()
      .filter(|c| c.kind() == K::WherePred)
      .map(|p| super::types::rewrite_where_predicate(&p, clause_shape))
      .collect::<Option<_>>()?;
    if predicates.is_empty() {
      return None;
    }
    let tail_text = flat_tokens(&tail, spacing);
    let pad = " ".repeat(inner);
    // A blank line written after a predicate's comma, before another, stays (`new_lines`),
    // before style edition 2024.
    let mut blank_before = Vec::new();
    let (mut seen, mut comma, mut blank) = (0, false, false);
    for element in clause.children_with_tokens_iter() {
      match &element {
        SyntaxElement::Token(t) if t.kind() == K::Comma => comma = true,
        SyntaxElement::Token(t) if t.kind() == K::Whitespace => {
          blank = comma && t.text().matches('\n').count() > 1;
        }
        SyntaxElement::Node(n) if n.kind() == K::WherePred => {
          if blank {
            blank_before.push(seen);
          }
          seen += 1;
          (comma, blank) = (false, false);
        }
        _ => {}
      }
    }
    let mut block = format!("\n{}where", " ".repeat(indent));
    for (i, predicate) in predicates.iter().enumerate() {
      let last = i + 1 == predicates.len();
      let comma = if last && tail_text == ";" { "" } else { "," };
      if blank_before.contains(&i) && config.style_edition < 2024 {
        block.push('\n');
      }
      block.push_str(&format!("\n{}{}{}", pad, predicate, comma));
    }
    match tail_text.as_str() {
      "" => {}
      ";" => block.push(';'),
      // A type alias's value after a `where` before it (`= Vec<T>;`), on a line of its own.
      _ if indent + str_width(&tail_text) > config.max_width => return None,
      _ => block.push_str(&format!("\n{}{}", " ".repeat(indent), tail_text)),
    }
    if body.is_some() {
      block.push_str(&format!("\n{}", " ".repeat(indent)));
    }
    // A type alias's `where` before its value: its head on one line, then the clause and
    // the value.
    if node.kind() == K::TypeAlias && tail_text.starts_with('=') {
      let head = flat_tokens(&tokens, spacing);
      return (indent + str_width(&head) <= config.max_width)
        .then(|| (format!("{}{}", head, block), end));
    }
    where_block = Some(block);
  }
  // A function's signature laid out as the standard formatter's `rewrite_fn_base` lays it
  // out, but for comments among its parameters (placed below).
  if node.kind() == K::Fn
    && !node
      .children_iter()
      .filter(|c| c.kind() == K::ParamList)
      .any(|list| {
        list
          .children_with_tokens_iter()
          .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
      })
  {
    let text = rewrite_fn(
      node,
      &tokens,
      indent,
      where_block.as_deref(),
      body.is_some(),
      config,
    )?;
    return Some((text, end));
  }
  // A struct's, enum's or union's generic parameters as a list in angle brackets.
  if matches!(node.kind(), K::Struct | K::Enum | K::Union)
    && node
      .children_iter()
      .any(|c| c.kind() == K::GenericParamList)
    && !node.children_iter().any(|c| c.kind() == K::TupleFieldList)
  {
    let empty_body = body
      .as_ref()
      .is_some_and(|b| b.children_iter().next().is_none());
    return rewrite_generic_item(
      node,
      &tokens,
      indent,
      where_block.as_deref(),
      body.as_ref().map(|_| empty_body),
      config,
    )
    .map(|text| (text, end));
  }
  if node.kind() == K::TypeAlias {
    return rewrite_type_alias(node, &tokens, indent, where_block.as_deref(), config)
      .map(|text| (text, end));
  }
  let one_line = flat_tokens(&tokens, spacing);
  // An `impl` with one `where` predicate and an empty body on one line when it fits
  // (`impl<T> Send for X<T> where T: Send {}`).
  if let (Some(clause), Some(body)) = (&where_clause, &body)
    && node.kind() == K::Impl
    && body
      .children_with_tokens_iter()
      .all(|e| matches!(e.kind(), K::LBrace | K::RBrace | K::Whitespace))
  {
    let predicates: Vec<String> = clause
      .children_iter()
      .filter(|c| c.kind() == K::WherePred)
      .map(|p| flat_tokens(&code_tokens(&[SyntaxElement::Node(p)]), spacing))
      .collect();
    let whole = format!("{} where {} {{}}", one_line, predicates.join(", "));
    // Measured without the indentation and the ` {}` (the standard formatter's
    // `is_impl_single_line`).
    if predicates.len() == 1 && str_width(&whole) - 3 <= config.max_width {
      return Some((whole, body.text_range().end as usize));
    }
    // Too wide, `where` ends the header's line, the predicate on the next one level in with
    // no comma, and the braces on lines of their own.
    if predicates.len() == 1 && indent + str_width(&one_line) <= config.max_width {
      let pad = " ".repeat(indent);
      return Some((
        format!(
          "{} where\n{}{}\n{}{{\n{}}}",
          one_line,
          " ".repeat(indent + config.tab_spaces),
          predicates[0],
          pad,
          pad
        ),
        body.text_range().end as usize,
      ));
    }
  }
  if node.kind() == K::Trait && node.children_with_tokens_iter().any(|e| e.kind() == K::Eq) {
    return rewrite_trait_alias(node, &tokens, indent, config).map(|text| (text, end));
  }
  if node.kind() == K::Trait && body.is_some() {
    return rewrite_trait(node, &tokens, indent, where_block.as_deref(), config)
      .map(|text| (text, end));
  }
  if node.kind() == K::Impl {
    return rewrite_impl(
      node,
      &tokens,
      indent,
      where_block.as_deref(),
      body.is_some(),
      config,
    )
    .map(|text| (text, end));
  }
  // A tuple struct's fields as a call's arguments (`rewrite_with_parens`): on the line
  // within `fn_call_width`, else one per line.
  if node.kind() == K::Struct
    && where_block.is_none()
    && let Some(fields) = node.children_iter().find(|c| c.kind() == K::TupleFieldList)
  {
    let items: Vec<SyntaxNode> = fields
      .children_iter()
      .filter(|c| c.kind() == K::TupleField)
      .collect();
    let from = fields.text_range().start;
    let before: Vec<SyntaxToken> = tokens
      .iter()
      .filter(|t| t.text_range().start < from)
      .cloned()
      .collect();
    let lead = flat_tokens(&before, spacing);
    // 1 = `;`
    let shape = super::shape::Shape {
      indent,
      align: 0,
      offset: 0,
      width: config.max_width.checked_sub(indent + 1)?,
      limits: super::shape::Limits::of(config),
    };
    let text = super::expr::rewrite_list(&lead, &items, shape, config.fn_call_width, "(", ")")?;
    return Some((format!("{};", text), end));
  }
  // Lists and patterns held to their widths are not laid out here yet: a header holding
  // one too wide for its width is left as written.
  let listed = |kind| {
    matches!(
      kind,
      K::TupleType
        | K::TuplePat
        | K::TupleStructPat
        | K::TupleFieldList
        | K::RecordPat
        | K::SlicePat
        | K::ArrayType
    )
  };
  // Only the header's own nodes: the body may be most of the item.
  let header_nodes = header.iter().filter_map(|e| match e {
    SyntaxElement::Node(n) => Some(n),
    SyntaxElement::Token(_) => None,
  });
  let too_wide = header_nodes
    .flat_map(|n| n.descendant_nodes_where(listed))
    .any(|d| {
      let limit = match d.kind() {
        K::TupleType | K::TupleFieldList => config.fn_call_width,
        K::TuplePat | K::TupleStructPat => config.max_width,
        K::RecordPat => config.struct_lit_width,
        K::SlicePat | K::ArrayType => config.array_width,
        _ => return false,
      };
      let at = d.text_range().start as usize;
      start <= at
        && at < end
        && str_width(&flat_tokens(
          &code_tokens(&[SyntaxElement::Node(d.clone())]),
          Spacing::of(config),
        )) > limit
    });
  if too_wide {
    return None;
  }
  // ` {` after a header followed by a body; a declaration with a return type keeps a column
  // free before its `;` (found by measuring the standard formatter).
  let has_ret = node.children_iter().any(|c| c.kind() == K::RetType);
  // Before a `where` clause the line needs no room after it.
  let brace = if where_block.is_some() {
    0
  } else if body.is_some() {
    2
  } else {
    usize::from(has_ret)
  };
  // With `brace_style = "AlwaysNextLine"` the body's `{` starts the next line.
  let suffix = match (&where_block, body.is_some()) {
    (Some(block), _) => block.clone(),
    (None, true) if config.brace_next_line => format!("\n{}", " ".repeat(indent)),
    (None, true) => " ".to_string(),
    (None, false) => String::new(),
  };

  let width = indent + str_width(&one_line) + brace;
  // Comments among a function's parameters: line comments put them one per line; block
  // comments only are not laid out here (the line would lose them).
  let param_list = node.children_iter().find(|c| c.kind() == K::ParamList);
  let param_comments = param_list.as_ref().and_then(|list| {
    let params: Vec<SyntaxNode> = list.children_iter().collect();
    super::lists::list_comments(list, &params)
  });
  if param_list.as_ref().is_some_and(|list| {
    list
      .children_with_tokens_iter()
      .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
  }) {
    let line = |c: &Option<String>| c.as_ref().is_some_and(|c| c.trim_start().starts_with("//"));
    if !param_comments
      .as_ref()
      .is_some_and(|cs| cs.iter().any(|c| line(&c.pre) || line(&c.post)))
    {
      // Block comments on a line only: the parameters on the header's line with them,
      // when it fits (`rewrite_params`, its snippet kept trimmed with no parameter).
      let list = param_list.as_ref()?;
      let params: Vec<SyntaxNode> = list.children_iter().collect();
      let inside = if params.is_empty() {
        let comments: Vec<String> = list
          .children_with_tokens_iter()
          .filter_map(|e| match e {
            SyntaxElement::Token(t) if t.kind() == K::BlockComment && !t.text().contains('\n') => {
              Some(t.text().to_string())
            }
            SyntaxElement::Token(t) if matches!(t.kind(), K::LineComment | K::BlockComment) => {
              Some(String::from("\n"))
            }
            _ => None,
          })
          .collect();
        comments.join(" ")
      } else {
        let comments = param_comments.as_ref()?;
        params
          .iter()
          .zip(comments)
          .map(|(p, c)| {
            let mut item = String::new();
            if let Some(pre) = &c.pre {
              item.push_str(pre.trim());
              item.push(' ');
            }
            item.push_str(&super::expr::flat(p, spacing));
            if let Some(post) = &c.post {
              item.push(' ');
              item.push_str(post.trim());
            }
            item
          })
          .collect::<Vec<_>>()
          .join(", ")
      };
      let laid = one_line.replacen(
        &super::expr::flat(list, spacing),
        &format!("({})", inside),
        1,
      );
      // Each comment counted with five columns more (`total_item_width`).
      let counted = 5
        * list
          .children_with_tokens_iter()
          .filter(|e| e.kind() == K::BlockComment)
          .count();
      if !inside.contains('\n')
        && indent + str_width(&laid) + brace + if params.is_empty() { 0 } else { counted }
          <= config.max_width
      {
        return Some((format!("{}{}", laid, suffix), end));
      }
      // Else one per line with them, as below, for two parameters or more.
      if params.len() < 2 {
        return None;
      }
    }
  }
  // A type alias, const or static: on one line, its calls and lists each within their
  // widths, or left as written.
  if matches!(node.kind(), K::TypeAlias | K::Const | K::Static) {
    if width <= config.max_width && super::stmt::nested_fit(node, indent, config) {
      return Some((format!("{}{}", one_line, suffix), end));
    }
    // A const's or static's value on the next line one level in, when it fits there on
    // one line (as a `let`'s moves).
    let eq = node
      .children_with_tokens_iter()
      .find(|e| e.kind() == K::Eq)?;
    // A value holding a block, which has a layout of its own, is left as written.
    if node.subtree_has(|kind| matches!(kind, K::BlockExpr | K::MatchArmList)) {
      return None;
    }
    let at = eq.text_range().end;
    let (head, value): (Vec<SyntaxToken>, Vec<SyntaxToken>) = tokens
      .iter()
      .cloned()
      .partition(|t| t.text_range().start < at);
    let (head, value) = (flat_tokens(&head, spacing), flat_tokens(&value, spacing));
    let inner = indent + config.tab_spaces;
    // A string spanning lines stays on the item's line.
    return (indent + str_width(&head) <= config.max_width
      && !value.contains('\n')
      && inner + str_width(&value) <= config.max_width
      && super::stmt::nested_fit(node, inner, config))
    .then(|| (format!("{}\n{}{}", head, " ".repeat(inner), value), end));
  }
  if width <= config.max_width && param_comments.is_none() {
    return Some((format!("{}{}", one_line, suffix), end));
  }
  // A declaration exactly that column over: its return type on the next line, one level in.
  if param_comments.is_none()
    && body.is_none()
    && where_block.is_none()
    && has_ret
    && width == config.max_width + 1
    && let Some(ret) = node.children_iter().find(|c| c.kind() == K::RetType)
  {
    let at = ret.text_range().start as usize;
    let (head, tail): (Vec<SyntaxToken>, Vec<SyntaxToken>) = tokens
      .iter()
      .cloned()
      .partition(|t| (t.text_range().start as usize) < at);
    return Some((
      format!(
        "{}\n{}{}",
        flat_tokens(&head, spacing),
        // One level in before style edition 2024, level with the item from it.
        " ".repeat(if config.style_edition >= 2024 {
          indent
        } else {
          indent + config.tab_spaces
        }),
        flat_tokens(&tail, spacing)
      ),
      end,
    ));
  }
  // A function's parameters one per line.
  let params = node
    .children_iter()
    .find(|c| c.kind() == K::ParamList)
    .filter(|_| node.kind() == K::Fn)?;
  let list_start = params.text_range().start as usize;
  let list_end = params.text_range().end as usize;
  let before: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| (t.text_range().start as usize) < list_start)
    .cloned()
    .collect();
  let after: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| (t.text_range().start as usize) >= list_end)
    .cloned()
    .collect();
  let items: Vec<String> = params
    .children_iter()
    .map(|p| flat_tokens(&code_tokens(&[SyntaxElement::Node(p.clone())]), spacing))
    .collect();
  if items.is_empty() {
    return None;
  }
  let open = format!("{}(", flat_tokens(&before, spacing));
  let close = format!("){}", spaced_after(&after, spacing));
  let inner = indent + config.tab_spaces;
  // 1 = `,`; an opening line reaching the width would have its generics broken, which is not
  // done here.
  // Without generics the opening line is written whatever its width.
  let fits = (indent + str_width(&open) < config.max_width || !open.contains('<'))
    && items
      .iter()
      .all(|p| inner + str_width(p) < config.max_width)
    && indent + str_width(&close) <= config.max_width;
  if !fits {
    return None;
  }
  // The body's `{` on a line of its own unless the closing line with it is within the line
  // width less the indentation: the standard formatter counts the indentation twice there
  // (found by measuring it).
  let suffix = if where_block.is_none()
    && body.is_some()
    && 2 * indent + str_width(&close) + 2 > config.max_width
  {
    format!("\n{}", " ".repeat(indent))
  } else {
    suffix
  };
  let pad = " ".repeat(inner);
  // A blank line written after a parameter's comma, before another, stays.
  let mut blank_after = Vec::new();
  let (mut seen, mut comma, mut blank) = (0, false, false);
  for element in params.children_with_tokens_iter() {
    match &element {
      SyntaxElement::Token(t) if t.kind() == K::Comma => comma = true,
      SyntaxElement::Token(t) if t.kind() == K::Whitespace => {
        blank = comma && t.text().matches('\n').count() > 1;
      }
      SyntaxElement::Node(n) if matches!(n.kind(), K::Param | K::SelfParam) => {
        if blank {
          blank_after.push(seen);
        }
        seen += 1;
        (comma, blank) = (false, false);
      }
      _ => {}
    }
  }
  // With comments: one per line, the comments placed as a list's.
  if let Some(comments) = &param_comments {
    // One parameter with a comment the standard formatter writes oddly: left as written.
    if !blank_after.is_empty() || items.len() < 2 || items.iter().any(|p| p.ends_with("...")) {
      return None;
    }
    let body_text = super::lists::write_vertical(
      &items,
      comments,
      inner,
      config.max_width.saturating_sub(inner),
      config.max_width,
      true,
      false,
    )?;
    let suffix = match suffix.strip_prefix(&format!("\n{}where", " ".repeat(indent))) {
      Some(rest) if close == ")" => format!(" where{}", rest),
      _ => suffix,
    };
    return Some((
      format!(
        "{}\n{}{}\n{}{}{}",
        open,
        pad,
        body_text,
        " ".repeat(indent),
        close,
        suffix
      ),
      end,
    ));
  }
  // A C variadic `...` takes no comma after it.
  let lines: String = items
    .iter()
    .enumerate()
    .map(|(i, p)| {
      let comma = if p.ends_with("...") { "" } else { "," };
      let gap = if blank_after.contains(&i) { "\n" } else { "" };
      format!("{}\n{}{}{}", gap, pad, p, comma)
    })
    .collect();
  // A `where` clause after a bare `)` starts on its line (`) where`).
  let suffix = match suffix.strip_prefix(&format!("\n{}where", " ".repeat(indent))) {
    Some(rest) if close == ")" => format!(" where{}", rest),
    _ => suffix,
  };
  Some((
    format!(
      "{}{}\n{}{}{}",
      open,
      lines,
      " ".repeat(indent),
      close,
      suffix
    ),
    end,
  ))
}

/// Whether an enum has both a variant written on several lines and one on a single line.
fn mixed_variants(list: &SyntaxNode, indent: usize, config: &FormatConfig) -> bool {
  let spacing = Spacing::of(config);
  let one_line = |variant: &SyntaxNode| {
    // Attributes, doc comments or comments anywhere in it put it on several lines.
    let attributed = variant.subtree_has(|kind| {
      kind == K::Attr || is_doc(kind) || matches!(kind, K::LineComment | K::BlockComment)
    });
    let fields_fit = variant
      .children_iter()
      .filter(|c| c.kind() == K::RecordFieldList)
      .all(|f| {
        str_width(&flat_tokens(
          &code_tokens(&[SyntaxElement::Node(f.clone())]),
          spacing,
        )) <= config.struct_variant_width + 4
      });
    let text = flat_tokens(
      &code_tokens(&[SyntaxElement::Node(variant.clone())]),
      spacing,
    );
    // The `<` leaves a column for the comma.
    !attributed && fields_fit && indent + str_width(&text) < config.max_width
  };
  let variants: Vec<SyntaxNode> = list
    .children_iter()
    .filter(|v| v.kind() == K::Variant)
    .collect();
  variants.iter().any(|v| !one_line(v)) && variants.iter().any(one_line)
}

/// A function's signature from column `indent`, as the standard formatter's
/// `rewrite_fn_base` lays it out: the name and generic parameters (broken as a list in angle
/// brackets when they do not fit), the parameters on the line when they fit there before the
/// return type, else one per line one level in; the return type after `)`, or on the next
/// line at the parameters' indentation when it would pass the line; then the `where` clause
/// and the body's `{` (on a line of its own when the last line leaves no room for it), or
/// the `;`. `tokens` are the header's code tokens before any `where` clause.
fn rewrite_fn(
  node: &SyntaxNode,
  tokens: &[SyntaxToken],
  indent: usize,
  where_block: Option<&str>,
  has_body: bool,
  config: &FormatConfig,
) -> Option<String> {
  let limits = super::shape::Limits::of(config);
  let max = config.max_width;
  let tab = config.tab_spaces;
  let name = node.children_iter().find(|c| c.kind() == K::Name)?;
  let generics = node
    .children_iter()
    .find(|c| c.kind() == K::GenericParamList);
  let params = node.children_iter().find(|c| c.kind() == K::ParamList)?;
  let ret = node.children_iter().find(|c| c.kind() == K::RetType);
  let at = name.text_range().start;
  let before: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| t.text_range().start < at)
    .cloned()
    .collect();
  // Everything the parts leave out but a `;` would be lost.
  let parts_end = ret
    .as_ref()
    .map_or(params.text_range().end, |r| r.text_range().end);
  if tokens
    .iter()
    .any(|t| t.text_range().start >= parts_end && t.kind() != K::Semi)
  {
    return None;
  }
  let spacing = Spacing::of(config);
  let mut result = format!("{} ", flat_tokens(&before, spacing));
  let name = super::expr::flat(&name, spacing);
  // With predicates the body's `{` goes on a line of its own (`brace_style`
  // "SameLineWhere"), as with `brace_style = "AlwaysNextLine"`.
  let next_line_brace = has_body && (where_block.is_some() || config.brace_next_line);
  // 4 = `() {`, 2 = `()`
  let overhead = if has_body && !next_line_brace { 4 } else { 2 };
  let used = indent + str_width(&result);
  let shape = super::shape::Shape {
    align: 0,
    indent,
    offset: str_width(&result),
    width: max.saturating_sub(used + overhead),
    limits,
  };
  let generics_text = match &generics {
    Some(list) => {
      let items: Vec<SyntaxNode> = list.children_iter().collect();
      super::expr::rewrite_list(&name, &items, shape, max, "<", ">")?
    }
    None => name,
  };
  result.push_str(&generics_text);
  let snuggled = generics_text
    .rsplit('\n')
    .next()
    .is_some_and(|l| l.trim_start().len() == 1);
  let _ = snuggled;
  // The return type laid out from the item's indentation, to be laid out again where it
  // goes when it moves (`Shape::indented`).
  let indented = super::shape::Shape {
    align: 0,
    indent,
    offset: 0,
    width: max.saturating_sub(indent),
    limits,
  };
  let ret_type = |shape: super::shape::Shape| -> Option<String> {
    let Some(ret) = &ret else {
      return Some(String::new());
    };
    let ty = ret.children_iter().next_back()?;
    // 3 = `-> `
    let inner = if config.style_edition >= 2024 {
      shape.offset_left(3)?
    } else {
      super::types::legacy(shape, 3)?
    };
    Some(format!("-> {}", super::expr::rewrite(&ty, inner)?))
  };
  let ret_text = ret_type(indented)?;
  let multi_line_ret = ret_text.contains('\n');
  let ret_len = if multi_line_ret {
    0
  } else {
    str_width(&ret_text)
  };
  // The parameters' budgets (`compute_budgets_for_params`).
  let param_indent = indent + tab;
  let multi_line_budget = max.saturating_sub(param_indent + 1);
  let one_line_budget = if !result.contains('\n') && !multi_line_ret {
    // 2 = `()`, 3 = `() `
    let overhead = if ret_len == 0 { 2 } else { 3 };
    let brace = if !has_body {
      1
    } else if next_line_brace {
      0
    } else {
      2
    };
    max.saturating_sub(indent + str_width(&result) + ret_len + overhead + brace)
  } else {
    0
  };
  let param_shape = super::shape::Shape {
    align: 0,
    indent: param_indent,
    offset: 0,
    width: multi_line_budget,
    limits,
  };
  let param_nodes: Vec<SyntaxNode> = params.children_iter().collect();
  // One that does not lay out is kept as written (`rewrite_params`).
  let texts: Vec<String> = param_nodes
    .iter()
    .map(|p| super::types::rewrite_param(p, param_shape).unwrap_or_else(|| p.text().to_string()))
    .collect();
  let variadic = texts.last().is_some_and(|t| t.ends_with("..."));
  let total = texts.iter().map(|t| str_width(t)).sum::<usize>() + 2 * texts.len().saturating_sub(1);
  let horizontal = total <= one_line_budget && !texts.iter().any(|t| t.contains('\n'));
  // A blank line written after a parameter's comma, before another, stays.
  let mut blank_after = Vec::new();
  let (mut seen, mut comma, mut blank) = (0, false, false);
  for element in params.children_with_tokens_iter() {
    match &element {
      SyntaxElement::Token(t) if t.kind() == K::Comma => comma = true,
      SyntaxElement::Token(t) if t.kind() == K::Whitespace => {
        blank = comma && t.text().matches('\n').count() > 1;
      }
      SyntaxElement::Node(n) if matches!(n.kind(), K::Param | K::SelfParam) => {
        if blank {
          blank_after.push(seen);
        }
        seen += 1;
        (comma, blank) = (false, false);
      }
      _ => {}
    }
  }
  // No parameters but block comments: they stand in the parentheses, spaced
  // (`rewrite_params` keeps the snippet, trimmed).
  let comment_tokens: Vec<SyntaxToken> = params
    .children_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if matches!(t.kind(), K::LineComment | K::BlockComment) => Some(t),
      _ => None,
    })
    .collect();
  let lone_comments: Vec<String> = comment_tokens
    .iter()
    .map(|t| t.text().to_string())
    .collect();
  let param_text = if texts.is_empty() && !lone_comments.is_empty() {
    if comment_tokens
      .iter()
      .any(|t| t.kind() == K::LineComment || t.text().contains('\n'))
    {
      return None;
    }
    lone_comments.join(" ")
  } else if horizontal {
    texts.join(", ")
  } else {
    let pad = " ".repeat(param_indent);
    let mut text = String::new();
    for (i, t) in texts.iter().enumerate() {
      if i > 0 {
        text.push_str(",\n");
        if blank_after.contains(&i) {
          text.push('\n');
        }
        text.push_str(&pad);
      }
      text.push_str(t);
    }
    if !variadic && !config.trailing_comma_never {
      text.push(',');
    }
    text
  };
  let mut no_params_over = false;
  let in_block =
    !texts.is_empty() && (param_text.contains('\n') || str_width(&param_text) > one_line_budget);
  if in_block {
    result.push_str(&format!(
      "(\n{}{}\n{})",
      " ".repeat(param_indent),
      param_text,
      " ".repeat(indent)
    ));
  } else {
    result.push('(');
    result.push_str(&param_text);
    // No parameters and the return type past the line: `)` on a line of its own before
    // style edition 2024, the return type there from it.
    let used = if result.contains('\n') {
      super::expr::last_line_width(&result)
    } else {
      indent + str_width(&result)
    } + super::expr::first_line_width(&ret_text);
    let overflow = texts.is_empty() && used + 1 > max;
    if overflow && config.style_edition >= 2024 {
      result.push_str(&format!(")\n{}", " ".repeat(indent)));
      no_params_over = true;
    } else if overflow {
      result.push_str(&format!("\n{})", " ".repeat(indent)));
    } else {
      result.push(')');
    }
  }
  let mut newline_brace = false;
  if ret.is_some() {
    let should_indent = if in_block || texts.is_empty() {
      false
    } else if result.contains('\n') || multi_line_ret {
      true
    } else {
      // 1 = the space before `->`, 2 = ` {` without predicates.
      let mut length = str_width(&result) + indent + ret_len + 1;
      if where_block.is_none() {
        length += 2;
      }
      length > max
    };
    let ret_shape = if should_indent {
      let at = if config.style_edition >= 2024 {
        indent
      } else if texts.is_empty() {
        newline_brace = true;
        indent + 4
      } else {
        param_indent
      };
      result.push_str(&format!("\n{}", " ".repeat(at)));
      let mut shape = super::shape::Shape {
        align: 0,
        indent: at,
        offset: 0,
        width: max.saturating_sub(at),
        limits,
      };
      if config.style_edition >= 2024 && texts.is_empty() {
        newline_brace = true;
        shape = shape.offset_left(4).unwrap_or(shape);
      }
      shape
    } else {
      if !no_params_over {
        result.push(' ');
      }
      let used = super::expr::last_line_width(&result);
      indented.offset_left(used).unwrap_or(indented)
    };
    if multi_line_ret || should_indent {
      result.push_str(&ret_type(ret_shape)?);
    } else {
      result.push_str(&ret_text);
    }
  }
  // The `where` clause, `) where` after parameters on lines of their own and no return type.
  if let Some(block) = where_block {
    let lead = format!("\n{}where", " ".repeat(indent));
    match block.strip_prefix(&lead) {
      Some(rest) if in_block && ret.is_none() => result.push_str(&format!(" where{}", rest)),
      _ => result.push_str(block),
    }
    return Some(result);
  }
  if !has_body {
    result.push(';');
    return Some(result);
  }
  // 2 = ` {`, the last line counted with its indentation against the line less the
  // item's (`rewrite_fn_before_block`).
  if newline_brace
    || config.brace_next_line
    || super::expr::last_line_width(&result) + 2 > max.saturating_sub(indent)
  {
    result.push_str(&format!("\n{}", " ".repeat(indent)));
  } else {
    result.push(' ');
  }
  Some(result)
}

/// A struct's, enum's or union's header with generic parameters, as the standard
/// formatter's `format_generics` lays it out: the parameters as a list in angle brackets
/// after the name, then a `where` clause (after a lone `>` on its line), or the body's `{`
/// after a space when the line leaves room for it (counted without the indentation), else
/// on a line of its own; or the `;`. `body` is whether the body is empty, when there is one.
fn rewrite_generic_item(
  node: &SyntaxNode,
  tokens: &[SyntaxToken],
  indent: usize,
  where_block: Option<&str>,
  body: Option<bool>,
  config: &FormatConfig,
) -> Option<String> {
  let spacing = Spacing::of(config);
  let generics = node
    .children_iter()
    .find(|c| c.kind() == K::GenericParamList)?;
  let (from, to) = (generics.text_range().start, generics.text_range().end);
  let before: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| t.text_range().start < from)
    .cloned()
    .collect();
  // Nothing after the parameters but a `;`.
  if tokens
    .iter()
    .any(|t| t.text_range().start >= to && t.kind() != K::Semi)
  {
    return None;
  }
  let header = flat_tokens(&before, spacing);
  let used = str_width(&header);
  let shape = super::shape::Shape {
    indent,
    align: 0,
    offset: used,
    width: config.max_width.checked_sub(used + indent)?,
    limits: super::shape::Limits::of(config),
  };
  let items: Vec<SyntaxNode> = generics.children_iter().collect();
  let list = super::expr::rewrite_list("", &items, shape, config.max_width, "<", ">")?;
  let mut result = format!("{}{}", header, list);
  if let Some(block) = where_block {
    // After a lone `>`, `where` follows it on its line.
    let lead = format!("\n{}where", " ".repeat(indent));
    match block.strip_prefix(&lead) {
      Some(rest) if super::expr::last_line_width(&list) == 1 && list.contains('\n') => {
        result.push_str(&format!(" where{}", rest))
      }
      _ => result.push_str(block),
    }
    // An empty body's `{}` stays after the last predicate (`BracePos::ForceSameLine`).
    if body == Some(true) {
      let trimmed = result.trim_end_matches(' ').strip_suffix('\n')?.to_string();
      result = format!("{} ", trimmed);
    }
    return Some(result);
  }
  let Some(empty) = body else {
    result.push(';');
    return Some(result);
  };
  let total = if list.contains('\n') {
    super::expr::last_line_width(&list)
  } else {
    used + str_width(&list)
  };
  // 3 = ` {}`, 2 = ` {`
  let overhead = if empty { 3 } else { 2 };
  let same_line = empty
    || !config.brace_next_line
    || list
      .rsplit('\n')
      .next()
      .is_some_and(|l| l.trim().len() == 1);
  if config.max_width.saturating_sub(total) >= overhead && same_line {
    result.push(' ');
  } else {
    result.push_str(&format!("\n{}", " ".repeat(indent)));
  }
  Some(result)
}

/// An `impl`'s header from column `indent`, as the standard formatter's
/// `format_impl_ref_and_type` lays it out: `impl` and its generic parameters (broken as a
/// list, before style edition 2024 one level further in), the trait on the line when it fits
/// there on one line, else on the next one level in, then `for` and the type on the line
/// when it fits there on one line (counted without the indentation), else on the next one
/// level in; then a `where` clause, and the body's `{`, on a line of its own after a header
/// across lines.
fn rewrite_impl(
  node: &SyntaxNode,
  tokens: &[SyntaxToken],
  indent: usize,
  where_block: Option<&str>,
  has_body: bool,
  config: &FormatConfig,
) -> Option<String> {
  let limits = super::shape::Limits::of(config);
  let spacing = Spacing::of(config);
  let max = config.max_width;
  let tab = config.tab_spaces;
  let impl_kw = tokens.iter().find(|t| t.kind() == K::ImplKw)?;
  let generics = node
    .children_iter()
    .find(|c| c.kind() == K::GenericParamList);
  let for_kw = tokens
    .iter()
    .find(|t| t.kind() == K::ForKw && t.parent().kind() == K::Impl);
  let types: Vec<SyntaxNode> = node
    .children_iter()
    .filter(|c| super::types::is_type(c.kind()) || c.kind() == K::MacroType)
    .collect();
  let (trait_ref, self_ty) = match (&for_kw, types.as_slice()) {
    (Some(_), [t, s]) => (Some(t.clone()), s.clone()),
    (None, [s]) => (None, s.clone()),
    _ => return None,
  };
  // Only `!` may stand between the generics and the trait.
  let after_generics = generics
    .as_ref()
    .map_or(impl_kw.text_range().end, |g| g.text_range().end);
  let polarity: Vec<&SyntaxToken> = tokens
    .iter()
    .filter(|t| {
      t.text_range().start >= after_generics
        && trait_ref.as_ref().unwrap_or(&self_ty).text_range().start > t.text_range().start
    })
    .collect();
  let polarity = match polarity.as_slice() {
    [] => "",
    [t] if t.kind() == K::Bang && trait_ref.is_some() => "!",
    _ => return None,
  };
  let before: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| t.text_range().start < impl_kw.text_range().start)
    .cloned()
    .collect();
  let mut result = flat_tokens(&before, spacing);
  if !result.is_empty() {
    result.push(' ');
  }
  let used = str_width(&result);
  let generics_shape = if config.style_edition >= 2024 {
    super::shape::Shape {
      indent,
      align: used,
      offset: used,
      width: max.checked_sub(indent + used)?,
      limits,
    }
  } else {
    // 1 = `,`
    super::shape::Shape {
      indent: indent + tab,
      align: 0,
      offset: 0,
      width: max.checked_sub(indent + tab + 1)?,
      limits,
    }
  };
  match &generics {
    Some(list) => {
      let items: Vec<SyntaxNode> = list.children_iter().collect();
      result.push_str(&super::expr::rewrite_list(
        "impl",
        &items,
        generics_shape,
        max,
        "<",
        ">",
      )?);
    }
    None => result.push_str("impl"),
  }
  let indented = |at: usize| super::shape::Shape {
    indent: at,
    align: 0,
    offset: 0,
    width: max.saturating_sub(at),
    limits,
  };
  let last_width = |text: &str| super::expr::last_line_width(text);
  if let Some(trait_ref) = &trait_ref {
    // 1 = the space before the trait
    let used = 1 + polarity.len() + last_width(&result);
    let shape = super::shape::Shape {
      offset: used,
      align: used,
      width: max.saturating_sub(indent + used),
      ..indented(indent)
    };
    match super::expr::rewrite(trait_ref, shape).filter(|t| !t.contains('\n')) {
      Some(text) => result.push_str(&format!(" {}{}", polarity, text)),
      None => {
        let text = super::expr::rewrite(trait_ref, indented(indent + tab))?;
        result.push_str(&format!(
          "\n{}{}{}",
          " ".repeat(indent + tab),
          polarity,
          text
        ));
      }
    }
  }
  let for_len = if trait_ref.is_some() { 4 } else { 0 };
  let brace = if where_block.is_none() && !config.brace_next_line {
    2
  } else {
    0
  };
  // 1 = the space before the type; the budget leaves the indentation out.
  let budget = max.saturating_sub(last_width(&result) + for_len + brace + 1);
  let one_line = super::expr::rewrite(
    &self_ty,
    super::shape::Shape {
      width: budget,
      ..indented(indent)
    },
  )
  .filter(|t| !t.contains('\n'));
  match one_line {
    Some(text) => {
      result.push_str(if trait_ref.is_some() { " for " } else { " " });
      result.push_str(&text);
    }
    None => {
      result.push_str(&format!("\n{}", " ".repeat(indent + tab)));
      if trait_ref.is_some() {
        result.push_str("for ");
      }
      let budget = max.saturating_sub(last_width(&result));
      let text = super::expr::rewrite(
        &self_ty,
        super::shape::Shape {
          width: budget,
          ..indented(indent + tab)
        },
      )?;
      result.push_str(&text);
    }
  }
  if let Some(block) = where_block {
    result.push_str(block);
    return Some(result);
  }
  if !has_body {
    return None;
  }
  if result.contains('\n') || config.brace_next_line {
    result.push_str(&format!("\n{}", " ".repeat(indent)));
  } else {
    result.push(' ');
  }
  Some(result)
}

/// A trait's header without bounds from column `indent`, as the standard formatter's
/// `format_trait` lays it out: the name and generic parameters (broken as a list), then a
/// `where` clause (after a lone `>` on its line), and the body's `{` after a space, or on a
/// line of its own after a header across lines or when the line leaves it no room.
fn rewrite_trait(
  node: &SyntaxNode,
  tokens: &[SyntaxToken],
  indent: usize,
  where_block: Option<&str>,
  config: &FormatConfig,
) -> Option<String> {
  let spacing = Spacing::of(config);
  let name = node.children_iter().find(|c| c.kind() == K::Name)?;
  let generics = node
    .children_iter()
    .find(|c| c.kind() == K::GenericParamList);
  let bounds = node.children_iter().find(|c| c.kind() == K::TypeBoundList);
  let at = name.text_range().start;
  let parts_end = bounds
    .as_ref()
    .or(generics.as_ref())
    .map_or(name.text_range().end, |g| g.text_range().end);
  if tokens.iter().any(|t| t.text_range().start >= parts_end) {
    return None;
  }
  let before: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| t.text_range().start < at)
    .cloned()
    .collect();
  let mut result = format!("{} ", flat_tokens(&before, spacing));
  let name = super::expr::flat(&name, spacing);
  let shape = super::shape::Shape {
    indent,
    align: 0,
    offset: str_width(&result),
    width: config.max_width.checked_sub(indent + str_width(&result))?,
    limits: super::shape::Limits::of(config),
  };
  let generics_text = match &generics {
    Some(list) => {
      let items: Vec<SyntaxNode> = list.children_iter().collect();
      super::expr::rewrite_list(&name, &items, shape, config.max_width, "<", ">")?
    }
    None => name,
  };
  result.push_str(&generics_text);
  // The bounds after `:` on the line when they fit there, else on the next line one level
  // in (`rewrite_assign_rhs_with` and `RhsTactics::ForceNextLineWithoutIndent`): what is
  // before the bounds counted from the shape's offset, which holds the text before the name
  // already.
  if let Some(bounds) = bounds.filter(|b| b.children_iter().next().is_some()) {
    result.push(':');
    let used = if result.contains('\n') {
      super::expr::last_line_width(&result).saturating_sub(indent)
    } else {
      super::expr::last_line_width(&result)
    };
    let same_shape = shape.offset_left(used + 1);
    let same = same_shape.and_then(|s| super::types::join_bounds(&bounds, s, true));
    match (same, same_shape) {
      (Some(same), Some(s)) if !same.contains('\n') && str_width(&same) <= s.width => {
        result.push_str(&format!(" {}", same));
      }
      (same, _) => {
        let next_shape = super::shape::Shape {
          width: config.max_width.checked_sub(2 * indent)?,
          ..shape
        };
        let next = super::types::join_bounds(&bounds, next_shape, true);
        let fits = |text: &str| {
          super::expr::first_line_width(text) <= next_shape.width
            && text
              .lines()
              .skip(1)
              .all(|l| str_width(l) <= config.max_width)
            && (!text.contains('\n')
              || super::expr::last_line_width(text)
                <= next_shape.indent + next_shape.offset + next_shape.width)
        };
        match (same, next) {
          (Some(same), Some(next)) if !fits(&next) => result.push_str(&format!(" {}", same)),
          (_, Some(next)) => result.push_str(&format!(
            "\n{}{}",
            " ".repeat(indent + config.tab_spaces),
            next
          )),
          (Some(same), None) => result.push_str(&format!(" {}", same)),
          (None, None) => return None,
        }
      }
    }
  }
  if let Some(block) = where_block {
    let lead = format!("\n{}where", " ".repeat(indent));
    match block.strip_prefix(&lead) {
      Some(rest) if generics_text.contains('\n') && super::expr::last_line_width(&result) == 1 => {
        result.push_str(&format!(" where{}", rest))
      }
      _ => result.push_str(block),
    }
    return Some(result);
  }
  // 2 = ` {`, against the line less the indentation, the last line counted with its own.
  if result.contains('\n')
    || config.brace_next_line
    || super::expr::last_line_width(&result) + 2 > config.max_width.saturating_sub(indent)
  {
    result.push_str(&format!("\n{}", " ".repeat(indent)));
  } else {
    result.push(' ');
  }
  Some(result)
}

/// A trait alias (`trait A<T> = B + C;`), as the standard formatter's `rewrite_trait_alias`
/// lays it out: the name and generic parameters, then the bounds after ` =` placed as a
/// right-hand side.
fn rewrite_trait_alias(
  node: &SyntaxNode,
  tokens: &[SyntaxToken],
  indent: usize,
  config: &FormatConfig,
) -> Option<String> {
  let spacing = Spacing::of(config);
  let name = node.children_iter().find(|c| c.kind() == K::Name)?;
  let generics = node
    .children_iter()
    .find(|c| c.kind() == K::GenericParamList);
  let bounds = node
    .children_iter()
    .find(|c| c.kind() == K::TypeBoundList)
    .filter(|b| b.children_iter().next().is_some())?;
  let eq = tokens.iter().find(|t| t.kind() == K::Eq)?;
  // Nothing but the parts: the name, its generics, `=`, the bounds and the `;`.
  let at = name.text_range().start;
  let generics_end = generics
    .as_ref()
    .map_or(name.text_range().end, |g| g.text_range().end);
  if tokens.iter().any(|t| {
    let start = t.text_range().start;
    start >= generics_end && start < eq.text_range().start
      || start >= bounds.text_range().end && t.kind() != K::Semi
  }) || where_clause_of(node)
  {
    return None;
  }
  let before: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| t.text_range().start < at)
    .cloned()
    .collect();
  let limits = super::shape::Limits::of(config);
  let shape = super::shape::Shape {
    indent,
    align: 0,
    offset: 0,
    width: config.max_width.checked_sub(indent)?,
    limits,
  };
  let name = super::expr::flat(&name, spacing);
  let generics_text = match &generics {
    Some(list) => {
      // 6 = `trait `, 2 = ` =`
      let g_shape = shape.offset_left(6)?.sub_width(2)?;
      let items: Vec<SyntaxNode> = list.children_iter().collect();
      super::expr::rewrite_list(&name, &items, g_shape, config.max_width, "<", ">")?
    }
    None => name,
  };
  let head = format!("{} {} =", flat_tokens(&before, spacing), generics_text);
  // 1 = `;`
  let text = super::types::assign_rhs(&head, shape.sub_width(1)?, |s| {
    super::types::join_bounds(&bounds, s, true)
  })?;
  Some(format!("{};", text))
}

/// A type alias (or an associated type) from column `indent`, as the standard formatter's
/// `rewrite_ty` lays it out: the name and generic parameters, its bounds after `: `, then
/// the type after `= ` on the line, or on the next one level in when that is better.
fn rewrite_type_alias(
  node: &SyntaxNode,
  tokens: &[SyntaxToken],
  indent: usize,
  where_block: Option<&str>,
  config: &FormatConfig,
) -> Option<String> {
  let limits = super::shape::Limits::of(config);
  let spacing = Spacing::of(config);
  let name = node.children_iter().find(|c| c.kind() == K::Name)?;
  let generics = node
    .children_iter()
    .find(|c| c.kind() == K::GenericParamList);
  let bounds = node.children_iter().find(|c| c.kind() == K::TypeBoundList);
  let eq = node
    .children_with_tokens_iter()
    .find(|e| e.kind() == K::Eq)
    .map(|e| e.text_range().end);
  let ty = eq.and_then(|at| node.children_iter().find(|c| c.text_range().start >= at));
  let at = name.text_range().start;
  let before: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| t.text_range().start < at)
    .cloned()
    .collect();
  // Nothing past the parts but the `;`.
  let parts_end = ty
    .as_ref()
    .or(bounds.as_ref())
    .or(generics.as_ref())
    .map_or(name.text_range().end, |n| n.text_range().end);
  if tokens.iter().any(|t| {
    t.text_range().start >= parts_end && t.kind() != K::Semi
      || eq.is_some_and(|e| t.text_range().start < e && t.text_range().start >= parts_end)
  }) {
    return None;
  }
  if eq.is_some() && ty.is_none() {
    return None;
  }
  let indented = super::shape::Shape {
    indent,
    align: 0,
    offset: 0,
    width: config.max_width.saturating_sub(indent),
    limits,
  };
  let mut result = format!("{} ", flat_tokens(&before, spacing));
  let name = super::expr::flat(&name, spacing);
  match &generics {
    Some(list) => {
      // 2 = `= `
      let shape = indented.offset_left(str_width(&result))?.sub_width(2)?;
      let items: Vec<SyntaxNode> = list.children_iter().collect();
      result.push_str(&super::expr::rewrite_list(
        &name,
        &items,
        shape,
        config.max_width,
        "<",
        ">",
      )?);
    }
    None => result.push_str(&name),
  }
  if let Some(bounds) = &bounds
    && bounds.children_iter().next().is_some()
  {
    // 2 = `: `
    let shape = indented.offset_left(str_width(&result) + 2)?;
    let text = super::types::join_bounds(bounds, shape, true)?;
    result.push_str(&format!(": {}", text));
  }
  // A `where` clause after it (the `;` its own): the `=` and the type on the next line one
  // level in, the clause after them.
  let Some(ty) = ty else {
    match where_block {
      Some(block) => result.push_str(block),
      None => result.push(';'),
    }
    return Some(result);
  };
  if let Some(block) = where_block {
    let head = format!("{}\n{}=", result, " ".repeat(indent + config.tab_spaces));
    let text = super::types::assign_rhs(&head, indented, |s| super::expr::rewrite(&ty, s))?;
    return Some(format!("{}{}", text, block));
  }
  let head = format!("{} =", result);
  // 1 = `;`
  let shape = indented.sub_width(1)?;
  let text = super::types::assign_rhs(&head, shape, |s| super::expr::rewrite(&ty, s))?;
  Some(format!("{};", text))
}

/// Whether two texts hold the same tokens and comments, in order, but for those a layout
/// adds or takes away (a list's trailing comma, an arm's braces, a `;`).
fn same_header_tokens(written: &str, laid: &str) -> bool {
  let tokens = |text: &str| -> Vec<String> {
    let mut at = 0;
    crate::syntax::lexer::tokenize(text, crate::syntax::Edition::E2021)
      .into_iter()
      .filter_map(|token| {
        let piece = &text[at..at + token.len as usize];
        at += token.len as usize;
        (token.kind != K::Whitespace && super::stmt::compared(token.kind)
          || matches!(token.kind, K::LineComment | K::BlockComment))
        .then(|| piece.trim_end().to_string())
      })
      .collect::<Vec<String>>()
  };
  // Less the `::` (two `:` here) a visibility's path from the root drops (`pub(in ::a)`),
  // and the `in` of `pub(in crate)`, an empty `where`, an empty `<>`.
  let rooted = |mut tokens: Vec<String>| {
    let mut i = 0;
    while i + 1 < tokens.len() {
      if tokens[i] == "<" && tokens[i + 1] == ">" {
        tokens.drain(i..i + 2);
      } else {
        i += 1;
      }
    }
    let mut i = 0;
    while i < tokens.len() {
      if tokens[i] == "where"
        && tokens
          .get(i + 1)
          .is_none_or(|next| matches!(next.as_str(), "{" | ";" | "="))
      {
        tokens.remove(i);
      }
      i += 1;
    }
    let mut i = 2;
    while i + 2 < tokens.len() {
      if tokens[i] == "in"
        && tokens[i - 1] == "("
        && tokens[i - 2] == "pub"
        && matches!(tokens[i + 1].as_str(), "crate" | "super" | "self")
        && tokens[i + 2] == ")"
      {
        tokens.remove(i);
      }
      i += 1;
    }
    let mut i = 3;
    while i + 1 < tokens.len() {
      if tokens[i] == ":"
        && tokens[i + 1] == ":"
        && tokens[i - 1] == "in"
        && tokens[i - 2] == "("
        && tokens[i - 3] == "pub"
      {
        tokens.drain(i..i + 2);
      }
      i += 1;
    }
    tokens
  };
  rooted(tokens(written)) == rooted(tokens(laid))
}

/// `-> R` or `;` after a parameter list, with the space before an arrow.
fn spaced_after(after: &[SyntaxToken], spacing: Spacing) -> String {
  match after.first() {
    None => String::new(),
    Some(t) if t.kind() == K::Semi => flat_tokens(after, spacing),
    Some(_) => format!(" {}", flat_tokens(after, spacing)),
  }
}

/// Whether an item has a `where` clause.
fn where_clause_of(node: &SyntaxNode) -> bool {
  node.children_iter().any(|c| c.kind() == K::WhereClause)
}

#[cfg(test)]
mod tests;
