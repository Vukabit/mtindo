//! Item headers laid out as the Rust style lays them out: everything before an item's body
//! (`pub fn name<T>(a: T) -> R`, `impl<T> Trait for Type`, `extern "C"`, `pub struct Name`)
//! on one line with the body's `{`, or a whole item ending in `;` (a type alias, const or
//! static too) on one line, when it fits;
//! else a declaration's return type on the next line when it is just one column over, else a
//! function's parameters one per line, each with a comma, the body's `{` on a line of its own
//! when it does not fit after them, with any line comments among them placed as a list's.
//! A trait `impl` too wide for its line puts `for Type` on the next line. A tuple struct's
//! fields go one per line when they do not fit. A `where` clause follows on lines of its
//! own, a predicate per line (an `impl` with one predicate and no items stays on one line
//! when it fits). The body itself is left to the whole-file rules. The widths were found by
//! measuring the standard formatter.
//!
//! Left as written: a header holding a comment (but line comments among a function's
//! parameters), a macro, a parameter with more than one attribute, a tuple or pattern wider
//! than its width, trait bounds or predicates that would break, generics that would break,
//! a type alias's, const's or static's `where`, and a header that does not fit even so.

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
      .is_some_and(|owner| matches!(owner.kind(), K::Struct | K::Union)),
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
  let start = header_start(node)?;
  let elements: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(|e| e.text_range().start as usize >= start)
    .collect();
  let unwanted = |e: &SyntaxElement| match e {
    SyntaxElement::Token(t) => {
      (t.kind().is_trivia() && t.kind() != K::Whitespace) || is_doc(t.kind())
    }
    SyntaxElement::Node(n) => n.subtree_has(|kind| {
      (kind.is_trivia() && kind != K::Whitespace)
        || is_doc(kind)
        || matches!(
          kind,
          K::Attr | K::MacroCall | K::MacroType | K::MacroExpr | K::BlockExpr
        )
    }),
  };
  if elements.iter().any(unwanted) {
    return None;
  }
  let spacing = Spacing::of(config);
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
    let mut lines = String::new();
    for field in fields.children_iter() {
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
        matches!(p.kind(), K::Param | K::TupleField)
          && p.children_iter().filter(|c| c.kind() == K::Attr).count() == 1
      }) && !a
        .token_texts()
        .any(|(kind, text, _)| kind.is_literal() && text.contains('\n'))
        // One the standard formatter leaves as written keeps its lines.
        && (super::file::meta_like(a) || !a.text().to_string().contains('\n'))
    })
  };
  // A comment among a function's parameters, placed by the list layout.
  let parameter_comment = |t: &SyntaxToken| {
    matches!(t.kind(), K::LineComment | K::BlockComment)
      && !t.text().contains('\n')
      && t.parent().kind() == K::ParamList
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
    SyntaxElement::Node(n) if n.kind() == K::ParamList => {
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
        || n.subtree_has(|kind| matches!(kind, K::BlockExpr | K::MatchArmList))
        || n.subtree_has(|kind| {
          (kind.is_trivia() && kind != K::Whitespace)
            || is_doc(kind)
            || matches!(kind, K::MacroCall | K::MacroType | K::MacroExpr)
        })
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
  // A type alias's `where` (an associated type's, before or after its value), a const's and
  // a static's have layouts of their own: left as written.
  if where_clause.is_some() && matches!(node.kind(), K::TypeAlias | K::Const | K::Static) {
    return None;
  }
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
    let predicates: Vec<String> = clause
      .children_iter()
      .filter(|c| c.kind() == K::WherePred)
      .map(|p| flat_tokens(&code_tokens(&[SyntaxElement::Node(p)]), spacing))
      .collect();
    let inner = indent + config.tab_spaces;
    if predicates.is_empty()
      || predicates
        .iter()
        .any(|p| inner + str_width(p) + 1 > config.max_width)
    {
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
      _ => block.push_str(&format!("\n{}{}", " ".repeat(indent), tail_text)),
    }
    if body.is_some() {
      block.push_str(&format!("\n{}", " ".repeat(indent)));
    }
    where_block = Some(block);
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
  // A tuple struct's fields as a call's arguments (`rewrite_with_parens`): on the line
  // within `fn_call_width`, else one per line.
  if node.kind() == K::Struct
    && where_block.is_none()
    && let Some(fields) = node.children_iter().find(|c| c.kind() == K::TupleFieldList)
  {
    let texts: Vec<String> = fields
      .children_iter()
      .filter(|c| c.kind() == K::TupleField)
      .map(|f| flat_tokens(&code_tokens(&[SyntaxElement::Node(f)]), spacing))
      .collect();
    let from = fields.text_range().start;
    let before: Vec<SyntaxToken> = tokens
      .iter()
      .filter(|t| t.text_range().start < from)
      .cloned()
      .collect();
    let lead = flat_tokens(&before, spacing);
    let joined = texts.join(", ");
    // 3 = `(`, `)` and `;`
    // One field only needs to fit the line.
    if (texts.len() == 1 || str_width(&joined) <= config.fn_call_width)
      && indent + str_width(&lead) + str_width(&joined) + 3 <= config.max_width
    {
      return Some((format!("{}({});", lead, joined), end));
    }
    let inner = indent + config.tab_spaces;
    if texts.is_empty()
      || indent + str_width(&lead) + 1 > config.max_width
      || texts
        .iter()
        .any(|t| inner + str_width(t) + 1 > config.max_width)
    {
      return None;
    }
    let pad = " ".repeat(inner);
    let lines: Vec<String> = texts.iter().map(|t| format!("{}{},", pad, t)).collect();
    return Some((
      format!("{}(\n{}\n{});", lead, lines.join("\n"), " ".repeat(indent)),
      end,
    ));
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
  // A trait's bounds have a layout of their own.
  if node.kind() == K::Trait
    && let Some(bounds) = node.children_iter().find(|c| c.kind() == K::TypeBoundList)
  {
    body.as_ref()?;
    return trait_bounds(&tokens, &bounds, indent, where_block.as_deref(), config)
      .map(|text| (text, end));
  }
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
      return None;
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
  // A trait `impl` too wide for its line: `for Type` on the next one level in, the body's
  // `{` on a line of its own (the standard formatter's `format_impl_ref_and_type`).
  // The standard formatter's budget for an `impl`'s one line leaves the indentation out
  // (`context.budget`).
  if node.kind() == K::Impl && str_width(&one_line) + brace <= config.max_width {
    return Some((format!("{}{}", one_line, suffix), end));
  }
  if node.kind() == K::Impl
    && let Some(for_at) = tokens
      .iter()
      .find(|t| t.kind() == K::ForKw && t.parent().kind() == K::Impl)
      .map(|t| t.text_range().start)
  {
    let (head, rest): (Vec<SyntaxToken>, Vec<SyntaxToken>) = tokens
      .iter()
      .cloned()
      .partition(|t| t.text_range().start < for_at);
    let (head, rest) = (flat_tokens(&head, spacing), flat_tokens(&rest, spacing));
    let inner = indent + config.tab_spaces;
    if indent + str_width(&head) > config.max_width || inner + str_width(&rest) > config.max_width {
      return None;
    }
    let suffix = match (&where_block, body.is_some()) {
      (Some(block), _) => block.clone(),
      (None, true) => format!("\n{}", " ".repeat(indent)),
      (None, false) => String::new(),
    };
    return Some((
      format!("{}\n{}{}{}", head, " ".repeat(inner), rest, suffix),
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

/// A macro call standing as an item or statement, its arguments in parentheses or brackets,
/// laid out as one in an expression is (the standard formatter's `rewrite_macro` in item
/// position), with its `;`. `None` for one holding a comment, or that does not fit.
pub fn rewrite_macro_item(
  node: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<String> {
  if config.hard_tabs || config.tab_spaces == 0 || node.kind() != K::MacroCall {
    return None;
  }
  let start = header_start(node)?;
  let commented = node.token_texts().any(|(kind, _, range)| {
    range.start as usize >= start && kind.is_trivia() && kind != K::Whitespace
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
  super::stmt::same_tokens(node, &text).then_some(text)
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
  let head = flat_tokens(&code_tokens(&head), Spacing::of(config));
  // Types are printed flat: one whose head runs past the line would have to break.
  if indent + str_width(&head) + 2 > config.max_width {
    return None;
  }
  // 1 = `;`
  let shape = super::stmt::statement_shape(indent, config)?.sub_width(1)?;
  let text = format!("{};", super::expr::assign_value(&head, &value, shape)?);
  super::stmt::same_tokens(node, &text).then_some(text)
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

/// `-> R` or `;` after a parameter list, with the space before an arrow.
fn spaced_after(after: &[SyntaxToken], spacing: Spacing) -> String {
  match after.first() {
    None => String::new(),
    Some(t) if t.kind() == K::Semi => flat_tokens(after, spacing),
    Some(_) => format!(" {}", flat_tokens(after, spacing)),
  }
}

/// A trait's header with its bounds, as the standard formatter's `format_trait` lays it out:
/// the bounds after `:` on the header's line when they fit there, else on the next line one
/// level in, together or one per line each after the first led by `+`; the body's `{` after
/// a space, or on a line of its own after a header across lines or one too long for it.
/// A `where` clause follows on lines of its own. The widths are measured as the standard
/// formatter measures them, the text before the trait's name counted twice on the header's
/// line and the indentation twice on the next.
fn trait_bounds(
  tokens: &[SyntaxToken],
  bounds: &SyntaxNode,
  indent: usize,
  where_block: Option<&str>,
  config: &FormatConfig,
) -> Option<String> {
  let spacing = Spacing::of(config);
  let (at, to) = (bounds.text_range().start, bounds.text_range().end);
  let head: Vec<SyntaxToken> = tokens
    .iter()
    .filter(|t| t.text_range().start < at)
    .cloned()
    .collect();
  // Only the `where` clause may follow the bounds.
  if where_block.is_none() && tokens.iter().any(|t| t.text_range().start >= to) {
    return None;
  }
  let head = flat_tokens(&head, spacing);
  let prefix = head.find("trait ")? + "trait ".len();
  let plus = if spacing.compressed_type_punctuation {
    "+"
  } else {
    " + "
  };
  let parts: Vec<String> = bounds
    .children_iter()
    .map(|b| flat_tokens(&code_tokens(&[SyntaxElement::Node(b)]), spacing))
    .collect();
  if parts.is_empty() {
    return None;
  }
  let joined = parts.join(plus);
  let pad = " ".repeat(indent + config.tab_spaces);
  let one_line = indent + prefix + str_width(&head) + 1 + str_width(&joined) <= config.max_width;
  let text = if one_line {
    format!("{} {}", head, joined)
  } else if str_width(&joined) <= config.max_width.saturating_sub(2 * indent) {
    format!("{}\n{}{}", head, pad, joined)
  } else {
    let sep = format!("\n{}+ ", pad);
    format!("{}\n{}{}", head, pad, parts.join(&sep))
  };
  // A `where` clause on lines of its own after them, with the body's `{`.
  if let Some(block) = where_block {
    return Some(format!("{}{}", text, block));
  }
  let last = text.rsplit('\n').next().unwrap_or("");
  let brace_apart = text.contains('\n')
    || config.brace_next_line
    || str_width(last) + 2 > config.max_width.saturating_sub(indent);
  Some(if brace_apart {
    format!("{}\n{}", text, " ".repeat(indent))
  } else {
    format!("{} ", text)
  })
}

#[cfg(test)]
mod tests;
