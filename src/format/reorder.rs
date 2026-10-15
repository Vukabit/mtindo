//! Runs of `use` declarations and of `mod name;` declarations sorted as the standard
//! formatter sorts them (`reorder_imports`, `reorder_modules`), before the file is formatted.
//!
//! A run is the declarations of one kind that follow each other in a file, module or block
//! with no blank line between them; a comment does not end a run. A declaration marked
//! `#[macro_use]` or skipped ends a run and stays where it is. Imports sort by their use
//! trees ([`crate::order::UseTree`]), visibility and attributes aside; modules by
//! name. A run holding a comment is left in its order, since the standard formatter moves
//! such comments in ways that depend on more than the order; so is a run in a `// mto: off`
//! region. The meaning check compares each run of imports as a set (`use-trees`) and
//! modules by their names (`import-and-module-order`).

use SyntaxKind as K;

use crate::options::format::FormatConfig;
use crate::order::{UseTree, version_sort};
use crate::syntax::ast::Attr;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode};

/// `text` with its runs sorted, or `None` when nothing moves.
pub fn reorder(
  root: &SyntaxNode,
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
  for node in root.descendant_nodes_where(containers) {
    for run in runs(&node, config.blank_lines_upper_bound > 0) {
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
      for (slot, &from) in order.iter().enumerate() {
        moves.push((
          start_of(slot),
          run[slot].text_range().end as usize,
          text[start_of(from)..run[from].text_range().end as usize].to_string(),
        ));
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

/// Whether a run holds a comment, in a declaration or between two, doc comments aside.
/// The comments the run's first item carries before it stay where they are: they come
/// before the run, which the standard formatter sorts without them.
fn has_comment(run: &[SyntaxNode]) -> bool {
  let comment = |e: &SyntaxElement| matches!(e, SyntaxElement::Token(t) if matches!(t.kind(), K::LineComment | K::BlockComment));
  let commented = |item: &SyntaxNode, from: usize| {
    item.token_texts().any(|(kind, _, range)| {
      range.start as usize >= from && matches!(kind, K::LineComment | K::BlockComment)
    })
  };
  commented(&run[0], code_start(&run[0]))
    || run[1..]
      .iter()
      .any(|item| item.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment)))
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

fn module_name(node: &SyntaxNode) -> Option<String> {
  let name = node.children_iter().find(|c| c.kind() == K::Name)?;
  Some(name.text().to_string())
}

/// The ranges between a `// mto: off` comment and the next `// mto: on` (or the end).
fn off_regions(root: &SyntaxNode) -> Vec<(usize, usize)> {
  let mut regions = Vec::new();
  let mut start: Option<usize> = None;
  for (_, text, range) in root
    .token_texts()
    .filter(|&(kind, _, _)| kind == K::LineComment)
  {
    match text.trim() {
      "// mto: off" if start.is_none() => start = Some(range.start as usize),
      "// mto: on" => {
        if let Some(s) = start.take() {
          regions.push((s, range.end as usize));
        }
      }
      _ => {}
    }
  }
  if let Some(s) = start {
    regions.push((s, usize::MAX));
  }
  regions
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
      // Comments stay within a run (the run is then left in its order).
      SyntaxElement::Token(t) if t.kind().is_trivia() => {}
      SyntaxElement::Node(n) if sortable(n) => {
        if current.first().is_some_and(|f| f.kind() != n.kind()) {
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

/// A `use`, or a `mod name;`, with no `#[macro_use]` and not skipped.
fn sortable(node: &SyntaxNode) -> bool {
  let declaration = match node.kind() {
    K::Use => true,
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

#[cfg(test)]
mod tests;
