//! Comments among a list's items (a call's arguments), placed as the standard formatter's
//! `itemize_list` and `write_list` place them: a comment before an item (a pre-comment) on
//! lines of its own above it, or on its line when it is a block comment written there; a
//! comment after an item's comma on its line (a post-comment), aligned with those of the
//! items around it that have one. A list holding comments is laid out one item per line.

use crate::syntax::SyntaxNode;
use crate::text::unicode::str_width;

/// An item's comments.
#[derive(Clone, Debug, Default)]
pub struct ItemComments {
  pub pre: Option<String>,
  /// Whether the pre-comment is a block comment ending on the item's line.
  pub pre_same_line: bool,
  pub post: Option<String>,
  /// Whether a blank line follows the item.
  pub new_lines: bool,
}

/// The comments around each of `items`, the children of `list` between its delimiters;
/// `None` when there are none, or when they hold what is not placed here (a block comment
/// spanning lines).
pub fn list_comments(list: &SyntaxNode, items: &[SyntaxNode]) -> Option<Vec<ItemComments>> {
  // No comment anywhere in the list (a walk that makes no nodes): none around its items.
  if items.is_empty()
    || !list.subtree_has(|kind| {
      matches!(
        kind,
        crate::syntax::SyntaxKind::LineComment | crate::syntax::SyntaxKind::BlockComment
      )
    })
  {
    return None;
  }
  // An item's code, without the comments attached to either end of it; from the
  // attributes written right before it, which a list holds beside its element.
  let code = |item: &SyntaxNode| {
    let mut tokens = item
      .descendants_with_tokens_iter()
      .filter(|e| matches!(e, crate::syntax::SyntaxElement::Token(t) if !t.kind().is_trivia()));
    let first = tokens.next()?.text_range();
    let last = tokens.last().map_or(first.end, |e| e.text_range().end);
    let mut start = first.start as usize;
    let mut before = item.prev_sibling_or_token();
    while let Some(element) = before {
      match &element {
        crate::syntax::SyntaxElement::Token(t)
          if t.kind() == crate::syntax::SyntaxKind::Whitespace => {}
        crate::syntax::SyntaxElement::Node(n) if n.kind() == crate::syntax::SyntaxKind::Attr => {
          start = n.text_range().start as usize;
        }
        _ => break,
      }
      before = match &element {
        crate::syntax::SyntaxElement::Token(t) => t.prev_sibling_or_token(),
        crate::syntax::SyntaxElement::Node(n) => n.prev_sibling_or_token(),
      };
    }
    Some((start, last as usize))
  };
  let spans: Vec<(usize, usize)> = items.iter().map(code).collect::<Option<_>>()?;
  spanned_comments(list, &spans)
}

/// The spaces `write_vertical` puts before each item's comment after it on its line, for
/// the items laid out one per line elsewhere (a struct's fields).
pub fn post_comment_spaces(
  texts: &[String],
  comments: &[ItemComments],
  indent: usize,
  max_width: usize,
  separator: bool,
) -> Vec<Option<usize>> {
  let mut spaces = vec![None; texts.len()];
  let width = max_width.saturating_sub(indent);
  let _ = write_list(
    texts,
    comments,
    indent,
    width,
    max_width,
    separator,
    false,
    true,
    separator,
    &mut spaces,
  );
  spaces
}

/// [`list_comments`] for items given by where their code starts and ends (a struct
/// literal's `..base` starts at its `..`).
pub fn spanned_comments(list: &SyntaxNode, spans: &[(usize, usize)]) -> Option<Vec<ItemComments>> {
  if spans.is_empty() {
    return None;
  }
  let inside = |at: usize| spans.iter().any(|&(lo, hi)| lo <= at && at < hi);
  let has = list.token_texts().any(|(kind, _, range)| {
    matches!(
      kind,
      crate::syntax::SyntaxKind::LineComment | crate::syntax::SyntaxKind::BlockComment
    ) && !inside(range.start as usize)
  });
  if !has {
    return None;
  }
  let text = list.text().to_string();
  let base = list.text_range().start as usize;
  let open_end = list.first_token()?.text_range().end as usize - base;
  let close_start = list.last_token()?.text_range().start as usize - base;
  let mut prev_end = open_end;
  let mut out = Vec::with_capacity(spans.len());
  for (i, &(lo, hi)) in spans.iter().enumerate() {
    let (lo, hi) = (lo - base, hi - base);
    let pre_snippet = text.get(prev_end..lo)?;
    let (pre, pre_same_line) = extract_pre_comment(pre_snippet);
    let is_last = i + 1 == spans.len();
    let next_start = match spans.get(i + 1) {
      Some(&(next, _)) => next - base,
      None => close_start,
    };
    let post_snippet = text.get(hi..next_start)?;
    let comment_end = get_comment_end(post_snippet, is_last);
    let new_lines = has_extra_newline(post_snippet, comment_end);
    let post = extract_post_comment(post_snippet, comment_end, is_last);
    prev_end = hi + comment_end;
    out.push(ItemComments {
      pre,
      pre_same_line,
      post,
      new_lines,
    });
  }
  // A block comment across lines is re-indented line by line, which is not done here; but
  // in a comment after an item, one whose later lines all start with `*`, each written a
  // space in (`rewrite_comment`).
  let spans = |c: &Option<String>, post: bool| {
    c.as_ref().is_some_and(|c| {
      let mut rest = c.as_str();
      while let Some(at) = rest.find("/*") {
        let after = &rest[at..];
        let end = after.find("*/").map_or(after.len(), |e| e + 2);
        if after[..end].contains('\n')
          && !(post
            && after[..end]
              .lines()
              .skip(1)
              .all(|l| l.trim_start().starts_with('*')))
        {
          return true;
        }
        rest = &after[end..];
      }
      false
    })
  };
  if out
    .iter()
    .any(|c| spans(&c.pre, false) || spans(&c.post, true))
  {
    return None;
  }
  Some(out)
}

/// The items one per line from column `indent` with their comments, each followed by a
/// comma (the last only with `trailing`), as `write_list` writes a vertical list.
pub fn write_vertical(
  texts: &[String],
  comments: &[ItemComments],
  indent: usize,
  width: usize,
  max_width: usize,
  trailing: bool,
  mixed: bool,
) -> Option<String> {
  write_list(
    texts,
    comments,
    indent,
    width,
    max_width,
    trailing,
    mixed,
    false,
    true,
    &mut [],
  )
}

/// [`write_vertical`] keeping a blank line written after an item (`preserve_newline`, as a
/// struct literal's fields are written).
pub fn write_vertical_kept(
  texts: &[String],
  comments: &[ItemComments],
  indent: usize,
  width: usize,
  max_width: usize,
  trailing: bool,
) -> Option<String> {
  write_list(
    texts,
    comments,
    indent,
    width,
    max_width,
    trailing,
    false,
    true,
    true,
    &mut [],
  )
}

/// A comment's lines, each trimmed, the later ones indented to `indent`
/// (`light_rewrite_comment` for line comments).
fn comment_lines(comment: &str, indent: usize) -> String {
  let pad = " ".repeat(indent);
  let mut out = String::new();
  for (i, line) in comment.trim().lines().map(str::trim).enumerate() {
    if i > 0 {
      out.push('\n');
      if !line.is_empty() {
        out.push_str(&pad);
      }
      // A block comment's later line, a space in under its `/*`.
      if line.starts_with('*') {
        out.push(' ');
      }
    }
    out.push_str(line);
  }
  out
}

/// The comment after an item, its comma and spaces taken off (`extract_post_comment`).
fn extract_post_comment(snippet: &str, comment_end: usize, is_last: bool) -> Option<String> {
  let white: &[char] = &[' ', '\t'];
  let snippet = snippet[..comment_end].trim();
  let inline_ends_with_separator = is_last
    && snippet
      .lines()
      .last()
      .is_some_and(|line| line.ends_with(',') && line.trim().starts_with("//"));
  let trimmed = if let Some(rest) = snippet.strip_prefix(',') {
    rest.trim_matches(white)
  } else if inline_ends_with_separator {
    snippet.trim_matches(white)
  } else if snippet.ends_with(',')
    && (!snippet.trim().starts_with("//") || snippet.trim().contains('\n'))
  {
    snippet[..snippet.len() - 1].trim_matches(white)
  } else {
    snippet
  };
  let bare = trimmed.trim();
  (!trimmed.is_empty() && (bare.starts_with("//") || bare.starts_with("/*")))
    .then(|| trimmed.to_string())
}

/// The comment before an item, and whether it is a block comment ending on the item's line
/// (`extract_pre_comment`).
fn extract_pre_comment(snippet: &str) -> (Option<String>, bool) {
  let trimmed = snippet.trim();
  if trimmed.ends_with("*/") {
    let end = snippet.rfind('/').unwrap_or(0);
    (Some(trimmed.to_string()), !snippet[end..].contains('\n'))
  } else if trimmed.starts_with("//") || trimmed.starts_with("/*") {
    (Some(trimmed.to_string()), false)
  } else {
    (None, false)
  }
}

/// Where in `snippet` (after an item) the first `,` outside a comment is.
fn find_separator(snippet: &str) -> Option<usize> {
  let mut i = 0;
  while let Some(rest) = snippet.get(i..) {
    let Some(c) = rest.chars().next() else {
      break;
    };
    if rest.starts_with("//") {
      i += rest.find('\n').unwrap_or(rest.len());
    } else if rest.starts_with("/*") {
      i += rest.find("*/").map_or(rest.len(), |n| n + 2);
    } else if c == ',' {
      return Some(i);
    } else {
      i += c.len_utf8();
    }
  }
  None
}

/// Where an item's post-comment ends in the text after it (`get_comment_end`).
fn get_comment_end(snippet: &str, is_last: bool) -> usize {
  if is_last {
    return snippet.len();
  }
  let mut block_open = snippet.find("/*");
  if let Some(i) = block_open {
    match snippet.find('/') {
      Some(j) if j < i => block_open = None,
      _ if snippet[..i].ends_with('/') => block_open = None,
      _ => {}
    }
  }
  let newline = snippet.find('\n');
  let comment_end = |i: usize| snippet[i..].find("*/").map_or(snippet.len(), |e| i + e + 2);
  if let Some(separator) = find_separator(snippet) {
    match (block_open, newline) {
      (Some(i), None) if i > separator => separator + 1,
      (Some(i), None) => comment_end(i).max(separator + 1),
      (Some(i), Some(j)) if i < j => comment_end(i).max(separator + 1),
      (_, Some(j)) if j > separator => j + 1,
      _ => snippet.len(),
    }
  } else if let Some(newline) = newline {
    newline + 1
  } else {
    0
  }
}

/// The widest of the items from `i` on that have post-comments, as one alignment group
/// (`max_width_of_item_with_post_comment`).
fn group_width(
  texts: &[String],
  comments: &[ItemComments],
  i: usize,
  overhead: usize,
  max: usize,
  preserve_newline: bool,
) -> usize {
  let mut width = 0;
  for (j, (text, c)) in texts.iter().zip(comments).enumerate().skip(i) {
    let w = str_width(text);
    // An item spanning lines, or with a comment before it or one across lines after it,
    // starts a group of its own (`is_different_group`).
    let different =
      text.contains('\n') || c.pre.is_some() || c.post.as_ref().is_some_and(|p| p.contains('\n'));
    if j != i && (different || c.post.is_none() || w + overhead > max) {
      return width;
    }
    width = width.max(w);
    // A blank line between items ends the group (`new_lines`) where the list keeps it; where
    // it drops it a second run would align across it: not done there, for a stable layout.
    if preserve_newline && c.new_lines {
      return width;
    }
  }
  width
}

/// Whether a blank line follows the item's comma and its comment line
/// (`has_extra_newline`).
fn has_extra_newline(snippet: &str, comment_end: usize) -> bool {
  if snippet.is_empty() || comment_end == 0 {
    return false;
  }
  let last_len = snippet[..comment_end]
    .chars()
    .next_back()
    .map_or(1, char::len_utf8);
  let test = &snippet[comment_end - last_len..];
  let first_newline = test.find('\n').unwrap_or(test.len());
  let test = &test[first_newline..];
  let first = test
    .find(|c: char| !c.is_whitespace())
    .unwrap_or(test.len());
  test[..first].matches('\n').count() > 1
}

#[allow(clippy::too_many_arguments)]
fn write_list(
  texts: &[String],
  comments: &[ItemComments],
  indent: usize,
  width: usize,
  max_width: usize,
  trailing: bool,
  mixed: bool,
  preserve_newline: bool,
  // Whether items are separated by commas the list adds (a match's arms carry their own).
  separator: bool,
  spaces: &mut [Option<usize>],
) -> Option<String> {
  let pad = " ".repeat(indent);
  let mut out = String::new();
  let mut line_start = 0;
  let mut item_max: Option<usize> = None;
  // Packed (`Mixed`): the width of the line so far, and whether the item before had a
  // post-comment, which ends its line.
  let mut line_len = 0;
  let mut prev_post = false;
  let comment_len = |c: &Option<String>| c.as_ref().map_or(0, |c| c.trim().len() + 6);
  // Packed, a line comment above an item would be written after the item before, where a
  // second run reads it as that item's: written there from the first (the standard
  // formatter's layout once it is stable).
  let mut comments: Vec<ItemComments> = comments.to_vec();
  if mixed {
    // Where packing would put the item after the one before on its line.
    let mut moves = Vec::new();
    let (mut len, mut prev) = (0usize, false);
    for (i, (text, c)) in texts.iter().zip(&comments).enumerate() {
      let separate = i + 1 < texts.len() || trailing;
      let total =
        str_width(text) + comment_len(&c.pre) + comment_len(&c.post) + usize::from(separate);
      let breaks = len > 0 && len + 1 + total > width || prev;
      let line_comment = c
        .pre
        .as_ref()
        .is_some_and(|p| p.trim_start().starts_with("//") && !p.trim().contains('\n'));
      if !breaks && len > 0 && line_comment && !c.pre_same_line {
        moves.push(i);
      }
      len = if breaks {
        total
      } else if len > 0 {
        len + 1 + total
      } else {
        total
      };
      if c.pre.is_some() && !c.pre_same_line {
        len = str_width(text);
      }
      prev = c.post.is_some();
    }
    for i in moves {
      if comments[i - 1].post.is_none() {
        comments[i - 1].post = comments[i].pre.take();
      }
    }
  }
  let comments = &comments[..];
  for (i, (text, c)) in texts.iter().zip(comments).enumerate() {
    let last = i + 1 == texts.len();
    let separate = separator && (!last || trailing);
    if mixed {
      let total =
        str_width(text) + comment_len(&c.pre) + comment_len(&c.post) + usize::from(separate);
      if line_len > 0 && line_len + 1 + total > width || prev_post {
        out.push('\n');
        line_start = out.len();
        out.push_str(&pad);
        line_len = 0;
      } else if line_len > 0 {
        out.push(' ');
        line_len += 1;
      }
      line_len += total;
      prev_post = c.post.is_some();
    } else if i > 0 {
      out.push('\n');
      // A blank line written after the item before, where the list keeps it.
      if preserve_newline && separator && comments[i - 1].new_lines {
        out.push('\n');
      }
      line_start = out.len();
      out.push_str(&pad);
    }
    if let Some(pre) = &c.pre {
      out.push_str(&comment_lines(pre, indent));
      // The item with its comments, each counted six columns wider, as the standard
      // formatter's `total_item_width` counts them.
      let counted = |c: &Option<String>| c.as_ref().map_or(0, |c| c.trim().len().max(4) + 6);
      let total = counted(&c.pre) + counted(&c.post) + text.len() + usize::from(separate) + 1;
      if c.pre_same_line && total <= width {
        out.push(' ');
      } else {
        out.push('\n');
        line_start = out.len();
        out.push_str(&pad);
        line_len = str_width(text);
      }
      item_max = None;
    }
    out.push_str(text);
    if separate {
      out.push(',');
    }
    if let Some(at) = out.rfind('\n')
      && at + 1 > line_start
    {
      line_start = at + 1;
    }
    let Some(post) = &c.post else {
      item_max = None;
      continue;
    };
    if post.starts_with('\n') {
      out.push('\n');
      out.push_str(&pad);
      out.push_str(&comment_lines(post, indent));
      continue;
    }
    let line_width = |out: &str| str_width(&out[line_start..]);
    let overhead = line_width(&out) + str_width(post.trim().lines().next().unwrap_or(""));
    // The group's width, found again when it is missing (`rewrite_post_comment`).
    let group = |item_max: &mut Option<usize>| {
      if item_max.is_none() && !last && !text.contains('\n') {
        *item_max = Some(group_width(
          texts,
          comments,
          i,
          overhead,
          max_width,
          preserve_newline,
        ));
      }
    };
    let last_line = text.rsplit('\n').next().unwrap_or("");
    // A packed list's last item without its comma, which is not the list's separator
    // there (`item_sep_len`).
    let item_width = str_width(last_line.trim_start()) + usize::from(separate && !(mixed && last));
    let column = |item_max: Option<usize>| match item_max {
      Some(max) => indent + max + 2,
      None => indent + item_width + 1,
    };
    group(&mut item_max);
    let mut alignment = item_max.unwrap_or(0).saturating_sub(str_width(text));
    let mut formatted = comment_lines(post, column(item_max));
    if str_width(formatted.lines().next().unwrap_or("")) + line_width(&out) + alignment + 1
      > max_width
    {
      item_max = None;
      group(&mut item_max);
      alignment = item_max.unwrap_or(0).saturating_sub(str_width(text));
      formatted = comment_lines(post, column(item_max));
    }
    let mut count = alignment + 1;
    if last && item_max.is_some() && !separate && separator {
      count += 1;
    }
    for _ in 0..count {
      out.push(' ');
    }
    if let Some(slot) = spaces.get_mut(i) {
      *slot = Some(count);
    }
    if formatted.contains('\n') {
      item_max = None;
    }
    out.push_str(&formatted);
    if preserve_newline && !last && c.new_lines {
      item_max = None;
    }
  }
  Some(out)
}
