//! Item headers laid out as the Rust style lays them out: everything before an item's body
//! (`pub fn name<T>(a: T) -> R`, `impl<T> Trait for Type`, `extern "C"`, `pub struct Name`)
//! on one line with the body's `{`, or a whole item ending in `;` on one line, when it fits;
//! else a declaration's return type on the next line when it is just one column over, else a
//! function's parameters one per line, each with a comma, the body's `{` on a line of its own
//! when it does not fit after them. The body itself is left to the whole-file rules. The
//! widths were found by measuring the standard formatter.
//!
//! Left as written: a header holding a comment, a `where` clause, a macro or an attribute
//! (on a parameter), a tuple or pattern wider than its width, trait bounds that would break,
//! generics that would break, and a header that does not fit even so.

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
    .children_with_tokens()
    .into_iter()
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
      .is_some_and(|owner| matches!(owner.kind(), K::Struct | K::Union)),
    _ => false,
  }
}

/// The items whose header this module lays out.
pub fn is_item(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    K::Fn | K::Struct | K::Enum | K::Union | K::Trait | K::Impl | K::ExternBlock | K::Module
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
  let start = header_start(node)?;
  let elements: Vec<SyntaxElement> = node
    .children_with_tokens()
    .into_iter()
    .filter(|e| e.text_range().start as usize >= start)
    .collect();
  let unwanted = |e: &SyntaxElement| match e {
    SyntaxElement::Token(t) => {
      (t.kind().is_trivia() && t.kind() != K::Whitespace) || is_doc(t.kind())
    }
    SyntaxElement::Node(n) => n.descendants_with_tokens().iter().any(|d| {
      (d.kind().is_trivia() && d.kind() != K::Whitespace)
        || is_doc(d.kind())
        || matches!(
          d.kind(),
          K::Attr | K::MacroCall | K::MacroType | K::MacroExpr | K::BlockExpr
        )
    }),
  };
  if elements.iter().any(unwanted) {
    return None;
  }
  let spacing = Spacing::of(config);
  let too_wide = node.descendants().iter().any(|d| {
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
      .and_then(|list| list.children().into_iter().last())
      .is_some_and(|last| &last == node)
    {
      return None;
    }
    ","
  } else {
    ","
  };
  let fields = node
    .children()
    .into_iter()
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
    let mut lines = String::new();
    for field in fields.children() {
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
pub fn rewrite_header(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<(String, usize)> {
  if config.hard_tabs || config.tab_spaces == 0 || !is_item(node.kind()) {
    return None;
  }
  let start = header_start(node)?;
  let body = node.children().into_iter().find(|c| {
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
  let end = match &body {
    Some(body) => body.text_range().start as usize,
    None => node.text_range().end as usize,
  };
  let header: Vec<SyntaxElement> = node
    .children_with_tokens()
    .into_iter()
    .filter(|e| {
      let at = e.text_range().start as usize;
      start <= at && at < end
    })
    .collect();
  let unwanted = |e: &SyntaxElement| match e {
    SyntaxElement::Token(t) => {
      (t.kind().is_trivia() && t.kind() != K::Whitespace) || is_doc(t.kind())
    }
    SyntaxElement::Node(n) => {
      matches!(n.kind(), K::WhereClause | K::MacroCall | K::Attr)
        || n.descendants().iter().any(|d| d.kind() == K::BlockExpr)
        || n.descendants_with_tokens().iter().any(|d| {
          (d.kind().is_trivia() && d.kind() != K::Whitespace)
            || is_doc(d.kind())
            || matches!(
              d.kind(),
              K::Attr | K::MacroCall | K::MacroType | K::MacroExpr
            )
        })
    }
  };
  if header.iter().any(unwanted) {
    return None;
  }
  let spacing = Spacing::of(config);
  let tokens = code_tokens(&header);
  let one_line = flat_tokens(&tokens, spacing);
  // Lists and patterns held to their widths are not laid out here yet: a header holding
  // one too wide for its width is left as written.
  let too_wide = node.descendants().iter().any(|d| {
    let limit = match d.kind() {
      K::TupleType | K::TuplePat | K::TupleStructPat | K::TupleFieldList => config.fn_call_width,
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
  let has_ret = node.children().iter().any(|c| c.kind() == K::RetType);
  let brace = if body.is_some() {
    2
  } else {
    usize::from(has_ret)
  };
  // With `brace_style = "AlwaysNextLine"` the body's `{` starts the next line.
  let suffix = match body {
    Some(_) if config.brace_next_line => format!("\n{}", " ".repeat(indent)),
    Some(_) => " ".to_string(),
    None => String::new(),
  };
  // When the standard formatter breaks a trait's bounds is not known yet: a trait with
  // bounds keeps its header as written.
  if node.kind() == K::Trait && node.children().iter().any(|c| c.kind() == K::TypeBoundList) {
    return None;
  }
  let width = indent + str_width(&one_line) + brace;
  if width <= config.max_width {
    return Some((format!("{}{}", one_line, suffix), end));
  }
  // A declaration exactly that column over: its return type on the next line, one level in.
  if body.is_none()
    && has_ret
    && width == config.max_width + 1
    && let Some(ret) = node.children().into_iter().find(|c| c.kind() == K::RetType)
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
    .children()
    .into_iter()
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
    .children()
    .iter()
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
  let fits = indent + str_width(&open) < config.max_width
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
  let suffix = if body.is_some() && 2 * indent + str_width(&close) + 2 > config.max_width {
    format!("\n{}", " ".repeat(indent))
  } else {
    suffix
  };
  let pad = " ".repeat(inner);
  // A C variadic `...` takes no comma after it.
  let lines: String = items
    .iter()
    .map(|p| {
      let comma = if p.ends_with("...") { "" } else { "," };
      format!("\n{}{}{}", pad, p, comma)
    })
    .collect();
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

/// Whether an enum has both a variant written on several lines and one on a single line.
fn mixed_variants(list: &SyntaxNode, indent: usize, config: &FormatConfig) -> bool {
  let spacing = Spacing::of(config);
  let one_line = |variant: &SyntaxNode| {
    // Attributes, doc comments or comments anywhere in it put it on several lines.
    let attributed = variant.descendants_with_tokens().iter().any(|e| {
      e.kind() == K::Attr
        || is_doc(e.kind())
        || matches!(e.kind(), K::LineComment | K::BlockComment)
    });
    let fields_fit = variant
      .children()
      .iter()
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
    .children()
    .into_iter()
    .filter(|v| v.kind() == K::Variant)
    .collect();
  variants.iter().any(|v| !one_line(v)) && variants.iter().any(one_line)
}

/// `-> R` or `;` after a parameter list, with the space before an arrow.
fn spaced_after(after: &[SyntaxToken], spacing: Spacing) -> String {
  match after.first() {
    None => String::new(),
    Some(t) if t.kind() == K::Semi => flat_tokens(after, spacing),
    Some(_) => format!(" {}", flat_tokens(after, spacing)),
  }
}

#[cfg(test)]
mod tests;
