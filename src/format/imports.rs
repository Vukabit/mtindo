//! `use` declarations laid out as the Rust style lays them out: a list on one line when it
//! fits, else, under the default `imports_layout`, its items filling lines one level in,
//! where a path starts a line of its own in a list holding another list, and so does the item
//! after it.
//!
//! The standard formatter also rewrites trees (`a::{b}` to `a::b`, lists sorted, duplicates
//! dropped); a declaration that would need one of those, a list of fewer than two items, is
//! left as written, as is one holding a comment or an attribute.

use SyntaxKind as K;

use super::expr::flat;
use super::shape::Spacing;
use super::stmt::same_tokens;
use crate::options::format::{FormatConfig, ImportsLayout};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};
use crate::text::unicode::str_width;

/// The declaration laid out, starting at column `indent` on a line of its own, its later
/// lines indented in full; `None` when it is not one this module takes.
pub fn rewrite_use(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<String> {
  if config.hard_tabs || config.tab_spaces == 0 || !takes(node) {
    return None;
  }
  let tree = node
    .children()
    .into_iter()
    .find(|c| c.kind() == K::UseTree)?;
  // `pub use `, everything before the tree.
  let head = format!(
    "{} ",
    flat(node, Spacing::of(config))
      .strip_suffix(&format!("{};", flat(&tree, Spacing::of(config))))?
      .trim_end()
  );
  // 1 = `;`
  let width = config
    .max_width
    .checked_sub(indent + str_width(&head) + 1)?;
  let text = format!("{}{};", head, tree_text(&tree, indent, width, config)?);
  same_tokens(node, &text).then_some(text)
}

/// The items of a list, its trees.
fn items(list: &SyntaxNode) -> Vec<SyntaxNode> {
  list
    .children()
    .into_iter()
    .filter(|c| c.kind() == K::UseTree)
    .collect()
}

/// A list's text: `{a, b}` on one line within `width`, else one level past `indent`.
fn list_text(
  list: &SyntaxNode,
  indent: usize,
  width: usize,
  config: &FormatConfig,
) -> Option<String> {
  let items = items(list);
  if items.len() < 2 {
    return None;
  }
  let inner_indent = indent + config.tab_spaces;
  // 1 = `,`
  let inner_width = config.max_width.checked_sub(inner_indent + 1)?;
  let texts = items
    .iter()
    .map(|item| tree_text(item, inner_indent, inner_width, config))
    .collect::<Option<Vec<_>>>()?;
  // A list holding a list of two or more never stays on one line.
  let nested = items.iter().any(|item| {
    item
      .children()
      .iter()
      .any(|c| c.kind() == K::UseTreeList && self::items(c).len() > 1)
  });
  let one_line: usize = texts.iter().map(|t| str_width(t)).sum::<usize>() + 2 * (texts.len() - 1);
  // 2 = `{}`; the standard formatter keeps two columns more free, found by measuring it.
  let fits = !nested && one_line + 2 + 2 <= width && texts.iter().all(|t| !t.contains('\n'));
  let layout = config.imports_layout;
  if fits && layout != ImportsLayout::Vertical {
    return Some(format!("{{{}}}", texts.join(", ")));
  }
  // A list that does not fit on its line under `Horizontal` is left as written.
  if layout == ImportsLayout::Horizontal {
    return None;
  }
  let pad = " ".repeat(inner_indent);
  if layout != ImportsLayout::Mixed {
    let lines: String = texts.iter().map(|t| format!("\n{}{},", pad, t)).collect();
    return Some(format!("{{{}\n{}}}", lines, " ".repeat(indent)));
  }
  let mut out = String::new();
  let mut line = 0;
  let mut after_path = false;
  let mut wrapped = false;
  for (i, text) in texts.iter().enumerate() {
    // An item spanning lines is measured whole, as the standard formatter measures it. The
    // last item's comma is not counted until a line has been broken, so the last item of a
    // single row alone may reach the full width.
    let taken = text.chars().count() + usize::from(i + 1 < texts.len() || wrapped);
    let path = text.contains("::");
    if (line > 0 && line + 1 + taken > inner_width) || (nested && (after_path || (i > 0 && path))) {
      out.push('\n');
      out.push_str(&pad);
      line = 0;
      wrapped = true;
    } else if line > 0 {
      out.push(' ');
      line += 1;
    }
    out.push_str(text);
    out.push(',');
    line += taken;
    after_path = path;
  }
  Some(format!("{{\n{}{}\n{}}}", pad, out, " ".repeat(indent)))
}

/// Whether the declaration is one this module lays out: no attribute and no comment.
fn takes(node: &SyntaxNode) -> bool {
  node.kind() == K::Use
    && node.descendants_with_tokens().iter().all(|e| match e {
      SyntaxElement::Token(t) => !t.kind().is_trivia() || t.kind() == K::Whitespace,
      SyntaxElement::Node(n) => n.kind() != K::Attr,
    })
}

/// A tree's text, its path up to the last segment within `width`: a list after it laid out
/// in what is left, a last segment running over as the standard formatter lets it.
fn tree_text(
  tree: &SyntaxNode,
  indent: usize,
  width: usize,
  config: &FormatConfig,
) -> Option<String> {
  let whole = flat(tree, Spacing::of(config));
  match tree
    .children()
    .into_iter()
    .find(|c| c.kind() == K::UseTreeList)
  {
    Some(list) => {
      let head = whole
        .strip_suffix(&flat(&list, Spacing::of(config)))?
        .to_string();
      let rest = width.checked_sub(str_width(&head))?;
      Some(format!(
        "{}{}",
        head,
        list_text(&list, indent, rest, config)?
      ))
    }
    None => {
      let head = whole.rfind("::").map_or(0, |at| at + 2);
      (str_width(&whole[..head]) <= width).then_some(whole)
    }
  }
}

#[cfg(test)]
mod tests;
