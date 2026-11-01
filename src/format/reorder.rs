//! Runs of `use` declarations, of `extern crate` items and of `mod name;` declarations
//! sorted as the standard formatter sorts them (`reorder_imports`, `reorder_modules`),
//! before the file is formatted.
//!
//! A run is the declarations of one kind that follow each other in a file, module or block
//! with no blank line between them; a comment does not end a run. A declaration marked
//! `#[macro_use]` or skipped ends a run and stays where it is. Imports sort by their use
//! trees ([`crate::order::UseTree`]), visibility and attributes aside; `extern crate`
//! items by crate name, then alias; modules by name. A run holding a comment is left in its order, since the standard formatter moves
//! such comments in ways that depend on more than the order, but for comments after the
//! last declaration's code, which stay after the run; so is a run in a `// mto: off`
//! region. The meaning check compares each run of imports as a set (`use-trees`) and
//! modules by their names (`import-and-module-order`).

use SyntaxKind as K;

use crate::options::format::FormatConfig;
use crate::order::{UseTree, version_sort};
use crate::syntax::ast::Attr;
use crate::syntax::marker;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode};

/// The nodes the passes here look at, found in one walk of the tree when a pass first asks
/// for them: most files no pass changes, and a walk of its own per pass was most of what
/// the passes cost. A pass that writes new text needs a new one for the tree of that text.
#[derive(Default)]
pub struct PassNodes(Option<Vec<SyntaxNode>>);

impl PassNodes {
  /// The nodes of `root` (itself among them) whose kind `wanted` takes, in source order:
  /// only of the kinds a pass here walks for.
  fn of<'a>(
    &'a mut self,
    root: &SyntaxNode,
    wanted: impl Fn(SyntaxKind) -> bool + 'a,
  ) -> impl Iterator<Item = &'a SyntaxNode> {
    self
      .0
      .get_or_insert_with(|| {
        root
          .descendant_nodes_where(|kind| {
            matches!(
              kind,
              K::Use
                | K::LoopExpr
                | K::WhileExpr
                | K::ForExpr
                | K::IfExpr
                | K::MatchArmList
                | K::BlockExpr
                | K::SourceFile
                | K::ItemList
                | K::RecordFieldList
                | K::TupleFieldList
                | K::VariantList
            )
          })
          .collect()
      })
      .iter()
      .filter(move |node| wanted(node.kind()))
  }
}

/// `text`, laid out, with each enum's variants given a discriminant lined up: the name of
/// each padded to the widest of those names at most `enum_discrim_align_threshold` wide,
/// so their `=` line up (the standard formatter's `format_variant_list`); `None` when
/// nothing moves. Only a variant written `Name = value` on one line, within the line width.
/// Not in a `// mto: off` region or a skipped item.
pub fn aligned_discriminants(
  root: &SyntaxNode,
  text: &str,
  config: &FormatConfig,
) -> Option<String> {
  let off = off_regions(root);
  let mut inserts: Vec<(usize, usize)> = Vec::new();
  for list in root.descendant_nodes_where(|kind| kind == K::VariantList) {
    let range = list.text_range();
    if skipped_within(&list)
      || off
        .iter()
        .any(|&(s, e)| s < range.end as usize && (range.start as usize) < e)
    {
      continue;
    }
    // Each variant with a discriminant: its name's end and width, and the line's width.
    let named: Vec<(usize, usize, usize, bool)> = list
      .children_iter()
      .filter(|v| {
        v.kind() == K::Variant && v.children_with_tokens_iter().any(|e| e.kind() == K::Eq)
      })
      .filter_map(|variant| {
        let name = variant.children_iter().find(|c| c.kind() == K::Name)?;
        let end = name.text_range().end as usize;
        let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i);
        let line_start = text[..end].rfind('\n').map_or(0, |i| i + 1);
        // A unit variant: `=` right after its name.
        let unit = text[end..].trim_start_matches(' ').starts_with('=');
        Some((
          end,
          name.text().chars().count(),
          line_end - line_start,
          unit,
        ))
      })
      .collect();
    let pad_to = named
      .iter()
      .map(|&(_, width, _, _)| width)
      .filter(|&w| w <= config.enum_discrim_align_threshold)
      .max()
      .unwrap_or(0);
    for &(end, width, line, unit) in &named {
      if unit && width < pad_to && line + pad_to - width <= config.max_width {
        inserts.push((end, pad_to - width));
      }
    }
  }
  if inserts.is_empty() {
    return None;
  }
  inserts.sort_unstable();
  let mut out = String::with_capacity(text.len() + inserts.len() * 8);
  let mut from = 0;
  for (at, pad) in inserts {
    out.push_str(&text[from..at]);
    out.push_str(&" ".repeat(pad));
    from = at;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text`, laid out, with the fields of each struct, variant and struct literal laid out a
/// line each lined up: in each group of them (fields between blank lines, a skipped one
/// ending its group) whose widths before their types differ by at most
/// `struct_field_align_threshold`, each type starts one space after the widest (`vis name:`
/// for a field, the name for a literal's, whose value follows `: `), as the standard
/// formatter's `rewrite_with_alignment` writes them; a literal only without a `..base` or
/// a field written by its name alone. `None` when nothing moves. A field
/// whose type starts on a later line, or that would pass the line width, is left as it is.
/// Not in a `// mto: off` region or a skipped item.
pub fn aligned_fields(
  root: &SyntaxNode,
  text: &str,
  config: &FormatConfig,
  edition: Edition,
) -> Option<String> {
  let off = off_regions(root);
  let mut inserts: Vec<(usize, usize)> = Vec::new();
  let mut commas: Vec<usize> = Vec::new();
  field_edits(
    root,
    text,
    0,
    config,
    edition,
    &off,
    &mut inserts,
    &mut commas,
  );
  if inserts.is_empty() && commas.is_empty() {
    return None;
  }
  // A comma is a pad of none written before its spaces.
  let mut edits: Vec<(usize, Option<usize>)> = inserts
    .into_iter()
    .map(|(at, pad)| (at, Some(pad)))
    .chain(commas.into_iter().map(|at| (at, None)))
    .collect();
  edits.sort_unstable();
  let mut out = String::with_capacity(text.len() + edits.len() * 4);
  let mut from = 0;
  for (at, pad) in edits {
    out.push_str(&text[from..at]);
    match pad {
      Some(pad) => out.push_str(&" ".repeat(pad)),
      None => out.push(','),
    }
    from = at;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text` with the last two or more `_` of each tuple pattern (`(a, _, _)`, `S(a, _, _)`)
/// written `..` (the standard formatter's `count_wildcard_suffix_len`); `None` when there is
/// none; a wildcard with a comment around it is the first of them. Not a pattern that has a
/// `..` already (the standard formatter writes a second one), nor with a comment among those
/// wildcards, nor in a `// mto: off` region or a skipped item.
pub fn condensed_wildcards(root: &SyntaxNode, text: &str) -> Option<String> {
  if !text.contains('_') {
    return None;
  }
  let off = off_regions(root);
  let mut changes: Vec<(usize, usize)> = Vec::new();
  for pattern in root.descendant_nodes_where(|kind| matches!(kind, K::TuplePat | K::TupleStructPat))
  {
    let fields: Vec<SyntaxNode> = pattern
      .children_iter()
      .filter(|c| c.kind() != K::Path)
      .collect();
    if fields.iter().any(|f| f.kind() == K::RestPat) {
      continue;
    }
    // Each field's span with the comments around it, to the separators on either side.
    let separators: Vec<u32> = pattern
      .children_with_tokens_iter()
      .filter(|e| matches!(e.kind(), K::Comma | K::LParen | K::RParen))
      .map(|e| e.text_range().start)
      .collect();
    let comments: Vec<u32> = pattern
      .token_texts()
      .filter(|(kind, _, _)| matches!(kind, K::LineComment | K::BlockComment))
      .map(|(_, _, range)| range.start)
      .collect();
    let has_comment = |field: &SyntaxNode| {
      let range = field.text_range();
      let before = separators
        .iter()
        .rev()
        .find(|&&s| s < range.start)
        .copied()
        .unwrap_or(0);
      let after = separators
        .iter()
        .find(|&&s| s >= range.end)
        .copied()
        .unwrap_or(u32::MAX);
      comments.iter().any(|&c| before < c && c < after)
    };
    // The wildcards ending the list; one with a comment is the last taken.
    let mut run = 0;
    for field in fields.iter().rev() {
      if field.kind() != K::WildcardPat {
        break;
      }
      run += 1;
      if has_comment(field) {
        break;
      }
    }
    if run < 2 {
      continue;
    }
    let first = &fields[fields.len() - run];
    let last = &fields[fields.len() - 1];
    let (start, end) = (
      first.text_range().start as usize,
      last.text_range().end as usize,
    );
    // A comment the wildcards' `..` would take the place of.
    let commented = comments
      .iter()
      .any(|&c| start <= c as usize && (c as usize) < end);
    if commented || off.iter().any(|&(s, e)| s < end && start < e) || skipped_within(&pattern) {
      continue;
    }
    changes.push((start, end));
  }
  if changes.is_empty() {
    return None;
  }
  // A pattern inside another's wildcards is not one: the ranges never overlap.
  changes.sort_unstable();
  let mut out = String::with_capacity(text.len());
  let mut from = 0;
  for (start, end) in changes {
    out.push_str(&text[from..start]);
    out.push_str("..");
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text`, laid out, with each control expression's `{` that ends a line put on the next, at
/// its line's indentation, and `else` after a `}` on a line of its own; with
/// `ClosingNextLine` only the `else`, a `let`'s too. An empty block's `{}` moves as well, but
/// for a `match`'s, and a match arm's block of any kind under `AlwaysNextLine` (the standard
/// formatter's `ControlFlow`, `rewrite_match_body` and `rewrite_let_else_block`). `None` when
/// nothing moves. Not in a `// mto: off` region or a skipped item.
pub fn control_braces(
  root: &SyntaxNode,
  text: &str,
  style: crate::options::format::ControlBraceStyle,
  edition: Edition,
) -> Option<String> {
  let off = off_regions(root);
  let mut gaps: Vec<(usize, usize, usize)> = Vec::new();
  control_gaps(root, text, 0, style, edition, &mut gaps);
  gaps.retain(|&(s, e, _)| !off.iter().any(|&(a, b)| a < e && s < b));
  if gaps.is_empty() {
    return None;
  }
  gaps.sort_unstable();
  gaps.dedup();
  let mut out = String::with_capacity(text.len() + gaps.len() * 8);
  let mut from = 0;
  for (start, end, anchor) in gaps {
    let line_start = text[..anchor].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = text[line_start..anchor]
      .chars()
      .take_while(|c| *c == ' ' || *c == '\t')
      .collect();
    out.push_str(&text[from..start]);
    out.push('\n');
    out.push_str(&indent);
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text` with the comments between a `loop`, `while`, `for` or `if` head and its block each
/// on a line of its own at the head's indentation, the block's `{` on the line after them,
/// as the standard formatter writes them. `None` when there are none. Not in a `// mto: off`
/// region or a skipped item.
pub fn control_comments(root: &SyntaxNode, nodes: &mut PassNodes, text: &str) -> Option<String> {
  let mut edits: Vec<(usize, usize, String)> = Vec::new();
  let controls = nodes.of(root, |kind| {
    matches!(kind, K::LoopExpr | K::WhileExpr | K::ForExpr | K::IfExpr)
  });
  for control in controls {
    let elements: Vec<SyntaxElement> = control.children_with_tokens_iter().collect();
    // An `if`'s first block, the others' last (the head may hold blocks too).
    let block = |e: &SyntaxElement| matches!(e, SyntaxElement::Node(n) if n.kind() == K::BlockExpr);
    let mut bodies = Vec::new();
    if control.kind() == K::IfExpr {
      bodies.extend(elements.iter().position(block));
      // And the block after `else`, at the indentation of the line `else` is on.
      if let Some(at) = elements.iter().position(|e| e.kind() == K::ElseKw)
        && let Some(after) = (at + 1..elements.len()).find(|&i| !elements[i].kind().is_trivia())
        && block(&elements[after])
      {
        bodies.push(after);
      }
    } else {
      bodies.extend(elements.iter().rposition(block));
    }
    for body in bodies {
      // The trivia just before the block.
      let mut first = body;
      while first > 0 && elements[first - 1].kind().is_trivia() {
        first -= 1;
      }
      let between = &elements[first..body];
      if first == 0
      || !between
        .iter()
        .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
      || between
        .iter()
        .any(|e| matches!(e, SyntaxElement::Token(t) if t.kind() == K::BlockComment && t.text().contains('\n')))
      || skipped_within(control)
    {
      continue;
    }
      // The line of the head's first token, or of the `else`.
      let start = match elements[..body].iter().rposition(|e| e.kind() == K::ElseKw) {
        Some(at) => elements[at].text_range().start,
        None => control.text_range().start,
      } as usize;
      let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
      let indent: String = text[line_start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
      let line = format!("\n{}", indent);
      let mut at = between[0].text_range().start as usize;
      for element in between {
        let range = element.text_range();
        if element.kind() == K::Whitespace {
          continue;
        }
        // One already starting its line keeps it, so that a second run changes nothing.
        if !text[at..range.start as usize].contains('\n') {
          edits.push((at, range.start as usize, line.clone()));
        }
        at = range.end as usize;
      }
      let block_start = elements[body].text_range().start as usize;
      if !text[at..block_start].contains('\n') {
        edits.push((at, block_start, line));
      }
    }
  }
  if edits.is_empty() {
    return None;
  }
  let off = off_regions(root);
  edits.retain(|&(start, end, _)| {
    !off
      .iter()
      .any(|&(s, e)| s < end.max(start + 1) && start < e)
  });
  if edits.is_empty() {
    return None;
  }
  edits.sort_unstable_by_key(|e| e.0);
  let mut out = String::with_capacity(text.len());
  let mut at = 0;
  for (start, end, replacement) in edits {
    if start < at {
      continue;
    }
    out.push_str(&text[at..start]);
    out.push_str(&replacement);
    at = end;
  }
  out.push_str(&text[at..]);
  (out != text).then_some(out)
}

/// `text` with each `#[doc = "..."]` (and `#![doc = "..."]`) written as doc comments, `///`
/// (`//!`) then each line of the string (the standard formatter's `DocCommentFormatter`);
/// `None` when there is none. Only a string whose escapes are `\n`, `\t`, `\"` and `\\`, or a
/// raw string without `\`; not one with a comment, a skipped attribute, nor in a
/// `// mto: off` region or a skipped item.
pub fn doc_attributes(root: &SyntaxNode, text: &str) -> Option<String> {
  if !text.contains("doc") {
    return None;
  }
  let off = off_regions(root);
  let mut changes: Vec<(usize, usize, String)> = Vec::new();
  for attr in root.descendant_nodes_where(|kind| kind == K::Attr) {
    let tokens: Vec<(SyntaxKind, &str)> = attr
      .token_texts()
      .filter(|(kind, _, _)| *kind != K::Whitespace)
      .map(|(kind, text, _)| (kind, text))
      .collect();
    let (marker, string) = match tokens.as_slice() {
      [
        (_, "#"),
        (_, "["),
        (_, "doc"),
        (_, "="),
        (kind, string),
        (_, "]"),
      ] => ("///", (*kind, *string)),
      [
        (_, "#"),
        (_, "!"),
        (_, "["),
        (_, "doc"),
        (_, "="),
        (kind, string),
        (_, "]"),
      ] => ("//!", (*kind, *string)),
      _ => continue,
    };
    let value = match string {
      (K::Str, s) => {
        let inner = &s[1..s.len() - 1];
        let mut value = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        let mut plain = true;
        while let Some(c) = chars.next() {
          if c != '\\' {
            value.push(c);
            continue;
          }
          match chars.next() {
            Some('n') => value.push('\n'),
            Some('t') => value.push('\t'),
            Some('"') => value.push('"'),
            Some('\\') => value.push('\\'),
            _ => plain = false,
          }
        }
        if !plain {
          continue;
        }
        value
      }
      (K::RawStr, s) if !s.contains('\\') => {
        let hashes = s[1..].bytes().take_while(|&b| b == b'#').count();
        s[2 + hashes..s.len() - 1 - hashes].to_string()
      }
      _ => continue,
    };
    if value.contains('\r') {
      continue;
    }
    let range = attr.text_range();
    let (start, end) = (range.start as usize, range.end as usize);
    if off.iter().any(|&(s, e)| s < end && start < e)
      || skipped_within(&attr)
      || crate::format::file::skipped_attribute(&attr)
    {
      continue;
    }
    // Later lines at the attribute's line's indentation, and the code after it on a line of
    // its own.
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = text[line_start..start]
      .chars()
      .take_while(|c| c.is_whitespace())
      .collect();
    let mut comment = String::new();
    let mut lines = value.lines().peekable();
    if lines.peek().is_none() {
      comment.push_str(marker);
    }
    while let Some(line) = lines.next() {
      comment.push_str(marker);
      comment.push_str(line);
      if lines.peek().is_some() {
        comment.push('\n');
        comment.push_str(&indent);
      }
    }
    let rest = &text[end..];
    let line_rest = &rest[..rest.find('\n').unwrap_or(rest.len())];
    if !line_rest.trim().is_empty() {
      comment.push('\n');
      comment.push_str(&indent);
    }
    changes.push((start, end, comment));
  }
  if changes.is_empty() {
    return None;
  }
  let mut out = String::with_capacity(text.len());
  let mut from = 0;
  for (start, end, rewritten) in changes {
    out.push_str(&text[from..start]);
    out.push_str(&rewritten);
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text` without the `use` declarations of empty lists (`use a::{};`, `use a::{b::{}};`),
/// each with its line when it stands alone there, which the standard formatter drops when it
/// reorders imports; `None` when there are none. Kept: one with attributes or a comment, one
/// from the root, one in a `// mto: off` region, and `use self;` (invalid, which the standard
/// formatter drops too, but which is not nothing).
pub fn empty_uses(
  root: &SyntaxNode,
  nodes: &mut PassNodes,
  text: &str,
  edition: Edition,
  config: &FormatConfig,
) -> Option<String> {
  if !config.reorder_imports {
    return None;
  }
  let global = edition >= Edition::E2018;
  let version = config.style_edition >= 2024;
  let mut cut: Vec<(usize, usize)> = Vec::new();
  for node in nodes.of(root, |kind| kind == K::Use) {
    let Some(tree) = node.children_iter().find(|c| c.kind() == K::UseTree) else {
      continue;
    };
    // From its visibility or `use` on: the comments before it are not its own.
    let Some(body) = crate::format::imports::body_start(node) else {
      continue;
    };
    let commented = node.token_texts().any(|(kind, _, range)| {
      kind.is_trivia() && kind != K::Whitespace && range.start as usize >= body
    });
    if node.children_iter().any(|c| c.kind() == K::Attr)
      || commented
      || tree.first_token().is_none_or(|t| t.kind() == K::Colon2)
      || !tree.subtree_has(|kind| kind == K::UseTreeList)
      || !UseTree::from_syntax(&tree, global, version).is_empty()
    {
      continue;
    }
    let (mut start, mut end) = (body, node.text_range().end as usize);
    // Its line too, when nothing else is on it.
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i);
    if text[line_start..start].trim().is_empty() && text[end..line_end].trim().is_empty() {
      start = line_start;
      end = (line_end + 1).min(text.len());
    }
    cut.push((start, end));
  }
  if cut.is_empty() {
    return None;
  }
  let off = off_regions(root);
  cut.retain(|&(start, end)| !off.iter().any(|&(s, e)| s < end && start < e));
  if cut.is_empty() {
    return None;
  }
  cut.sort_unstable();
  let mut out = String::with_capacity(text.len());
  let mut at = 0;
  for (start, end) in cut {
    if start < at {
      continue;
    }
    out.push_str(&text[at..start]);
    at = end;
  }
  out.push_str(&text[at..]);
  Some(out)
}

/// `text` with `extern` written alone given its ABI, `extern "C"` (`force_explicit_abi`);
/// `None` when there is none to give. Not `extern crate`, and not in a `// mto: off`
/// region.
pub fn explicit_abi(root: &SyntaxNode, text: &str) -> Option<String> {
  // No ABI and no `extern` block (`extern crate` is neither): nothing to write. A walk that
  // makes no nodes, after a look at the text: both hold an `extern` keyword.
  if !text.contains("extern") || !root.subtree_has(|kind| matches!(kind, K::Abi | K::ExternBlock)) {
    return None;
  }
  let off = off_regions(root);
  let mut at: Vec<usize> = Vec::new();
  for token in root
    .descendants_where(|kind| kind == K::ExternKw)
    .filter_map(|e| match e {
      SyntaxElement::Token(t) => Some(t),
      SyntaxElement::Node(_) => None,
    })
  {
    let parent = token.parent();
    // An ABI node, or an `extern { .. }` block's own keyword; with no string after it.
    let abi = match parent.kind() {
      K::Abi => parent
        .children_with_tokens_iter()
        .all(|e| !e.kind().is_literal()),
      K::ExternBlock => !parent.children_iter().any(|c| c.kind() == K::Abi),
      _ => false,
    };
    let end = token.text_range().end as usize;
    if abi && !off.iter().any(|&(s, e)| s < end && end <= e) {
      at.push(end);
    }
  }
  if at.is_empty() {
    return None;
  }
  at.sort_unstable();
  let mut out = String::with_capacity(text.len() + at.len() * 4);
  let mut from = 0;
  for end in at {
    out.push_str(&text[from..end]);
    out.push_str(" \"C\"");
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text` with a field's or a variant's comma written on a line after it, with only
/// whitespace between, joined to it, as the standard formatter writes it. `None` when there is
/// none. Not in a `// mto: off` region or a skipped item.
pub fn field_commas(root: &SyntaxNode, nodes: &mut PassNodes, text: &str) -> Option<String> {
  let mut gaps: Vec<(usize, usize)> = Vec::new();
  let lists = nodes.of(root, |kind| {
    matches!(
      kind,
      K::RecordFieldList | K::TupleFieldList | K::VariantList
    )
  });
  for list in lists {
    let elements: Vec<SyntaxElement> = list.children_with_tokens_iter().collect();
    for i in 2..elements.len() {
      if elements[i].kind() == K::Comma
        && matches!(&elements[i - 1], SyntaxElement::Token(t)
          if t.kind() == K::Whitespace && t.text().contains('\n'))
        && matches!(&elements[i - 2], SyntaxElement::Node(_))
        && !skipped_within(list)
      {
        let range = elements[i - 1].text_range();
        gaps.push((range.start as usize, range.end as usize));
      }
    }
  }
  let off = off_regions(root);
  gaps.retain(|&(start, end)| !off.iter().any(|&(s, e)| s < end && start < e));
  if gaps.is_empty() {
    return None;
  }
  let mut out = String::with_capacity(text.len());
  let mut at = 0;
  for (start, end) in gaps {
    out.push_str(&text[at..start]);
    at = end;
  }
  out.push_str(&text[at..]);
  Some(out)
}

/// `text` with each run of imports merged or split as `imports_granularity` says (the
/// standard formatter's `normalize_use_trees_with_granularity`), each written on one line
/// for the layout to lay out, one with a comment in it as written; `None` when none
/// changes. Not a run with a doc comment, nor in a `// mto: off` region or a skipped item.
pub fn granular_imports(
  root: &SyntaxNode,
  nodes: &mut PassNodes,
  text: &str,
  edition: Edition,
  config: &FormatConfig,
) -> Option<String> {
  use crate::order::{Granularity, Import, regroup};
  if config.imports_granularity == Granularity::Preserve || !text.contains("use") {
    return None;
  }
  let off = off_regions(root);
  let version = config.style_edition >= 2024;
  let global_paths = edition >= Edition::E2018;
  let containers = |kind| matches!(kind, K::SourceFile | K::ItemList | K::BlockExpr);
  let mut changes: Vec<(usize, usize, String)> = Vec::new();
  for node in nodes.of(root, containers) {
    for run in runs(node, config.blank_lines_upper_bound > 0) {
      if run[0].kind() != K::Use {
        continue;
      }
      let start = run[0].text_range().start as usize;
      let end = run[run.len() - 1].text_range().end as usize;
      let doc = root
        .token_texts_between(start as u32, end as u32)
        .any(|(kind, _, _)| super::items::is_doc(kind));
      if doc || run.iter().any(skipped_within) || off.iter().any(|&(s, e)| s < end && start < e) {
        continue;
      }
      let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
      let indent = &text[line_start..start];
      if !indent.trim().is_empty() {
        continue;
      }
      let imports: Option<Vec<Import>> = run
        .iter()
        .map(|item| {
          let attributes: String = item
            .children_iter()
            .filter(|c| c.kind() == K::Attr)
            .map(|a| format!("{}\n{}", a.text(), indent))
            .collect();
          let visibility = item
            .children_iter()
            .find(|c| c.kind() == K::Visibility)
            .map(|v| {
              format!(
                "{} ",
                super::expr::flat(&v, super::shape::Spacing::of(config))
              )
            })
            .unwrap_or_default();
          let tree = UseTree::from_syntax(
            &item.children_iter().find(|c| c.kind() == K::UseTree)?,
            global_paths,
            version,
          );
          // One with a comment in it is kept as written.
          let written = item
            .subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment))
            .then(|| item.text().to_string());
          (!tree.is_empty()).then_some(Import {
            attributes,
            visibility,
            tree,
            written,
          })
        })
        .collect();
      let Some(imports) = imports else {
        continue;
      };
      let written: Vec<String> = regroup(imports, config.imports_granularity, version)
        .into_iter()
        .map(|i| match i.written {
          Some(written) => written,
          None => format!("{}{}use {};", i.attributes, i.visibility, i.tree.text()),
        })
        .collect();
      let joined = written.join(&format!("\n{}", indent));
      if joined != text[start..end] {
        changes.push((start, end, joined));
      }
    }
  }
  if changes.is_empty() {
    return None;
  }
  changes.sort_by_key(|c| c.0);
  let mut out = String::with_capacity(text.len());
  let mut from = 0;
  for (start, end, rewritten) in changes {
    out.push_str(&text[from..start]);
    out.push_str(&rewritten);
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text`, laid out, with each `use`, `extern crate` and `mod name;` that has one attribute,
/// not a doc comment, written after it on its line when the two, a space between, are at
/// most `inline_attribute_width` long, the item counted whole (the standard formatter's
/// `rewrite_attrs`); `None` when none is. Not one with a comment between them, nor in a
/// `// mto: off` region or a skipped item.
pub fn inline_attributes(root: &SyntaxNode, text: &str, config: &FormatConfig) -> Option<String> {
  let off = off_regions(root);
  let mut gaps: Vec<(usize, usize)> = Vec::new();
  for item in
    root.descendant_nodes_where(|kind| matches!(kind, K::Use | K::ExternCrate | K::Module))
  {
    if item.kind() == K::Module && item.children_iter().any(|c| c.kind() == K::ItemList) {
      continue;
    }
    let attributes: Vec<SyntaxNode> = item
      .children_iter()
      .filter(|c| c.kind() == K::Attr)
      .collect();
    let [attribute] = attributes.as_slice() else {
      continue;
    };
    if super::items::is_doc(attribute.kind())
      || attribute.text().starts_with("///")
      || attribute.text().starts_with("//!")
      || item
        .children_with_tokens_iter()
        .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment))
    {
      continue;
    }
    let attribute_end = attribute.text_range().end as usize;
    let item_end = item.text_range().end as usize;
    let gap_end =
      attribute_end + text[attribute_end..].len() - text[attribute_end..].trim_start().len();
    let gap = &text[attribute_end..gap_end];
    if !gap.contains('\n') || gap.matches('\n').count() > 1 {
      continue;
    }
    let rest = &text[gap_end..item_end];
    let attribute_start = attribute.text_range().start as usize;
    let line_start = text[..attribute_start].rfind('\n').map_or(0, |i| i + 1);
    let indent = attribute_start - line_start;
    let first = rest.lines().next().unwrap_or("");
    if attribute.text().contains('\n')
      || attribute.text().len() + 1 + rest.len() > config.inline_attribute_width
      || indent + attribute.text().len() + 1 + first.len() > config.max_width
      || skipped_within(&item)
      || off.iter().any(|&(s, e)| s < item_end && line_start < e)
    {
      continue;
    }
    gaps.push((attribute_end, gap_end));
  }
  if gaps.is_empty() {
    return None;
  }
  gaps.sort_unstable();
  let mut out = String::with_capacity(text.len());
  let mut from = 0;
  for (start, end) in gaps {
    out.push_str(&text[from..start]);
    out.push(' ');
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text` with the comments on lines of their own after a `match`'s last arm, written
/// without a comma (or with one after a plain block, which goes), moved to the end of the
/// arm's line, as the standard formatter takes them
/// for the arm's comments after it: the first after one space, each later one under it, a
/// blank line among them kept. `None` when there are none to move. Not in a
/// `// mto: off` region or a skipped item.
pub fn last_arm_comments(root: &SyntaxNode, nodes: &mut PassNodes, text: &str) -> Option<String> {
  let mut edits: Vec<(usize, usize, String)> = Vec::new();
  for list in nodes.of(root, |kind| kind == K::MatchArmList) {
    let Some(arm) = list.children_iter().rfind(|c| c.kind() == K::MatchArm) else {
      continue;
    };
    let mut after = Vec::new();
    let arm_end = arm.text_range().end;
    for element in list
      .children_with_tokens_iter()
      .filter(|e| e.text_range().start >= arm_end)
    {
      match element {
        SyntaxElement::Token(t) if t.kind() == K::RBrace => break,
        SyntaxElement::Token(t) if t.kind().is_trivia() => after.push(t),
        _ => {
          after.clear();
          break;
        }
      }
    }
    // Whitespace, then comments each ending its line, then the `}` on a line of its own.
    let starts_line = |i: usize| {
      after
        .get(i)
        .is_some_and(|t| t.kind() == K::Whitespace && t.text().contains('\n'))
    };
    let comments: Vec<usize> = (0..after.len())
      .filter(|&i| after[i].kind() != K::Whitespace)
      .collect();
    // The first comment may end the arm's own line instead; the rest go under it.
    let on_arm_line = !after.is_empty() && !starts_line(0);
    if comments.is_empty()
      || after.len() != comments.len() * 2 + 1
      || !(0..after.len()).all(|i| (i % 2 == 1) == (after[i].kind() != K::Whitespace))
      || !(0..=comments.len()).all(|n| starts_line(n * 2) || n == 0 && on_arm_line)
      || on_arm_line && (comments.len() < 2 || after[1].kind() != K::LineComment)
      || !on_arm_line
        && arm.last_token().is_some_and(|t| t.kind() == K::Comma)
        && !plain_block_body(&arm)
      || skipped_within(list)
    {
      continue;
    }
    let end = if on_arm_line {
      after[1].text_range().start as usize - 1
    } else {
      arm.text_range().end as usize
    };
    let line_start = text[..end].rfind('\n').map_or(0, |i| i + 1);
    // Under the first comment: the arm's line, each of its characters a space but tabs.
    let mut under: String = text[line_start..end]
      .chars()
      .map(|c| match c {
        '\t' => "\t".to_string(),
        c => " ".repeat(crate::text::unicode::char_width(c)),
      })
      .collect();
    under.push(' ');
    for (n, &i) in comments.iter().enumerate() {
      let space = &after[i - 1];
      let range = space.text_range();
      let replacement = if n == 0 {
        if on_arm_line {
          continue;
        }
        " ".to_string()
      } else if space.text().matches('\n').count() > 1 {
        // A blank line among them stays.
        format!("\n\n{}", under)
      } else {
        format!("\n{}", under)
      };
      edits.push((range.start as usize, range.end as usize, replacement));
    }
  }
  if edits.is_empty() {
    return None;
  }
  let off = off_regions(root);
  edits.retain(|&(start, end, _)| !off.iter().any(|&(s, e)| s < end && start < e));
  if edits.is_empty() {
    return None;
  }
  edits.sort_unstable_by_key(|e| e.0);
  let mut out = String::with_capacity(text.len());
  let mut at = 0;
  for (start, end, replacement) in edits {
    out.push_str(&text[at..start]);
    out.push_str(&replacement);
    at = end;
  }
  out.push_str(&text[at..]);
  Some(out)
}

/// `text` with each run of `#[derive(..)]` attributes merged into one, their traits in order
/// (`merge_derives`); `None` when there is none to merge. A run is of attributes with no
/// comment between them (`take_while_with_pred`), across a blank line too, which the
/// standard formatter merges on its second run once it has dropped that line; not one in a
/// `// mto: off` region, on what is skipped or skips derives, or holding a comment. A run
/// with a derive whose arguments are not a list of paths stays as written whole, as the
/// standard formatter keeps attributes it cannot read (`format_derive`).
pub fn merge_derives(root: &SyntaxNode, text: &str) -> Option<String> {
  // Walking the tree is most of this pass's cost: only a file with a derive written right
  // after another is walked.
  if !adjacent_derives(text) {
    return None;
  }
  let off = off_regions(root);
  // The traits of a derive, each as written, and whether each is a path (with arguments
  // or a value, as a meta item may have): `None` for any other attribute.
  let traits = |attr: &SyntaxNode| -> Option<(Vec<String>, bool)> {
    let a = Attr::cast(attr.clone())?;
    if a.is_inner() || a.path() != "derive" {
      return None;
    }
    let tree = attr
      .descendant_nodes_where(|kind| kind == K::TokenTree)
      .next()?;
    if tree.first_token()?.kind() != K::LParen
      || tree.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment))
    {
      return None;
    }
    let inner = tree.text().to_string();
    let inner = &inner[1..inner.len() - 1];
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in inner.chars() {
      match c {
        '(' | '[' | '{' | '<' => depth += 1,
        ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
        _ => {}
      }
      if c == ',' && depth == 0 {
        items.push(std::mem::take(&mut current));
      } else {
        current.push(c);
      }
    }
    items.push(current);
    let items: Vec<String> = items
      .into_iter()
      .map(|i| i.split_whitespace().collect::<Vec<_>>().join(" "))
      .filter(|i| !i.is_empty())
      .collect();
    let read = items.iter().all(|item| meta_item(item));
    (!items.is_empty()).then_some((items, read))
  };
  let mut edits: Vec<(usize, usize, String)> = Vec::new();
  // `rustfmt::skip::attributes(derive)` on what holds them or on what holds that.
  let skips_derives = |attr: &SyntaxNode| {
    let text: String = attr.text().to_string().split_whitespace().collect();
    text.contains("rustfmt::skip::attributes(") && text.contains("derive")
  };
  // The ranges of what skips derives, found once.
  let skipping: Vec<(u32, u32)> = if !text.contains("attributes") {
    Vec::new()
  } else {
    root
      .descendant_nodes_where(|kind| kind == K::Attr)
      .filter(|a| skips_derives(a))
      .filter_map(|a| a.parent())
      .map(|p| (p.text_range().start, p.text_range().end))
      .collect()
  };
  for node in root.descendants_iter() {
    // Only what holds two attributes or more has derives to merge.
    if node
      .children_iter()
      .filter(|c| c.kind() == K::Attr)
      .nth(1)
      .is_none()
    {
      continue;
    }
    let range = node.text_range();
    if node
      .children_iter()
      .filter_map(Attr::cast)
      .any(|a| a.is_rustfmt_skip())
      || skipping
        .iter()
        .any(|&(s, e)| s <= range.start && range.end <= e)
    {
      continue;
    }
    let elements: Vec<SyntaxElement> = node.children_with_tokens_iter().collect();
    let mut i = 0;
    while i < elements.len() {
      let SyntaxElement::Node(first) = &elements[i] else {
        i += 1;
        continue;
      };
      let Some((mut all, mut read)) = (first.kind() == K::Attr).then(|| traits(first)).flatten()
      else {
        i += 1;
        continue;
      };
      let mut last = first.clone();
      let mut count = 1;
      let mut j = i + 1;
      loop {
        // Whitespace with at most one line break, then another derive.
        let mut k = j;
        if let Some(SyntaxElement::Token(t)) = elements.get(k)
          && t.kind() == K::Whitespace
        {
          k += 1;
        }
        match elements.get(k) {
          Some(SyntaxElement::Node(n)) if n.kind() == K::Attr => match traits(n) {
            Some((more, also)) => {
              all.extend(more);
              read &= also;
              last = n.clone();
              count += 1;
              j = k + 1;
            }
            None => break,
          },
          _ => break,
        }
      }
      let (start, end) = (
        first.text_range().start as usize,
        last.text_range().end as usize,
      );
      if count > 1 && read && !off.iter().any(|&(s, e)| s < end && start < e) {
        edits.push((start, end, format!("#[derive({})]", all.join(", "))));
      }
      i = j.max(i + 1);
    }
  }
  if edits.is_empty() {
    return None;
  }
  edits.sort_by_key(|e| e.0);
  let mut out = String::with_capacity(text.len());
  let mut at = 0;
  for (start, end, replacement) in edits {
    if start < at {
      continue;
    }
    out.push_str(&text[at..start]);
    out.push_str(&replacement);
    at = end;
  }
  out.push_str(&text[at..]);
  Some(out)
}

/// `text` with each number literal of an expression or a pattern written as
/// `hex_literal_case` and `float_literal_trailing_zero` say (the standard formatter's
/// `rewrite_int_lit` and `rewrite_float_lit`), in attributes and macro arguments too;
/// `None` when none changes. Not a tuple field's index, nor in a `// mto: off` region, a
/// skipped item or a skipped macro's arguments.
pub fn number_literals(root: &SyntaxNode, text: &str, config: &FormatConfig) -> Option<String> {
  use crate::options::format::{FloatTrailingZero, HexLiteralCase};
  if config.hex_literal_case == HexLiteralCase::Preserve
    && config.float_trailing_zero == FloatTrailingZero::Preserve
  {
    return None;
  }
  let off = off_regions(root);
  let mut changes: Vec<(usize, usize, String)> = Vec::new();
  for token in root
    .descendants_where(|kind| matches!(kind, K::IntNumber | K::FloatNumber))
    .filter_map(|e| match e {
      SyntaxElement::Token(t) => Some(t),
      SyntaxElement::Node(_) => None,
    })
  {
    let parent = token.parent();
    let literal = match parent.kind() {
      K::Literal | K::LiteralPat => true,
      // In a token tree, not after a lone `.` (`x.0.0`); after `..`, two of them there, it
      // ends a range.
      K::TokenTree => {
        let start = token.text_range().start;
        let before: Vec<SyntaxKind> = parent
          .children_with_tokens_iter()
          .take_while(|e| e.text_range().start < start)
          .filter(|e| !e.kind().is_trivia())
          .map(|e| e.kind())
          .collect();
        !matches!(before.as_slice(), [.., before, K::Dot] if *before != K::Dot)
          && !parent
            .ancestors_iter()
            .any(|a| a.kind() == K::MacroCall && crate::format::file::skipped_macro(&a))
      }
      _ => false,
    };
    if !literal {
      continue;
    }
    let written = token.text();
    let rewritten = if let Some(digits) = written.strip_prefix("0x") {
      let end = digits
        .find(|c: char| !c.is_ascii_hexdigit() && c != '_')
        .unwrap_or(digits.len());
      let (digits, suffix) = digits.split_at(end);
      match config.hex_literal_case {
        HexLiteralCase::Preserve => continue,
        HexLiteralCase::Upper => format!("0x{}{}", digits.to_ascii_uppercase(), suffix),
        HexLiteralCase::Lower => format!("0x{}{}", digits.to_ascii_lowercase(), suffix),
      }
    } else {
      // An integer with a float's suffix is a float (`is_semantic_float`).
      let semantic_float = token.kind() == K::FloatNumber
        || !written.starts_with("0b")
          && !written.starts_with("0o")
          && (written.ends_with("f32") || written.ends_with("f64"));
      if !semantic_float {
        continue;
      }
      match float_literal(written, config.float_trailing_zero) {
        Some(text) => text,
        None => continue,
      }
    };
    let range = token.text_range();
    let (start, end) = (range.start as usize, range.end as usize);
    if rewritten != written
      && !off.iter().any(|&(s, e)| s < end && start < e)
      && !skipped_within(&parent)
    {
      changes.push((start, end, rewritten));
    }
  }
  if changes.is_empty() {
    return None;
  }
  let mut out = String::with_capacity(text.len() + changes.len() * 2);
  let mut from = 0;
  for (start, end, rewritten) in changes {
    out.push_str(&text[from..start]);
    out.push_str(&rewritten);
    // A float now ending in `.` before a `.` (a method's, a range's): spaced from it, as
    // the layout writes such a float as written (`(1.).max()`, `1. ..2`).
    if rewritten.ends_with('.') && text[end..].starts_with('.') {
      out.push(' ');
    }
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text` with its runs sorted, or `None` when nothing moves.
pub fn reorder(
  root: &SyntaxNode,
  nodes: &mut PassNodes,
  text: &str,
  edition: Edition,
  config: &FormatConfig,
) -> Option<String> {
  if !config.reorder_imports && !config.reorder_modules {
    return None;
  }
  // Found only once a run could move.
  let mut off: Option<Vec<(usize, usize)>> = None;
  let mut moves: Vec<(usize, usize, String)> = Vec::new();
  let containers = |kind| matches!(kind, K::SourceFile | K::ItemList | K::BlockExpr);
  for node in nodes.of(root, containers) {
    for run in runs(node, config.blank_lines_upper_bound > 0) {
      let start = run[0].text_range().start as usize;
      let end = run[run.len() - 1].text_range().end as usize;
      if run.len() < 2 || has_comment(&run) {
        continue;
      }
      let off = off.get_or_insert_with(|| off_regions(root));
      if off.iter().any(|&(s, e)| s < end && start < e) {
        continue;
      }
      let mut order: Vec<usize> = (0..run.len()).collect();
      match run[0].kind() {
        K::Use if config.reorder_imports => {
          let version = config.style_edition >= 2024;
          let global_paths = edition >= Edition::E2018;
          let trees: Vec<UseTree> = run
            .iter()
            .map(|item| {
              item
                .children_iter()
                .find(|c| c.kind() == K::UseTree)
                .map(|t| UseTree::from_syntax(&t, global_paths, version))
            })
            .collect::<Option<Vec<_>>>()?;
          order.sort_by(|&a, &b| trees[a].compare(&trees[b], version));
        }
        K::Module if config.reorder_modules => {
          let names: Vec<String> = run.iter().map(module_name).collect::<Option<Vec<_>>>()?;
          order.sort_by(|&a, &b| {
            let (x, y) = (
              names[a].trim_start_matches("r#"),
              names[b].trim_start_matches("r#"),
            );
            // By bytes through style edition 2024, by version after it.
            if config.style_edition > 2024 {
              version_sort(x, y)
            } else {
              x.cmp(y)
            }
          });
        }
        // By the crate's name, then its alias: none first.
        K::ExternCrate if config.reorder_imports => {
          let names: Vec<(String, Option<String>)> = run
            .iter()
            .map(|item| {
              let name = item.children_iter().find(|c| c.kind() == K::NameRef)?;
              let alias = item
                .children_iter()
                .find(|c| c.kind() == K::Rename)
                .and_then(|r| r.children_iter().find(|c| c.kind() == K::Name))
                .map(|n| n.text().to_string());
              Some((name.text().to_string(), alias))
            })
            .collect::<Option<Vec<_>>>()?;
          let by = |x: &str, y: &str| {
            if config.style_edition > 2024 {
              version_sort(x, y)
            } else {
              x.cmp(y)
            }
          };
          order.sort_by(|&a, &b| {
            let ((x, x_alias), (y, y_alias)) = (&names[a], &names[b]);
            by(x, y).then_with(|| match (x_alias, y_alias) {
              (Some(_), None) => std::cmp::Ordering::Greater,
              (None, Some(_)) => std::cmp::Ordering::Less,
              (None, None) => std::cmp::Ordering::Equal,
              (Some(x), Some(y)) => by(x, y),
            })
          });
        }
        _ => continue,
      }
      if order.iter().enumerate().all(|(i, &o)| i == o) {
        continue;
      }
      // The first item's leading comments stay in the first slot, before the run.
      let start_of = |i: usize| {
        if i == 0 {
          code_start(&run[0])
        } else {
          run[i].text_range().start as usize
        }
      };
      // The last item's comments after its code stay in the last slot, after the run.
      let last = run.len() - 1;
      let end_of = |i: usize| {
        if i == last {
          code_end(&run[last])
        } else {
          run[i].text_range().end as usize
        }
      };
      for (slot, &from) in order.iter().enumerate() {
        let mut moved = text[start_of(from)..end_of(from)].to_string();
        // One ending in a line comment, put where code follows on the line, ends its line.
        let rest = &text[end_of(slot)..];
        let line_rest = rest.split('\n').next().unwrap_or("");
        let ends_in_comment = run[from].last_token().is_some_and(|t| {
          t.kind() == K::LineComment && t.text_range().end as usize <= end_of(from)
        });
        if ends_in_comment && !line_rest.trim().is_empty() {
          moved.push('\n');
        }
        moves.push((start_of(slot), end_of(slot), moved));
      }
    }
  }
  if moves.is_empty() {
    return None;
  }
  moves.sort_by_key(|&(start, _, _)| start);
  let mut out = String::with_capacity(text.len());
  let mut at = 0;
  for (start, end, replacement) in moves {
    out.push_str(&text[at..start]);
    out.push_str(&replacement);
    at = end;
  }
  out.push_str(&text[at..]);
  Some(out)
}

/// `text`, laid out, with each empty function, impl and trait body written `{` and `}` on
/// lines of their own without `empty_item_single_line`, and with `fn_single_line` each
/// function whose body is one statement on one line, without a comment, written on its
/// header's line when that fits (the standard formatter's `single_line_fn`); `None` when
/// nothing changes. Not in a `// mto: off` region or a skipped item.
pub fn single_line_items(root: &SyntaxNode, text: &str, config: &FormatConfig) -> Option<String> {
  let off = off_regions(root);
  let width = |s: &str| {
    s.chars()
      .map(|c| if c == '\t' { config.tab_spaces } else { 1 })
      .sum::<usize>()
  };
  let mut changes: Vec<(usize, usize, String)> = Vec::new();
  // A `where` of one predicate: `where` and it on a line, a function's `{` (or `;`) after
  // them when its parameters are on one line, another item's on the next (`where_single_line`).
  if config.where_single_line {
    for item in root.descendant_nodes_where(|kind| {
      matches!(
        kind,
        K::Fn | K::Impl | K::Struct | K::Enum | K::Union | K::Trait
      )
    }) {
      let Some(clause) = item.children_iter().find(|c| c.kind() == K::WhereClause) else {
        continue;
      };
      let predicates: Vec<SyntaxNode> = clause
        .children_iter()
        .filter(|c| c.kind() == K::WherePred)
        .collect();
      let [predicate] = predicates.as_slice() else {
        continue;
      };
      let function = item.kind() == K::Fn;
      let params_one_line = !function
        || item
          .children_iter()
          .find(|c| c.kind() == K::ParamList)
          .is_some_and(|p| !p.text().contains('\n'));
      let range = clause.text_range();
      let (start, end) = (range.start as usize, range.end as usize);
      let after = text[end..].trim_start();
      let terminator = if !function {
        ""
      } else if after.starts_with('{') {
        " {"
      } else if after.starts_with(';') {
        ";"
      } else {
        continue;
      };
      let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
      let one = format!("where {}", predicate.text());
      if !params_one_line
        || predicate.text().contains('\n')
        || clause.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment))
        || !text[line_start..start].trim().is_empty()
        || skipped_within(&item)
        || off.iter().any(|&(s, e)| s < end && start < e)
        || (start - line_start) + one.len() + terminator.len() > config.max_width
      {
        continue;
      }
      // A function's through the `{` or `;` after the clause.
      let close = if function {
        end + (text[end..].len() - after.len()) + 1
      } else {
        end
      };
      changes.push((start, close, format!("{}{}", one, terminator)));
    }
  }
  for item in root.descendant_nodes_where(|kind| matches!(kind, K::Fn | K::Impl | K::Trait)) {
    let body = item
      .children_iter()
      .find(|c| matches!(c.kind(), K::BlockExpr | K::AssocItemList));
    let Some(body) = body else {
      continue;
    };
    let range = body.text_range();
    let (start, end) = (range.start as usize, range.end as usize);
    if skipped_within(&item) || off.iter().any(|&(s, e)| s < end && start < e) {
      continue;
    }
    let inside: Vec<SyntaxElement> = body
      .children_with_tokens_iter()
      .filter(|e| !matches!(e.kind(), K::LBrace | K::RBrace | K::Whitespace))
      .collect();
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = text[line_start..start]
      .chars()
      .take_while(|c| *c == ' ' || *c == '\t')
      .collect();
    if inside.is_empty() {
      if !config.empty_item_single_line && &text[start..end] == "{}" {
        changes.push((start, end, format!("{{\n{}}}", indent)));
      }
      continue;
    }
    // One statement or expression, nothing else, on one line, in a function written on one
    // line before its body.
    let [SyntaxElement::Node(statement)] = inside.as_slice() else {
      continue;
    };
    let header_start =
      crate::format::items::header_start(&item).unwrap_or(item.text_range().start as usize);
    let header = text[header_start..start].trim_end();
    let header_line = text[..header_start].rfind('\n').map_or(0, |i| i + 1);
    let line = statement.text();
    if !config.fn_single_line
      || item.kind() != K::Fn
      || body.kind() != K::BlockExpr
      // Its `{` on the next line by `brace_style = "AlwaysNextLine"` joins it.
      || header.contains('\n')
      || line.contains('\n')
      || statement.subtree_has(|kind| kind == K::Attr)
      // Not a macro call standing as a statement, with a `;` or in braces, nor an item
      // (`format_stmt` gives up).
      || statement.kind() == K::ExprStmt
        && statement.children_iter().next().is_some_and(|e| e.kind() == K::MacroExpr)
      || statement.kind() == K::MacroExpr
        && statement
          .descendant_nodes_where(|kind| kind == K::TokenTree)
          .next()
          .and_then(|tree| tree.first_token())
          .is_some_and(|t| t.kind() == K::LBrace)
      || crate::format::items::is_item(statement.kind())
      || text[header_line..header_start].trim() != ""
    {
      continue;
    }
    let joined = format!(" {{ {} }}", line);
    if width(&text[header_line..header_start]) + width(header) + width(&joined) <= config.max_width
    {
      changes.push((header_start + header.len(), end, joined));
    }
  }
  if changes.is_empty() {
    return None;
  }
  changes.sort_by_key(|c| c.0);
  let mut out = String::with_capacity(text.len());
  let mut from = 0;
  for (start, end, rewritten) in changes {
    if start < from {
      continue;
    }
    out.push_str(&text[from..start]);
    out.push_str(&rewritten);
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// `text` without the `;`s standing alone in a block (empty statements), which the standard
/// formatter drops; `None` when there are none. Not in a `// mto: off` region or a skipped
/// item.
pub fn strip_semicolons(root: &SyntaxNode, nodes: &mut PassNodes, text: &str) -> Option<String> {
  let mut stray: Vec<(usize, usize)> = Vec::new();
  for block in nodes.of(root, |kind| kind == K::BlockExpr) {
    // One after an item (`struct S {};`) the standard formatter keeps.
    let mut after_item = false;
    // Read from the green tree, without a handle for each child.
    for (kind, range, node) in block.child_spans() {
      match kind {
        K::Semi if !node => {
          if !after_item {
            stray.push((range.start as usize, range.end as usize));
          }
          after_item = false;
        }
        _ if !node && kind.is_trivia() => {}
        _ if node => {
          after_item = !matches!(kind, K::ExprStmt | K::LetStmt) && !kind.name().ends_with("_EXPR");
        }
        _ => after_item = false,
      }
    }
  }
  if stray.is_empty() {
    return None;
  }
  let off = off_regions(root);
  let skipped = |at: usize| {
    root
      .descendant_nodes_where(|kind| kind != K::Attr)
      .filter(|n| {
        let r = n.text_range();
        (r.start as usize) <= at && at < r.end as usize
      })
      .any(|n| {
        n.children_iter()
          .filter_map(Attr::cast)
          .any(|a| a.is_rustfmt_skip())
      })
  };
  stray.retain(|&(start, end)| !off.iter().any(|&(s, e)| s < end && start < e) && !skipped(start));
  if stray.is_empty() {
    return None;
  }
  // One alone on its line goes with the line.
  for (start, end) in stray.iter_mut() {
    let line_start = text[..*start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[*end..].find('\n').map_or(text.len(), |i| *end + i);
    if text[line_start..*start].trim().is_empty() && text[*end..line_end].trim().is_empty() {
      (*start, *end) = (line_start, (line_end + 1).min(text.len()));
    }
  }
  stray.sort_unstable();
  let mut out = String::with_capacity(text.len());
  let mut at = 0;
  for (start, end) in stray {
    out.push_str(&text[at..start]);
    at = end;
  }
  out.push_str(&text[at..]);
  Some(out)
}

/// `text` with each `try!(e)` (or `r#try!(e)`) whose arguments are one expression that `?`
/// binds to whole written `e?` (the standard formatter's `convert_try_mac`); `None` when
/// there is none. Not in a `// mto: off` region or a skipped item.
pub fn try_shorthand(root: &SyntaxNode, text: &str, edition: Edition) -> Option<String> {
  if !text.contains("try!") {
    return None;
  }
  let off = off_regions(root);
  let mut changes: Vec<(usize, usize, String)> = Vec::new();
  for call in root.descendant_nodes_where(|kind| kind == K::MacroCall) {
    let path = call.children_iter().find(|c| c.kind() == K::Path);
    let tree = call.children_iter().find(|c| c.kind() == K::TokenTree);
    let (Some(path), Some(tree)) = (path, tree) else {
      continue;
    };
    if !matches!(path.text().trim(), "try" | "r#try")
      || call.parent().is_none_or(|p| p.kind() != K::MacroExpr)
    {
      continue;
    }
    let range = call.text_range();
    let (start, end) = (range.start as usize, range.end as usize);
    if off.iter().any(|&(s, e)| s < end && start < e) || skipped_within(&call) {
      continue;
    }
    // In parentheses, as the meaning check reads it.
    let inner = tree.text();
    if !inner.starts_with('(') {
      continue;
    }
    let inner = inner[1..inner.len() - 1].trim();
    // The arguments one expression, all of them.
    let wrapped = format!("fn __mto() {{\n_ = {};\n}}", inner);
    let parsed = crate::syntax::parse(&wrapped, edition);
    if !parsed.errors().is_empty() {
      continue;
    }
    let Some(expression) = parsed
      .syntax_node()
      .descendant_nodes_where(|kind| kind == K::BinExpr)
      .next()
      .and_then(|assign| assign.children_iter().nth(1))
      .filter(|e| e.text().trim() == inner)
    else {
      continue;
    };
    // Any other is left as written: the standard formatter writes `a + b?`, which is not
    // what `try!(a + b)` means.
    if crate::verify::format::question_binds_whole(expression.kind()) {
      changes.push((start, end, format!("{}?", inner)));
    }
  }
  if changes.is_empty() {
    return None;
  }
  // Outermost first, and one inside another left to the next round.
  changes.sort_by_key(|c| c.0);
  let mut out = String::with_capacity(text.len());
  let mut from = 0;
  for (start, end, rewritten) in changes {
    if start < from {
      continue;
    }
    out.push_str(&text[from..start]);
    out.push_str(&rewritten);
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// A `use`, an `extern crate` or a `mod name;`, with no `#[macro_use]` and not skipped.
pub(super) fn sortable(node: &SyntaxNode) -> bool {
  let declaration = match node.kind() {
    K::Use | K::ExternCrate => true,
    K::Module => node
      .children_with_tokens_iter()
      .any(|e| e.kind() == K::Semi),
    _ => false,
  };
  declaration
    && !node
      .children_iter()
      .filter_map(Attr::cast)
      .any(|a| a.is_rustfmt_skip() || a.path() == "macro_use")
}

/// Whether `text` may hold a `#[derive(..)]` followed, past whitespace only, by another:
/// read from the text alone, so it may say yes where the tree says no, never the reverse.
fn adjacent_derives(text: &str) -> bool {
  let bytes = text.as_bytes();
  let skip_space = |mut at: usize| {
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
      at += 1;
    }
    at
  };
  // Where a `#[derive` written with any spaces starts at `at`, the `[`'s position.
  let derive_at = |at: usize| -> Option<usize> {
    if bytes.get(at) != Some(&b'#') {
      return None;
    }
    let open = skip_space(at + 1);
    if bytes.get(open) != Some(&b'[') {
      return None;
    }
    let name = skip_space(open + 1);
    text[name..].starts_with("derive").then_some(open)
  };
  let mut from = 0;
  while let Some(found) = text[from..].find('#') {
    let at = from + found;
    from = at + 1;
    let Some(open) = derive_at(at) else {
      continue;
    };
    // Its closing `]`.
    let mut depth = 0usize;
    let mut close = None;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
      match b {
        b'[' => depth += 1,
        b']' => {
          depth -= 1;
          if depth == 0 {
            close = Some(i);
            break;
          }
        }
        _ => {}
      }
    }
    let Some(close) = close else {
      return true;
    };
    if derive_at(skip_space(close + 1)).is_some() {
      return true;
    }
  }
  false
}

/// Where an item's code ends: before the comments it carries after it.
fn code_end(item: &SyntaxNode) -> usize {
  item
    .children_with_tokens_iter()
    .rfind(|e| !e.kind().is_trivia())
    .map_or(item.text_range().end as usize, |e| {
      e.text_range().end as usize
    })
}

/// Where an item's code starts, its attributes included: after the comments it carries
/// before it.
fn code_start(item: &SyntaxNode) -> usize {
  item
    .children_with_tokens_iter()
    .find(|e| !e.kind().is_trivia())
    .map_or(item.text_range().start as usize, |e| {
      e.text_range().start as usize
    })
}

/// [`control_braces`]'s gaps in `root`, whose text is `text`, each with where the line whose
/// indentation it takes is, at `text`'s offsets moved by `shift` (a macro's arguments parsed
/// apart sit elsewhere in the file). Into a macro's arguments too that read as a call's, as
/// the standard formatter formats them.
fn control_gaps(
  root: &SyntaxNode,
  text: &str,
  shift: isize,
  style: crate::options::format::ControlBraceStyle,
  edition: Edition,
  gaps: &mut Vec<(usize, usize, usize)>,
) {
  use crate::options::format::ControlBraceStyle;
  let next_line = style == ControlBraceStyle::AlwaysNextLine;
  let moved = |at: usize| (at as isize + shift) as usize;
  let mut push = |gap: Option<(usize, usize)>, anchor: Option<usize>| {
    if let Some((s, e)) = gap {
      gaps.push((moved(s), moved(e), moved(anchor.unwrap_or(s))));
    }
  };
  // The whitespace before an element, when it is spaces on its line after code.
  let gap = |element: &SyntaxElement| -> Option<(usize, usize)> {
    let start = element.text_range().start as usize;
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..start];
    let spaces = before.len() - before.trim_end_matches([' ', '\t']).len();
    (spaces > 0 && !before.trim().is_empty()).then_some((start - spaces, start))
  };
  // A block's `{` that ends its line, or opens it empty (`{}`).
  let brace = |block: &SyntaxNode| -> Option<SyntaxElement> {
    let open = block
      .children_with_tokens_iter()
      .find(|e| !e.kind().is_trivia())
      .filter(|e| e.kind() == K::LBrace)?;
    let end = open.text_range().end as usize;
    (text[end..].starts_with('\n') || text[end..].starts_with('}')).then_some(open)
  };
  for node in root.descendant_nodes_where(|kind| {
    matches!(
      kind,
      K::IfExpr | K::WhileExpr | K::ForExpr | K::LoopExpr | K::MatchExpr | K::MatchArm | K::LetElse
    )
  }) {
    if skipped_within(&node) {
      continue;
    }
    match node.kind() {
      K::IfExpr => {
        let elements: Vec<SyntaxElement> = node.children_with_tokens_iter().collect();
        if next_line
          && let Some(then) = node.children_iter().find(|c| c.kind() == K::BlockExpr)
          && let Some(open) = brace(&then)
        {
          push(gap(&open), None);
        }
        if let Some(at) = elements.iter().position(|e| e.kind() == K::ElseKw) {
          // After a block's `}` on a line of its own.
          let after_close = elements[..at]
            .iter()
            .rev()
            .find(|e| !e.kind().is_trivia())
            .is_some_and(|e| {
              let close = e.text_range().end as usize - 1;
              let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
              e.kind() == K::BlockExpr && text[line_start..close].trim().is_empty()
            });
          if after_close {
            push(gap(&elements[at]), None);
          }
          if next_line
            && let Some(SyntaxElement::Node(block)) =
              elements[at + 1..].iter().find(|e| !e.kind().is_trivia())
            && block.kind() == K::BlockExpr
            && let Some(open) = brace(block)
          {
            push(gap(&open), None);
          }
        }
      }
      // Not an empty one's.
      K::MatchExpr if next_line => {
        if let Some(arms) = node.children_iter().find(|c| c.kind() == K::MatchArmList)
          && let Some(open) = brace(&arms)
          && !text[open.text_range().end as usize..].starts_with('}')
        {
          push(gap(&open), None);
        }
      }
      // Any block, `unsafe` or labelled too (`rewrite_match_body`).
      K::MatchArm if next_line => {
        if let Some(body) = node
          .children_iter()
          .next_back()
          .filter(|b| b.kind() == K::BlockExpr)
          && let Some(first) = body
            .children_with_tokens_iter()
            .find(|e| !e.kind().is_trivia())
        {
          // At the arm's first line's indentation, below a guard too.
          push(gap(&first), Some(node.text_range().start as usize));
        }
      }
      // A `let`'s `else`, and its block's `{`.
      K::LetElse => {
        if let Some(keyword) = node
          .children_with_tokens_iter()
          .find(|e| e.kind() == K::ElseKw)
        {
          push(gap(&keyword), None);
        }
        if next_line
          && let Some(block) = node.children_iter().find(|c| c.kind() == K::BlockExpr)
          && let Some(open) = brace(&block)
        {
          push(gap(&open), None);
        }
      }
      K::WhileExpr | K::ForExpr | K::LoopExpr if next_line => {
        if let Some(body) = node.children_iter().rfind(|c| c.kind() == K::BlockExpr)
          && let Some(open) = brace(&body)
        {
          push(gap(&open), None);
        }
      }
      _ => {}
    }
  }
  for call in root.descendant_nodes_where(|kind| kind == K::MacroCall) {
    let Some(tree) = call.children_iter().find(|c| c.kind() == K::TokenTree) else {
      continue;
    };
    let whole = tree.text();
    if !matches!(whole.as_bytes().first(), Some(b'(' | b'[')) || skipped_within(&call) {
      continue;
    }
    let inner = &whole[1..whole.len() - 1];
    let prefix = "fn __mto() {\n__mto_macro(";
    let wrapped = format!("{}{});\n}}\n", prefix, inner);
    let parsed = crate::syntax::parse(&wrapped, edition);
    if !parsed.errors().is_empty() {
      continue;
    }
    let from = tree.text_range().start as isize + 1 + shift - prefix.len() as isize;
    control_gaps(&parsed.syntax_node(), &wrapped, from, style, edition, gaps);
  }
}

/// [`aligned_fields`]'s pads (where, how many spaces) and commas in `root`, whose text is
/// `text`, at `text`'s offsets moved by `shift`; into macros' arguments that read as a
/// call's too, as [`control_gaps`] goes.
#[allow(clippy::too_many_arguments)]
fn field_edits(
  root: &SyntaxNode,
  text: &str,
  shift: isize,
  config: &FormatConfig,
  edition: Edition,
  off: &[(usize, usize)],
  inserts: &mut Vec<(usize, usize)>,
  commas: &mut Vec<usize>,
) {
  let column = |at: usize| {
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[line_start..at]
      .chars()
      .map(|c| if c == '\t' { config.tab_spaces } else { 1 })
      .sum::<usize>()
  };
  let line_end = |at: usize| text[at..].find('\n').map_or(text.len(), |i| at + i);
  for list in
    root.descendant_nodes_where(|kind| matches!(kind, K::RecordFieldList | K::RecordExprFieldList))
  {
    let range = list.text_range();
    let (start, end) = (range.start as usize, range.end as usize);
    if !text[start..end].contains('\n')
      || skipped_within(&list)
      || off
        .iter()
        .any(|&(s, e)| (s as isize) < end as isize + shift && (start as isize + shift) < e as isize)
    {
      continue;
    }
    let literal = list.kind() == K::RecordExprFieldList;
    let fields: Vec<SyntaxNode> = list
      .children_iter()
      .filter(|c| matches!(c.kind(), K::RecordField | K::RecordExprField))
      .collect();
    // A literal with a `..base`, or a field written `name` alone, is not lined up
    // (`struct_lit_can_be_aligned`).
    if literal
      && (list
        .children_with_tokens_iter()
        .any(|e| e.kind() == K::Dot2)
        || fields
          .iter()
          .any(|f| !f.children_with_tokens_iter().any(|e| e.kind() == K::Colon)))
    {
      continue;
    }
    // Each field: the width before its type, and where its type starts when on that line.
    struct Field {
      prefix: usize,
      value: Option<usize>,
      line_width: usize,
    }
    let measure = |field: &SyntaxNode| -> Option<Field> {
      let name = field
        .children_iter()
        .find(|c| matches!(c.kind(), K::Name | K::NameRef))?;
      let name_end = name.text_range().end as usize;
      // From the field's first token after its attributes and doc comments, a comment
      // before it aside.
      let first = field
        .children_with_tokens_iter()
        .find(|e| e.kind() != K::Attr && !e.kind().is_trivia() && !super::items::is_doc(e.kind()))?
        .text_range()
        .start as usize;
      if text[first..name_end].contains('\n') {
        return None;
      }
      let indent = column(first);
      let colon = field
        .children_with_tokens_iter()
        .find(|e| e.kind() == K::Colon)
        .map(|e| e.text_range().end as usize);
      let prefix_end = if literal { name_end } else { colon? };
      let value = colon.and_then(|colon| {
        let after = colon + text[colon..].len() - text[colon..].trim_start_matches(' ').len();
        (!text[colon..after].contains('\n') && after < line_end(colon)).then_some(after)
      });
      Some(Field {
        prefix: column(prefix_end) - indent,
        value,
        line_width: column(line_end(name_end)),
      })
    };
    // The last field ends with a comma, in a macro's arguments too: the list is written as
    // `trailing_comma` says.
    if literal
      && let Some(last) = fields.last()
      && list
        .children_with_tokens_iter()
        .skip_while(|e| e.text_range().start < last.text_range().end)
        .find(|e| !e.kind().is_trivia())
        .is_some_and(|e| e.kind() == K::RBrace)
    {
      commas.push((last.text_range().end as isize + shift) as usize);
    }
    let mut group: Vec<(SyntaxNode, Field)> = Vec::new();
    let mut flush = |group: &mut Vec<(SyntaxNode, Field)>| {
      let widest = group.iter().map(|(_, f)| f.prefix).max().unwrap_or(0);
      let narrowest = group.iter().map(|(_, f)| f.prefix).min().unwrap_or(0);
      if group.len() > 1 && widest - narrowest <= config.struct_field_align_threshold {
        for (_, field) in group.iter() {
          let pad = widest - field.prefix;
          if let Some(at) = field.value
            && pad > 0
            && field.line_width + pad <= config.max_width
          {
            inserts.push(((at as isize + shift) as usize, pad));
          }
        }
      }
      group.clear();
    };
    let mut aligned = true;
    for (i, field) in fields.iter().enumerate() {
      let Some(measured) = measure(field) else {
        aligned = false;
        break;
      };
      let skipped = field
        .children_iter()
        .filter_map(crate::syntax::ast::Attr::cast)
        .any(|a| a.is_rustfmt_skip());
      group.push((field.clone(), measured));
      // A blank line before the next field ends the group, as a skipped field does.
      let blank = fields.get(i + 1).is_some_and(|next| {
        let gap = &text[field.text_range().end as usize..next.text_range().start as usize];
        let lines: Vec<&str> = gap.split('\n').collect();
        lines.len() > 2
          && lines[1..lines.len() - 1]
            .iter()
            .any(|l| l.trim().is_empty())
      });
      if blank || skipped {
        flush(&mut group);
      }
    }
    if aligned {
      flush(&mut group);
    }
  }
  for call in root.descendant_nodes_where(|kind| kind == K::MacroCall) {
    let Some(tree) = call.children_iter().find(|c| c.kind() == K::TokenTree) else {
      continue;
    };
    let whole = tree.text();
    if !matches!(whole.as_bytes().first(), Some(b'(' | b'[')) || skipped_within(&call) {
      continue;
    }
    let inner = &whole[1..whole.len() - 1];
    let prefix = "fn __mto() {\n__mto_macro(";
    let wrapped = format!("{}{});\n}}\n", prefix, inner);
    let parsed = crate::syntax::parse(&wrapped, edition);
    if !parsed.errors().is_empty() {
      continue;
    }
    let from = tree.text_range().start as isize + 1 + shift - prefix.len() as isize;
    field_edits(
      &parsed.syntax_node(),
      &wrapped,
      from,
      config,
      edition,
      off,
      inserts,
      commas,
    );
  }
}

/// A float literal as `float_literal_trailing_zero` writes it: its integer part, then its
/// fraction, exponent and suffix, the `.0` added or dropped (`rewrite_float_lit`).
fn float_literal(
  written: &str,
  style: crate::options::format::FloatTrailingZero,
) -> Option<String> {
  use crate::options::format::FloatTrailingZero;
  let (symbol, suffix) = match written.find('f') {
    Some(at) => written.split_at(at),
    None => (written, ""),
  };
  let digits = |s: &str| {
    s.bytes()
      .take_while(|b| b.is_ascii_digit() || *b == b'_')
      .count()
  };
  let integer_end = digits(symbol);
  if integer_end == 0 {
    return None;
  }
  let integer = &symbol[..integer_end];
  let mut rest = &symbol[integer_end..];
  let fraction = match rest.strip_prefix('.') {
    Some(after) => {
      let end = digits(after);
      rest = &after[end..];
      (end > 0).then(|| &after[..end])
    }
    None => None,
  };
  let exponent = (!rest.is_empty()).then_some(rest);
  if exponent.is_some_and(|e| !e.starts_with(['e', 'E'])) {
    return None;
  }
  let postfix = exponent.is_some() || !suffix.is_empty();
  let nonzero = fraction.is_some_and(|f| f.bytes().any(|b| b != b'0' && b != b'_'));
  let (period, keep_fraction) = match style {
    FloatTrailingZero::Preserve => return None,
    FloatTrailingZero::Always => (true, true),
    FloatTrailingZero::IfNoPostfix => (nonzero || !postfix, nonzero || !postfix),
    FloatTrailingZero::Never => (nonzero || !postfix, nonzero),
  };
  Some(format!(
    "{}{}{}{}{}",
    integer,
    if period { "." } else { "" },
    if keep_fraction {
      fraction.unwrap_or("0")
    } else {
      ""
    },
    exponent.unwrap_or(""),
    suffix
  ))
}

/// Whether a run holds a comment that sorting would misplace: one inside a declaration's
/// code, or between two declarations outside them. The comments the run's first item
/// carries before it stay where they are (they come before the run, which the standard
/// formatter sorts without them); so do those its last carries after its code. Another
/// item's comment after its `;`, or a block comment before it on its line, moves with it;
/// one on a line of its own before it starts a run ([`runs`]).
fn has_comment(run: &[SyntaxNode]) -> bool {
  let comment = |e: &SyntaxElement| matches!(e, SyntaxElement::Token(t) if matches!(t.kind(), K::LineComment | K::BlockComment));
  // One inside a `use`'s braces moves with it, and is laid out with its list; so does a block
  // comment before its `;`, which goes after it, and one between its attributes and its code
  // (a `use`'s, a module's, an `extern crate`'s), kept with them.
  let commented = |item: &SyntaxNode, from: usize, to: usize| {
    let body = match item.kind() {
      K::Use => super::imports::body_start(item),
      // A module's or `extern crate`'s between its attributes and its code too.
      K::Module | K::ExternCrate => super::items::header_start(item),
      _ => None,
    }
    .unwrap_or(from);
    item.descendants_with_tokens_iter().any(|e| {
      matches!(&e, SyntaxElement::Token(t)
        if matches!(t.kind(), K::LineComment | K::BlockComment)
          // One inside a module's attribute (its arguments) counts wherever it is.
          && (t.text_range().start as usize >= body
            || item.kind() != K::Use
              && (t.text_range().start as usize) >= from
              && &t.parent() != item)
          && (t.text_range().start as usize) < to
          && !(item.kind() == K::Use
            && (t.parent().kind() == K::UseTreeList
              || t.kind() == K::BlockComment
                && !t.text().contains('\n')
                && &t.parent() == item
                && item
                  .children_iter()
                  .find(|c| c.kind() == K::UseTree)
                  .is_some_and(|tree| tree.text_range().end <= t.text_range().start))))
    })
  };
  run
    .iter()
    .any(|item| commented(item, code_start(item), code_end(item)))
    || run.windows(2).any(|pair| {
      let mut next = pair[0].next_sibling_or_token();
      while let Some(element) = next {
        if let SyntaxElement::Node(n) = &element
          && n == &pair[1]
        {
          return false;
        }
        if comment(&element) {
          return true;
        }
        next = match element {
          SyntaxElement::Token(t) => t.next_sibling_or_token(),
          SyntaxElement::Node(n) => n.next_sibling_or_token(),
        };
      }
      false
    })
}

/// Whether a derive's argument, its spaces collapsed, reads as a meta item: a path, alone or
/// followed by arguments in parentheses or by `= value`.
fn meta_item(item: &str) -> bool {
  let path = match (item.find('('), item.find('=')) {
    (Some(open), _) if item.ends_with(')') => &item[..open],
    (None, Some(equals)) => &item[..equals],
    (None, None) => item,
    _ => return false,
  };
  let path = path.trim();
  let path = path.strip_prefix("::").unwrap_or(path);
  !path.is_empty()
    && path.split("::").all(|segment| {
      let segment = segment.trim();
      let name = segment.strip_prefix("r#").unwrap_or(segment);
      name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
    })
}

fn module_name(node: &SyntaxNode) -> Option<String> {
  let name = node.children_iter().find(|c| c.kind() == K::Name)?;
  Some(name.text().to_string())
}

/// The regions `// mto: off` and `// mto: on` leave as written ([`marker::scan`]).
fn off_regions(root: &SyntaxNode) -> Vec<(usize, usize)> {
  marker::scan(root, root.text()).regions
}

/// Whether an item carries a comment before it on a line of its own.
fn own_line_comment(item: &SyntaxNode) -> bool {
  let mut comment = false;
  for element in item.children_with_tokens_iter() {
    match element.kind() {
      K::LineComment | K::BlockComment => comment = true,
      K::Whitespace => {
        if comment && matches!(&element, SyntaxElement::Token(t) if t.text().contains('\n')) {
          return true;
        }
      }
      _ => return false,
    }
  }
  false
}

/// Whether a match arm's body is a block with nothing before its `{`, whose comma after it
/// is dropped: the arm then ends without one too.
fn plain_block_body(arm: &SyntaxNode) -> bool {
  arm
    .children_iter()
    .rfind(|c| c.kind() != K::Attr)
    .is_some_and(|body| {
      body.kind() == K::BlockExpr && body.first_token().is_some_and(|t| t.kind() == K::LBrace)
    })
}

/// The runs of a container: declarations of one sortable kind, one after another with no
/// blank line between them. With blank lines removed (`blank_lines_upper_bound = 0`) they
/// end no run, so that a second run sorts nothing more.
fn runs(container: &SyntaxNode, blank_ends_run: bool) -> Vec<Vec<SyntaxNode>> {
  let mut runs: Vec<Vec<SyntaxNode>> = Vec::new();
  let mut current: Vec<SyntaxNode> = Vec::new();
  for element in container.children_with_tokens_iter() {
    match &element {
      SyntaxElement::Token(t) if t.kind() == K::Whitespace => {
        if blank_ends_run && t.text().matches('\n').count() > 1 {
          runs.push(std::mem::take(&mut current));
        }
      }
      // A comment between declarations ends the run, as one carried before a declaration
      // on a line of its own does.
      SyntaxElement::Token(t) if matches!(t.kind(), K::LineComment | K::BlockComment) => {
        runs.push(std::mem::take(&mut current));
      }
      SyntaxElement::Token(t) if t.kind().is_trivia() => {}
      SyntaxElement::Node(n) if sortable(n) => {
        // A comment on a line of its own before it ends the run before.
        if current.first().is_some_and(|f| f.kind() != n.kind()) || own_line_comment(n) {
          runs.push(std::mem::take(&mut current));
        }
        current.push(n.clone());
      }
      _ => runs.push(std::mem::take(&mut current)),
    }
  }
  runs.push(current);
  runs.retain(|r| !r.is_empty());
  runs
}

/// Whether a node is inside an item, statement or expression marked `#[rustfmt::skip]`.
fn skipped_within(node: &SyntaxNode) -> bool {
  node.ancestors_iter().any(|owner| {
    owner
      .children_iter()
      .filter_map(Attr::cast)
      .any(|a| a.is_rustfmt_skip())
  })
}

#[cfg(test)]
mod tests;
