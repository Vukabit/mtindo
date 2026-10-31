//! Comments rewritten as the standard formatter rewrites them under `normalize_comments`
//! and `wrap_comments`, once the rest is laid out.
//!
//! With `wrap_comments` a comment's line wider than `comment_width` (from the comment's
//! column) is broken between words, a list item or block quote as a whole; a Markdown
//! heading in a doc comment, a line holding a URL and a table's row are not, nor is a fenced
//! code block.
//!
//! A comment ending its line is rewritten as line comments of its kind: `/* a */` as
//! `// a`, `/** a */` as `/// a` and `/*! a */` as `//! a`, a block over lines as one line
//! comment per line, and every line comment with a space after its marker (`//a` as
//! `// a`). Comments next to each other on lines of their own, of one kind and with no
//! blank line between them, are rewritten together, as one group, up to a block comment
//! that is not a doc comment; a block comment loses its empty last line. A comment with code
//! after it on its line stays as written, and so does one over lines after code, one in a
//! macro's arguments, a skipped item or a `// mto: off` region.

mod string;

use SyntaxKind as K;

use crate::options::format::FormatConfig;
use crate::syntax::ast::Attr;
use crate::syntax::marker;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};
use crate::text::unicode::str_width;

/// Where a comment stands, which sets the width its text may take.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
  Plain,
  /// Between an item's attributes (a doc comment among them).
  Attributes,
  /// In an item's header, after the start of its code (`fn f() // a`).
  Header,
  /// An enum variant's doc comment.
  VariantDoc,
  /// In a list of items (fields, parameters, arguments), after one or before one.
  ListItem,
  /// Among a match's arms.
  Arm,
}

impl Place {
  fn of(token: &crate::syntax::SyntaxToken) -> Place {
    let parent = token.parent();
    if crate::format::items::is_doc(token.kind()) && parent.kind() == K::Variant {
      return Place::VariantDoc;
    }
    if matches!(parent.kind(), K::MatchArm | K::MatchArmList) {
      return Place::Arm;
    }
    let step = |e: &SyntaxElement, back: bool| match (e, back) {
      (SyntaxElement::Token(t), true) => t.prev_sibling_or_token(),
      (SyntaxElement::Token(t), false) => t.next_sibling_or_token(),
      (SyntaxElement::Node(n), true) => n.prev_sibling_or_token(),
      (SyntaxElement::Node(n), false) => n.next_sibling_or_token(),
    };
    let skipped =
      |e: &SyntaxElement| matches!(e.kind(), K::Whitespace | K::LineComment | K::BlockComment);
    let mut before = token.prev_sibling_or_token();
    while let Some(e) = before.as_ref().filter(|e| skipped(e)) {
      before = step(e, true);
    }
    let mut after = token.next_sibling_or_token();
    while let Some(e) = after.as_ref().filter(|e| skipped(e)) {
      after = step(e, false);
    }
    let attribute = |e: &SyntaxElement| match e {
      SyntaxElement::Node(n) => n.kind() == K::Attr,
      SyntaxElement::Token(t) => matches!(t.kind(), K::OuterLineDoc | K::OuterBlockDoc),
    };
    if before.as_ref().is_some_and(attribute) && after.as_ref().is_some_and(attribute) {
      return Place::Attributes;
    }
    let item = matches!(
      parent.kind(),
      K::Fn | K::Struct | K::Enum | K::Union | K::Trait | K::Impl | K::TypeAlias
    );
    if item
      && before
        .as_ref()
        .is_some_and(|e| !attribute(e) && !e.kind().is_trivia())
    {
      return Place::Header;
    }
    let list = matches!(
      parent.kind(),
      K::RecordFieldList
        | K::TupleFieldList
        | K::VariantList
        | K::ParamList
        | K::ArgList
        | K::RecordExprFieldList
        | K::ArrayExpr
        | K::TupleExpr
        | K::GenericParamList
        | K::GenericArgList
        | K::RecordPatFieldList
        | K::TuplePat
        | K::TupleStructPat
        | K::SlicePat
    );
    // After an item, or before one (not in a list of comments only).
    let item_after = after
      .as_ref()
      .is_some_and(|e| !matches!(e.kind(), K::RBrace | K::RParen | K::RBrack | K::Gt));
    if list && (before.is_some_and(|e| e.kind() == K::Comma) || item_after) {
      return Place::ListItem;
    }
    Place::Plain
  }
}

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

/// A comment found, and where it stands.
#[derive(Clone, Copy)]
struct Found {
  kind: SyntaxKind,
  start: usize,
  end: usize,
  place: Place,
}

/// A list item or block quote in a comment, wrapped as a whole (`ItemizedBlock`): it starts
/// with `* `, `- `, `> `, `+ ` or a number of one or two digits and `. ` or `) `, and takes
/// the lines after it indented at least as far as its text.
struct Itemized {
  lines: Vec<String>,
  /// Where its text starts.
  indent: usize,
  /// What comes before its text on its first line.
  opener: String,
  /// What comes before its text on each later line.
  line_start: String,
}

impl Itemized {
  fn marker_length(trimmed: &str) -> Option<usize> {
    if ["* ", "- ", "> ", "+ "]
      .iter()
      .any(|m| trimmed.starts_with(m))
    {
      return Some(2);
    }
    for suffix in [". ", ") "] {
      if let Some((prefix, _)) = trimmed.split_once(suffix)
        && (1..=2).contains(&prefix.len())
        && prefix.chars().all(|c| c.is_ascii_digit())
      {
        return Some(prefix.len() + suffix.len());
      }
    }
    None
  }

  fn new(line: &str) -> Option<Itemized> {
    let marker = Itemized::marker_length(line.trim_start())?;
    let space = line.chars().take_while(|c| c.is_whitespace()).count();
    let mut indent = space + marker;
    let mut line_start = " ".repeat(indent);
    if line.trim_start().starts_with('>') {
      // `> ` for each level of quoting in place of two of the spaces.
      let level = line
        .chars()
        .take_while(|c| !c.is_alphanumeric())
        .filter(|&c| c == '>')
        .count();
      line_start.truncate(line_start.len().saturating_sub(2));
      for _ in 0..level {
        line_start.push_str("> ");
      }
      indent = line_start.len();
    }
    Some(Itemized {
      lines: vec![line.get(indent..)?.to_string()],
      indent,
      opener: line.get(..indent)?.to_string(),
      line_start,
    })
  }

  /// Takes `line` when it continues the item.
  fn add(&mut self, line: &str) -> bool {
    if Itemized::marker_length(line.trim_start()).is_none()
      && self.indent <= line.chars().take_while(|c| c.is_whitespace()).count()
    {
      self.lines.push(line.to_string());
      return true;
    }
    false
  }

  fn trimmed(&self) -> String {
    let mut out = String::new();
    for line in &self.lines {
      out.push_str(line.trim_start());
      out.push(' ');
    }
    out
  }
}

/// A comment being rewritten line by line (`CommentRewrite`).
struct Rewrite<'a> {
  result: String,
  /// The lines of a fenced code block being read, while one is.
  code: Option<String>,
  /// The lines of a list item or block quote being read, while one is.
  item: Option<Itemized>,
  /// Whether the line before was broken over lines, so the next joins its last.
  prev_multi: bool,
  separator: String,
  /// A line break and the comment's indentation.
  indent_str: String,
  indent: &'a str,
  opener: &'a str,
  closer: &'a str,
  prefix: &'a str,
  block: bool,
  newlines: usize,
  is_doc: bool,
  rules: Rules,
  /// The width a line's text may take, less the markers.
  max_width: usize,
  /// The width the next text may take, and where it starts, after a line broken over lines.
  width: usize,
  offset: usize,
}

impl Rewrite<'_> {
  fn layout(&self) -> string::Layout<'_> {
    string::Layout {
      line_start: self.prefix,
      indent: self.indent,
      width: self.width,
      offset: self.offset,
      max_width: self.rules.max_width,
    }
  }

  fn reset_width(&mut self) {
    self.width = self.max_width;
    self.offset = 0;
  }

  /// Writes an item read to its end.
  fn flush_item(&mut self, item: Itemized, new_line: bool) {
    self.reset_width();
    if new_line {
      self.result.push_str(&self.separator);
    }
    self.result.push_str(&item.opener);
    let layout = string::Layout {
      line_start: "",
      indent: "",
      width: self.width.saturating_sub(item.indent),
      offset: 0,
      max_width: self.rules.max_width,
    };
    match string::rewrite_string(
      &item.trimmed(),
      &layout,
      self.max_width.saturating_sub(item.indent),
    ) {
      Some(s) => {
        let separator = format!("{}{}", self.separator, item.line_start);
        self.result.push_str(&join(&s, &separator));
      }
      None => self
        .result
        .push_str(&join(&item.lines.join("\n"), &self.separator)),
    }
  }

  /// Adds a line of the comment; true when the comment ends here. `spaced` is whether
  /// the line kept a space after its marker.
  fn line(&mut self, line: &str, spaced: bool, last: bool) -> bool {
    let needs_new_line = if self.block {
      self.newlines > 0 || self.result.len() != self.opener.len()
    } else {
      self.result.len() != self.opener.len()
    };
    if let Some(mut item) = self.item.take() {
      if item.add(line) {
        self.item = Some(item);
        return false;
      }
      self.prev_multi = false;
      self.flush_item(item, needs_new_line);
    } else if let Some(code) = &mut self.code {
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
    } else if self.rules.wrap
      && let Some(item) = Itemized::new(line)
    {
      self.item = Some(item);
      return false;
    }
    if self.result == self.opener {
      let forced = self.opener == "/* " && self.newlines == 0;
      if !spaced && !forced && self.result.ends_with(' ') {
        self.result.pop();
      }
      if line.is_empty() {
        return false;
      }
    } else if self.prev_multi && !line.is_empty() {
      self.result.push(' ');
    } else if last && line.is_empty() {
      // Trailing blank lines are dropped.
      if !self.closer.is_empty() {
        self.result.push_str(&self.indent_str);
      }
      return true;
    } else {
      self.result.push_str(&self.separator);
      if !spaced && self.result.ends_with(' ') {
        self.result.pop();
      }
    }
    let header = self.is_doc && line.starts_with('#');
    let wrap = self.rules.wrap
      && !header
      && str_width(line) > self.width
      && !has_url(line)
      && !is_table_item(line);
    if !wrap {
      if line.is_empty() && self.result.ends_with(' ') && !last {
        self.result.pop();
      }
      if self.code.is_some() && self.prev_multi {
        self.result.push_str(&self.separator);
      }
      self.result.push_str(line);
      self.reset_width();
      self.prev_multi = false;
      return false;
    }
    match string::rewrite_string(line, &self.layout(), self.max_width) {
      Some(s) => {
        self.prev_multi = s.contains('\n');
        self.result.push_str(&s);
      }
      None if self.prev_multi => {
        // It did not fit after the line before: on a line of its own instead.
        self.result.pop();
        self.result.push_str(&self.separator);
        self.reset_width();
        match string::rewrite_string(line, &self.layout(), self.max_width) {
          Some(s) => {
            self.prev_multi = s.contains('\n');
            self.result.push_str(&s);
          }
          None => {
            self.prev_multi = false;
            self.result.push_str(line);
          }
        }
      }
      None => {
        self.prev_multi = false;
        self.result.push_str(line);
      }
    }
    if self.prev_multi {
      // 1 = ` `
      let last_line = self.result.rsplit('\n').next().unwrap_or("");
      let offset =
        (1 + str_width(last_line)).saturating_sub(self.prefix.len() + str_width(self.indent));
      self.width = self.max_width.saturating_sub(offset);
      self.offset += offset;
    } else {
      self.reset_width();
    }
    false
  }

  fn finish(mut self) -> String {
    if let Some(code) = self.code.take().filter(|c| !c.is_empty()) {
      // A code block never closed is kept as it is.
      self.result.push_str(&self.separator);
      self.result.push_str(&join(&code, &self.separator));
    }
    if let Some(item) = self.item.take() {
      let new_line = self.result.len() != self.opener.len();
      self.flush_item(item, new_line);
    }
    self.result.push_str(self.closer);
    if self.result.ends_with(self.opener) && self.opener.ends_with(' ') {
      self.result.pop();
    }
    self.result
  }
}

/// What a rewrite does, from the settings: `normalize_comments`, `wrap_comments`, and the
/// line width.
#[derive(Clone, Copy)]
pub struct Rules {
  pub normalize: bool,
  pub wrap: bool,
  pub max_width: usize,
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

/// The comments `orig` holds (one group or several, with nothing but whitespace between
/// them), normalized, their lines after the first behind `indent`. `block` keeps a block
/// comment a block comment.
pub fn normalized(orig: &str, block: bool, indent: &str) -> String {
  let rules = Rules {
    normalize: true,
    wrap: false,
    max_width: usize::MAX,
  };
  rewritten(orig, block, indent, usize::MAX, false, rules)
}

/// The text with its comments rewritten as `normalize_comments` and `wrap_comments` say,
/// or `None` when none changes.
pub fn rewrite_comments(root: &SyntaxNode, text: &str, config: &FormatConfig) -> Option<String> {
  let rules = Rules {
    normalize: config.normalize_comments,
    wrap: config.wrap_comments,
    max_width: config.max_width,
  };
  let column = |text: &str| {
    text
      .chars()
      .map(|c| {
        if c == '\t' {
          config.tab_spaces
        } else {
          crate::text::unicode::char_width(c)
        }
      })
      .sum::<usize>()
  };
  let off = marker::scan(root, root.text()).regions;
  let comments: Vec<Found> = root
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if is_comment(t.kind()) && !held(&t.parent()) => Some(Found {
        kind: class(t.kind()),
        start: t.text_range().start as usize,
        end: t.text_range().end as usize,
        place: Place::of(&t),
      }),
      _ => None,
    })
    .collect();
  let mut edits: Vec<(usize, usize, String)> = Vec::new();
  let mut i = 0;
  while i < comments.len() {
    let Found {
      kind,
      start,
      end,
      place,
    } = comments[i];
    i += 1;
    let line_start = text[..start].rfind('\n').map_or(0, |n| n + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |n| end + n);
    if !text[end..line_end].trim().is_empty() || off.iter().any(|&(s, e)| s < end && start < e) {
      continue;
    }
    let before = &text[line_start..start];
    let own_line = before.trim().is_empty();
    // One over lines after code is rewritten from its column before style edition 2024,
    // and only between statements, items or a list's items (after `;`, `,`, `{` or `}`),
    // where the standard formatter rewrites what it skipped, not after a match's arm; from
    // 2024 its first line stays as written.
    let after_boundary = before.trim_end().ends_with([';', ',', '{', '}']);
    if !own_line
      && text[start..end].contains('\n')
      && (config.style_edition >= 2024 || !after_boundary || place == Place::Arm)
    {
      continue;
    }
    // The comments after it on lines of their own, of its kind, at its indentation and
    // with no blank line between.
    let mut last = end;
    // A comment other than a doc comment written `/* */` ends the run (the standard
    // formatter reads the line break after it as code).
    let mut block = kind == K::LineComment && text[start..end].starts_with("/*");
    while own_line && !block && i < comments.len() {
      let Found {
        kind: next_kind,
        start: next_start,
        end: next_end,
        ..
      } = comments[i];
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
      block = next_kind == K::LineComment && text[next_start..next_end].starts_with("/*");
      i += 1;
    }
    let orig = &text[start..last];
    let indent = if own_line {
      before.to_string()
    } else {
      " ".repeat(column(before))
    };
    // The text within the comment width from the comment's column (`Shape::comment`); one
    // after code from style edition 2024 keeps its first line as written.
    // The text within the comment width from the comment's column (`Shape::comment`); among
    // attributes or in an item's header within the line width alone (`with_max_width`), and
    // after a list's item from its column within the list's width (`write_list`).
    let limit = match place {
      Place::Attributes | Place::Header | Place::ListItem => config.max_width,
      Place::Plain | Place::VariantDoc | Place::Arm => config.max_width.min(config.comment_width),
    };
    // A variant's doc comment a column short for its comma, before style edition 2024; one
    // after a list's item one short for the list's separator.
    let comma = usize::from(
      place == Place::VariantDoc && config.style_edition < 2024
        || place == Place::ListItem && !own_line,
    );
    let width = limit.saturating_sub(column(before) + comma);
    let list_item = place == Place::ListItem && !own_line;
    // One after code from style edition 2024 keeps its first line as written; one after a
    // list's item is rewritten as a block comment when it is too wide or over lines.
    let rules = Rules {
      wrap: rules.wrap && (own_line || list_item || config.style_edition < 2024),
      ..rules
    };
    let block = list_item
      && (rules.normalize || rules.wrap)
      && (text[start..last].trim().contains('\n') || str_width(&text[start..last]) > width);
    let doc = matches!(kind, K::OuterLineDoc | K::InnerLineDoc);
    let new = rewritten(orig, block, &indent, width, doc, rules);
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

/// The comments `orig` holds, rewritten by `rules` (`identify_comment`): their lines after
/// the first behind `indent`, their text within `width`; `is_doc` for doc comments.
pub fn rewritten(
  orig: &str,
  block: bool,
  indent: &str,
  width: usize,
  is_doc: bool,
  rules: Rules,
) -> String {
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
  // A block comment with lines not starting `*` keeps its layout unless normalized
  // (`trim_left_preserve_layout`), as one under neither setting does.
  let first = if !rules.normalize && (bare && style.is_block() || !rules.wrap) {
    first.trim_end().to_string()
  } else {
    group(first, block, style, indent, width, is_doc, rules)
  };
  if rest.trim().is_empty() {
    return first;
  }
  format!(
    "{}\n{}{}{}",
    first,
    if bare && style.is_line() { "\n" } else { "" },
    indent,
    rewritten(rest.trim_start(), block, indent, width, is_doc, rules)
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

/// One group of comments of one style, rewritten (`rewrite_comment_inner`).
fn group(
  orig: &str,
  block: bool,
  written: Style,
  indent: &str,
  width: usize,
  is_doc: bool,
  rules: Rules,
) -> String {
  // A doc comment's empty last line goes, as a block comment's does; a group of other
  // line comments keeps one.
  let drops_last = is_doc || written.is_block();
  let is_doc = is_doc || written.is_doc();
  let style = if block {
    Style::SingleBullet
  } else {
    Style::of(orig, rules.normalize)
  };
  let max_width = width
    .checked_sub(style.closer().len() + style.opener().len())
    .unwrap_or(1);
  let mut out = Rewrite {
    result: style.opener().to_string(),
    code: None,
    item: None,
    prev_multi: false,
    separator: format!("\n{}{}", indent, style.prefix()),
    indent_str: format!("\n{}", indent),
    indent,
    opener: style.opener(),
    closer: style.closer(),
    prefix: style.prefix(),
    block: style.is_block(),
    newlines: orig.matches('\n').count(),
    is_doc,
    rules,
    max_width,
    width: max_width,
    offset: 0,
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
    let (mut line, spaced) = left_trim(line, written);
    if orig.starts_with("/*") && breaks == 0 {
      line = line.trim_start();
    }
    if out.line(line, spaced || rules.normalize, i == newlines && drops_last) {
      break;
    }
  }
  out.finish()
}

/// Whether a line may hold a URL or a reference link's target (`[a]: b`).
fn has_url(line: &str) -> bool {
  ["https://", "http://", "ftp://", "file://"]
    .iter()
    .any(|scheme| line.contains(scheme))
    || line.starts_with('[')
      && line.match_indices(']').any(|(close, _)| {
        let after = line[close + 1..].trim_start_matches([' ', '\t']);
        close > 1 && after.starts_with(':') && line[close + 1..].len() - after.len() <= 1
      })
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

/// Whether a line is a row of a Markdown table (`| a | b |`).
fn is_table_item(line: &str) -> bool {
  let line = line.trim_start();
  line.starts_with('|') && line.rfind('|').is_some_and(|at| at > 0)
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
