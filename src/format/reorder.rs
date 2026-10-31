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
