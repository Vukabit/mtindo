//! `use` declarations laid out as the Rust style lays them out: a list on one line when it
//! fits, else, under the default `imports_layout`, its items filling lines one level in,
//! where a path starts a line of its own in a list holding another list, and so does the item
//! after it.
//!
//! The standard formatter also rewrites trees (`a::{b}` to `a::b`, lists sorted, duplicates
//! dropped); a declaration that would need one of those, a list of fewer than two items, is
//! left as written, as is one holding a comment. Its attributes are laid out on their own.

use SyntaxKind as K;

use super::expr::flat;
use super::shape::Spacing;
use crate::options::format::{FormatConfig, ImportsLayout};
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, parse_tokens};
use crate::text::unicode::str_width;

/// Where a declaration's own text starts: its visibility or `use`, after its attributes and
/// the comments before it.
pub fn body_start(node: &SyntaxNode) -> Option<usize> {
  node
    .children_with_tokens_iter()
    .find(|e| matches!(e.kind(), K::Visibility | K::UseKw))
    .map(|e| e.text_range().start as usize)
}

/// Where a declaration's code ends: before a comment after its `;`, which is not its own.
pub fn code_end(node: &SyntaxNode) -> usize {
  node
    .children_with_tokens_iter()
    .rfind(|e| !e.kind().is_trivia())
    .map_or(node.text_range().end as usize, |e| {
      e.text_range().end as usize
    })
}

/// The declaration laid out, starting at column `indent` on a line of its own, its later
/// lines indented in full; `None` when it is not one this module takes.
pub fn rewrite_use(node: &SyntaxNode, indent: usize, config: &FormatConfig) -> Option<String> {
  if config.hard_tabs || config.tab_spaces == 0 || node.kind() != K::Use {
    return None;
  }
  // From the visibility or `use` on; the attributes and comments before are laid out on
  // their own.
  let start = body_start(node)?;
  let end = code_end(node);
  let body: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(|e| {
      let at = e.text_range().start as usize;
      at >= start && at < end
    })
    .collect();
  // A block comment on its line between the tree and the `;` goes after the `;`
  // (`use a::b; /* c */`).
  let tree_end = node
    .children_iter()
    .find(|c| c.kind() == K::UseTree)
    .map_or(0, |t| t.text_range().end as usize);
  let moved: Vec<String> = body
    .iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t)
        if t.kind() == K::BlockComment
          && !t.text().contains('\n')
          && t.text_range().start as usize >= tree_end =>
      {
        Some(t.text().to_string())
      }
      _ => None,
    })
    .collect();
  // Comments only among a list's items, which move with them (placed by the list layout).
  let comment = |e: &SyntaxElement| match e {
    SyntaxElement::Token(t) => {
      t.kind().is_trivia()
        && t.kind() != K::Whitespace
        && !(t.kind() == K::BlockComment
          && !t.text().contains('\n')
          && t.text_range().start as usize >= tree_end)
    }
    SyntaxElement::Node(n) => n.descendants_with_tokens_iter().any(|d| {
      matches!(&d, SyntaxElement::Token(t)
        if t.kind().is_trivia()
          && t.kind() != K::Whitespace
          && (t.parent().kind() != K::UseTreeList || t.text().contains('\n')))
    }),
  };
  if body.iter().any(comment) {
    return None;
  }
  let mut tree = node.children_iter().find(|c| c.kind() == K::UseTree)?;
  // Reordered, the tree is normalized as the standard formatter normalizes it (`a::{b}` is
  // `a::b`, `a::self` is `a`), then laid out read again.
  let mut normalized = None;
  // Not one with a `::` starting a tree inside a list, which the normalized tree drops.
  let nested_root = tree
    .descendant_nodes_where(|kind| kind == K::UseTree)
    .any(|t| {
      t != tree
        && t
          .first_token()
          .is_some_and(|first| first.kind() == K::Colon2)
    });
  // Read again from its text, a tree would lose its comments: one with them is not.
  let commented = tree.subtree_has(|kind| matches!(kind, K::LineComment | K::BlockComment));
  if config.reorder_imports && !nested_root && !commented {
    let global = super::file::edition() >= Edition::E2018;
    let version = config.style_edition >= 2024;
    let written = crate::order::UseTree::from_syntax(&tree, global, version);
    // One importing nothing, or `::{a}`, as written (the first goes before formatting:
    // `reorder::empty_uses`).
    let text = written.text();
    if !written.is_empty() && !written.unbraced_root() && text != flat(&tree, Spacing::of(config)) {
      let source = format!("use {};", text);
      let parsed = crate::syntax::parse(&source, Edition::E2021);
      if !parsed.errors().is_empty() {
        return None;
      }
      tree = parsed
        .syntax_node()
        .descendant_nodes_where(|kind| kind == K::UseTree)
        .next()?;
      normalized = Some(tree.clone());
    }
  }
  let spacing = Spacing::of(config);
  // `pub use `, everything before the tree.
  let head = match node.children_iter().find(|c| c.kind() == K::Visibility) {
    Some(visibility) => format!("{} use ", flat(&visibility, spacing)),
    None => "use ".to_string(),
  };
  // 1 = `;`
  let width = config
    .max_width
    .checked_sub(indent + str_width(&head) + 1)?;
  let mut text = format!("{}{};", head, tree_text(&tree, indent, width, config)?);
  for comment in &moved {
    text.push(' ');
    text.push_str(comment);
  }
  match normalized {
    // The tokens of the tree as normalized, the head's as written.
    Some(tree) => {
      let mut ours: Vec<SyntaxElement> = body
        .into_iter()
        .filter(|e| e.kind() != K::UseTree)
        .collect();
      ours.push(SyntaxElement::Node(tree));
      same_tokens(&ours, &text).then_some(text)
    }
    None => same_tokens(&body, &text).then_some(text),
  }
}

/// The items of a list, its trees.
fn items(list: &SyntaxNode) -> Vec<SyntaxNode> {
  list
    .children_iter()
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
  let mut items = items(list);
  // One item alone keeps its braces only with a comment (`{self // why}`), or when it is
  // `self` (`{self as name}`).
  let commented = list
    .children_with_tokens_iter()
    .any(|e| matches!(e.kind(), K::LineComment | K::BlockComment));
  let lone_self = items.len() == 1 && items[0].first_token().is_some_and(|t| t.text() == "self");
  if items.is_empty() || items.len() < 2 && !commented && !lone_self {
    return None;
  }
  // In the order the standard formatter sorts a nested list in (`UseTree::normalize`).
  if config.reorder_imports {
    let version = config.style_edition >= 2024;
    let trees: Vec<crate::order::UseTree> = items
      .iter()
      .map(|item| crate::order::UseTree::from_syntax(item, true, version))
      .collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| trees[a].compare(&trees[b], version));
    items = order.into_iter().map(|i| items[i].clone()).collect();
  }
  let inner_indent = indent + config.tab_spaces;
  // 1 = `,`
  let inner_width = config.max_width.checked_sub(inner_indent + 1)?;
  let texts = items
    .iter()
    .map(|item| tree_text(item, inner_indent, inner_width, config))
    .collect::<Option<Vec<_>>>()?;
  // With comments among the items: one per line, each item's comments moved with it.
  if commented {
    let written = self::items(list);
    let comments = super::lists::list_comments(list, &written)?;
    let comments: Vec<super::lists::ItemComments> = items
      .iter()
      .map(|item| {
        let at = written.iter().position(|w| w == item)?;
        Some(comments[at].clone())
      })
      .collect::<Option<_>>()?;
    // Block comments on a line only: on one line when it fits there.
    let line_comment = |c: &Option<String>| {
      c.as_ref()
        .is_some_and(|c| c.trim_start().starts_with("//") || c.contains('\n'))
    };
    if !comments
      .iter()
      .any(|c| line_comment(&c.pre) || line_comment(&c.post))
      && texts.iter().all(|t| !t.contains('\n'))
    {
      let joined: Vec<String> = texts
        .iter()
        .zip(&comments)
        .map(|(t, c)| {
          let mut item = String::new();
          if let Some(pre) = &c.pre {
            item.push_str(pre.trim());
            item.push(' ');
          }
          item.push_str(t);
          if let Some(post) = &c.post {
            item.push(' ');
            item.push_str(post.trim());
          }
          item
        })
        .collect();
      let one_line = format!("{{{}}}", joined.join(", "));
      if str_width(&one_line) <= width {
        return Some(one_line);
      }
    }
    let body = super::lists::write_vertical(
      &texts,
      &comments,
      inner_indent,
      inner_width,
      config.max_width,
      true,
      false,
    )?;
    let pad = " ".repeat(inner_indent);
    return Some(format!("{{\n{}{}\n{}}}", pad, body, " ".repeat(indent)));
  }
  // A list holding a list of two or more, or of `self` (`a::{self}`), never stays on one
  // line.
  let nested = items.iter().any(|item| {
    item.children_iter().any(|c| {
      c.kind() == K::UseTreeList && {
        let inner = self::items(&c);
        inner.len() > 1
          || inner.len() == 1 && inner[0].first_token().is_some_and(|t| t.text() == "self")
      }
    })
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

/// Whether `text` holds the tokens of `body`, commas aside, in any order (a nested list is
/// sorted).
fn same_tokens(body: &[SyntaxElement], text: &str) -> bool {
  let code = |&(kind, _): &(SyntaxKind, &str)| !kind.is_trivia() && kind != K::Comma;
  let mut theirs: Vec<(SyntaxKind, &str)> = Vec::new();
  for element in body {
    match element {
      SyntaxElement::Node(n) => theirs.extend(n.token_texts().map(|(kind, text, _)| (kind, text))),
      SyntaxElement::Token(t) => theirs.push((t.kind(), t.text())),
    }
  }
  let mut theirs: Vec<&str> = theirs
    .into_iter()
    .filter(code)
    .map(|(_, text)| text)
    .collect();
  let wrapped = format!("{}\nfn f() {{}}", text);
  let mut ours: Vec<&str> = parse_tokens(&wrapped, Edition::E2021)
    .into_iter()
    .take_while(|&(kind, _)| kind != K::FnKw)
    .filter(code)
    .map(|(_, text)| text)
    .collect();
  // As a multiset: a nested list is sorted.
  theirs.sort_unstable();
  ours.sort_unstable();
  theirs == ours
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
  match tree.children_iter().find(|c| c.kind() == K::UseTreeList) {
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
