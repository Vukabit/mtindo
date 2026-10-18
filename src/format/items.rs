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

/// A `const` block standing as an item (`const { f() }`, which the parser takes for a
/// `const` with no name): its block of one expression on one line, as an expression's is,
/// when that fits from column `indent`; `None` otherwise.
pub fn const_block_item(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<String> {
  if node.kind() != K::Const
    || config.hard_tabs
    || node
      .children_iter()
      .any(|c| matches!(c.kind(), K::Name | K::Attr))
    || node.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment) || is_doc(kind))
  {
    return None;
  }
  let block = node.children_iter().find(|c| c.kind() == K::BlockExpr)?;
  if !super::expr::joins(&block) {
    return None;
  }
  let expression = super::expr::lone_expression(&block)?;
  let shape = super::stmt::statement_shape(indent, config)?
    .offset_left(8)?
    .sub_width(2)?;
  let inner = super::expr::rewrite(&expression, shape)?;
  if inner.contains('\n') {
    return None;
  }
  Some(format!("const {{ {} }}", inner))
}

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
/// holding a comment or a macro (but one as a type), or one that does not fit.
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
          || matches!(kind, K::MacroExpr | K::BlockExpr)
      })
        // A macro call only as a type (`T![,]`).
        || n.subtree_has(|kind| matches!(kind, K::MacroCall | K::MacroType)) && !macros_in_types(n)
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
    // A discriminant laid out as an expression (`= m! {}`), else as written on a line.
    let value = tail.iter().find_map(|e| match e {
      SyntaxElement::Node(n) => Some(n.clone()),
      SyntaxElement::Token(_) => None,
    });
    let laid = value.and_then(|v| {
      let shape = super::stmt::statement_shape(indent, config)?;
      super::expr::rewrite(&v, shape).filter(|t| !t.contains('\n'))
    });
    let tail = match laid {
      Some(value) => format!(" = {}", value),
      None => flat_tokens(&code_tokens(&tail), spacing),
    };
    let tail = if tail.is_empty() || tail.starts_with(' ') {
      tail
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
  // A struct-like variant whose fields hold block comments on a line only, between them:
  // on one line with them when that fits `struct_variant_width` (each comment counted with
  // six columns more, `total_item_width`), as the standard formatter writes its fields
  // horizontally.
  if node.kind() == K::Variant
    && let Some(fields) = node
      .children_iter()
      .find(|c| c.kind() == K::RecordFieldList)
    && !fields.children().is_empty()
    && fields
      .children_with_tokens_iter()
      .any(|e| e.kind() == K::BlockComment)
    && fields.children_with_tokens_iter().all(|e| match e {
      SyntaxElement::Token(t) => {
        !(t.kind().is_trivia() && t.kind() != K::Whitespace)
          || t.kind() == K::BlockComment && !t.text().contains('\n')
      }
      SyntaxElement::Node(n) => {
        !n.subtree_has(|kind| {
          (kind.is_trivia() && kind != K::Whitespace) || is_doc(kind) || kind == K::Attr
        }) && !unwanted(&SyntaxElement::Node(n))
      }
    })
    && !elements
      .iter()
      .filter(|e| e.text_range() != fields.text_range())
      .any(unwanted)
    && !node
      .parent()
      .is_some_and(|list| mixed_variants(&list, indent, config))
  {
    let items: Vec<SyntaxNode> = fields.children_iter().collect();
    let comments = super::lists::list_comments(&fields, &items)?;
    let mut joined = Vec::new();
    let mut counted = 0;
    for (item, c) in items.iter().zip(&comments) {
      let text = flat_tokens(&code_tokens(&[SyntaxElement::Node(item.clone())]), spacing);
      let mut piece = String::new();
      if let Some(pre) = &c.pre {
        piece.push_str(pre.trim());
        piece.push(' ');
        counted += pre.trim().len().max(4) + 6;
      }
      counted += text.len();
      piece.push_str(&text);
      if let Some(post) = &c.post {
        piece.push(' ');
        piece.push_str(post.trim());
        counted += post.trim().len().max(4) + 6;
      }
      joined.push(piece);
    }
    let inside = joined.join(", ");
    counted += 2 * (items.len() - 1);
    if counted <= config.struct_variant_width && inside.len() <= config.struct_variant_width {
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
      let tail = flat_tokens(&code_tokens(&tail), spacing);
      let tail = if tail.is_empty() {
        String::new()
      } else {
        format!(" {}", tail)
      };
      let comma = if comma_follows(node) { "" } else { "," };
      let text = format!(
        "{} {{ {} }}{}{}",
        flat_tokens(&code_tokens(&head), spacing),
        inside,
        tail,
        comma
      );
      if indent + str_width(&text) <= config.max_width {
        return Some((text, node.text_range().end as usize));
      }
    }
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
  // A tuple variant's fields holding attributes or block comments, and comments on a line
  // between them only, are laid out as a list (below), which places the comments.
  let listed_fields = |e: &SyntaxElement| {
    match e {
    SyntaxElement::Node(n) => {
      node.kind() == K::Variant
        && n.kind() == K::TupleFieldList
        && n.subtree_has(|kind| matches!(kind, K::Attr | K::BlockComment))
        && n.children_iter().all(|f| {
          !f.subtree_has(|kind| (kind.is_trivia() && kind != K::Whitespace) || is_doc(kind))
        })
        && n.children_with_tokens_iter().all(|t| {
          !matches!(t, SyntaxElement::Token(ref t) if t.kind() == K::BlockComment && t.text().contains('\n'))
        })
    }
    SyntaxElement::Token(_) => false,
  }
  };
  if elements.iter().filter(|e| !listed_fields(e)).any(unwanted) {
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
  // A tuple variant whose fields hold attributes or block comments: its fields as a list
  // (`V(#[a] T)`), as a tuple struct's.
  if node.kind() == K::Variant
    && let Some(fields) = node.children_iter().find(|c| c.kind() == K::TupleFieldList)
    && fields.subtree_has(|kind| matches!(kind, K::Attr | K::BlockComment))
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
    // After the fields, only a discriminant (`= 3`), on the line after them.
    let after: Vec<SyntaxElement> = elements
      .iter()
      .filter(|e| e.text_range().start >= fields.text_range().end && e.kind() != K::Comma)
      .cloned()
      .collect();
    let code: Vec<&SyntaxElement> = after.iter().filter(|e| !e.kind().is_trivia()).collect();
    let discriminant_only = match code.as_slice() {
      [] => true,
      [eq, SyntaxElement::Node(_)] => eq.kind() == K::Eq,
      _ => false,
    };
    if !discriminant_only
      || after
        .iter()
        .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
    {
      return None;
    }
    let discriminant = match after.iter().find(|e| !e.kind().is_trivia()) {
      Some(_) => format!(" {}", flat_tokens(&code_tokens(&after), spacing)),
      None => String::new(),
    };
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
    let text = format!("{}{}", text, discriminant);
    // 1 = `,`
    let last = match text.rsplit_once('\n') {
      Some((_, last)) => str_width(last),
      None => indent + str_width(&text),
    };
    if last + 1 > config.max_width {
      return None;
    }
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
/// position), with its `;`. `None` for one holding a comment but among its arguments, or
/// that does not fit.
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
  // But for those among its arguments, which they are laid out with.
  let commented = node.descendants_with_tokens_iter().any(|e| match e {
    SyntaxElement::Token(t) => {
      let range = t.text_range();
      range.start as usize >= start
        && (range.start as usize) < end
        && t.kind().is_trivia()
        && t.kind() != K::Whitespace
        && !super::stmt::safe(&SyntaxElement::Token(t))
    }
    SyntaxElement::Node(_) => false,
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

/// `prev` and `next` with the comments between them, as the standard formatter's
/// `combine_strs_with_missing_comments` puts them: the comments (each on a line at `inner`,
/// joined) after `prev` on its line when they were written there (`same_line`) and fit in
/// `width`, else on the next line; `next` after them on their line when it all fits on one
/// line and they are no line comment, else on the next line.
pub(super) fn combine_with_comments(
  prev: &str,
  next: &str,
  comments: &[String],
  same_line: bool,
  inner: usize,
  width: usize,
) -> String {
  let pad = format!("\n{}", " ".repeat(inner));
  let missing = comments.join(&pad);
  let prev_width = super::expr::last_line_width(prev);
  let first_sep = if same_line && prev_width + super::expr::first_line_width(&missing) < width {
    " ".to_string()
  } else {
    pad.clone()
  };
  let one_line = !prev.contains('\n')
    && !next.contains('\n')
    && !missing.contains('\n')
    && prev_width + super::expr::first_line_width(next) + missing.len() + 2 <= width;
  let second_sep = if !missing.starts_with("//") && same_line && one_line {
    " ".to_string()
  } else {
    pad
  };
  format!("{}{}{}{}{}", prev, first_sep, missing, second_sep, next)
}

/// [`rewrite_header`] before its tokens are checked.
/// A type alias's comments on one line each between its name (generics, bounds) and its
/// `=`, children of it; none for one with a `where` before the `=`.
fn alias_comments(node: &SyntaxNode) -> Vec<SyntaxToken> {
  if node.kind() != K::TypeAlias || node.children_iter().any(|c| c.kind() == K::WhereClause) {
    return Vec::new();
  }
  let Some(name) = node.children_iter().find(|c| c.kind() == K::Name) else {
    return Vec::new();
  };
  let mut comments = Vec::new();
  for element in node.children_with_tokens_iter() {
    match element {
      SyntaxElement::Token(t) if t.kind() == K::Eq => return comments,
      SyntaxElement::Token(t)
        if matches!(t.kind(), K::LineComment | K::BlockComment)
          && t.text_range().start > name.text_range().start
          && !t.text().contains('\n') =>
      {
        comments.push(t)
      }
      _ => {}
    }
  }
  Vec::new()
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

/// A struct whose fields' braces or parentheses hold only comments, as the standard
/// formatter's `format_empty_struct_or_tuple` writes it: one block comment on one line
/// between them, touching (`{/* c */}`); else the comments on lines one level in, joined,
/// a block comment across lines keeping its layout, and the closer on a line of its own.
fn comments_only_struct(
  node: &SyntaxNode,
  start: usize,
  indent: usize,
  config: &FormatConfig,
) -> Option<(String, usize)> {
  if node.kind() != K::Struct {
    return None;
  }
  let list = node
    .children_iter()
    .find(|c| matches!(c.kind(), K::RecordFieldList | K::TupleFieldList))?;
  let inside: Vec<SyntaxElement> = list.children_with_tokens_iter().collect();
  let (open, close) = (inside.first()?, inside.last()?);
  let comments: Vec<SyntaxToken> = inside[1..inside.len() - 1]
    .iter()
    .map(|e| match e {
      SyntaxElement::Token(t) if t.kind().is_trivia() => Some(t.clone()),
      _ => None,
    })
    .collect::<Option<Vec<_>>>()?
    .into_iter()
    .filter(|t| t.kind() != K::Whitespace)
    .collect();
  if comments.is_empty() || comments.iter().any(|t| is_doc(t.kind())) {
    return None;
  }
  // The head before the list, laid out as an item's without a body.
  let head_tokens: Vec<SyntaxToken> = node
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t)
        if (t.text_range().start as usize) >= start
          && t.text_range().end <= list.text_range().start =>
      {
        Some(t)
      }
      _ => None,
    })
    .collect();
  if head_tokens
    .iter()
    .any(|t| t.kind().is_trivia() && t.kind() != K::Whitespace)
    || node
      .children_iter()
      .any(|c| matches!(c.kind(), K::GenericParamList | K::WhereClause))
  {
    return None;
  }
  let spacing = Spacing::of(config);
  let head = flat_tokens(
    &head_tokens
      .into_iter()
      .filter(|t| !t.kind().is_trivia())
      .collect::<Vec<_>>(),
    spacing,
  );
  let inner = indent + config.tab_spaces;
  let pad = format!("\n{}", " ".repeat(inner));
  let texts: Vec<String> = comments
    .iter()
    .map(|t| {
      let text = t.text().trim_end();
      if text.contains('\n') {
        super::file::reindent_block_comment(text, inner, config.tab_spaces, config, false)
      } else {
        text.to_string()
      }
    })
    .collect();
  let joined = texts.join(&pad);
  let tuple = list.kind() == K::TupleFieldList;
  let separator = if tuple { "" } else { " " };
  let after = if tuple { ";" } else { "" };
  let text_of = |e: &SyntaxElement| match e {
    SyntaxElement::Token(t) => t.text().to_string(),
    SyntaxElement::Node(n) => n.text().to_string(),
  };
  let (open, close) = (text_of(open), text_of(close));
  let single = !joined.contains('\n') && !joined.starts_with("//");
  let text = if single {
    format!("{}{}{}{}{}{}", head, separator, open, joined, close, after)
  } else {
    format!(
      "{}{}{}{}{}\n{}{}{}",
      head,
      separator,
      open,
      pad,
      joined,
      " ".repeat(indent),
      close,
      after
    )
  };
  let end = if tuple {
    super::imports::code_end(node)
  } else {
    list.text_range().end as usize
  };
  (indent + str_width(text.lines().next()?) <= config.max_width).then_some((text, end))
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
  // Comments between `=` and the value are placed as `rewrite_assign_rhs_with_comments`
  // places them (below); others would be lost.
  let between = |at: u32| eq.end <= at && at < inside.start;
  let commented = node.token_texts().any(|(kind, text, range)| {
    (kind.is_trivia() && kind != K::Whitespace)
      && range.start as usize >= start
      && (range.start as usize) < end
      && !(inside.start <= range.start && range.start < inside.end)
      && !(between(range.start) && !text.contains('\n'))
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
  let comments: Vec<(SyntaxKind, String, u32)> = node
    .token_texts()
    .filter(|(kind, _, range)| {
      matches!(kind, K::LineComment | K::BlockComment) && between(range.start)
    })
    .map(|(kind, text, range)| (kind, text.trim_end().to_string(), range.start))
    .collect();
  let text = if comments.is_empty() {
    // 1 = `;`
    let shape = super::stmt::statement_shape(indent, config)?.sub_width(1)?;
    format!("{};", super::expr::assign_value(&head, &value, shape)?)
  } else {
    // The value one level in; the comments joined, each on a line there. They follow `=`
    // on its line when they were written there and fit; the value follows them on that
    // line when it all fits on one line and they are no line comment.
    let inner = indent + config.tab_spaces;
    let shape = super::stmt::statement_shape(inner, config)?.sub_width(1)?;
    let value_text = super::expr::rewrite(&value, shape)?;
    let pad = format!("\n{}", " ".repeat(inner));
    let missing = comments
      .iter()
      .map(|(_, text, _)| text.as_str())
      .collect::<Vec<_>>()
      .join(&pad);
    let written = node.text().to_string();
    let from = node.text_range().start;
    let first_comment = comments[0].2;
    let prefer_same_line =
      !written[(eq.end - from) as usize..(first_comment - from) as usize].contains('\n');
    let head_width = super::expr::last_line_width(&head);
    let first_sep =
      if prefer_same_line && head_width + super::expr::first_line_width(&missing) < shape.width {
        " ".to_string()
      } else {
        pad.clone()
      };
    let one_line = !head.contains('\n')
      && !value_text.contains('\n')
      && !missing.contains('\n')
      && head_width + super::expr::first_line_width(&value_text) + missing.len() + 2 <= shape.width;
    let second_sep = if !missing.starts_with("//") && prefer_same_line && one_line {
      " ".to_string()
    } else {
      pad.clone()
    };
    format!(
      "{}{}{}{}{};",
      head, first_sep, missing, second_sep, value_text
    )
  };
  super::stmt::same_tokens_until(node, &text, end).then_some(text)
}

/// Whether a node is the last of its list.
fn is_last(node: &SyntaxNode) -> bool {
  node
    .parent()
    .and_then(|list| list.children_iter().next_back())
    .is_some_and(|last| &last == node)
}

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
  if let Some(laid) = comments_only_struct(node, start, indent, config) {
    return Some(laid);
  }
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
  // parameters, placed by the list layout; or alone in a function type's `()`
  // (`Fn(/* c */)`).
  let parameter_comment = |t: &SyntaxToken| {
    matches!(t.kind(), K::LineComment | K::BlockComment)
      && !t.text().contains('\n')
      && (matches!(
        t.parent().kind(),
        K::ParamList | K::TupleFieldList | K::GenericParamList
      ) || super::types::alone_in_function_type(t)
        || super::types::in_reference(t)
        || super::types::between_bounds(t))
  };
  // A block comment on one line right before a `where` clause, and one right after its
  // last predicate: on a line of their own before `where`, and after that predicate's
  // comma (`missed_spans`).
  let clause_node = header.iter().find_map(|e| match e {
    SyntaxElement::Node(n) if n.kind() == K::WhereClause => Some(n.clone()),
    _ => None,
  });
  let lone_block = |e: &Option<SyntaxElement>| match e {
    Some(SyntaxElement::Token(t)) if t.kind() == K::BlockComment && !t.text().contains('\n') => {
      Some(t.clone())
    }
    _ => None,
  };
  let skip_space = |mut e: Option<SyntaxElement>, back: bool| {
    while let Some(SyntaxElement::Token(t)) = &e {
      if t.kind() != K::Whitespace {
        break;
      }
      e = if back {
        t.prev_sibling_or_token()
      } else {
        t.next_sibling_or_token()
      };
    }
    e
  };
  let before_where = clause_node
    .as_ref()
    .and_then(|c| lone_block(&skip_space(c.prev_sibling_or_token(), true)));
  // After the last predicate a line comment too, which ends that line.
  let lone_after = |e: &Option<SyntaxElement>| {
    lone_block(e).or_else(|| match e {
      Some(SyntaxElement::Token(t)) if t.kind() == K::LineComment => Some(t.clone()),
      _ => None,
    })
  };
  let after_where = clause_node.as_ref().and_then(|c| {
    let last = c.children_iter().rfind(|p| p.kind() == K::WherePred)?;
    // Past its comma.
    let mut after = last.next_sibling_or_token();
    if let Some(SyntaxElement::Token(t)) = &after
      && t.kind() == K::Comma
    {
      after = t.next_sibling_or_token();
    }
    let on_line = |e: &Option<SyntaxElement>| {
      !matches!(e, Some(SyntaxElement::Token(w)) if w.kind() == K::Whitespace && w.text().contains('\n'))
    };
    (on_line(&after).then(|| lone_after(&skip_space(after.clone(), false))).flatten()).or_else(|| {
      // After the clause itself, before the body.
      (after.as_ref().is_none_or(|e| e.kind() == K::Whitespace)
        && skip_space(after.clone(), false).is_none())
      .then(|| {
        let next = c.next_sibling_or_token();
        on_line(&next)
          .then(|| lone_after(&skip_space(next, false)))
          .flatten()
      })
      .flatten()
    })
  });
  let mut where_comments: Vec<SyntaxToken> =
    before_where.iter().chain(&after_where).cloned().collect();
  // A type alias's comments before its `=`, placed with it (`rewrite_ty`).
  where_comments.extend(alias_comments(node));
  let commented = |n: &SyntaxNode| {
    n.descendants_with_tokens_iter().any(|e| match e {
      SyntaxElement::Token(t) => {
        (t.kind().is_trivia()
          && t.kind() != K::Whitespace
          && !parameter_comment(&t)
          && !where_comments.contains(&t))
          // A parameter's doc comment, laid out with it.
          || is_doc(t.kind()) && t.parent().kind() != K::Param
      }
      SyntaxElement::Node(_) => false,
    })
  };
  let unwanted = |e: &SyntaxElement| match e {
    SyntaxElement::Token(t) if where_comments.contains(t) => false,
    SyntaxElement::Token(t) => {
      (t.kind().is_trivia() && t.kind() != K::Whitespace) || is_doc(t.kind())
    }
    SyntaxElement::Node(n) if matches!(n.kind(), K::ParamList | K::TupleFieldList) => {
      commented(n)
        || n
          .descendant_nodes_where(|kind| matches!(kind, K::BlockExpr | K::MatchArmList))
          .any(|b| !b.parent().is_some_and(|p| p.kind() == K::ConstArg))
        || n.subtree_has(|kind| matches!(kind, K::MacroCall | K::MacroExpr)) && !macros_in_types(n)
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
          && !macros_in_types(n)
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
    let mut block = match &before_where {
      Some(comment) => format!(
        "\n{}{}\n{}where",
        " ".repeat(indent),
        comment.text(),
        " ".repeat(indent)
      ),
      None => format!("\n{}where", " ".repeat(indent)),
    };
    for (i, predicate) in predicates.iter().enumerate() {
      let last = i + 1 == predicates.len();
      let comma = if last && tail_text == ";" { "" } else { "," };
      if blank_before.contains(&i) && config.style_edition < 2024 {
        block.push('\n');
      }
      block.push_str(&format!("\n{}{}{}", pad, predicate, comma));
      if last && let Some(comment) = &after_where {
        if tail_text == ";" {
          block.push(';');
        }
        block.push(' ');
        block.push_str(comment.text());
      }
    }
    // A `;` the comment after the last predicate took.
    let tail_text = if after_where.is_some() && tail_text == ";" {
      String::new()
    } else {
      tail_text
    };
    // A type alias's second `where`, after its value (`= E where F: G;`): the value on a
    // line of its own, then that clause on lines too.
    let after = node
      .children_iter()
      .filter(|c| c.kind() == K::WhereClause)
      .nth(1)
      .filter(|_| node.kind() == K::TypeAlias);
    if let Some(after) = &after {
      let value: Vec<SyntaxToken> = tail
        .iter()
        .filter(|t| t.text_range().start < after.text_range().start)
        .cloned()
        .collect();
      let value = flat_tokens(&value, spacing);
      let predicates: Vec<String> = after
        .children_iter()
        .filter(|c| c.kind() == K::WherePred)
        .map(|p| super::types::rewrite_where_predicate(&p, clause_shape))
        .collect::<Option<_>>()?;
      if predicates.is_empty() || indent + str_width(&value) > config.max_width {
        return None;
      }
      let head = flat_tokens(&tokens, spacing);
      let mut text = format!("{}{}\n{}{}", head, block, " ".repeat(indent), value);
      text.push_str(&format!("\n{}where", " ".repeat(indent)));
      for (i, predicate) in predicates.iter().enumerate() {
        let comma = if i + 1 == predicates.len() { ";" } else { "," };
        text.push_str(&format!("\n{}{}{}", pad, predicate, comma));
      }
      return (indent + str_width(&head) <= config.max_width).then_some((text, end));
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
    return rewrite_trait_alias(node, &tokens, indent, where_block.as_deref(), config)
      .map(|text| (text, end));
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
    // One parameter is laid out so only with a line comment before it and none after it
    // (one after it the standard formatter writes oddly: left as written).
    let line_comment =
      |c: &Option<String>| c.as_ref().is_some_and(|c| c.trim_start().starts_with("//"));
    let lone = items.len() == 1 && line_comment(&comments[0].pre) && comments[0].post.is_none();
    if items.len() < 2 && !lone || items.iter().any(|p| p.ends_with("...")) {
      return None;
    }
    // A blank line written after a parameter is kept (`preserve_newline`).
    let body_text = super::lists::write_vertical_kept(
      &items,
      comments,
      inner,
      config.max_width.saturating_sub(inner),
      config.max_width,
      true,
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

/// Whether every macro call in `node` stands as a type, or alone in a const generic
/// argument's block (`S<{ m!(..) }>`), which the type layout lays out.
fn macros_in_types(node: &SyntaxNode) -> bool {
  node
    .descendant_nodes_where(|kind| kind == K::MacroCall)
    .all(|call| {
      call.parent().is_some_and(|p| {
        p.kind() == K::MacroType
          || p.kind() == K::MacroExpr
            && p.parent().is_some_and(|b| {
              b.kind() == K::BlockExpr && b.parent().is_some_and(|a| a.kind() == K::ConstArg)
            })
      })
    })
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

/// Where a type alias's code before its first comment ends (its name, generics or bounds).
fn name_end(node: &SyntaxNode) -> Option<u32> {
  let first = alias_comments(node).first()?.text_range().start;
  node
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() && t.text_range().end <= first => {
        Some(t.text_range().end)
      }
      _ => None,
    })
    .last()
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
/// right-hand side, with a `where` clause (`where_block`, laid out on lines) after them:
/// on their line when both fit there on one line (`TraitAliasBounds`), else below them.
fn rewrite_trait_alias(
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
  let bounds = node
    .children_iter()
    .find(|c| c.kind() == K::TypeBoundList)
    .filter(|b| b.children_iter().next().is_some());
  let clause = node.children_iter().find(|c| c.kind() == K::WhereClause);
  if bounds.is_none() && where_block.is_none() {
    return None;
  }
  let eq = tokens.iter().find(|t| t.kind() == K::Eq)?;
  let rhs_start = bounds
    .as_ref()
    .map_or(eq.text_range().end, |b| b.text_range().end);
  // Nothing but the parts: the name, its generics, `=`, the bounds and the `;`.
  let at = name.text_range().start;
  let generics_end = generics
    .as_ref()
    .map_or(name.text_range().end, |g| g.text_range().end);
  if tokens.iter().any(|t| {
    let start = t.text_range().start;
    start >= generics_end && start < eq.text_range().start
      || start >= rhs_start && t.kind() != K::Semi
  }) || where_block.is_none() && where_clause_of(node)
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
  // The clause on one line: ` where` and its predicates.
  let one_line_where = clause.as_ref().and_then(|clause| {
    let wide = shape.block_indent(0);
    let predicates: Vec<String> = clause
      .children_iter()
      .filter(|c| c.kind() == K::WherePred)
      .map(|p| super::types::rewrite_where_predicate(&p, wide))
      .collect::<Option<_>>()?;
    let text = predicates.join(", ");
    (!text.contains('\n')).then(|| format!(" where {}", text))
  });
  // 1 = `;`
  let rhs = |s: super::shape::Shape| -> Option<String> {
    let bounds_text = match &bounds {
      Some(bounds) => super::types::join_bounds(bounds, s, true)?,
      None => String::new(),
    };
    let Some(block) = where_block else {
      return Some(bounds_text);
    };
    match &one_line_where {
      // No bounds: the clause alone, as a right-hand side.
      Some(one) if bounds_text.is_empty() && str_width(one.trim_start()) <= s.width => {
        Some(one.trim_start().to_string())
      }
      Some(one)
        if !bounds_text.is_empty()
          && !bounds_text.contains('\n')
          && str_width(&bounds_text) + str_width(one) < s.width =>
      {
        Some(format!("{}{}", bounds_text, one))
      }
      // No bounds: `where` follows `=`, the predicates below it.
      _ if bounds_text.is_empty() => None,
      _ => Some(format!("{}{}", bounds_text, block.trim_end_matches(';'))),
    }
  };
  match super::types::assign_rhs(&head, shape.sub_width(1)?, rhs) {
    Some(text) => Some(format!("{};", text)),
    None if bounds.is_none() => where_block.map(|block| format!("{} {}", head, block.trim_start())),
    None => None,
  }
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
  let comments = alias_comments(node);
  let head = if comments.is_empty() {
    format!("{} =", result)
  } else {
    // Written on the name's line: no line break before the first comment.
    let first = comments[0].text_range().start;
    let same_line = !node.text().to_string()[(name_end(node)? - node.text_range().start) as usize
      ..(first - node.text_range().start) as usize]
      .contains('\n');
    let texts: Vec<String> = comments
      .iter()
      .map(|c| c.text().trim_end().to_string())
      .collect();
    let inner = indent + config.tab_spaces;
    combine_with_comments(
      &result,
      "=",
      &texts,
      same_line,
      inner,
      config.max_width - inner,
    )
  };
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
    let all: Vec<(SyntaxKind, &str)> =
      crate::syntax::lexer::tokenize(text, crate::syntax::Edition::E2021)
        .into_iter()
        .map(|token| {
          let piece = &text[at..at + token.len as usize];
          at += token.len as usize;
          (token.kind, piece)
        })
        .collect();
    // `vec!(..)` and `vec! {..}` are written `vec![..]`: their delimiters read as brackets.
    let mut bracket = vec![false; all.len()];
    let code: Vec<usize> = (0..all.len()).filter(|&i| !all[i].0.is_trivia()).collect();
    for w in 0..code.len().saturating_sub(2) {
      let (name, bang, open) = (code[w], code[w + 1], code[w + 2]);
      if all[name].1 != "vec" || all[bang].0 != K::Bang {
        continue;
      }
      let (open_kind, close_kind) = match all[open].0 {
        K::LParen => (K::LParen, K::RParen),
        K::LBrace => (K::LBrace, K::RBrace),
        _ => continue,
      };
      let mut depth = 0;
      for &i in &code[w + 2..] {
        if all[i].0 == open_kind {
          depth += 1;
        } else if all[i].0 == close_kind {
          depth -= 1;
          if depth == 0 {
            bracket[open] = true;
            bracket[i] = true;
            break;
          }
        }
      }
    }
    all
      .iter()
      .enumerate()
      .filter_map(|(i, &(kind, piece))| {
        if bracket[i] {
          return Some(
            if matches!(kind, K::LParen | K::LBrace) {
              "["
            } else {
              "]"
            }
            .to_string(),
          );
        }
        // A block comment across lines may be re-indented: its lines compared trimmed.
        if kind == K::BlockComment {
          return Some(piece.lines().map(str::trim).collect::<Vec<_>>().join("\n"));
        }
        (kind != K::Whitespace && super::stmt::compared(kind) || kind == K::LineComment)
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
