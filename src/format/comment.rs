//! Comments rewritten as the standard formatter rewrites them under `normalize_comments`,
//! once the rest is laid out.
//!
//! A comment ending its line is rewritten as line comments of its kind: `/* a */` as
//! `// a`, `/** a */` as `/// a` and `/*! a */` as `//! a`, a block over lines as one line
//! comment per line, and every line comment with a space after its marker (`//a` as
//! `// a`). Comments next to each other on lines of their own, of one kind and with no
//! blank line between them, are rewritten together, as one group; a block comment loses
//! its empty last line. A comment with code after it on its line stays as
//! written, and so does one over lines after code, one in a macro's arguments, a skipped
//! item or a `// mto: off` region.

use SyntaxKind as K;

use crate::options::format::FormatConfig;
use crate::syntax::ast::Attr;
use crate::syntax::marker;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

#[derive(Copy, Clone, PartialEq, Eq)]
enum Style<'a> {
  DoubleSlash,
  TripleSlash,
  Doc,
  SingleBullet,
  DoubleBullet,
  Exclamation,
  Custom(&'a str),
}

impl<'a> Style<'a> {
  /// The style of the comment `orig` starts with, `normalized` or as written.
  fn of(orig: &'a str, normalized: bool) -> Style<'a> {
    let triple = orig.starts_with("///") && orig.chars().nth(3) != Some('/');
    let double_bullet = orig.starts_with("/**") && !orig.starts_with("/**/");
    if !normalized {
      if double_bullet {
        Style::DoubleBullet
      } else if orig.starts_with("/*!") {
        Style::Exclamation
      } else if orig.starts_with("/*") {
        Style::SingleBullet
      } else if triple {
        Style::TripleSlash
      } else if orig.starts_with("//!") {
        Style::Doc
      } else if custom(orig) {
        Style::Custom(custom_opener(orig))
      } else {
        Style::DoubleSlash
      }
    } else if triple || double_bullet {
      Style::TripleSlash
    } else if orig.starts_with("//!") || orig.starts_with("/*!") {
      Style::Doc
    } else if custom(orig) {
      Style::Custom(custom_opener(orig))
    } else {
      Style::DoubleSlash
    }
  }

  fn is_block(self) -> bool {
    matches!(
      self,
      Style::SingleBullet | Style::DoubleBullet | Style::Exclamation
    )
  }

  fn is_line(self) -> bool {
    !self.is_block()
  }

  fn is_doc(self) -> bool {
    matches!(self, Style::TripleSlash | Style::Doc)
  }

  fn opener(self) -> &'a str {
    match self {
      Style::DoubleSlash => "// ",
      Style::TripleSlash => "/// ",
      Style::Doc => "//! ",
      Style::SingleBullet => "/* ",
      Style::DoubleBullet => "/** ",
      Style::Exclamation => "/*! ",
      Style::Custom(opener) => opener,
    }
  }

  fn closer(self) -> &'a str {
    if self.is_block() { " */" } else { "" }
  }

  fn prefix(self) -> &'a str {
    match self {
      Style::DoubleSlash => "// ",
      Style::TripleSlash => "/// ",
      Style::Doc => "//! ",
      Style::SingleBullet | Style::DoubleBullet | Style::Exclamation => " * ",
      Style::Custom(opener) => opener,
    }
  }
}

struct Rewrite<'a> {
  result: String,
  /// The lines of a fenced code block being read, while one is.
  code: Option<String>,
  separator: String,
  indent: String,
  opener: &'a str,
  closer: &'a str,
}

impl Rewrite<'_> {
  /// Adds one line of the comment; true when the comment ends here.
  fn line(&mut self, line: &str, last: bool) -> bool {
    if let Some(code) = &mut self.code {
      if !line.starts_with("```") {
        code.push_str(line);
        code.push('\n');
        return false;
      }
      let code = self.code.take().unwrap_or_default();
      if !code.is_empty() {
        self.result.push_str(&self.separator);
        self.result.push_str(&join(&code, &self.separator));
      }
      self.result.push_str(&self.separator);
      self.result.push_str(line);
      return false;
    }
    if line.starts_with("```") {
      self.code = Some(String::new());
    }
    if self.result == self.opener {
      if line.is_empty() {
        return false;
      }
    } else if last && line.is_empty() {
      // Trailing blank lines are dropped.
      if !self.closer.is_empty() {
        self.result.push_str(&self.indent);
      }
      return true;
    } else {
      self.result.push_str(&self.separator);
    }
    if line.is_empty() && self.result.ends_with(' ') && !last {
      self.result.pop();
    }
    self.result.push_str(line);
    false
  }

  fn finish(mut self) -> String {
    if let Some(code) = self.code.take().filter(|c| !c.is_empty()) {
      // A code block never closed is kept as it is.
      self.result.push_str(&self.separator);
      self.result.push_str(&join(&code, &self.separator));
    }
    self.result.push_str(self.closer);
    if self.result.ends_with(self.opener) && self.opener.ends_with(' ') {
      self.result.pop();
    }
    self.result
  }
}

/// An item's body whose braces (or a tuple struct's parentheses) hold one block comment, on
/// their line, written as the standard formatter writes it for the item's kind: a trait's,
/// an impl's or an extern block's on a line of its own, one level in; a module's or a
/// function's after the `{`, with the `}` on the next line; a struct's, a union's or an
/// enum's between the delimiters, touching. With `normalize` the comment becomes a line
/// comment, so it goes on a line of its own, but after a module's or a function's `{`.
pub fn lone_comments(
  root: &SyntaxNode,
  text: &str,
  config: &FormatConfig,
  normalize: bool,
) -> Option<String> {
  let off = marker::scan(root, root.text()).regions;
  let mut edits: Vec<(usize, usize, String)> = Vec::new();
  for body in root.descendant_nodes_where(|kind| {
    matches!(
      kind,
      K::AssocItemList
        | K::ExternItemList
        | K::ItemList
        | K::BlockExpr
        | K::RecordFieldList
        | K::TupleFieldList
        | K::VariantList
    )
  }) {
    let Some(owner) = body.parent() else {
      continue;
    };
    let after_open = match (body.kind(), owner.kind()) {
      (K::ItemList, K::Module) | (K::BlockExpr, K::Fn) => true,
      (K::AssocItemList | K::ExternItemList, _) => false,
      (K::RecordFieldList, K::Struct | K::Union)
      | (K::TupleFieldList, K::Struct)
      | (K::VariantList, K::Enum) => {
        if !normalize {
          // Already written touching where the layout writes it.
          if owner.kind() == K::Struct {
            continue;
          }
        }
        false
      }
      _ => continue,
    };
    let inside: Vec<SyntaxElement> = body.children_with_tokens_iter().collect();
    let comments: Vec<&SyntaxElement> = inside
      .iter()
      .filter(|e| e.kind() != K::Whitespace)
      .collect();
    let [open, comment, close] = comments.as_slice() else {
      continue;
    };
    let (start, end) = (
      open.text_range().start as usize,
      close.text_range().end as usize,
    );
    if comment.kind() != K::BlockComment
      || !matches!(open.kind(), K::LBrace | K::LParen)
      || text[start..comment.text_range().end as usize].contains('\n')
      || !after_open && text[start..end].contains('\n')
      || held(&body)
      || off.iter().any(|&(s, e)| s < end && start < e)
    {
      continue;
    }
    let comment_text =
      &text[comment.text_range().start as usize..comment.text_range().end as usize];
    let line_start = text[..start].rfind('\n').map_or(0, |n| n + 1);
    let indent: String = text[line_start..]
      .chars()
      .take_while(|c| *c == ' ' || *c == '\t')
      .collect();
    let (opener, closer) = if open.kind() == K::LParen {
      ("(", ")")
    } else {
      ("{", "}")
    };
    let touching = !normalize && matches!(owner.kind(), K::Struct | K::Union | K::Enum);
    let new = if touching {
      format!("{}{}{}", opener, comment_text, closer)
    } else if after_open {
      let comment_text = if normalize {
        normalized(comment_text, false, &indent)
      } else {
        comment_text.to_string()
      };
      format!("{} {}\n{}{}", opener, comment_text, indent, closer)
    } else {
      let inner = if config.hard_tabs {
        format!("{}\t", indent)
      } else {
        format!("{}{}", indent, " ".repeat(config.tab_spaces))
      };
      let comment_text = if normalize {
        normalized(comment_text, false, &inner)
      } else {
        comment_text.to_string()
      };
      format!(
        "{}\n{}{}\n{}{}",
        opener, inner, comment_text, indent, closer
      )
    };
    if new != text[start..end] {
      edits.push((start, end, new));
    }
  }
  if edits.is_empty() {
    return None;
  }
  edits.sort_unstable_by_key(|e| e.0);
  let mut out = String::with_capacity(text.len() + 64);
  let mut from = 0;
  for (start, end, new) in edits {
    out.push_str(&text[from..start]);
    out.push_str(&new);
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// The text with its comments normalized (`normalize_comments`), or `None` when none
/// changes.
pub fn normalize(root: &SyntaxNode, text: &str, _config: &FormatConfig) -> Option<String> {
  let off = marker::scan(root, root.text()).regions;
  let comments: Vec<(SyntaxKind, usize, usize)> = root
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if is_comment(t.kind()) && !held(&t.parent()) => Some((
        class(t.kind()),
        t.text_range().start as usize,
        t.text_range().end as usize,
      )),
      _ => None,
    })
    .collect();
  let mut edits: Vec<(usize, usize, String)> = Vec::new();
  let mut i = 0;
  while i < comments.len() {
    let (kind, start, end) = comments[i];
    i += 1;
    let line_start = text[..start].rfind('\n').map_or(0, |n| n + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |n| end + n);
    if !text[end..line_end].trim().is_empty() || off.iter().any(|&(s, e)| s < end && start < e) {
      continue;
    }
    let before = &text[line_start..start];
    let own_line = before.trim().is_empty();
    if !own_line && text[start..end].contains('\n') {
      continue;
    }
    // The comments after it on lines of their own, of its kind, at its indentation and
    // with no blank line between.
    let mut last = end;
    while own_line && i < comments.len() {
      let (next_kind, next_start, next_end) = comments[i];
      let gap = &text[last..next_start];
      let next_line_end = text[next_end..]
        .find('\n')
        .map_or(text.len(), |n| next_end + n);
      if next_kind != kind
        || gap.matches('\n').count() != 1
        || gap.trim_start_matches([' ', '\t']) != format!("\n{}", before)
        || !text[next_end..next_line_end].trim().is_empty()
        || off.iter().any(|&(s, e)| s < next_end && next_start < e)
      {
        break;
      }
      last = next_end;
      i += 1;
    }
    let orig = &text[start..last];
    let indent = if own_line {
      before.to_string()
    } else {
      " ".repeat(before.chars().count())
    };
    let new = normalized(orig, false, &indent);
    if new != orig {
      edits.push((start, last, new));
    }
  }
  if edits.is_empty() {
    return None;
  }
  let mut out = String::with_capacity(text.len());
  let mut from = 0;
  for (start, end, new) in edits {
    out.push_str(&text[from..start]);
    out.push_str(&new);
    from = end;
  }
  out.push_str(&text[from..]);
  Some(out)
}

/// The comments `orig` holds (one group or several, with nothing but whitespace between
/// them), normalized, their lines after the first behind `indent`. `block` keeps a block
/// comment a block comment.
pub fn normalized(orig: &str, block: bool, indent: &str) -> String {
  let style = Style::of(orig, false);
  // A line's length with its line ending, while it is not the text's last.
  let length = |rest: &str, line: &str| match rest.as_bytes().get(line.len()) {
    Some(b'\r') => line.len() + 2,
    Some(_) => line.len() + 1,
    None => line.len(),
  };
  let (bare, end) = match style {
    Style::DoubleSlash | Style::TripleSlash | Style::Doc | Style::Custom(_) => {
      let start = match style {
        Style::Custom(opener) => opener.trim_end(),
        _ => style.prefix().trim_start(),
      };
      let mut end = 0;
      let mut bare = false;
      for line in orig.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
          bare = true;
          break;
        } else if trimmed.starts_with(start) || Style::of(trimmed, false) == style {
          end += length(&orig[end..], line);
        } else {
          break;
        }
      }
      (bare, end)
    }
    Style::SingleBullet | Style::DoubleBullet | Style::Exclamation => {
      let closer = style.closer().trim_start();
      let mut count = orig.matches(closer).count();
      let mut end = 0;
      let mut bare = false;
      for (i, line) in orig.lines().enumerate() {
        end += length(&orig[end..], line);
        let mut trimmed = line.trim_start();
        if !trimmed.starts_with('*') && !trimmed.starts_with("//") && !trimmed.starts_with("/*") {
          bare = true;
        }
        if i == 0 {
          trimmed = trimmed.get(style.opener().trim_end().len()..).unwrap_or("");
        }
        if trimmed.ends_with(closer) {
          count = count.saturating_sub(1);
          if count == 0 {
            break;
          }
        }
      }
      (bare, end)
    }
  };
  let (first, rest) = orig.split_at(end.min(orig.len()));
  let first = group(first, block, style, indent);
  if rest.trim().is_empty() {
    return first;
  }
  format!(
    "{}\n{}{}{}",
    first,
    if bare && style.is_line() { "\n" } else { "" },
    indent,
    normalized(rest.trim_start(), block, indent)
  )
}

/// The kind a comment groups by: plain, outer doc or inner doc.
fn class(kind: SyntaxKind) -> SyntaxKind {
  match kind {
    K::OuterBlockDoc => K::OuterLineDoc,
    K::InnerBlockDoc => K::InnerLineDoc,
    K::BlockComment => K::LineComment,
    other => other,
  }
}

/// A line comment whose marker is followed by punctuation (`//#`, `////`).
fn custom(comment: &str) -> bool {
  comment.starts_with("//")
    && comment
      .chars()
      .nth(2)
      .is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace())
}

/// A custom comment's marker: its first line up to and with its first space.
fn custom_opener(comment: &str) -> &str {
  comment.lines().next().map_or("", |first| {
    first.find(' ').map_or(first, |space| &first[..=space])
  })
}

/// One group of comments of one style, rewritten.
fn group(orig: &str, block: bool, written: Style, indent: &str) -> String {
  let is_doc = written.is_doc();
  let style = if block {
    Style::SingleBullet
  } else {
    Style::of(orig, true)
  };
  let mut out = Rewrite {
    result: style.opener().to_string(),
    code: None,
    separator: format!("\n{}{}", indent, style.prefix()),
    indent: format!("\n{}", indent),
    opener: style.opener(),
    closer: style.closer(),
  };
  let breaks = orig.trim_end().matches('\n').count();
  let newlines = orig.matches('\n').count();
  for (i, line) in orig.lines().enumerate() {
    let mut line = line.trim_start();
    if !(is_doc && line.ends_with("  ")) {
      line = line.trim_end();
    }
    if i == breaks && line.ends_with("*/") && !line.starts_with("//") {
      line = line[..line.len() - 2].trim_end();
    }
    let (mut line, _) = left_trim(line, written);
    if orig.starts_with("/*") && breaks == 0 {
      line = line.trim_start();
    }
    // Only a block comment's empty last line goes; a group of line comments keeps one.
    if out.line(line, i == newlines && written.is_block()) {
      break;
    }
  }
  out.finish()
}

/// Whether a comment under `parent` is kept as written: in a macro's arguments or a
/// skipped item.
fn held(parent: &SyntaxNode) -> bool {
  parent.ancestors_iter().any(|owner| {
    owner.kind() == K::TokenTree
      || owner
        .children_iter()
        .filter_map(Attr::cast)
        .any(|a| a.is_rustfmt_skip())
  })
}

fn is_comment(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    K::LineComment
      | K::BlockComment
      | K::OuterLineDoc
      | K::InnerLineDoc
      | K::OuterBlockDoc
      | K::InnerBlockDoc
  )
}

/// `text`'s lines joined by `separator`, without its trailing space before an empty line.
fn join(text: &str, separator: &str) -> String {
  let mut out = String::with_capacity(text.len() + 64);
  let mut lines = text.lines().peekable();
  while let Some(line) = lines.next() {
    out.push_str(line);
    match lines.peek() {
      Some(&"") => out.push_str(separator.trim_end()),
      Some(_) => out.push_str(separator),
      None => {}
    }
  }
  out
}

/// A comment line without its marker (and one space after it), and whether the space was
/// there.
fn left_trim<'a>(line: &'a str, style: Style) -> (&'a str, bool) {
  if ["//! ", "/// ", "/*! ", "/** "]
    .iter()
    .any(|p| line.starts_with(p))
  {
    (&line[4..], true)
  } else if let Style::Custom(opener) = style {
    match line.strip_prefix(opener) {
      Some(stripped) => (stripped, true),
      None => (line.get(opener.trim_end().len()..).unwrap_or(""), false),
    }
  } else if ["/* ", "// ", "//!", "///", "** ", "/*!"]
    .iter()
    .any(|p| line.starts_with(p))
    || (line.starts_with("/**") && !line.starts_with("/**/"))
  {
    (&line[3..], line.as_bytes()[2] == b' ')
  } else if ["/*", "* ", "//", "**"].iter().any(|p| line.starts_with(p)) {
    (&line[2..], line.as_bytes()[1] == b' ')
  } else if let Some(stripped) = line.strip_prefix('*') {
    (stripped, false)
  } else {
    (line, line.starts_with(' '))
  }
}

#[cfg(test)]
mod tests;
