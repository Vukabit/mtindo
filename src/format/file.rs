//! The whole-file rules: what formatting does before expressions are laid out.
//!
//! Only the whitespace between tokens changes, so a program means what it meant. For each
//! line break the formatter decides how many blank lines stay and how far the next line is
//! indented:
//!
//! - **Lines that start something in a container** (an item, statement, field, variant,
//!   match arm, list element, or a comment among them) are indented one level past the line
//!   the container's owner starts on: the closure, `if`, match arm or item a block belongs
//!   to, the call an argument list belongs to (for a method call, the line of its `.`). A
//!   line starting with the container's closing delimiter is indented as that line.
//!   Containers are blocks, item and field lists, match arms, and delimited lists
//!   (arguments, parameters, arrays, tuples, struct literals, use lists).
//! - **Other lines** continue the construct above them and move by as much as it moved, so a
//!   multi-line expression keeps its shape. A comment just before a closing delimiter moves
//!   with its container, and a line comment written under a comment that ends a line of
//!   code stays under it.
//! - **Within a line**, the space between two tokens follows [`super::space`].
//! - Trailing whitespace is trimmed after code and line comments (doc comments keep
//!   theirs), blank lines are kept within `blank_lines_upper_bound` and dropped at the start
//!   and end of a container (a module body keeps its first, an extern block both), and the
//!   file ends with one line break, in the `newline_style`.
//!
//! Left as written, as the standard formatter leaves them: macro bodies (a braced call's
//! re-indented, keeping its layout, unless `rustfmt::skip::macros` names it),
//! `macro_rules!` definitions, attributes it cannot read as names and values, anything
//! marked `#[rustfmt::skip]`, `// mto: off` regions, files that skip themselves, files with
//! syntax errors, files `disable_all_formatting` or `format_generated_files` turn away, a
//! statement with a comment between a binary operator's operands, and a statement or item
//! with a line of code longer than `max_width`, which the standard formatter gives up on
//! (a braced macro call aside).
//! The lines of a closure, an argument or a match arm count toward the statement around it,
//! since the standard formatter formats them as part of its expression. Lines after a
//! statement's first are measured as written; its first line is measured as the pass reaches
//! it, at its new indentation, so that a second run decides the same.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

use crate::format::arms::{Arm, rewrite_arm};
use crate::format::attrs::rewrite_attr;
use crate::format::expr::{is_statement, lines_block};
use crate::format::imports::rewrite_use;
use crate::format::items::{is_item, rewrite_header};
use crate::format::space::{spacing, wants_space};
use crate::format::stmt::{
  rewrite_control_head, rewrite_head, rewrite_let_control, rewrite_statement,
};
use crate::options::format::{FormatConfig, NewlineStyle};
use crate::syntax::ast::Attr;
use crate::syntax::{
  Edition, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken, TextRange, parse, parse_cached,
};
use crate::text::unicode::char_width;

/// A set of byte offsets.
type OffsetSet = HashSet<usize, BuildHasherDefault<OffsetHasher>>;

/// A map keyed by byte offsets.
type Offsets<V> = HashMap<usize, V, BuildHasherDefault<OffsetHasher>>;

/// Ranges of the text, as `(start, end)`.
type Ranges = Vec<(usize, usize)>;

/// Hashes a byte offset with one multiplication. Offsets are only looked up, never iterated
/// in hash order, and come from the file being formatted, so a hash built to resist
/// attackers buys nothing here and costs time on every token.
#[derive(Default)]
struct OffsetHasher(u64);

impl Hasher for OffsetHasher {
  fn finish(&self) -> u64 {
    self.0
  }

  fn write(&mut self, bytes: &[u8]) {
    for byte in bytes {
      self.write_u64(u64::from(*byte));
    }
  }

  fn write_usize(&mut self, n: usize) {
    self.write_u64(n as u64);
  }

  fn write_u64(&mut self, n: u64) {
    // Fibonacci hashing, folded so both the high and the low bits vary.
    let h = (self.0 ^ n).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    self.0 = h ^ (h >> 32);
  }
}

/// The roles tokens play in the indentation.
#[derive(Default)]
struct Roles {
  /// A token that starts something in a container, with the container's opening offset
  /// (`None` at the top level).
  starts: Offsets<Option<usize>>,
  /// A container's closing delimiter, with its opening offset.
  closers: Offsets<usize>,
  /// Containers' opening delimiters, with where the line their indentation follows is.
  openers: Offsets<usize>,
  /// Comments just before a container's closing delimiter, with its opening offset.
  trailing: Offsets<usize>,
  /// Containers whose blank lines after the opening delimiter stay (module and extern
  /// bodies).
  keeps_leading_blank: OffsetSet,
  /// Containers whose contents are not indented past them (match arms, with
  /// `match_arm_indent = false`), by their opening offset.
  unindented: OffsetSet,
  /// Comments before the `}` of an `if`'s block that an `else` follows: at the `}`'s
  /// indentation, not the block's contents', by where they start.
  outdented: OffsetSet,
  /// Containers whose blank lines before the closing delimiter stay (extern bodies), by
  /// their opening offset.
  keeps_trailing_blank: OffsetSet,
  /// Ranges left exactly as written.
  verbatim: Vec<(usize, usize)>,
  /// A braced macro call's arguments, by where they start: where they end. Not laid out,
  /// but re-indented as the standard formatter's `trim_left_preserve_layout` does.
  preserved: Offsets<usize>,
  /// Statements left as written whole, from their first token, by where they start.
  kept_whole: OffsetSet,
  /// Item headers with a written line wider than the line width, by where their own text
  /// starts: where they end. Left as written when they cannot be laid out.
  wide_headers: Offsets<usize>,
  /// For `mto debug layout`: why each statement and header with a written line past the
  /// line width is wide, by where it starts.
  wide_reasons: HashMap<usize, String>,
  /// `macro_rules!` definitions, by where their `macro_rules` starts: written anew when
  /// their rules format, else left as written from there.
  macro_rules: Offsets<SyntaxNode>,
  /// Where a token starts that follows the one before it on its line after one space: an
  /// `else` after its `if`'s block, and what follows the `else`.
  spaced: OffsetSet,
  /// Where a `}` starts that closes braces holding nothing but whitespace: it follows the
  /// `{` (`{}`).
  tight: OffsetSet,
  /// Statements and items, by where they start: where they end.
  statements: Offsets<usize>,
  /// Where line comments and line doc comments start, in order.
  line_comments: Vec<usize>,
  /// Statements in blocks the width-budget code may lay out, `use` declarations and
  /// attributes, by where they start.
  layable: Offsets<SyntaxNode>,
  /// Where a token starts that the Rust style puts at the start of a line, even when it is
  /// written after other code.
  breaks: OffsetSet,
  /// Where an attribute starts that follows another of the same item: no blank line comes
  /// between them.
  joined: OffsetSet,
  /// Where a skipped `mod name;` starts: its first line keeps the indentation written.
  kept_indent: OffsetSet,
  /// The spaces after a skipped `mod name;` before an item on its line, and between a
  /// skipped member's attributes and its code on their line: the line's indentation.
  after_skipped_mod: OffsetSet,
  /// Comments on lines of their own closing an `if`'s first block before its `else`, after
  /// one ending the block's last line: by where they start, the block's `{`, whose line's
  /// indentation they take ([`else_close_comments`]).
  else_close: Offsets<usize>,
  /// Every token of the file, in order: found by the same walk, which makes the handles of
  /// the nodes above them once.
  tokens: Vec<SyntaxToken>,
}

/// What [`overflowing`] measures written lines against: the statements in tree order, where
/// lines start, the chains, the raw strings spanning lines, the attributes, where line
/// comments start and the block comments spanning lines.
#[derive(Clone, Copy)]
struct Spans<'a> {
  statements: &'a [(usize, usize)],
  breaks: &'a OffsetSet,
  chains: &'a [(usize, usize)],
  raw_strings: &'a [(usize, usize)],
  /// From the 2024 style, strings whose lines but the last end with `\`, kept however wide
  /// (`rewrite_string_lit`): no line holding one is measured.
  continued_strings: &'a [(usize, usize)],
  attrs: &'a [(usize, usize)],
  line_comments: &'a [usize],
  block_comments: &'a [(usize, usize)],
  /// Where an item starts after a block comment before it on its line, in a list whose
  /// layout may put the comment on a line of its own: measured apart.
  after_pre_comments: &'a OffsetSet,
  /// Statements that are all a match arm's block holds: the standard formatter flattens
  /// the block, so one too wide fails the statement around the `match`.
  flattened: &'a OffsetSet,
}

/// A token, with where it starts.
struct Tok<'a> {
  kind: SyntaxKind,
  /// Its text, borrowed from the file's.
  text: &'a str,
  start: usize,
  token: SyntaxToken,
  /// An empty space put between two tokens written touching, where a line may break.
  synthetic: bool,
}

/// Whether this pass writes a block's insides on lines of their own: a block of lines, a
/// control expression's, a plain block ending a block that holds what goes on lines, or one
/// holding a comment.
pub fn block_of_lines(node: &SyntaxNode) -> bool {
  node.kind() == SyntaxKind::BlockExpr
    && (lines_block(node)
      || control_block(node)
      || crate::format::expr::plain_tail_block(node) && !crate::format::expr::joins(node)
      // A block standing as a statement: on lines, as the visitor writes a statement's.
      || standing_block(node)
      // An arm's block around an `if` whose condition spans lines (`flatten_arm_body`).
      || node.parent().is_some_and(|arm| arm.kind() == SyntaxKind::MatchArm)
        && crate::format::expr::lone_expression(node).is_some_and(|e| e.kind() == SyntaxKind::IfExpr))
    || commented_block(node)
}

/// The file formatted; the input itself when it cannot be formatted safely.
pub fn format_file(source: &str, edition: Edition, config: &FormatConfig) -> String {
  format_text(source, edition, config, true)
}

/// [`format_file`]; a macro's body keeps its empty statements (`strip`), which a macro may
/// match on.
pub fn format_text(source: &str, edition: Edition, config: &FormatConfig, strip: bool) -> String {
  let _memo = crate::format::expr::RewriteMemo::open();
  let before =
    SKIPPED_INVOCATIONS.with(|names| names.replace(config.skip_macro_invocations.clone()));
  let crate_before = CRATE_SKIPS.with(|names| {
    names.replace((
      config.crate_skipped_macros.clone(),
      config.crate_skipped_attributes.clone(),
    ))
  });
  let edition_before = EDITION.with(|e| e.replace(edition));
  let style_before = STYLE.with(|e| e.replace(config.style_edition));
  let out = format_text_with(source, edition, config, strip);
  STYLE.with(|e| e.set(style_before));
  SKIPPED_INVOCATIONS.with(|names| names.replace(before));
  CRATE_SKIPS.with(|names| names.replace(crate_before));
  EDITION.with(|e| e.set(edition_before));
  out
}

/// The edition of the text being formatted.
pub(super) fn edition() -> Edition {
  EDITION.with(|e| e.get())
}

pub(super) fn meta_like(attr: &SyntaxNode) -> bool {
  // Inside an argument list a path's `::` is two colons, written touching.
  let colons: Vec<(usize, usize)> = attr
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Colon => {
        Some((t.text_range().start as usize, t.text_range().end as usize))
      }
      _ => None,
    })
    .collect();
  let in_path = |start: usize, end: usize| colons.iter().any(|&(s, e)| e == start || s == end);
  let mut previous: Option<SyntaxKind> = None;
  for element in attr.descendants_with_tokens_iter() {
    let SyntaxElement::Token(token) = element else {
      if previous == Some(SyntaxKind::Eq) && element.kind() != SyntaxKind::Literal {
        return false;
      }
      continue;
    };
    let kind = token.kind();
    if kind.is_trivia() {
      continue;
    }
    let allowed = kind.is_literal()
      || kind.name().ends_with("_KW")
      || matches!(
        kind,
        SyntaxKind::Ident
          | SyntaxKind::RawIdent
          | SyntaxKind::Colon2
          | SyntaxKind::Comma
          | SyntaxKind::Eq
          | SyntaxKind::LParen
          | SyntaxKind::RParen
          | SyntaxKind::LBrack
          | SyntaxKind::RBrack
          | SyntaxKind::Pound
          | SyntaxKind::Bang
      );
    // A `!` only as an inner attribute's own: a macro call among the arguments is not a
    // name or a value. Brackets only as its own too (`["a"]` is no meta item), and a `(`
    // only after a name (`a(..)`, not `()`).
    let allowed = allowed
      && (kind != SyntaxKind::Bang || token.parent().kind() == SyntaxKind::Attr)
      && (!matches!(kind, SyntaxKind::LBrack | SyntaxKind::RBrack)
        || token.parent().kind() == SyntaxKind::Attr)
      && (kind != SyntaxKind::LParen
        || matches!(previous, Some(SyntaxKind::Ident | SyntaxKind::RawIdent))
        || previous.is_some_and(|p| p.name().ends_with("_KW")));
    // A literal with a suffix it cannot take (`""huh`, `1omg`) is no value.
    let allowed = allowed && (!kind.is_literal() || valid_suffix(kind, token.text()));
    let allowed = allowed
      || kind == SyntaxKind::Colon
        && in_path(
          token.text_range().start as usize,
          token.text_range().end as usize,
        );
    let value = kind.is_literal()
      || matches!(kind, SyntaxKind::TrueKw | SyntaxKind::FalseKw)
      || matches!(token.text(), "true" | "false");
    if !allowed || (previous == Some(SyntaxKind::Eq) && !value) {
      return false;
    }
    previous = Some(kind);
  }
  true
}

/// A braced macro call's arguments as the standard formatter's `trim_left_preserve_layout`
/// writes them: the first line as written, without trailing whitespace; each later line
/// trimmed and indented from `indent` by as much as it was past the least indented of them
/// (tabs counting `tab_spaces`); a blank line empty. A line starting inside a string is kept
/// as written, and counts toward no indentation (nor does the one after it, while the
/// string goes on).
pub(super) fn preserve_layout(snippet: &str, indent: usize, config: &FormatConfig) -> String {
  let tab = config.tab_spaces.max(1);
  // Where each line break inside a string literal is.
  let mut in_string: HashSet<usize> = HashSet::new();
  let mut at = 0;
  for (kind, text) in crate::syntax::parse_tokens(snippet, Edition::E2021) {
    if kind.is_literal() {
      for (i, _) in text.match_indices('\n') {
        in_string.insert(at + i);
      }
    }
    at += text.len();
  }
  let leading = |line: &str| {
    line
      .chars()
      .take_while(|c| *c == ' ' || *c == '\t')
      .map(|c| if c == '\t' { tab } else { 1 })
      .sum::<usize>()
  };
  // Each line after the first: its text, whether it starts inside a string, whether it
  // ends inside one.
  let mut lines: Vec<(&str, bool, bool)> = Vec::new();
  let mut offset = snippet.find('\n').map_or(snippet.len(), |n| n + 1);
  let first = snippet[..offset].trim_end();
  while offset <= snippet.len()
    && offset > 0
    && offset - 1 < snippet.len()
    && snippet.as_bytes()[offset - 1] == b'\n'
  {
    let starts_in = in_string.contains(&(offset - 1));
    let end = snippet[offset..]
      .find('\n')
      .map_or(snippet.len(), |n| offset + n);
    let ends_in = in_string.contains(&end);
    lines.push((&snippet[offset..end], starts_in, ends_in));
    if end >= snippet.len() {
      break;
    }
    offset = end + 1;
  }
  // `InString`: starting and ending inside a string, kept as written unless it continues
  // with `\`; the lines inside strings count toward no minimum.
  let mut veto = false;
  let mut kept: Vec<bool> = Vec::with_capacity(lines.len());
  // The lines measured for the least indentation.
  let mut measured: Vec<usize> = Vec::new();
  // Whether the line before ends with `\` (a string continued past its line break, whose
  // leading whitespace is not the string's).
  let mut continued = first.ends_with('\\');
  for (i, &(line, starts_in, ends_in)) in lines.iter().enumerate() {
    // A line whose first character opens a string counts as one in a string
    // (`LineClasses` takes a line's kind from its first character).
    let opens = !starts_in && line.starts_with('"');
    // The last line, with no line break to take its kind from, has that of its last
    // character.
    let last = i + 1 == lines.len();
    let new_veto = (starts_in || opens) && ends_in && !line.ends_with('\\');
    // A line starting inside a string not continued with `\` starts with the string's own
    // whitespace: kept, where the standard formatter would re-indent it.
    kept.push(veto || new_veto || starts_in && !continued);
    veto = new_veto;
    continued = line.ends_with('\\');
    if !line.trim().is_empty() && (last || !starts_in && !opens) {
      measured.push(i);
    }
  }
  let least_of = |measured: &[usize]| measured.iter().map(|&i| leading(lines[i].0)).min();
  // At column 0, a measured line starting with a string that would land there would not be
  // measured on a second run: it is left out now, until none lands there.
  if indent == 0 {
    while let Some(least) = least_of(&measured) {
      let before = measured.len();
      measured
        .retain(|&i| !(leading(lines[i].0) == least && lines[i].0.trim_start().starts_with('"')));
      if measured.len() == before {
        break;
      }
    }
  }
  // No line to measure (all inside a string): as written (`.min()?`).
  let Some(least) = least_of(&measured) else {
    return snippet.to_string();
  };
  let mut out = first.to_string();
  for (&(line, _, ends_in), keep) in lines.iter().zip(kept) {
    out.push('\n');
    if keep {
      out.push_str(line);
    } else if !line.trim().is_empty() {
      out.push_str(&" ".repeat(indent + leading(line).saturating_sub(least)));
      // A line ending inside a string keeps its end, which is the string's.
      out.push_str(if ends_in {
        line.trim_start()
      } else {
        line.trim()
      });
    }
  }
  out
}

/// The display width of text: tabs count as `tab_spaces`, wide characters as two columns.
/// A block comment across lines with its later lines placed from column `column`: each
/// trimmed, one starting with `*` a space in (`light_rewrite_comment`); when any does not
/// start with `*`, their layout kept, the least indented at the column
/// (`trim_left_preserve_layout`).
pub(super) fn reindent_block_comment(
  text: &str,
  column: usize,
  tab: usize,
  config: &FormatConfig,
  // After code on its line from the 2024 style: its later lines all trimmed.
  trimmed: bool,
) -> String {
  let mut lines = text.split('\n');
  let first = lines.next().unwrap_or("").trim_end();
  let rest: Vec<&str> = lines.collect();
  let bare = rest.iter().any(|l| {
    let t = l.trim_start();
    !t.is_empty() && !t.starts_with('*')
  });
  let mut out = first.to_string();
  if bare && !trimmed {
    // A line inside a string the comment's code opened stays as written, and does not count
    // toward the least indentation.
    let mut in_string = first.matches('"').count() % 2 == 1;
    let mut kept = Vec::with_capacity(rest.len());
    for line in &rest {
      kept.push(in_string);
      if line.matches('"').count() % 2 == 1 {
        in_string = !in_string;
      }
    }
    let prefix = |l: &str| width_of(&l[..l.len() - l.trim_start().len()], tab);
    let min = rest
      .iter()
      .zip(&kept)
      .filter(|(l, k)| !l.trim().is_empty() && !**k)
      .map(|(l, _)| prefix(l))
      .min()
      .unwrap_or(0);
    for (line, &k) in rest.iter().zip(&kept) {
      out.push('\n');
      if k {
        out.push_str(line);
      } else if !line.trim().is_empty() {
        out.push_str(&indentation(
          column + prefix(line).saturating_sub(min),
          config,
        ));
        out.push_str(line.trim());
      }
    }
  } else {
    for line in &rest {
      out.push('\n');
      let t = line.trim();
      // Trimmed, a blank line is empty (its trailing whitespace goes).
      if trimmed && t.is_empty() {
        continue;
      }
      out.push_str(&indentation(column, config));
      // A `*` line a space in, but among bare lines unless all are trimmed; one written at
      // the start of its line only when the comment is indented (as a second run would
      // read it).
      if t.starts_with('*')
        && (!bare || trimmed)
        && (column > 0 || line.len() > line.trim_start().len())
      {
        out.push(' ');
      }
      out.push_str(t);
    }
  }
  out
}

/// Whether a macro call is one a `#[rustfmt::skip::macros(..)]` around it names, or
/// `skip_macro_invocations` does: left exactly as written.
pub(super) fn skipped_macro(call: &SyntaxNode) -> bool {
  let Some(path) = call.children_iter().find(|c| c.kind() == SyntaxKind::Path) else {
    return false;
  };
  let written: String = path.text().to_string().split_whitespace().collect();
  if SKIPPED_INVOCATIONS.with(|names| names.borrow().iter().any(|n| n == "*" || *n == written)) {
    return true;
  }
  let name = path.text().to_string();
  let name = name.rsplit("::").next().unwrap_or("").trim().to_string();
  if CRATE_SKIPS.with(|names| names.borrow().0.contains(&name)) {
    return true;
  }
  call.ancestors_iter().any(|owner| {
    owner
      .children_iter()
      .filter(|c| c.kind() == SyntaxKind::Attr)
      .any(|attr| {
        let text: String = attr.text().to_string().split_whitespace().collect();
        text
          .split_once("rustfmt::skip::macros(")
          .and_then(|(_, rest)| rest.split_once(')'))
          .is_some_and(|(names, _)| names.split(',').any(|n| n == name))
      })
  })
}

thread_local! {
  /// `skip_macro_invocations` for the text being formatted.
  static SKIPPED_INVOCATIONS: std::cell::RefCell<Vec<String>> =
    const { std::cell::RefCell::new(Vec::new()) };
  /// The crate root's skipped macros and attributes, for the text being formatted.
  static CRATE_SKIPS: std::cell::RefCell<(Vec<String>, Vec<String>)> =
    const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
  /// The edition of the text being formatted.
  static EDITION: std::cell::Cell<Edition> = const { std::cell::Cell::new(Edition::E2021) };
  /// The style edition of the text being formatted.
  static STYLE: std::cell::Cell<u16> = const { std::cell::Cell::new(2015) };
}

/// Whether the text being formatted is in style edition 2024 or later.
pub(super) fn style_2024() -> bool {
  STYLE.with(|e| e.get()) >= 2024
}

/// For a struct's fields, an enum's variants or a match's arms laid out one per line from
/// column `indent`, the spaces before each item's comment after it on its line, as a list's
/// are aligned (`write_list`); `None` where there is none.
fn align_list_comments(
  list: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<Vec<Option<usize>>> {
  let items = list_items_of(list);
  let comments = crate::format::lists::list_comments(list, &items)?;
  let spacing = crate::format::shape::Spacing::of(config);
  // An arm spanning lines counts all its text, as the standard formatter measures it; one
  // laid out in parts counts past any width.
  let texts: Vec<String> = items
    .iter()
    .map(|item| match item.kind() {
      SyntaxKind::MatchArm => match rewrite_arm(item, indent, config) {
        Some(Arm::Whole(text)) => text,
        _ => format!("\n{}", " ".repeat(config.max_width)),
      },
      _ => match crate::format::items::rewrite_field(item, indent, config) {
        Some((text, _)) => text.trim_end_matches(',').to_string(),
        None => crate::format::expr::flat(item, spacing),
      },
    })
    // With its attributes and doc comments, which put it on lines of its own.
    .zip(&items)
    .map(|(text, item)| {
      let start = item.text_range().start as usize;
      let attributed = item
        .children_with_tokens_iter()
        .any(|e| e.kind() == SyntaxKind::Attr || crate::format::items::is_doc(e.kind()));
      match crate::format::items::header_start(item) {
        Some(code) if code > start && attributed => {
          let attributes = item.text().to_string()[..code - start].trim().to_string();
          format!("{}\n{}{}", attributes, " ".repeat(indent), text)
        }
        _ => text,
      }
    })
    .collect();
  Some(crate::format::lists::post_comment_spaces(
    &texts,
    &comments,
    indent,
    config.max_width,
    list.kind() != SyntaxKind::MatchArmList,
  ))
}

/// Whether the comment after the whitespace `tokens[i]`, which follows an attribute's `]`
/// on `line` (the output's line so far), goes to the next line: see where it is called.
fn attr_comment_moves(
  tokens: &[Tok<'_>],
  i: usize,
  line: &str,
  tab: usize,
  config: &FormatConfig,
) -> bool {
  let before = &tokens[i - 1].token;
  if before.kind() != SyntaxKind::RBrack || before.parent().kind() != SyntaxKind::Attr {
    return false;
  }
  let attr = before.parent();
  let Some(owner) = attr.parent() else {
    return false;
  };
  // The comments up to the next code, and that code.
  let mut comments: Vec<&str> = Vec::new();
  let mut after = None;
  for t in &tokens[i + 1..] {
    match t.kind {
      SyntaxKind::LineComment | SyntaxKind::BlockComment => comments.push(t.text.trim_end()),
      SyntaxKind::Whitespace => {}
      _ => {
        after = Some(t);
        break;
      }
    }
  }
  let Some(after) = after else {
    return false;
  };
  let indent = width_of(&line[..line.len() - line.trim_start().len()], tab);
  let next_attribute = after.kind == SyntaxKind::Pound
    && after.token.parent().kind() == SyntaxKind::Attr
    && after.token.parent().parent().as_ref() == Some(&owner);
  if next_attribute {
    // The comments rewritten, each on a line at the indentation; 1 = the space before.
    let length: usize = comments.iter().map(|c| c.len()).sum::<usize>()
      + comments.len().saturating_sub(1) * (1 + indent)
      + 1;
    return length > config.max_width;
  }
  // Not an item, in a block too: its comment stays (`visit_attrs`). A field or a variant
  // combines its attributes as a statement does.
  // So does a module, `use` or `extern crate` formatted with the declarations sorted.
  let statement = matches!(
    owner.kind(),
    SyntaxKind::LetStmt | SyntaxKind::ExprStmt | SyntaxKind::RecordField | SyntaxKind::Variant
  ) || matches!(
    owner.kind(),
    SyntaxKind::Module | SyntaxKind::Use | SyntaxKind::ExternCrate
  ) && crate::format::reorder::sortable(&owner);
  statement
    && comments
      .first()
      .is_some_and(|c| width_of(line, tab) + 1 + width_of(c, tab) > config.max_width)
}

/// Whether a node is a macro call with braces (`m! { .. }`), or a statement of one.
fn braced_macro(node: &SyntaxNode) -> bool {
  let mut node = node.clone();
  while matches!(node.kind(), SyntaxKind::ExprStmt | SyntaxKind::MacroExpr)
    && let Some(inner) = node.children_iter().find(|c| c.kind() != SyntaxKind::Attr)
  {
    node = inner;
  }
  node.kind() == SyntaxKind::MacroCall
    && node.children_iter().any(|c| {
      c.kind() == SyntaxKind::TokenTree
        && c
          .first_token()
          .is_some_and(|t| t.kind() == SyntaxKind::LBrace)
    })
}

/// Whether a `let`'s value, an expression statement or a block's tail is a method chain (`a.b()`, `a.b()?`).
fn chain_value(node: &SyntaxNode) -> bool {
  let value = match node.kind() {
    SyntaxKind::ExprStmt => node.children_iter().find(|c| c.kind() != SyntaxKind::Attr),
    SyntaxKind::LetStmt => node
      .children_with_tokens_iter()
      .skip_while(|e| e.kind() != SyntaxKind::Eq)
      .find_map(|e| match e {
        SyntaxElement::Node(n) => Some(n),
        SyntaxElement::Token(_) => None,
      }),
    _ if crate::format::stmt::is_tail(node) => Some(node.clone()),
    _ => None,
  };
  let mut value = value;
  while let Some(v) = &value
    && matches!(v.kind(), SyntaxKind::TryExpr | SyntaxKind::AwaitExpr)
  {
    value = v.children_iter().next();
  }
  value.is_some_and(|v| v.kind() == SyntaxKind::MethodCallExpr)
}

/// A block comment on one line followed on its line by more: the standard formatter puts
/// what follows on the next line (`missed_spans`). After a block comment starting its line,
/// a comment goes to the next line, and so does each comment line after it up to a blank
/// line or code; code goes to the next line. So after one ending an item's line, between
/// items. Not after one ending a statement's line, nor among a list's items. (Its first run
/// puts those comments at column 1, which its second moves back: written where it settles.)
fn comment_runs(_root: &SyntaxNode, roles: &mut Roles) {
  let containers = |kind: SyntaxKind| {
    matches!(
      kind,
      SyntaxKind::SourceFile
        | SyntaxKind::ItemList
        | SyntaxKind::AssocItemList
        | SyntaxKind::BlockExpr
        | SyntaxKind::ExternItemList
    )
  };
  let item_level = |kind: SyntaxKind| {
    matches!(
      kind,
      SyntaxKind::SourceFile | SyntaxKind::ItemList | SyntaxKind::AssocItemList
    )
  };
  // Between what a container holds: in it, or the comments an item or statement in it
  // carries.
  let between = |t: &SyntaxToken| {
    let parent = t.parent();
    containers(parent.kind())
      || (is_item(parent.kind()) || is_statement(parent.kind()))
        && parent.parent().is_some_and(|p| containers(p.kind()))
        // Before its code (one it carries), not in its header; or after an item's code
        // (one it trails).
        && (crate::format::items::header_start(&parent)
          .is_some_and(|start| (t.text_range().start as usize) < start)
          || is_item(parent.kind())
            && t.text_range().start as usize >= crate::format::imports::code_end(&parent))
  };
  let tokens = roles.tokens.clone();
  let is_comment = |k: SyntaxKind| matches!(k, SyntaxKind::LineComment | SyntaxKind::BlockComment);
  for (i, comment) in tokens.iter().enumerate() {
    if comment.kind() != SyntaxKind::BlockComment
      || comment.text().contains('\n')
      || !between(comment)
    {
      continue;
    }
    // Before it on its line.
    let mut p = i;
    while p > 0
      && tokens[p - 1].kind() == SyntaxKind::Whitespace
      && !tokens[p - 1].text().contains('\n')
    {
      p -= 1;
    }
    let starts_line = p == 0 || tokens[p - 1].kind() == SyntaxKind::Whitespace;
    // Or ending an item's line between items: after its `;` or `}`.
    let ends_item = !starts_line
      && matches!(tokens[p - 1].kind(), SyntaxKind::Semi | SyntaxKind::RBrace)
      && tokens[p - 1].parent().ancestors_iter().any(|a| {
        is_item(a.kind())
          && a.parent().is_some_and(|q| item_level(q.kind()))
          && crate::format::imports::code_end(&a) == tokens[p - 1].text_range().end as usize
      });
    if !starts_line && !ends_item {
      continue;
    }
    // After it on its line.
    let mut n = i + 1;
    while n < tokens.len()
      && tokens[n].kind() == SyntaxKind::Whitespace
      && !tokens[n].text().contains('\n')
    {
      n += 1;
    }
    // Written touching it (`/**///`), it stays.
    if n >= tokens.len() || tokens[n].kind() == SyntaxKind::Whitespace || n == i + 1 {
      continue;
    }
    if is_comment(tokens[n].kind()) {
      // The comment, then each comment line after it.
      let mut at = n;
      loop {
        roles.breaks.insert(tokens[at].text_range().start as usize);
        if at + 2 < tokens.len()
          && tokens[at + 1].kind() == SyntaxKind::Whitespace
          && tokens[at + 1].text().matches('\n').count() <= 1
          && is_comment(tokens[at + 2].kind())
        {
          at += 2;
        } else {
          break;
        }
      }
    } else if starts_line && !matches!(tokens[n].kind(), SyntaxKind::RBrace) {
      // Code after it starts the next line.
      roles.breaks.insert(tokens[n].text_range().start as usize);
    }
  }
  // After an inner doc comment, a comment on its line before another inner attribute goes
  // to a line of its own, as the attributes are written together.
  for i in 0..tokens.len() {
    if tokens[i].kind() != SyntaxKind::InnerBlockDoc {
      continue;
    }
    let mut n = i + 1;
    if n < tokens.len()
      && tokens[n].kind() == SyntaxKind::Whitespace
      && !tokens[n].text().contains('\n')
    {
      n += 1;
    }
    if n >= tokens.len() || !is_comment(tokens[n].kind()) {
      continue;
    }
    let next_code = tokens[n..]
      .iter()
      .find(|t| !t.kind().is_trivia() && !crate::format::items::is_doc(t.kind()));
    let inner_attribute = next_code.is_some_and(|t| {
      t.kind() == SyntaxKind::Pound
        && t.parent().kind() == SyntaxKind::Attr
        && t
          .parent()
          .children_with_tokens_iter()
          .any(|e| e.kind() == SyntaxKind::Bang)
    });
    if inner_attribute {
      roles.breaks.insert(tokens[n].text_range().start as usize);
    }
  }
}

/// Whether a block of one expression holds a comment: the standard formatter writes it on
/// lines, as it neither joins it onto one line nor unwraps a closure's or an arm's body.
fn commented_block(node: &SyntaxNode) -> bool {
  node.kind() == SyntaxKind::BlockExpr
    && !crate::format::expr::joins(node)
    && !crate::format::expr::one_line_if_block(node)
    && node.children_iter().any(|c| c.kind() != SyntaxKind::Label)
    && node
      .descendants_where(|kind| matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment))
      .next()
      .is_some()
}

/// Where a container's child (of `kind`, at `range`, a node or not) starts, if it is
/// something that starts a line: a node, or a comment standing among them.
fn construct_start((kind, range, node): (SyntaxKind, TextRange, bool)) -> Option<usize> {
  let starts = node
    || matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment)
    || crate::format::items::is_doc(kind);
  starts.then_some(range.start as usize)
}

fn container(node: &SyntaxNode, text: &str, roles: &mut Roles, config: &FormatConfig) {
  let children: Vec<(SyntaxKind, TextRange, bool)> = node.child_spans().collect();
  let is_open = |k: SyntaxKind| {
    matches!(
      k,
      SyntaxKind::LBrace | SyntaxKind::LParen | SyntaxKind::LBrack
    )
  };
  let is_close = |k: SyntaxKind| {
    matches!(
      k,
      SyntaxKind::RBrace | SyntaxKind::RParen | SyntaxKind::RBrack
    )
  };
  let open = children
    .iter()
    .find(|&&(kind, _, node)| !node && is_open(kind))
    .map(|&(_, range, _)| range.start as usize);
  let Some(open) = open else {
    return;
  };
  // The line the contents' indentation follows: the owner's first line, or for a method
  // call's arguments the line of its `.`. Arrays, tuples, anything standing in another
  // container (a block that is a statement) and a binary operator's operand (`&& {`) are
  // their own owners.
  let owner = match node.parent() {
    Some(parent)
      if !is_container(parent.kind())
        && parent.kind() != SyntaxKind::BinExpr
        && !matches!(node.kind(), SyntaxKind::ArrayExpr | SyntaxKind::TupleExpr) =>
    {
      parent
    }
    _ => node.clone(),
  };
  let base = match Some(owner) {
    Some(owner) if owner.kind() == SyntaxKind::MethodCallExpr => owner
      .children_with_tokens_iter()
      .find_map(|e| match e {
        SyntaxElement::Token(t) if t.kind() == SyntaxKind::Dot => {
          Some(t.text_range().start as usize)
        }
        _ => None,
      })
      .unwrap_or(open),
    // A node with text starts with its first token.
    Some(owner) if owner.kind() != SyntaxKind::SourceFile && !owner.text_range().is_empty() => {
      owner.text_range().start as usize
    }
    Some(owner) if owner.kind() != SyntaxKind::SourceFile => owner
      .first_token()
      .map_or(open, |t| t.text_range().start as usize),
    _ => open,
  };
  roles.openers.insert(open, base);
  // `{` and `}` with only whitespace between them close up: an item's body only after a
  // header laid out on one line (marked as it is laid out).
  let item_body = node.parent().is_some_and(|p| is_item(p.kind()));
  // An `if`'s branch when it has an `else`, or is one, keeps its `}` on a line of its own.
  let if_with_else = node.parent().is_some_and(|p| {
    p.kind() == SyntaxKind::IfExpr
      && (p.children_iter().count() == 3
        || p.parent().is_some_and(|g| g.kind() == SyntaxKind::IfExpr))
  });
  // Empty, it is `{` and a `}` starting the next line.
  if if_with_else
    && let Some(open_at) = children
      .iter()
      .position(|&(kind, _, node)| !node && kind == SyntaxKind::LBrace)
    && let Some(&(SyntaxKind::RBrace, close, false)) = children[open_at + 1..]
      .iter()
      .find(|&&(kind, _, _)| kind != SyntaxKind::Whitespace)
  {
    roles.breaks.insert(close.start as usize);
  }
  if !item_body
    && !if_with_else
    && let Some(open_at) = children
      .iter()
      .position(|&(kind, _, node)| !node && kind == SyntaxKind::LBrace)
    && let Some(&(SyntaxKind::RBrace, close, false)) = children[open_at + 1..]
      .iter()
      .find(|&&(kind, _, _)| kind != SyntaxKind::Whitespace)
  {
    roles.tight.insert(close.start as usize);
  }
  // A block holding comments and no code keeps a blank line after its `{` too.
  let comments_only = node.kind() == SyntaxKind::BlockExpr
    && children.iter().all(|&(kind, _, is_node)| {
      !is_node && (kind.is_trivia() || matches!(kind, SyntaxKind::LBrace | SyntaxKind::RBrace))
    })
    && children
      .iter()
      .any(|&(kind, _, _)| matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment));
  if comments_only
    || matches!(
      node.kind(),
      SyntaxKind::ItemList | SyntaxKind::ExternItemList
    )
  {
    roles.keeps_leading_blank.insert(open);
  }
  if node.kind() == SyntaxKind::ExternItemList {
    roles.keeps_trailing_blank.insert(open);
  }
  if node.kind() == SyntaxKind::MatchArmList && !config.match_arm_indent {
    roles.unindented.insert(open);
  }
  let comment = |kind| matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment);
  // The comments between the last code and the closing delimiter, which keep their place
  // from it; in a block holding nothing else they are indented as its contents would be.
  let only_comments = matches!(
    node.kind(),
    SyntaxKind::BlockExpr
      | SyntaxKind::ItemList
      | SyntaxKind::AssocItemList
      | SyntaxKind::MatchArmList
  ) && children
    .iter()
    .all(|&(kind, _, is_node)| !is_node && (kind.is_trivia() || is_open(kind) || is_close(kind)));
  if !only_comments
    && let Some(close_at) = children.iter().rposition(|&(kind, _, _)| is_close(kind))
  {
    // In a block or a list of items they are indented as its contents are (the standard
    // formatter's `missed_spans`), elsewhere they keep their place.
    let block = matches!(
      node.kind(),
      SyntaxKind::BlockExpr
        | SyntaxKind::ItemList
        | SyntaxKind::AssocItemList
        | SyntaxKind::ExternItemList
        | SyntaxKind::VariantList
        | SyntaxKind::MatchArmList
    ) || node.kind() == SyntaxKind::RecordFieldList
      && node.parent().is_some_and(|p| {
        matches!(p.kind(), SyntaxKind::Struct | SyntaxKind::Union)
          || crate::format::items::variant_fields_on_lines(&p)
      });
    // Above an `else`, they go out to the `}`'s indentation, unless the first was written
    // further right than the `}` (the standard formatter's `unindent_comment_on_closing_brace`).
    let column = |at: usize| {
      let line = text[..at].rfind('\n').map_or(0, |n| n + 1);
      width_of(&text[line..at], config.tab_spaces.max(1))
    };
    let trailing: Vec<usize> = children[..close_at]
      .iter()
      .rev()
      .take_while(|&&(kind, _, node)| !node && (comment(kind) || kind == SyntaxKind::Whitespace))
      .filter(|&&(kind, _, _)| comment(kind))
      .map(|&(_, range, _)| range.start as usize)
      .collect();
    let has_statements = children
      .iter()
      .any(|&(kind, _, node)| node && kind != SyntaxKind::Attr);
    let above_else = node.kind() == SyntaxKind::BlockExpr
      && has_statements
      && node.parent().is_some_and(|p| {
        p.kind() == SyntaxKind::IfExpr
          && p
            .children_with_tokens_iter()
            .skip_while(|e| e.text_range() != node.text_range())
            .any(|e| e.kind() == SyntaxKind::ElseKw)
      })
      && trailing
        .last()
        .is_some_and(|&first| column(first) <= column(children[close_at].1.start as usize));
    for &(kind, range, node) in children[..close_at].iter().rev() {
      if !node && comment(kind) && block {
        roles.starts.insert(range.start as usize, Some(open));
        if above_else {
          roles.outdented.insert(range.start as usize);
        }
      } else if !node && comment(kind) {
        roles.trailing.insert(range.start as usize, open);
      } else if kind != SyntaxKind::Whitespace {
        break;
      }
    }
  }
  let mut inside = false;
  for &child in &children {
    let (kind, range, node) = child;
    if !node {
      let at = range.start as usize;
      if at == open {
        inside = true;
        continue;
      }
      if is_close(kind) && inside {
        roles.closers.insert(at, open);
        inside = false;
        continue;
      }
    }
    if inside && let Some(start) = construct_start(child) {
      roles.starts.insert(start, Some(open));
    }
  }
}

/// Whether a block is the body of a `while`, `for` or `loop`, wherever it stands, or of an
/// `if` or `else` that is a statement, a `let`'s value or a match arm's body, or the tail
/// of a block of lines: the standard formatter writes it on lines (an `if`'s may go on one
/// line as a tail, where the statement layout writes it so).
fn control_block(node: &SyntaxNode) -> bool {
  let Some(mut control) = node.parent() else {
    return false;
  };
  if !matches!(
    control.kind(),
    SyntaxKind::IfExpr | SyntaxKind::WhileExpr | SyntaxKind::ForExpr | SyntaxKind::LoopExpr
  ) || !node.children_iter().any(|c| c.kind() != SyntaxKind::Label)
  {
    return false;
  }
  // A loop's body, its last child: not a block it iterates over or tests
  // (`for x in unsafe { .. } {`).
  if control.kind() != SyntaxKind::IfExpr {
    return control.children_iter().next_back().as_ref() == Some(node);
  }
  while let Some(up) = control.parent().filter(|p| p.kind() == SyntaxKind::IfExpr) {
    control = up;
  }
  // A chain with a block on lines for its comment is on lines throughout.
  let mut link = Some(control.clone());
  while let Some(chain) = link {
    if chain
      .children_iter()
      .any(|c| c.kind() == SyntaxKind::BlockExpr && commented_block(&c))
    {
      return true;
    }
    link = chain
      .children_iter()
      .find(|c| c.kind() == SyntaxKind::IfExpr);
  }
  control.parent().is_some_and(|p| {
    matches!(
      p.kind(),
      SyntaxKind::ExprStmt | SyntaxKind::LetStmt | SyntaxKind::MatchArm
    ) || p.kind() == SyntaxKind::BlockExpr
      && (lines_block(&p)
        || control_block(&p)
        // An arm's block, kept around an `if` whose condition spans lines.
        || p.parent().is_some_and(|arm| arm.kind() == SyntaxKind::MatchArm))
  })
}

fn control_comment_lines(root: &SyntaxNode, roles: &mut Roles) {
  let controls = root.descendant_nodes_where(|kind| {
    matches!(
      kind,
      SyntaxKind::LoopExpr | SyntaxKind::WhileExpr | SyntaxKind::ForExpr | SyntaxKind::IfExpr
    )
  });
  for control in controls {
    // The statement it starts, by the container its line is indented in.
    let mut owner = control.clone();
    while let Some(parent) = owner.parent()
      && parent.kind() == SyntaxKind::IfExpr
    {
      owner = parent;
    }
    let Some(&container) = roles.starts.get(&(owner.text_range().start as usize)) else {
      continue;
    };
    let elements: Vec<SyntaxElement> = control.children_with_tokens_iter().collect();
    let block =
      |e: &SyntaxElement| matches!(e, SyntaxElement::Node(n) if n.kind() == SyntaxKind::BlockExpr);
    let mut bodies = Vec::new();
    if control.kind() == SyntaxKind::IfExpr {
      bodies.extend(elements.iter().position(block));
      if let Some(at) = elements.iter().position(|e| e.kind() == SyntaxKind::ElseKw)
        && let Some(after) = (at + 1..elements.len()).find(|&i| !elements[i].kind().is_trivia())
        && block(&elements[after])
      {
        bodies.push(after);
      }
    } else {
      bodies.extend(elements.iter().rposition(block));
    }
    for body in bodies {
      let mut first = body;
      while first > 0 && elements[first - 1].kind().is_trivia() {
        first -= 1;
      }
      let between = &elements[first..body];
      if first == 0
        || !between
          .iter()
          .any(|e| matches!(e.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment))
      {
        continue;
      }
      // Each starting its line, after the pass before this one.
      let mut line_start = false;
      let mut starts = Vec::new();
      for element in between {
        match element {
          SyntaxElement::Token(t) if t.kind() == SyntaxKind::Whitespace => {
            line_start = t.text().contains('\n');
          }
          _ if line_start => starts.push(element.text_range().start as usize),
          _ => {}
        }
      }
      if !line_start {
        continue;
      }
      starts.push(elements[body].text_range().start as usize);
      for at in starts {
        roles.starts.entry(at).or_insert(container);
      }
    }
  }
}

fn custom_comment(tok: &Tok) -> bool {
  tok.kind == SyntaxKind::LineComment
    && tok
      .text
      .chars()
      .nth(2)
      .is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace())
}

/// The comments on lines of their own between a `loop`, `while`, `for` or `if` head (or an
/// `else`) and its block, and the block's `{` after them, indented as the statement the
/// head starts (the standard formatter's block indentation for them).
/// The comments under one ending the last line of an `if`'s first block, before its `}`
/// and `else`: the standard formatter writes them at the `}`'s indentation (the visitor's,
/// once it closes the block), from the first before style edition 2024 and from a blank line
/// on in it, the ones before aligned under the first.
fn else_close_comments(root: &SyntaxNode, roles: &mut Roles, config: &FormatConfig) {
  for control in root.descendant_nodes_where(|kind| kind == SyntaxKind::IfExpr) {
    let elements: Vec<SyntaxElement> = control.children_with_tokens_iter().collect();
    let Some(else_at) = elements.iter().position(|e| e.kind() == SyntaxKind::ElseKw) else {
      continue;
    };
    let Some(SyntaxElement::Node(block)) =
      elements[..else_at].iter().rfind(|e| !e.kind().is_trivia())
    else {
      continue;
    };
    if block.kind() != SyntaxKind::BlockExpr {
      continue;
    }
    let Some(open) = block
      .children_with_tokens_iter()
      .find(|e| e.kind() == SyntaxKind::LBrace)
    else {
      continue;
    };
    let inside: Vec<SyntaxElement> = block.children_with_tokens_iter().collect();
    let Some(close) = inside.iter().rposition(|e| e.kind() == SyntaxKind::RBrace) else {
      continue;
    };
    // The comments and spaces before the `}`, back to the code.
    let mut first = close;
    while first > 0 && inside[first - 1].kind().is_trivia() {
      first -= 1;
    }
    if first == 0 || inside[first - 1].kind() == SyntaxKind::LBrace {
      continue;
    }
    let run = &inside[first..close];
    // The first comment ends the code's line.
    let trailing = matches!(run.first(), Some(SyntaxElement::Token(w))
      if w.kind() == SyntaxKind::Whitespace && !w.text().contains('\n'))
      && run
        .get(1)
        .is_some_and(|e| e.kind() == SyntaxKind::LineComment);
    if !trailing {
      continue;
    }
    let mut taken = config.style_edition >= 2024;
    for element in &run[2..] {
      match element {
        SyntaxElement::Token(w) if w.kind() == SyntaxKind::Whitespace => {
          taken |= w.text().matches('\n').count() > 1;
        }
        SyntaxElement::Token(c)
          if matches!(c.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment) =>
        {
          if taken {
            roles.else_close.insert(
              c.text_range().start as usize,
              open.text_range().start as usize,
            );
          }
        }
        _ => break,
      }
    }
  }
}

/// Where the `}` of an item's body holding nothing but whitespace starts.
fn empty_body_close(item: &SyntaxNode) -> Option<usize> {
  let body = item
    .children_iter()
    .find(|c| is_container(c.kind()) && c.kind() != SyntaxKind::ParamList)?;
  let tokens: Vec<(SyntaxKind, TextRange, bool)> = body.child_spans().collect();
  let open = tokens
    .iter()
    .position(|&(kind, _, node)| !node && kind == SyntaxKind::LBrace)?;
  match tokens[open + 1..]
    .iter()
    .find(|&&(kind, _, _)| kind != SyntaxKind::Whitespace)
  {
    Some(&(SyntaxKind::RBrace, range, false)) => Some(range.start as usize),
    _ => None,
  }
}

/// Notes for `mto debug layout` that what starts at `start` (its text from there, `from`)
/// is left as written, and why: named by the statement or item around it.
fn explain(root: &SyntaxNode, start: usize, from: &str, reason: impl FnOnce() -> String) {
  if !crate::format::explain::active() {
    return;
  }
  let kind = root
    .descendants_iter()
    .filter(|n| {
      let range = n.text_range();
      (range.start as usize) <= start
        && start < range.end as usize
        && (is_statement(n.kind())
          || is_item(n.kind())
          || n.kind() == SyntaxKind::Attr
          || crate::format::stmt::is_tail(n))
    })
    .last()
    .map_or("code", |n| n.kind().name())
    .to_string();
  crate::format::explain::note(&kind, from, reason);
}

fn format_text_with(source: &str, edition: Edition, config: &FormatConfig, strip: bool) -> String {
  let (bom, rest) = match source.strip_prefix('\u{feff}') {
    Some(rest) => ("\u{feff}", rest),
    None => ("", source),
  };
  let first_crlf = rest
    .find('\n')
    .is_some_and(|i| i > 0 && rest.as_bytes()[i - 1] == b'\r');
  let newline = match config.newline_style {
    NewlineStyle::Auto if first_crlf => "\r\n",
    NewlineStyle::Auto | NewlineStyle::Unix => "\n",
    NewlineStyle::Windows => "\r\n",
    NewlineStyle::Native if cfg!(windows) => "\r\n",
    NewlineStyle::Native => "\n",
  };
  if config.disable_all_formatting || (!config.format_generated_files && is_generated(rest, config))
  {
    return source.to_string();
  }
  let text = rest.replace("\r\n", "\n");
  if text.contains('\r') {
    // A carriage return of its own (in a string, most likely): left as written.
    return source.to_string();
  }
  let parsed = parse_cached(&text, edition);
  if !parsed.errors().is_empty() {
    return source.to_string();
  }
  let root = parsed.syntax_node();
  if skips_itself(&root) {
    return source.to_string();
  }
  // `extern` given its ABI first.
  let (text, root) = match config
    .force_explicit_abi
    .then(|| crate::format::reorder::explicit_abi(&root, &text))
    .flatten()
  {
    Some(written) => {
      let root = parse(&written, edition).syntax_node();
      (written, root)
    }
    None => (text, root),
  };
  // Empty statements dropped first, and imports of nothing.
  let stripped = if strip {
    crate::format::reorder::strip_semicolons(&root, &text)
  } else {
    None
  };
  let (text, root) = match stripped {
    Some(stripped) => {
      let root = parse(&stripped, edition).syntax_node();
      (stripped, root)
    }
    None => (text, root),
  };
  let stripped = crate::format::reorder::empty_uses(&root, &text, edition, config);
  let (text, root) = match stripped {
    Some(stripped) => {
      let root = parse(&stripped, edition).syntax_node();
      (stripped, root)
    }
    None => (text, root),
  };
  let (text, root) = match crate::format::reorder::last_arm_comments(&root, &text) {
    Some(moved) => {
      let root = parse(&moved, edition).syntax_node();
      (moved, root)
    }
    None => (text, root),
  };
  let (text, root) = match crate::format::reorder::control_comments(&root, &text) {
    Some(moved) => {
      let root = parse(&moved, edition).syntax_node();
      (moved, root)
    }
    None => (text, root),
  };
  // Runs of derives merged into one.
  let (text, root) = match config
    .merge_derives
    .then(|| crate::format::reorder::merge_derives(&root, &text))
    .flatten()
  {
    Some(merged) => {
      let root = parse(&merged, edition).syntax_node();
      (merged, root)
    }
    None => (text, root),
  };
  // Imports and module declarations sorted first; the sorted text is what is formatted.
  let (text, root) = match crate::format::reorder::reorder(&root, &text, edition, config) {
    Some(sorted) => {
      let root = parse(&sorted, edition).syntax_node();
      (sorted, root)
    }
    None => (text, root),
  };
  let body = reindent(&root, &text, edition, config);
  // Line endings inside a multi-line string are part of the string: a file whose strings
  // hold line breaks keeps its own endings.
  let strings_span_lines = root
    .token_texts()
    .any(|(kind, text, _)| kind.is_literal() && text.contains('\n'));
  let newline = if strings_span_lines {
    if first_crlf { "\r\n" } else { "\n" }
  } else {
    newline
  };
  let body = if newline == "\n" {
    body
  } else {
    body.replace('\n', newline)
  };
  if bom.is_empty() {
    return body;
  }
  format!("{}{}", bom, body)
}

fn indentation(width: usize, config: &FormatConfig) -> String {
  if config.hard_tabs && config.tab_spaces > 0 {
    let mut out = "\t".repeat(width / config.tab_spaces);
    out.push_str(&" ".repeat(width % config.tab_spaces));
    out
  } else {
    " ".repeat(width)
  }
}

fn is_container(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    SyntaxKind::ItemList
      | SyntaxKind::AssocItemList
      | SyntaxKind::ExternItemList
      | SyntaxKind::BlockExpr
      | SyntaxKind::RecordFieldList
      | SyntaxKind::TupleFieldList
      | SyntaxKind::VariantList
      | SyntaxKind::MatchArmList
      | SyntaxKind::ArgList
      | SyntaxKind::ParamList
      | SyntaxKind::ArrayExpr
      | SyntaxKind::TupleExpr
      | SyntaxKind::RecordExprFieldList
      | SyntaxKind::UseTreeList
  )
}

/// Whether a comment among the file's first `generated_marker_line_search_limit` lines
/// holds `@generated`.
fn is_generated(text: &str, config: &FormatConfig) -> bool {
  text
    .lines()
    .take(config.generated_marker_line_search_limit)
    .any(|line| {
      let line = line.trim_start();
      (line.starts_with("//") || line.starts_with("/*")) && line.contains("@generated")
    })
}

/// The list and the index of the item a token ends (a field's or variant's comma, an arm's
/// comma or last `}`), for a comment after it.
fn item_before(token: &SyntaxToken) -> Option<(SyntaxNode, usize)> {
  let end = token.text_range().end as usize;
  let (list, item) = match token.kind() {
    SyntaxKind::Comma if token.parent().kind() == SyntaxKind::MatchArm => {
      let arm = token.parent();
      (arm.parent()?, arm)
    }
    SyntaxKind::Comma
      if matches!(
        token.parent().kind(),
        SyntaxKind::RecordFieldList | SyntaxKind::VariantList
      ) && token.parent().parent().is_some_and(|p| {
        matches!(
          p.kind(),
          SyntaxKind::Struct | SyntaxKind::Union | SyntaxKind::Enum
        ) || crate::format::items::variant_fields_on_lines(&p)
      }) =>
    {
      let list = token.parent();
      let item = list
        .children_iter()
        .rfind(|c| c.text_range().end as usize <= token.text_range().start as usize)?;
      (list, item)
    }
    SyntaxKind::RBrace
      if token
        .parent()
        .ancestors_iter()
        .find(|a| a.kind() == SyntaxKind::MatchArm)
        .is_some_and(|arm| crate::format::imports::code_end(&arm) == end) =>
    {
      let arm = token
        .parent()
        .ancestors_iter()
        .find(|a| a.kind() == SyntaxKind::MatchArm)?;
      (arm.parent()?, arm)
    }
    // A field's or variant's last token, no comma written after it (the layout adds one).
    _ => {
      let item = token.parent().ancestors_iter().find(|a| {
        matches!(a.kind(), SyntaxKind::RecordField | SyntaxKind::Variant)
          && crate::format::imports::code_end(a) == end
      })?;
      let list = item.parent()?;
      if !list.parent().is_some_and(|p| {
        matches!(
          p.kind(),
          SyntaxKind::Struct | SyntaxKind::Union | SyntaxKind::Enum
        )
      }) {
        return None;
      }
      (list, item)
    }
  };
  if list.kind() != SyntaxKind::MatchArmList
    && !matches!(
      list.kind(),
      SyntaxKind::RecordFieldList | SyntaxKind::VariantList
    )
  {
    return None;
  }
  let index = list_items_of(&list).iter().position(|i| i == &item)?;
  Some((list, index))
}

/// A line comment whose opener runs past `//` into punctuation (as `//~`): the standard
/// formatter keeps its opener on every line it writes.
/// Whether the whitespace at `i` comes between a block's last statement, ended with `;`,
/// and a comment that only comments and the block's `}` follow.
fn last_statement_comment(tokens: &[Tok], i: usize) -> bool {
  let Some(semi) = tokens[..i].iter().rev().find(|t| !t.kind.is_trivia()) else {
    return false;
  };
  let Some(close) = tokens[i + 1..].iter().find(|t| !t.kind.is_trivia()) else {
    return false;
  };
  semi.kind == SyntaxKind::Semi
    && close.kind == SyntaxKind::RBrace
    && close.token.parent().kind() == SyntaxKind::BlockExpr
    && semi
      .token
      .parent()
      .parent()
      .is_some_and(|block| block == close.token.parent())
}

/// What the Rust style starts a line with wherever it is written: the items and attributes
/// of a file or a body, an item's or a statement's first token after its attributes, and in
/// a block holding a statement, or a function's body holding anything, each statement, the
/// tail and the closing brace. Each takes its container's indentation.
fn line_starts(root: &SyntaxNode, roles: &mut Roles, config: &FormatConfig) {
  // Only these kinds can hold lines: the walk makes no handles for the rest.
  let lines = |kind| {
    matches!(
      kind,
      SyntaxKind::SourceFile
        | SyntaxKind::ItemList
        | SyntaxKind::AssocItemList
        | SyntaxKind::ExternItemList
        | SyntaxKind::VariantList
        | SyntaxKind::RecordFieldList
        | SyntaxKind::BlockExpr
        | SyntaxKind::MatchArmList
    )
  };
  for node in root.descendant_nodes_where(lines) {
    // Fields of a struct or union definition and an enum's variants are one per line too.
    // A `match`'s arms too.
    let items = matches!(
      node.kind(),
      SyntaxKind::SourceFile
        | SyntaxKind::ItemList
        | SyntaxKind::AssocItemList
        | SyntaxKind::ExternItemList
        | SyntaxKind::VariantList
        | SyntaxKind::MatchArmList
    ) || node.kind() == SyntaxKind::RecordFieldList
      && node.parent().is_some_and(|p| {
        matches!(p.kind(), SyntaxKind::Struct | SyntaxKind::Union)
          || crate::format::items::variant_fields_on_lines(&p)
      });
    // A function's body with anything in it is one of lines too (`fn_single_line` off).
    let block = block_of_lines(&node);
    if !items && !block {
      continue;
    }
    // The first comment on a line of its own after the last field, variant or arm follows
    // it with no blank line (its post-comment, `itemize_list`), when the item's line ends
    // in none.
    if matches!(
      node.kind(),
      SyntaxKind::MatchArmList | SyntaxKind::RecordFieldList | SyntaxKind::VariantList
    ) && let Some(last) = node.children_iter().next_back()
    {
      let mut next = last.next_sibling_or_token();
      let mut broke = false;
      while let Some(element) = next {
        match &element {
          SyntaxElement::Token(t) if t.kind() == SyntaxKind::Whitespace => {
            broke |= t.text().contains('\n');
          }
          SyntaxElement::Token(t) if t.kind() == SyntaxKind::Comma && !broke => {}
          SyntaxElement::Token(t)
            if matches!(t.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment) =>
          {
            if broke {
              roles.joined.insert(t.text_range().start as usize);
            }
            break;
          }
          _ => break,
        }
        next = match element {
          SyntaxElement::Token(t) => t.next_sibling_or_token(),
          SyntaxElement::Node(n) => n.next_sibling_or_token(),
        };
      }
    }
    // A comment above a match arm is its pre-comment (`itemize_list`): the arm follows it
    // with no blank line.
    if matches!(
      node.kind(),
      SyntaxKind::MatchArmList | SyntaxKind::RecordFieldList | SyntaxKind::VariantList
    ) {
      let mut after_comment = false;
      // Whether the last whitespace broke the line: a comment on a line of its own.
      let mut line_start = true;
      for element in node.children_with_tokens_iter() {
        match element.kind() {
          SyntaxKind::LineComment | SyntaxKind::BlockComment => {
            after_comment = line_start;
            line_start = false;
          }
          SyntaxKind::Whitespace => {
            if matches!(&element, SyntaxElement::Token(t) if t.text().contains('\n')) {
              line_start = true;
            }
          }
          // A comment after `{` goes to a line of its own.
          SyntaxKind::LBrace => line_start = true,
          SyntaxKind::MatchArm | SyntaxKind::RecordField | SyntaxKind::Variant if after_comment => {
            roles.joined.insert(element.text_range().start as usize);
            after_comment = false;
            line_start = false;
          }
          _ => {
            after_comment = false;
            line_start = false;
          }
        }
      }
    }
    let mut any = false;
    // Doc comments of the file or module itself (`//!`), which are attributes: each starts
    // a line.
    for token in node.children_with_tokens_iter() {
      if let SyntaxElement::Token(t) = &token
        && crate::format::items::is_doc(t.kind())
      {
        roles.breaks.insert(t.text_range().start as usize);
      }
    }
    // A block comment on the `{`'s line of a struct's fields, an enum's variants or an
    // `impl`'s or `extern` block's items goes to a line of its own, indented as they are
    // (the first's pre-comment, `itemize_list`; the visitor's `missed_spans`).
    if items
      && matches!(
        node.kind(),
        SyntaxKind::RecordFieldList
          | SyntaxKind::VariantList
          | SyntaxKind::AssocItemList
          | SyntaxKind::ExternItemList
      )
      && let Some(first) = node.children_iter().next()
    {
      let mut after = node
        .children_with_tokens_iter()
        .skip_while(|e| e.kind() != SyntaxKind::LBrace)
        .skip(1);
      let mut next = after.next();
      if let Some(SyntaxElement::Token(t)) = &next
        && t.kind() == SyntaxKind::Whitespace
        && !t.text().contains('\n')
      {
        next = after.next();
      }
      if let Some(SyntaxElement::Token(comment)) = next
        && comment.kind() == SyntaxKind::BlockComment
        && comment.text_range().end <= first.text_range().start
      {
        let at = comment.text_range().start as usize;
        roles.breaks.insert(at);
        if let Some(&container) = roles.starts.get(&(first.text_range().start as usize)) {
          roles.starts.entry(at).or_insert(container);
        }
      }
    }
    for child in node.children_iter() {
      if child.kind() == SyntaxKind::Label {
        continue;
      }
      any = true;
      let start = child.text_range().start as usize;
      // A field, variant or arm after a block comment on its line keeps that line (the
      // comment is its pre-comment, `itemize_list`).
      let after_block_comment = matches!(
        child.kind(),
        SyntaxKind::RecordField | SyntaxKind::Variant | SyntaxKind::MatchArm
      ) && {
        let mut before = child.prev_sibling_or_token();
        while let Some(SyntaxElement::Token(t)) = &before
          && t.kind() == SyntaxKind::Whitespace
          && !t.text().contains('\n')
        {
          before = t.prev_sibling_or_token();
        }
        matches!(&before, Some(SyntaxElement::Token(t))
          if t.kind() == SyntaxKind::BlockComment && !t.text().contains('\n'))
      };
      if !after_block_comment {
        roles.breaks.insert(start);
      }
      let container = roles.starts.get(&start).copied();
      // Indented as the item; `breaks` also starts a line wherever it is written.
      let mut mark = |at: usize, breaks: bool| {
        if breaks {
          roles.breaks.insert(at);
        }
        if let Some(container) = container {
          roles.starts.entry(at).or_insert(container);
        }
      };
      // Its own attributes, each on a line, and the line after them; the comments before
      // its code, and its code's first line, are indented as it is.
      let mut after_attr = false;
      // After a doc comment, which joins like an attribute.
      let mut after_doc = false;
      // After a comment: a match arm's joins it (its pre-comment, `itemize_list`).
      let mut after_comment = false;
      let mut seen_attr = false;
      let mut pending = Vec::new();
      // The comment right after a doc comment, joined to it when code follows.
      let mut after_doc_comment = None;
      let mut after_block_lines = false;
      // A statement's, field's, variant's, match arm's, foreign item's, `use`'s and
      // `extern crate`'s attributes and doc comments join it with no blank line, as the
      // standard formatter combines them; another item's in a file or module keep one, as
      // does a macro call standing as a statement, whose attributes are visited apart.
      // One ending a block with no `;` and no braces is its tail expression.
      let macro_statement = child.kind() == SyntaxKind::ExprStmt
        && child
          .children_iter()
          .find(|c| c.kind() != SyntaxKind::Attr)
          .filter(|e| e.kind() == SyntaxKind::MacroExpr)
          .is_some_and(|call| {
            child
              .children_with_tokens_iter()
              .any(|e| e.kind() == SyntaxKind::Semi)
              || call
                .descendant_nodes_where(|kind| kind == SyntaxKind::TokenTree)
                .next()
                .and_then(|tree| tree.first_token())
                .is_some_and(|t| t.kind() == SyntaxKind::LBrace)
          });
      let combined = node.kind() == SyntaxKind::ExternItemList
        // A block's tail expression, a macro call too.
        || !macro_statement
          && matches!(
          child.kind(),
          SyntaxKind::LetStmt
            | SyntaxKind::ExprStmt
            | SyntaxKind::RecordField
            | SyntaxKind::Variant
            | SyntaxKind::MatchArm
            | SyntaxKind::Use
            | SyntaxKind::ExternCrate
        )
        // A `mod name;` is formatted with the declarations that are sorted, when it is one.
        || child.kind() == SyntaxKind::Module
          && config.reorder_modules
          && crate::format::reorder::sortable(&child);
      let mut joined = Vec::new();
      for element in child.children_with_tokens_iter() {
        match &element {
          SyntaxElement::Node(n) if n.kind() == SyntaxKind::Attr => {
            mark(n.text_range().start as usize, true);
            // A blank line between a doc comment and an attribute stays; one between an
            // attribute and a comment after it, or that comment and the next attribute,
            // goes, as the standard formatter writes the attributes together.
            if after_attr || after_comment && seen_attr {
              joined.push(n.text_range().start as usize);
              joined.append(&mut pending);
            }
            seen_attr = true;
            after_attr = true;
            after_doc = false;
            after_comment = false;
            after_doc_comment = None;
          }
          SyntaxElement::Token(t)
            if crate::format::items::is_doc(t.kind())
              || matches!(t.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment) =>
          {
            // A doc comment is an attribute: it starts a line.
            let doc = crate::format::items::is_doc(t.kind());
            mark(t.text_range().start as usize, doc);
            // So does what follows a block comment across lines, or any before an item that
            // is not an import (which the sorting pass keeps after its comment).
            after_block_lines = t.kind() == SyntaxKind::BlockComment
              && (t.text().contains('\n')
                || is_item(child.kind())
                  && !matches!(
                    child.kind(),
                    SyntaxKind::Use | SyntaxKind::ExternCrate | SyntaxKind::Module
                  ));
            // A line comment on an attribute's line ends that line: what follows joins
            // the attribute, where attributes are combined with what they are on.
            let mut before = t.prev_sibling_or_token();
            while let Some(SyntaxElement::Token(w)) = &before
              && w.kind() == SyntaxKind::Whitespace
              && !w.text().contains('\n')
            {
              before = w.prev_sibling_or_token();
            }
            let trailing = after_attr
              && t.kind() == SyntaxKind::LineComment
              && before.is_some_and(|b| b.kind() == SyntaxKind::Attr);
            if trailing && combined {
              continue;
            }
            // Joined to the attribute before it only when another attribute follows.
            if after_attr && !doc {
              pending.push(t.text_range().start as usize);
            }
            if after_doc && !doc {
              after_doc_comment = Some(t.text_range().start as usize);
            }
            after_attr = false;
            after_doc = doc;
            after_comment = !doc;
          }
          SyntaxElement::Token(t) if t.kind() == SyntaxKind::Whitespace => {}
          SyntaxElement::Token(t)
            if t.kind().is_trivia() || crate::format::items::is_doc(t.kind()) =>
          {
            mark(t.text_range().start as usize, false);
          }
          // After an attribute or a doc comment (a `/** .. */` one does not end its line),
          // the code starts a line.
          other => {
            mark(
              other.text_range().start as usize,
              after_attr || after_doc || after_block_lines,
            );
            // Code after a comment after the attributes of what combines them joins it too
            // (`combine_strs_with_missing_comments` keeps no blank line), and so does that
            // comment the last attribute.
            if (after_attr || after_doc) && combined
              || after_comment
                && matches!(
                  child.kind(),
                  SyntaxKind::MatchArm | SyntaxKind::RecordField | SyntaxKind::Variant
                )
              || after_comment && seen_attr && combined
            {
              joined.push(other.text_range().start as usize);
            }
            if after_comment && seen_attr && combined {
              joined.append(&mut pending);
            }
            if after_comment && combined {
              joined.extend(after_doc_comment);
            }
            break;
          }
        }
      }
      roles.joined.extend(joined);
      // A `;` ending the item on a line of its own (after comments) is indented as the item.
      if let Some(SyntaxElement::Token(semi)) = child.children_with_tokens_iter().next_back()
        && semi.kind() == SyntaxKind::Semi
        && matches!(
          semi.prev_sibling_or_token(),
          Some(SyntaxElement::Token(w)) if w.kind() == SyntaxKind::Whitespace && w.text().contains('\n')
        )
        && let Some(container) = container
      {
        roles
          .starts
          .entry(semi.text_range().start as usize)
          .or_insert(container);
      }
      // A stray `;` after an item ending in a comment starts a line of its own, indented as
      // the item: the standard formatter ends the comment's line.
      if child
        .last_token()
        .is_some_and(|t| t.kind() == SyntaxKind::BlockComment)
      {
        let mut after = child.next_sibling_or_token();
        while let Some(SyntaxElement::Token(t)) = &after
          && t.kind() == SyntaxKind::Whitespace
        {
          after = t.next_sibling_or_token();
        }
        if let Some(SyntaxElement::Token(semi)) = after
          && semi.kind() == SyntaxKind::Semi
        {
          roles.breaks.insert(semi.text_range().start as usize);
          if let Some(container) = container {
            roles
              .starts
              .entry(semi.text_range().start as usize)
              .or_insert(container);
          }
        }
      }
    }
    if any && node.kind() != SyntaxKind::SourceFile {
      for token in node.children_with_tokens_iter() {
        if token.kind() == SyntaxKind::RBrace {
          roles.breaks.insert(token.text_range().start as usize);
        }
      }
    }
  }
  // A function's body holding only comments closes on a line of its own, as the standard
  // formatter visits it (`{ /* c */` then `}`).
  for body in root.descendant_nodes_where(|kind| kind == SyntaxKind::BlockExpr) {
    if body.parent().is_some_and(|p| p.kind() == SyntaxKind::Fn)
      && body.children_iter().next().is_none()
      && body
        .children_with_tokens_iter()
        .any(|e| matches!(e.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment))
      && let Some(close) = body
        .children_with_tokens_iter()
        .find(|e| e.kind() == SyntaxKind::RBrace)
    {
      roles.breaks.insert(close.text_range().start as usize);
    }
  }
}

/// A line's width as the standard formatter's `last_line_width` has it: each tab a single
/// column.
fn line_width_of(line: &str) -> usize {
  line
    .chars()
    .map(|c| if c == '\t' { 1 } else { char_width(c) })
    .sum()
}

/// The items of a struct's field list, an enum's variant list or a match's arm list.
fn list_items_of(list: &SyntaxNode) -> Vec<SyntaxNode> {
  list
    .children_iter()
    .filter(|c| {
      matches!(
        c.kind(),
        SyntaxKind::RecordField | SyntaxKind::Variant | SyntaxKind::MatchArm
      )
    })
    .collect()
}

/// For `mto debug layout`: what in `node` has no layout. The innermost expressions,
/// patterns and types that do not lay out even with no line width (all inside them do),
/// and what the statement holds that its layout leaves to this pass: a comment, a block, a
/// macro in braces, a string spanning lines.
fn missing_part(node: &SyntaxNode, config: &FormatConfig) -> String {
  let mut wide = config.clone();
  wide.max_width = 100_000;
  let shape = crate::format::shape::Shape {
    align: 0,
    indent: 0,
    offset: 0,
    width: wide.max_width,
    limits: crate::format::shape::Limits::of(&wide),
  };
  let laid = |kind: SyntaxKind| {
    let name = kind.name();
    name.ends_with("_EXPR") || name.ends_with("_PAT") || name.ends_with("_TYPE")
  };
  let mut failing: Vec<SyntaxNode> = node
    .descendant_nodes_where(laid)
    .filter(|n| crate::format::expr::rewrite(n, shape).is_none())
    .collect();
  // All lay out with no line width: those that do not at the line width, from the
  // statement's indentation one level in.
  let mut within = false;
  if failing.is_empty()
    && let Some(real) = crate::format::stmt::statement_shape(config.tab_spaces * 2, config)
  {
    within = true;
    failing = node
      .descendant_nodes_where(laid)
      .filter(|n| crate::format::expr::rewrite(n, real).is_none())
      .collect();
  }
  let innermost: Vec<String> = failing
    .iter()
    .filter(|n| {
      !failing
        .iter()
        .any(|m| m != *n && m.ancestors_iter().any(|a| &a == *n))
    })
    .take(3)
    .map(|n| {
      let text: String = n
        .text()
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
      let text: String = text.chars().take(50).collect();
      format!("{} `{}`", n.kind().name(), text)
    })
    .collect();
  let mut holds: Vec<String> = Vec::new();
  let mut places: Vec<&str> = node
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t)
        if matches!(t.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment) =>
      {
        Some(t.parent().kind().name())
      }
      _ => None,
    })
    .collect();
  places.dedup();
  if !places.is_empty() {
    holds.push(format!("a comment (in {})", places.join(", ")));
  }
  if node
    .token_texts()
    .any(|(kind, text, _)| kind.is_literal() && text.contains('\n'))
  {
    holds.push("a string spanning lines".to_string());
  }
  if node
    .descendant_nodes_where(|k| k == SyntaxKind::MacroCall)
    .any(|m| {
      m.children_iter()
        .find(|c| c.kind() == SyntaxKind::TokenTree)
        .and_then(|t| t.first_token())
        .is_some_and(|t| t.kind() == SyntaxKind::LBrace)
    })
  {
    holds.push("a macro in braces".to_string());
  }
  let mut parts = Vec::new();
  if !innermost.is_empty() {
    parts.push(format!(
      "no layout {}for {}",
      if within { "within the width " } else { "" },
      innermost.join(", ")
    ));
  }
  if !holds.is_empty() {
    parts.push(format!("holds {}", holds.join(", ")));
  }
  if parts.is_empty() {
    "every part lays out alone; the whole was declined".to_string()
  } else {
    parts.join("; ")
  }
}

/// The statements (`statements` in tree order) with a line of their own code wider than
/// `max_width`, as written: a line belongs to the innermost statement it starts in, and its
/// code ends where a line comment starts or the statement ends. A statement's first line is
/// not measured: it is indented anew, so measuring it would decide differently on a second
/// run. A line inside a chain (of `chains`) belongs instead to the outermost statement
/// holding that chain: the standard formatter measures a chain's every line, those of the
/// statements in its closures given up on alone included. A written line is measured in the
/// parts the layout starts lines with (`breaks`). Each with its widest part's width and
/// where the line it is on starts.
fn overflowing(
  spans: &Spans<'_>,
  text: &str,
  config: &FormatConfig,
) -> Vec<((usize, usize), usize, usize)> {
  let Spans {
    statements,
    breaks,
    chains,
    raw_strings,
    continued_strings,
    attrs,
    line_comments,
    block_comments,
    after_pre_comments,
    flattened,
  } = *spans;
  let tab = config.tab_spaces.max(1);
  let mut found = Vec::new();
  let mut open: Vec<(usize, usize, bool)> = Vec::new();
  let mut next = 0;
  let mut at = 0;
  // Statements found too wide by a part of a line they start after its first token.
  let mut nested_found: HashSet<usize> = HashSet::new();
  for line in text.split('\n') {
    let line_end = at + line.len();
    let first = at + (line.len() - line.trim_start().len());
    // The statements that start before this line's first token are entered; those
    // ending before it are left.
    while next < statements.len() && statements[next].0 <= first {
      open.retain(|&(_, end, _)| end > statements[next].0);
      open.push((
        statements[next].0,
        statements[next].1,
        nested_found.contains(&statements[next].0),
      ));
      next += 1;
    }
    open.retain(|&(_, end, _)| end > first);
    // Attributes are formatted on their own; a long one does not stop its item.
    let in_attr = || {
      let idx = attrs.partition_point(|&(start, _)| start <= first);
      idx > 0 && first < attrs[idx - 1].1
    };
    // For each chain the line is in, the innermost statement holding it; the outermost of
    // those (a chain whose closure holds the line's statement), else the innermost.
    let owner = chains
      .iter()
      .filter(|&&(start, end)| start < first && first < end)
      .filter_map(|&(start, end)| {
        open
          .iter()
          .rposition(|s| s.0 < at && s.0 <= start && end <= s.1)
      })
      .min()
      .or_else(|| open.len().checked_sub(1))
      .map(|mut owner| {
        while owner > 0 && flattened.contains(&open[owner].0) {
          owner -= 1;
        }
        owner
      });
    // A line starting inside a raw string (written across lines), not one in a chain,
    // whose every line is measured.
    let in_chain = chains.iter().any(|&(start, end)| start < at && at < end);
    let in_raw = (raw_strings
      .iter()
      .any(|&(start, end)| start < at && at < end)
      || continued_strings
        .iter()
        .any(|&(start, end)| start < line_end && at < end))
      && !in_chain;
    // A line of a block comment, which the standard formatter does not measure
    // (`filter_normal_code`): one starting in it, with no code after it on the line.
    let in_comment = block_comments.iter().any(|&(start, end)| {
      start <= first && first < end && (end >= line_end || text[end..line_end].trim().is_empty())
    });
    if let Some(owner) = owner
      && open[owner].0 < at
      && !open[owner].2
      && !in_attr()
      && !in_raw
      && !in_comment
    {
      let comment = line_comments.partition_point(|&c| c < at);
      // Up to a line comment, or the statement's end when other code follows it.
      let code_end = line_comments
        .get(comment)
        .filter(|&&c| c < line_end)
        .copied()
        .unwrap_or(line_end)
        .min(open[owner].1);
      // Each part of the line that the layout starts a line of its own with is measured
      // apart, at the line's indentation.
      let indent = first - at;
      let mut too_wide = false;
      let mut widest = 0;
      let mut from = at;
      // Where a statement in a block starts on the line, which starts a line of its own
      // where the block does not fit on one.
      let inner_starts: Vec<usize> = statements[next..]
        .iter()
        .take_while(|s| s.0 < code_end)
        .map(|s| s.0)
        .collect();
      for p in first + 1..=code_end {
        if p == code_end
          || (breaks.contains(&p) || after_pre_comments.contains(&p) || inner_starts.contains(&p))
            && text.is_char_boundary(p)
        {
          let part = if from == at {
            text[at..p].trim_end().to_string()
          } else {
            format!("{}{}", &text[at..at + indent], text[from..p].trim())
          };
          let width = width_of(&part, tab);
          // A part in a statement starting on the line after its first token is that
          // statement's (the standard formatter gives up on a block's statement alone),
          // but in a chain holding it, measured whole; one starting the statement is its
          // first line, measured as the pass reaches it.
          let starts = if from == at { first } else { from };
          let nested = statements[next..]
            .iter()
            .take_while(|s| s.0 <= starts)
            .filter(|s| {
              s.1 > starts
                && !flattened.contains(&s.0)
                && !chains.iter().any(|&(cs, ce)| cs < s.0 && s.0 < ce)
            })
            .last();
          match nested {
            Some(&(start, end)) if width > config.max_width => {
              if start < starts && nested_found.insert(start) {
                found.push(((start, end), width, at));
              }
            }
            Some(_) => {}
            None => {
              too_wide |= width > config.max_width;
              widest = widest.max(width);
            }
          }
          from = p;
        }
      }
      if too_wide {
        open[owner].2 = true;
        found.push(((open[owner].0, open[owner].1), widest, at));
      }
    }
    at = line_end + 1;
  }
  found
}

fn reindent(root: &SyntaxNode, text: &str, edition: Edition, config: &FormatConfig) -> String {
  let mut roles = roles(root, text, config);
  // Statements given up on as the pass reaches them (see below), beside the ranges known
  // before it.
  let gave_up: std::cell::RefCell<Vec<(usize, usize)>> = Default::default();
  // The tail of a statement laid out around its block (`});`): each token follows the one
  // before it on its line, as the head was laid out to have it; a trailing comma among
  // them goes, as the arguments are on one line.
  let glued: std::cell::RefCell<HashSet<usize>> = Default::default();
  // Tokens a laid-out head puts one space after the token before them.
  let spaced_after: std::cell::RefCell<HashSet<usize>> = Default::default();
  // The spaces before a struct's field's or an enum's variant's comment after its comma,
  // by where the comment starts: aligned as a list's are (`write_list`), found for a whole
  // list when the pass first needs one.
  let comment_spaces: std::cell::RefCell<HashMap<usize, Option<Vec<Option<usize>>>>> =
    Default::default();
  // Chain links after a head's block, by where their `.` starts: each starts a line at the
  // statement's indentation.
  let link_lines: std::cell::RefCell<HashMap<usize, usize>> = Default::default();
  let dropped: std::cell::RefCell<HashSet<usize>> = Default::default();
  let in_verbatim = |at: usize| {
    let idx = roles.verbatim.partition_point(|&(start, _)| start < at);
    (idx > 0 && at < roles.verbatim[idx - 1].1)
      || gave_up.borrow().iter().any(|&(s, e)| s < at && at < e)
  };
  debug_assert_eq!(root.text(), text, "the tree is not the text's");
  let tokens: Vec<Tok> = std::mem::take(&mut roles.tokens)
    .into_iter()
    .map(|t| Tok {
      kind: t.kind(),
      // The tree is the text's own parse, so a token's range in it is its text.
      text: &text[t.text_range().start as usize..t.text_range().end as usize],
      start: t.text_range().start as usize,
      token: t,
      synthetic: false,
    })
    .collect();
  // A space where none is written before what must start a line, for the line break to
  // take its place, and before what follows the token before it after one space.
  let mut spaced: Vec<Tok> = Vec::with_capacity(tokens.len());
  for tok in tokens {
    if (roles.breaks.contains(&tok.start) || roles.spaced.contains(&tok.start))
      && spaced
        .last()
        .is_some_and(|before| before.kind != SyntaxKind::Whitespace)
    {
      spaced.push(Tok {
        kind: SyntaxKind::Whitespace,
        text: "",
        start: tok.start,
        token: tok.token.clone(),
        synthetic: true,
      });
    }
    spaced.push(tok);
  }
  let tokens = spaced;
  let tab = config.tab_spaces.max(1);
  let max_newlines = config.blank_lines_upper_bound + 1;
  let mut out = String::with_capacity(text.len());
  // Each line's first token, with the line's indentation as written and as output.
  let mut lines: Vec<(usize, usize, usize)> = vec![(0, 0, 0)];
  // How far the construct being continued moved.
  let mut delta: isize = 0;
  // Each container's indentation, as written and as output.
  let mut open_indent: Offsets<(usize, usize)> = Offsets::default();
  // Where a head laid out across lines ends at a block's `{`: that line's indentation.
  let mut laid_opens: HashMap<usize, usize> = HashMap::new();
  let mut previous_significant: Option<usize> = None;
  // A line comment that ended a line of code: its column as written and as output.
  // A line comment after code: the column it was written at, the one it is written at,
  // whether it follows a statement's end, and the indentation of its line.
  // A trailing comment's column, as written and as output; whether it ends a statement;
  // the indentation of the statement's line; whether it is a list's last item's.
  let mut comment_column: Option<(usize, usize, bool, usize, bool)> = None;
  // The line comment just written ends a line of code: no line break since.
  let mut trailing_fresh = false;
  let mut out_line_start = 0usize;
  let mut text_line_start = 0usize;
  let mut code_on_line = false;
  // Where an item's body's `}` starts that closes up, its header laid out on one line.
  let tight_items: std::cell::RefCell<HashSet<usize>> = Default::default();
  // Where the `}` of an empty body starts that goes on a line of its own, its header laid
  // out across lines (`{` and `}` apart even when written `{}`).
  let split: std::cell::RefCell<HashSet<usize>> = Default::default();
  // The end of a statement written out whole; its tokens are passed over.
  let mut laid_out_to = 0usize;
  for (i, tok) in tokens.iter().enumerate() {
    let next = tokens.get(i + 1);
    if tok.start < laid_out_to
      || dropped.borrow().contains(&tok.start)
      || tok.kind == SyntaxKind::Whitespace
        && next.is_some_and(|n| glued.borrow().contains(&n.start))
    {
      continue;
    }
    if tok.kind == SyntaxKind::Whitespace
      && next.is_some_and(|n| spaced_after.borrow().contains(&n.start))
    {
      out.push(' ');
      continue;
    }
    if let Some(&at) = link_lines.borrow().get(&tok.start) {
      out.push('\n');
      out_line_start = out.len();
      out.push_str(&indentation(at, config));
    }
    if tok.kind == SyntaxKind::Whitespace
      && !in_verbatim(tok.start)
      && next.is_some_and(|n| {
        (roles.tight.contains(&n.start) || tight_items.borrow().contains(&n.start))
          && !split.borrow().contains(&n.start)
          && !in_verbatim(n.start)
      })
    {
      continue;
    }
    if tok.kind == SyntaxKind::Whitespace
      && !in_verbatim(tok.start)
      && next.is_some_and(|n| roles.spaced.contains(&n.start) && !in_verbatim(n.start))
    {
      out.push(' ');
      continue;
    }
    if out.is_empty() && tok.kind == SyntaxKind::Whitespace {
      // Nothing before the first token.
      continue;
    }
    // A comment after an attribute on its line that the standard formatter puts on the next
    // line, at the attribute's indentation: before another attribute, when all the comments
    // between them would run past the line width (`recover_missing_comment_in_span`);
    // before a statement, when its first line would (`combine_strs_with_missing_comments`).
    let attr_comment_break = tok.kind == SyntaxKind::Whitespace
      && !tok.text.contains('\n')
      && i > 0
      && !in_verbatim(tok.start)
      && next.is_some_and(|n| n.kind == SyntaxKind::LineComment && !in_verbatim(n.start))
      && attr_comment_moves(&tokens, i, &out[out_line_start..], tab, config);
    // Space before what the Rust style starts a line with, written after other code: a
    // line break, the construct measured from the column it was written at.
    if tok.kind == SyntaxKind::Whitespace && roles.after_skipped_mod.contains(&tok.start) {
      let line = &out[out_line_start..];
      let indent = line.len() - line.trim_start().len();
      out.extend_from_within(out_line_start..out_line_start + indent);
      continue;
    }
    let breaks = attr_comment_break
      || tok.kind == SyntaxKind::Whitespace
        && !tok.text.contains('\n')
        && code_on_line
        && !in_verbatim(tok.start)
        && next.is_some_and(|n| {
          !in_verbatim(n.start)
            && (roles.breaks.contains(&n.start)
            // A comment kept after `{` that would run past the line goes to the next; but
            // a function body's, which the standard formatter visits: before style edition
            // 2024 it goes when it would start past `comment_width`, from it it stays.
            || n.kind == SyntaxKind::LineComment
              && previous_significant
                .is_some_and(|p| roles.openers.contains_key(&p) && text[p..].starts_with('{'))
              && {
                let line = width_of(&out[out.rfind('\n').map_or(0, |i| i + 1)..], tab);
                let body = n.token.parent().parent().is_some_and(|f| {
                  n.token.parent().kind() == SyntaxKind::BlockExpr && f.kind() == SyntaxKind::Fn
                });
                if !body {
                  line + 1 + width_of(n.text.trim_end(), tab) > config.max_width
                } else {
                  // 80 = `comment_width`, an unstable setting at its default.
                  config.style_edition < 2024 && line + 1 > 80
                }
              }
            // Before the 2024 style, a comment after a block's last statement goes to the
            // next line when it would start past `comment_width`.
            || config.style_edition < 2024
              && matches!(n.kind, SyntaxKind::LineComment | SyntaxKind::BlockComment)
              && last_statement_comment(&tokens, i)
              && width_of(&out[out.rfind('\n').map_or(0, |i| i + 1)..], tab) + 1 > 80)
        });
    if tok.synthetic && !breaks {
      continue;
    }
    let before = &tokens[i.saturating_sub(1)].token;
    // A comment after a block comment that ends a statement's or an item's line: on the
    // next line, indented as that line, where the standard formatter's output settles (its
    // `missed_spans` writes it at column 1 first); the last before a block's `}`, one space
    // after it.
    let comment = |t: &Tok| matches!(t.kind, SyntaxKind::LineComment | SyntaxKind::BlockComment);
    if tok.kind == SyntaxKind::Whitespace
      && i > 0
      && !in_verbatim(tok.start)
      && before.kind() == SyntaxKind::BlockComment
      && !before.text().contains('\n')
      && next.is_some_and(comment)
      && !tok.text.contains('\n')
      && {
        let mut j = i - 1;
        while j > 0
          && (comment(&tokens[j])
            || tokens[j].kind == SyntaxKind::Whitespace && !tokens[j].text.contains('\n'))
        {
          j -= 1;
        }
        matches!(tokens[j].kind, SyntaxKind::Semi | SyntaxKind::RBrace)
          && !in_verbatim(tokens[j].start)
          // Not a `use`'s, whose comments the sorting pass moves after its `;`.
          && tokens[j].token.parent().kind() != SyntaxKind::Use
      }
    {
      let mut k = i + 1;
      while k < tokens.len()
        && (comment(&tokens[k])
          || tokens[k].kind == SyntaxKind::Whitespace && !tokens[k].text.contains('\n'))
      {
        k += 1;
      }
      while k < tokens.len() && tokens[k].kind == SyntaxKind::Whitespace {
        k += 1;
      }
      if tokens.get(k).is_some_and(|t| t.kind == SyntaxKind::RBrace) {
        out.push(' ');
      } else {
        let line = &out[out_line_start..];
        let indent = width_of(&line[..line.len() - line.trim_start().len()], tab);
        out.push('\n');
        out_line_start = out.len();
        out.push_str(&indentation(indent, config));
      }
      continue;
    }
    if tok.kind == SyntaxKind::Whitespace && (breaks || tok.text.contains('\n')) {
      let (written_text, next_start) = match next {
        Some(next) if breaks => {
          let line_start = text[..next.start].rfind('\n').map_or(0, |at| at + 1);
          (&text[line_start..next.start], next.start)
        }
        _ => {
          let written_text = tok.text.rsplit('\n').next().unwrap_or("");
          let next_start = tok.start + tok.text.len();
          text_line_start = next_start - written_text.len();
          (written_text, next_start)
        }
      };
      let written = width_of(written_text, tab);
      code_on_line = false;
      let fresh = std::mem::take(&mut trailing_fresh);
      if in_verbatim(tok.start) {
        out.push_str(tok.text);
        out_line_start = out.len() - written_text.len();
        lines.push((next_start, written, written));
        comment_column = None;
        continue;
      }
      let Some(next) = next else {
        break;
      };
      let newlines = if roles.joined.contains(&next.start) {
        1
      } else {
        tok.text.matches('\n').count().max(1)
      };
      let after_open = previous_significant.filter(|p| roles.openers.contains_key(p));
      let mut newlines = match after_open {
        Some(open) if roles.keeps_leading_blank.contains(&open) => newlines.min(max_newlines),
        Some(_) => 1,
        None
          if roles
            .closers
            .get(&next.start)
            .is_some_and(|open| !roles.keeps_trailing_blank.contains(open)) =>
        {
          1
        }
        None => newlines.min(max_newlines),
      };
      let aligned = comment_column.filter(|&(column, _, after_statement, _, last_item)| {
        (newlines == 1 || last_item)
          && (next.kind == SyntaxKind::LineComment
            || next.kind == SyntaxKind::BlockComment
              && !next.text.contains('\n')
              // Not one before code on its line.
              && text[next.start + next.text.len()..]
                .split('\n')
                .next()
                .is_some_and(|rest| rest.trim().is_empty() || rest.trim_start().starts_with("//")))
          && (written == column || after_statement || last_item)
      });
      let indent = if attr_comment_break {
        let line = &out[out_line_start..];
        width_of(&line[..line.len() - line.trim_start().len()], tab)
      } else if let Some(open) = roles.closers.get(&next.start) {
        let new = open_indent.get(open).map_or(0, |&(_, new)| new);
        delta = new as isize - written as isize;
        new
      } else if let Some(open) = roles.trailing.get(&next.start) {
        let (was, new) = open_indent.get(open).copied().unwrap_or((0, 0));
        (written as isize + new as isize - was as isize).max(0) as usize
      } else if let Some(open) = roles.starts.get(&next.start) {
        let new = match open {
          Some(open) => {
            let step = if roles.unindented.contains(open) || roles.outdented.contains(&next.start) {
              0
            } else {
              tab
            };
            open_indent.get(open).map_or(0, |&(_, new)| new) + step
          }
          None => 0,
        };
        delta = new as isize - written as isize;
        new
      } else {
        (written as isize + delta).max(0) as usize
      };
      // A comment aligned under the one before it; what follows it (an item it starts) is
      // indented as the comment would have been.
      let shown = match aligned {
        _ if roles.kept_indent.contains(&next.start) => written,
        _ if let Some(open) = roles.else_close.get(&next.start) => {
          comment_column = None;
          open_indent.get(open).map_or(indent, |&(_, new)| new)
        }
        Some((_, column, _, _, true)) => column,
        Some((_, _, true, block, _)) if config.style_edition >= 2024 => {
          // A comment with its own opener (as `//~`) right under a comment ending a line of
          // code comes after a blank line: the standard formatter rewrites the lines after
          // the first apart, from a line break.
          if fresh && newlines == 1 && custom_comment(next) {
            newlines = 2;
          }
          block
        }
        Some((_, column, _, _, _)) => column,
        None => {
          comment_column = None;
          indent
        }
      };
      for _ in 0..newlines {
        out.push('\n');
      }
      out_line_start = out.len();
      out.push_str(&indentation(shown, config));
      lines.push((next_start, written, indent));
      continue;
    }
    if tok.kind == SyntaxKind::Whitespace && next.is_none() {
      break;
    }
    // Spaces within a line: as the Rust style spaces the two tokens around them; before a
    // line comment right after an item laid out to its `;` or `}`, though a region left as
    // written covers it.
    let after_laid = i > 0
      && matches!(tokens[i - 1].kind, SyntaxKind::Semi | SyntaxKind::RBrace)
      && tokens[i - 1].start + tokens[i - 1].text.len() == laid_out_to
      && next.is_some_and(|n| n.kind == SyntaxKind::LineComment);
    if tok.kind == SyntaxKind::Whitespace && (!in_verbatim(tok.start) || after_laid) && i > 0 {
      // A line comment after a statement, an item, an attribute or a brace: one space. Match
      // arms, like list items, keep their comments aligned.
      let before = &tokens[i - 1].token;
      let ends = match before.kind() {
        SyntaxKind::Semi | SyntaxKind::LBrace => true,
        SyntaxKind::RBrace => before
          .parent()
          .parent()
          .is_none_or(|owner| owner.kind() != SyntaxKind::MatchArm),
        SyntaxKind::RBrack => before.parent().kind() == SyntaxKind::Attr,
        // An inner doc comment, an attribute.
        SyntaxKind::InnerBlockDoc => true,
        _ => false,
      };
      if next.is_some_and(|n| matches!(n.kind, SyntaxKind::LineComment | SyntaxKind::BlockComment))
        && let Some((list, index)) = item_before(before)
      {
        let key = list.text_range().start as usize;
        if !comment_spaces.borrow().contains_key(&key) {
          let line = &out[out_line_start..];
          let indent = width_of(&line[..line.len() - line.trim_start().len()], tab);
          let spaces = align_list_comments(&list, indent, config);
          comment_spaces.borrow_mut().insert(key, spaces);
        }
        if let Some(Some(Some(count))) = comment_spaces
          .borrow()
          .get(&key)
          .map(|spaces| spaces.as_ref().and_then(|s| s.get(index).copied()))
        {
          out.push_str(&" ".repeat(count));
          continue;
        }
      }
      let spaced = match next {
        Some(n) if n.kind == SyntaxKind::LineComment && ends => Some(true),
        // After a block's last expression too.
        Some(n)
          if n.kind == SyntaxKind::LineComment
            && before.parent().ancestors_iter().any(|a| {
              crate::format::stmt::is_tail(&a)
                && a
                  .last_token()
                  .is_some_and(|t| t.text_range() == before.text_range())
            }) =>
        {
          Some(true)
        }
        // A block comment after a statement's end too (not after `{`).
        Some(n)
          if n.kind == SyntaxKind::BlockComment && ends && before.kind() != SyntaxKind::LBrace =>
        {
          Some(true)
        }
        _ => next.and_then(|n| spacing(before, &n.token, edition, config)),
      };
      match spaced {
        Some(true) => out.push(' '),
        Some(false) => {}
        None => out.push_str(tok.text),
      }
      continue;
    }
    // A statement, `use` declaration, attribute or item header laid out anew, starting its
    // line: written whole, up to where its layout ends.
    // A region left as written that covers all of `node` (a verbatim attribute inside one is
    // not that).
    let covered = |node: &SyntaxNode| {
      let (start, end) = (
        node.text_range().start as usize,
        node.text_range().end as usize,
      );
      roles
        .verbatim
        .iter()
        .chain(gave_up.borrow().iter())
        .any(|&(s, e)| s <= start && end <= e)
    };
    // A `macro_rules!` definition starting its line, its rules formatted.
    if let Some(node) = roles.macro_rules.get(&tok.start)
      && !in_verbatim(tok.start)
      && out[out_line_start..].trim().is_empty()
    {
      let line = &out[out_line_start..];
      let laid =
        crate::format::macros::rewrite_macro_rules(node, width_of(line, tab), edition, config);
      if laid.is_none() {
        explain(
          root,
          tok.start,
          &text[tok.start..node.text_range().end as usize],
          || {
            "a macro definition whose rules could not be laid out (a rule's body not read as \
           code, or a comment among the rules): kept as written"
              .to_string()
          },
        );
      }
      if let Some(laid) = laid {
        out.push_str(&laid);
        if let Some(at) = out.rfind('\n') {
          out_line_start = at + 1;
        }
        laid_out_to = crate::format::imports::code_end(node);
        previous_significant = None;
        code_on_line = true;
        comment_column = None;
        continue;
      }
    }
    // A braced macro call's arguments, re-indented from the line they start on.
    if let Some(&end) = roles.preserved.get(&tok.start)
      && !in_verbatim(tok.start)
    {
      let line = &out[out_line_start..];
      let indent = width_of(&line[..line.len() - line.trim_start().len()], tab);
      let snippet = &text[tok.start..end];
      // Holding nothing, they close up (`m! {}`).
      if snippet.len() >= 2 && snippet[1..snippet.len() - 1].trim().is_empty() {
        out.push_str("{}");
      } else if let Some(body) = tok
        .token
        .parent()
        .parent()
        .and_then(|call| crate::format::macros::lazy_static_body(&call, indent, config))
      {
        out.push_str(&body);
      } else {
        out.push_str(&preserve_layout(snippet, indent, config));
      }
      if let Some(at) = out.rfind('\n') {
        out_line_start = at + 1;
      }
      laid_out_to = end;
      previous_significant = None;
      code_on_line = true;
      comment_column = None;
      continue;
    }
    // An item after a skipped `mod name;` on its line: laid out from the line's indentation.
    let follows_skipped = roles.after_skipped_mod.contains(&tok.start)
      || i > 0 && roles.after_skipped_mod.contains(&tokens[i - 1].start);
    if let Some(node) = roles.layable.get(&tok.start)
      && !in_verbatim(tok.start)
      && !roles
        .kept_whole
        .contains(&(node.text_range().start as usize))
      // An `else if` follows `} else ` on its line.
      && (out[out_line_start..].chars().all(|c| c == ' ' || c == '\t')
        || follows_skipped
        || node.kind() == SyntaxKind::IfExpr
          && node.parent().is_some_and(|p| p.kind() == SyntaxKind::IfExpr))
      && let Some((text, to)) = {
        let line = &out[out_line_start..];
        let indent = if follows_skipped {
          width_of(&line[..line.len() - line.trim_start().len()], tab)
        } else {
          width_of(line, tab)
        };
        let whole = |text: Option<String>| text.map(|t| (t, node.text_range().end as usize));
        // A head laid out: its tail follows the block's `}` on its line.
        let take_head = |head: crate::format::stmt::Head| {
          for &at in &head.links {
            glued.borrow_mut().insert(at);
            link_lines.borrow_mut().insert(at, indent);
          }
          for (at, kind) in head.tail {
            glued.borrow_mut().insert(at);
            if kind == SyntaxKind::Comma && !head.keep_comma {
              dropped.borrow_mut().insert(at);
            }
          }
          for at in head.spaced {
            spaced_after.borrow_mut().insert(at);
          }
          (head.text, head.open)
        };
        // A control expression's head, as a statement or a tail, or after `} else `.
        let control = |node: &SyntaxNode| {
          let line = &out[out_line_start..];
          let indent = width_of(&line[..line.len() - line.trim_start().len()], tab);
          let head = rewrite_control_head(node, indent, width_of(line, tab), config)?;
          // A head on one line with its `{` on the next (past the line width) leaves an
          // empty body's `}` on a line of its own; after a condition across lines `{}`
          // closes up.
          if head.text.matches('\n').count() == 1
            && let Some(close) = empty_body_close(node)
          {
            split.borrow_mut().insert(close);
          }
          Some((head.text, head.open))
        };
        match node.kind() {
          SyntaxKind::IfExpr if node.parent().is_some_and(|p| p.kind() == SyntaxKind::IfExpr) => {
            control(node)
          }
          SyntaxKind::Use => rewrite_use(node, indent, config)
            .map(|text| (text, crate::format::imports::code_end(node))),
          SyntaxKind::MatchArm => match rewrite_arm(node, indent, config) {
            Some(Arm::Whole(text)) => Some((text, node.text_range().end as usize)),
            Some(Arm::Head(head)) => Some(take_head(head)),
            None => None,
          },
          // A `match`'s inner attribute on one line, or left as written (`rewrite_match`
          // gives up on one that does not fit).
          SyntaxKind::Attr => {
            let in_arms = node
              .parent()
              .is_some_and(|p| p.kind() == SyntaxKind::MatchArmList);
            // A `match`'s inner attribute measured in the `match`'s width (`rewrite_match`
            // gives it the match's shape, indented a level with its width kept).
            let measured = if in_arms {
              indent.saturating_sub(config.tab_spaces)
            } else {
              indent
            };
            whole(
              rewrite_attr(node, measured, config)
                .filter(|text| !text.contains('\n') || !in_arms),
            )
          }
          // Not an item given up on whole.
          SyntaxKind::RecordField | SyntaxKind::Variant => {
            crate::format::items::rewrite_field(node, indent, config)
          }
          // Not an item given up on whole.
          // A macro call standing as an item, in parentheses or brackets.
          // Laid out whole even where a written line is too wide, as a statement is.
          SyntaxKind::MacroCall => crate::format::items::rewrite_macro_item(node, indent, config)
            .map(|text| (text, crate::format::imports::code_end(node))),
          // A `const` block standing as an item, on one line where it fits.
          SyntaxKind::Const
            if crate::format::items::const_block_item(node, indent, config).is_some() =>
          {
            crate::format::items::const_block_item(node, indent, config)
              .map(|text| (text, node.text_range().end as usize))
          }
          kind if is_item(kind) => {
            if covered(node) {
              None
            } else {
              let laid = rewrite_header(node, indent, config);
              // An `impl`, struct, union or trait whose header the standard formatter gives
              // up on is left as written whole; another item, its header only.
              let whole = matches!(
                node.kind(),
                SyntaxKind::Impl | SyntaxKind::Struct | SyntaxKind::Union | SyntaxKind::Trait
              );
              let node_end = node.text_range().end as usize;
              if laid.is_none()
                && let Some(&end) = roles.wide_headers.get(&tok.start)
              {
                let end = if whole { node_end } else { end };
                explain(root, tok.start, &text[tok.start..end], || {
                  let why = roles.wide_reasons.get(&tok.start).cloned().unwrap_or_default();
                  format!("{}; its header could not be laid out", why)
                });
                gave_up.borrow_mut().push((tok.start, end));
              } else if laid.is_none()
                && let Some(end) = crate::format::items::header_gives_up(node, indent, config)
              {
                let end = if whole { node_end } else { end };
                explain(root, tok.start, &text[tok.start..end], || {
                  "its header fits no layout within max_width: left as written".to_string()
                });
                gave_up.borrow_mut().push((tok.start, end));
              }
              // An empty body after a header on one line closes up (`{}`); after one across
              // lines its `}` goes on a line of its own.
              if let Some((text, _)) = &laid
                && let Some(close) = empty_body_close(node)
              {
                // A struct's, enum's or union's closes up whatever its header (`> {}`); a
                // function's whose `{}` would pass the line width does not.
                let last = text.rsplit('\n').next().unwrap_or("");
                let start = if text.contains('\n') { 0 } else { indent };
                let past = node.kind() == SyntaxKind::Fn
                  && start + width_of(last, tab) + 2 > config.max_width;
                // An `impl` whose `where` stays on its line, its `{}` on the next.
                let where_line = node.kind() == SyntaxKind::Impl
                  && text.matches('\n').count() == 1
                  && last.trim().is_empty()
                  && text.contains(" where ");
                if past
                  || text.contains('\n')
                    && !where_line
                    && !matches!(
                      node.kind(),
                      SyntaxKind::Struct | SyntaxKind::Enum | SyntaxKind::Union
                    )
                {
                  split.borrow_mut().insert(close);
                } else {
                  tight_items.borrow_mut().insert(close);
                }
              }
              laid
            }
          }
          _ => whole(rewrite_statement(node, indent, config))
          .or_else(|| {
            // A statement holding one block: its head, the block's insides left to this pass,
            // and its tail kept on the block's closing line. Not one given up on whole: its
            // head laid out would hide the long line on a second run.
            if covered(node) {
              return None;
            }
            // Nor a method chain no layout fits: the standard formatter checks the chain
            // whole, its closures' lines too (`wrap_str` of the chain), and gives up on it.
            if chain_value(node)
              && crate::format::stmt::statement_shape(indent, config)
                .is_some_and(|shape| crate::format::block::gives_up(node, shape, false, true))
            {
              return None;
            }
            if let Some(text) = crate::format::stmt::rewrite_closure_tail(node, indent, config) {
              return Some((text, node.text_range().end as usize));
            }
            if let Some(head) = rewrite_let_control(node, indent, config) {
              return Some((head.text, head.open));
            }
            let Some(head) = rewrite_head(node, indent, config) else {
              let expression = if node.kind() == SyntaxKind::ExprStmt {
                node.children_iter().find(|c| c.kind() != SyntaxKind::Attr)?
              } else {
                node.clone()
              };
              return control(&expression).or_else(|| {
                crate::format::stmt::rewrite_hole_head(node, indent, config).map(take_head)
              });
            };
            Some(take_head(head))
          }),
        }
      }
    {
      out.push_str(&text);
      if let Some(at) = out.rfind('\n') {
        out_line_start = at + 1;
        // A `let`'s head across lines ending at its value's block's `{` (`let x =` with
        // `if c {` on the next line): the block is indented from the line the `{` is on.
        if node.kind() == SyntaxKind::LetStmt && roles.openers.contains_key(&to) {
          let line = &out[out_line_start..];
          laid_opens.insert(
            to,
            width_of(&line[..line.len() - line.trim_start().len()], tab),
          );
        }
      }
      laid_out_to = to;
      previous_significant = None;
      code_on_line = true;
      comment_column = None;
      continue;
    }
    // A statement the layout above could not lay out, which no layout fits: left as written,
    // as the standard formatter gives up on it.
    if let Some(node) = roles.layable.get(&tok.start)
      && (matches!(node.kind(), SyntaxKind::LetStmt | SyntaxKind::ExprStmt)
        || crate::format::stmt::is_tail(node))
      // Not a control expression, whose head and block the standard formatter lays out apart.
      && !std::iter::once(node.clone())
        .filter(crate::format::stmt::is_tail)
        .chain(node.children_iter())
        .any(|c| {
        matches!(
          c.kind(),
          SyntaxKind::IfExpr
            | SyntaxKind::MatchExpr
            | SyntaxKind::WhileExpr
            | SyntaxKind::ForExpr
            | SyntaxKind::LoopExpr
            | SyntaxKind::BlockExpr
        )
      })
      && !in_verbatim(tok.start)
      && !covered(node)
      && let Some(&end) = roles.statements.get(&tok.start)
      && let Some(shape) =
        crate::format::stmt::statement_shape(width_of(&out[out_line_start..], tab), config)
      && crate::format::block::gives_up(node, shape, false, false)
    {
      explain(root, tok.start, &text[tok.start..end], || {
        "no layout fits within max_width (or a string in it spans lines): left as written"
          .to_string()
      });
      gave_up.borrow_mut().push((tok.start, end));
    }
    // One the layout above could not lay out: its written lines kept, re-indented.
    if crate::format::explain::active()
      && let Some(node) = roles.layable.get(&tok.start)
      && !in_verbatim(tok.start)
      && !roles.wide_reasons.contains_key(&tok.start)
      && (matches!(
        node.kind(),
        SyntaxKind::LetStmt | SyntaxKind::ExprStmt | SyntaxKind::MatchArm
      ) || is_item(node.kind()))
    {
      let end = node.text_range().end as usize;
      explain(root, tok.start, &text[tok.start..end], || {
        format!(
          "not laid out whole ({}): its written lines are kept, re-indented",
          missing_part(node, config)
        )
      });
    }
    // A statement with a written line past the line width, not laid out whole above: left
    // as written.
    if let Some(why) = roles.wide_reasons.get(&tok.start)
      && !roles.wide_headers.contains_key(&tok.start)
      && !in_verbatim(tok.start)
    {
      let end = roles
        .statements
        .get(&tok.start)
        .copied()
        .unwrap_or(tok.start);
      explain(root, tok.start, &text[tok.start..end], || why.clone());
    }
    // A statement whose first line, at its new indentation, is longer than `max_width`
    // is left as written, as the standard formatter gives up on it. Measured here, where
    // the indentation is known, so that a second run decides the same.
    if let Some(&end) = roles.statements.get(&tok.start)
      && tok.kind != SyntaxKind::Pound
      && !in_verbatim(tok.start)
    {
      let line_end = text[tok.start..]
        .find('\n')
        .map_or(text.len(), |i| tok.start + i);
      let comment = roles.line_comments.partition_point(|&c| c < tok.start);
      let code_end = roles
        .line_comments
        .get(comment)
        .copied()
        .unwrap_or(text.len())
        .min(line_end)
        .min(end);
      // Up to where a line starts inside it (a block's first statement after its `{`).
      let code_end = tokens[i + 1..]
        .iter()
        .take_while(|t| t.start < code_end)
        .find(|t| roles.breaks.contains(&t.start))
        .map_or(code_end, |t| t.start);
      let width =
        width_of(&out[out_line_start..], tab) + width_of(text[tok.start..code_end].trim_end(), tab);
      if width > config.max_width {
        explain(root, tok.start, &text[tok.start..end], || {
          format!(
            "its first line would be {} columns, past max_width {}",
            width, config.max_width
          )
        });
        gave_up.borrow_mut().push((tok.start, end));
      }
    }
    if let Some(&base) = roles.openers.get(&tok.start) {
      let idx = lines.partition_point(|&(start, _, _)| start <= base);
      let (_, was, new) = lines[idx.saturating_sub(1)];
      let new = laid_opens.get(&tok.start).copied().unwrap_or(new);
      open_indent.insert(tok.start, (was, new));
    }
    if tok.kind == SyntaxKind::LineComment && code_on_line {
      // From the comment's own line: what is laid out whole leaves `text_line_start` behind.
      let line_start = text[..tok.start].rfind('\n').map_or(0, |i| i + 1);
      let written = width_of(&text[line_start.max(text_line_start)..tok.start], tab);
      // From the output's last line start: a statement laid out whole is pushed with its
      // lines.
      let output = line_width_of(&out[out.rfind('\n').map_or(0, |i| i + 1)..]);
      // After a statement's end, the comments below it are the standard formatter's
      // `missed_spans` comment: before the 2024 style, aligned under this one.
      // So after an inner doc comment across lines ending on this line (`*/ // c`), an
      // attribute of what holds it.
      let after_block = tokens[..i]
        .iter()
        .rev()
        .find(|t| t.kind != SyntaxKind::Whitespace)
        .is_some_and(|t| t.kind == SyntaxKind::InnerBlockDoc && t.text.contains('\n'));
      let after_statement = after_block
        || tokens[..i]
          .iter()
          .rev()
          .find(|t| !t.kind.is_trivia())
          .is_some_and(|t| match t.kind {
            SyntaxKind::Semi => true,
            // The `]` of a file's last inner attribute (the comments between them are laid out
            // with them).
            SyntaxKind::RBrack => {
              let attr = t.token.parent();
              let inner = |a: &SyntaxNode| {
                a.kind() == SyntaxKind::Attr
                  && a
                    .children_with_tokens_iter()
                    .any(|e| e.kind() == SyntaxKind::Bang)
              };
              let last_inner = inner(&attr)
                && attr
                  .parent()
                  .is_some_and(|p| p.kind() == SyntaxKind::SourceFile)
                && !attr.parent().is_some_and(|file| {
                  file
                    .children_iter()
                    .any(|n| n.text_range().start > attr.text_range().start && inner(&n))
                });
              // An item's last outer attribute where one of them is left as written: the
              // comments after it are a gap of their own, not the attributes'.
              let after_kept = attr.kind() == SyntaxKind::Attr
                && !inner(&attr)
                && attr.parent().is_some_and(|item| {
                  is_item(item.kind())
                    && !item.children_iter().any(|n| {
                      n.kind() == SyntaxKind::Attr && n.text_range().start > attr.text_range().start
                    })
                    && item.children_iter().any(|n| {
                      n.kind() == SyntaxKind::Attr
                        && (!meta_like(&n) || rewrite_attr(&n, 0, config).is_none())
                    })
                });
              last_inner || after_kept
            }
            // The `}` ending a statement or item (`if x { .. }`, `fn f() { .. }`).
            SyntaxKind::RBrace => t
              .token
              .parent()
              .ancestors_iter()
              .find(|a| {
                (is_statement(a.kind()) || crate::format::stmt::is_tail(a))
                // A macro in a statement: the statement.
                && !(a.kind() == SyntaxKind::MacroExpr
                  || a.kind() == SyntaxKind::MacroCall
                    && a.parent().is_some_and(|p| p.kind() == SyntaxKind::MacroExpr))
              })
              .is_some_and(|statement| {
                crate::format::imports::code_end(&statement) == t.token.text_range().end as usize
                  && statement.parent().is_some_and(|p| {
                    matches!(
                      p.kind(),
                      SyntaxKind::BlockExpr
                        | SyntaxKind::SourceFile
                        | SyntaxKind::ItemList
                        | SyntaxKind::AssocItemList
                    )
                  })
              }),
            _ => false,
          });
      // The indentation of the line the statement before it starts on.
      let statement_start = tokens[..i]
        .iter()
        .rev()
        .find(|t| !t.kind.is_trivia())
        .and_then(|t| {
          t.token
            .parent()
            .ancestors_iter()
            .find(|a| is_statement(a.kind()) || crate::format::stmt::is_tail(a))
        })
        .and_then(|statement| {
          // After an attribute, from the attributes' first line (the item's own is below).
          if tokens[..i]
            .iter()
            .rev()
            .find(|t| !t.kind.is_trivia())
            .is_some_and(|t| t.kind == SyntaxKind::RBrack)
          {
            Some(statement.text_range().start as usize)
          } else {
            crate::format::items::header_start(&statement)
          }
        });
      let block = match statement_start {
        Some(start) => {
          let idx = lines.partition_point(|&(line_start, _, _)| line_start <= start);
          lines.get(idx.wrapping_sub(1)).map_or(0, |&(_, _, new)| new)
        }
        None => {
          let line = &out[out_line_start..];
          width_of(&line[..line.len() - line.trim_start().len()], tab)
        }
      };
      // After a struct's last field or an enum's last variant, the comments up to the `}`
      // are its post-comment (`itemize_list`), aligned under this one in every style.
      let last_item = tokens[..i]
        .iter()
        .rev()
        .find(|t| !t.kind.is_trivia())
        .is_some_and(|t| {
          // Its comma, or its own last token where it has none (the layout adds one).
          let list_kinds =
            |k: SyntaxKind| matches!(k, SyntaxKind::RecordFieldList | SyntaxKind::VariantList);
          let item = t
            .token
            .parent()
            .ancestors_iter()
            .find(|a| matches!(a.kind(), SyntaxKind::RecordField | SyntaxKind::Variant))
            .filter(|item| {
              item.parent().is_some_and(|p| list_kinds(p.kind()))
                && item.last_token().is_some_and(|last| last == t.token)
            });
          let after = match &item {
            Some(item) => item.next_sibling_or_token(),
            None if t.kind == SyntaxKind::Comma && list_kinds(t.token.parent().kind()) => {
              t.token.next_sibling_or_token()
            }
            None => return false,
          };
          let mut next = after;
          while let Some(e) = &next
            && e.kind().is_trivia()
          {
            next = match e {
              SyntaxElement::Token(t) => t.next_sibling_or_token(),
              SyntaxElement::Node(n) => n.next_sibling_or_token(),
            };
          }
          next.is_some_and(|e| e.kind() == SyntaxKind::RBrace)
        });
      comment_column = Some((written, output, after_statement, block, last_item));
      trailing_fresh = true;
    }
    // An empty body's `}` kept apart from its `{`, written touching.
    if tok.kind == SyntaxKind::RBrace && split.borrow().contains(&tok.start) && out.ends_with('{') {
      // At its item's indentation, not that of a header line it ends.
      let line = &out[out_line_start..];
      let indent = match roles
        .closers
        .get(&tok.start)
        .and_then(|open| open_indent.get(open))
      {
        Some(&(_, new)) if !config.hard_tabs => " ".repeat(new),
        _ => line[..line.len() - line.trim_start().len()].to_string(),
      };
      out.push('\n');
      out_line_start = out.len();
      out.push_str(&indent);
    }
    // A doc comment keeps two spaces or more at its end (a markdown line break), not one
    // (`trim_end_unless_two_whitespaces`).
    let trimmed = !in_verbatim(tok.start)
      && (tok.kind == SyntaxKind::LineComment
        || matches!(
          tok.kind,
          SyntaxKind::OuterLineDoc | SyntaxKind::InnerLineDoc
        ) && !tok.text.ends_with("  "));
    if tok.kind == SyntaxKind::BlockComment && tok.text.contains('\n') && !in_verbatim(tok.start) {
      // Its later lines re-indented from its column, as the standard formatter writes it;
      // after code on its line, from the 2024 style, from the line's indentation.
      let line = &out[out.rfind('\n').map_or(0, |i| i + 1)..];
      let same_line = config.style_edition >= 2024 && !line.trim().is_empty();
      let column = if same_line {
        width_of(&line[..line.len() - line.trim_start().len()], tab)
      } else {
        width_of(line, tab)
      };
      let laid = reindent_block_comment(tok.text, column, tab, config, same_line);
      out.push_str(&laid);
    } else {
      out.push_str(if trimmed {
        tok.text.trim_end()
      } else {
        tok.text
      });
    }
    // Two tokens written touching that the Rust style spaces.
    if let Some(next) = next
      && next.kind != SyntaxKind::Whitespace
      && !in_verbatim(tok.start)
      && !in_verbatim(next.start)
      && wants_space(&tok.token, &next.token, config)
    {
      out.push(' ');
    }
    if !tok.kind.is_trivia() || tok.kind == SyntaxKind::BlockComment {
      code_on_line = true;
    }
    if !tok.kind.is_trivia()
      || matches!(tok.kind, SyntaxKind::LineComment | SyntaxKind::BlockComment)
    {
      previous_significant = Some(tok.start);
    }
  }
  // The end of the file gets exactly one line break.
  let mut out = out.trim_end_matches([' ', '\t', '\n']).to_string();
  out.push('\n');
  out
}

fn roles(root: &SyntaxNode, text: &str, config: &FormatConfig) -> Roles {
  let mut roles = Roles::default();
  // A file's frontmatter, and what is before it and after it up to the first comment or
  // code, as written: the standard formatter writes it as code it has not formatted.
  let mut leading = root.token_texts().take_while(|(kind, _, _)| {
    kind.is_trivia() && !matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment)
  });
  if leading.any(|(kind, _, _)| kind == SyntaxKind::Frontmatter)
    && let Some((_, _, after)) = root.token_texts().find(|(kind, _, _)| {
      !kind.is_trivia() || matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment)
    })
  {
    roles.verbatim.push((0, after.start as usize));
  }
  for child in root.child_spans() {
    if let Some(start) = construct_start(child) {
      roles.starts.insert(start, None);
    }
  }
  let mut region: Option<usize> = None;
  let mut statements = Vec::new();
  // Statements the standard formatter does not give up on alone (`flattened` in Spans).
  let mut flattened: OffsetSet = OffsetSet::default();
  let mut chains = Vec::new();
  let mut raw_strings = Vec::new();
  let mut continued_strings = Vec::new();
  // Items' headers, by their range, with where their own text starts.
  let mut headers: HashMap<(usize, usize), usize> = HashMap::new();
  let mut attrs = Vec::new();
  let mut stranded = Vec::new();
  let mut braced_macros: Vec<(usize, usize)> = Vec::new();
  let mut line_comments = Vec::new();
  let mut block_comments = Vec::new();
  let mut after_pre_comments = OffsetSet::default();
  // A `#[rustfmt::skip]` (or `rustfmt_skip`, or either in a `cfg_attr`) holds one of these
  // words: a file with neither has none, and no node's attributes need reading.
  let any_skip = root
    .token_texts()
    .any(|(kind, text, _)| kind == SyntaxKind::Ident && matches!(text, "skip" | "rustfmt_skip"));
  for element in root.descendants_with_tokens_iter() {
    match &element {
      SyntaxElement::Node(node) => {
        let range = node.text_range();
        let skipped = any_skip
          && node.kind() != SyntaxKind::SourceFile
          && node
            .children_of_kind(SyntaxKind::Attr)
            .filter_map(Attr::cast)
            .any(|a| a.is_rustfmt_skip());
        let in_list = node.parent().is_some_and(|p| {
          matches!(
            p.kind(),
            SyntaxKind::BlockExpr
              | SyntaxKind::SourceFile
              | SyntaxKind::ItemList
              | SyntaxKind::AssocItemList
              | SyntaxKind::ExternItemList
          )
        });
        if node.kind() == SyntaxKind::Attr {
          attrs.push((range.start as usize, range.end as usize));
        }
        // A chain's every line is measured by the standard formatter, which gives up on
        // its statement whole when one does not fit, lines of statements in its closures
        // included ([`overflowing`]).
        if matches!(
          node.kind(),
          SyntaxKind::MethodCallExpr
            | SyntaxKind::FieldExpr
            | SyntaxKind::TryExpr
            | SyntaxKind::AwaitExpr
        ) {
          chains.push((range.start as usize, range.end as usize));
        }
        if in_list && node.kind() != SyntaxKind::Attr {
          // A braced macro call is re-indented whatever its width, never given up on: it
          // still holds its own long lines.
          if braced_macro(node) {
            braced_macros.push((range.start as usize, range.end as usize));
          }

          // An item with a body is never given up on whole: the standard formatter lays out
          // its header (given up on when too long) and visits its body, whose statements are
          // measured on their own.
          let body = node.children_iter().find(|c| {
            matches!(
              c.kind(),
              SyntaxKind::BlockExpr
                | SyntaxKind::ItemList
                | SyntaxKind::AssocItemList
                | SyntaxKind::RecordFieldList
                | SyntaxKind::VariantList
                | SyntaxKind::ExternItemList
            )
          });
          let end = match &body {
            Some(body) if is_item(node.kind()) => body.text_range().start as usize,
            // A `use`'s comment after its `;` is not its own, nor a macro definition's after
            // its `}`.
            _ if matches!(node.kind(), SyntaxKind::Use | SyntaxKind::MacroRules) => {
              crate::format::imports::code_end(node)
            }
            _ => range.end as usize,
          };
          {
            // An item's header from after its attributes: its first line is indented anew,
            // and is not measured.
            match is_item(node.kind())
              .then(|| crate::format::items::header_start(node))
              .flatten()
            {
              Some(start) => {
                statements.push((start, end));
                headers.insert((start, end), start);
              }
              // A `let`, an expression statement, a macro definition or call, or a `use` from
              // after its attributes too, which are laid out on their own.
              None => {
                // A match arm's block holding only this: flattened into the arm, so not one
                // given up on alone.
                if crate::format::stmt::is_tail(node)
                  && node.parent().is_some_and(|block| {
                    block.children_iter().count() == 1
                      && block
                        .parent()
                        .is_some_and(|p| p.kind() == SyntaxKind::MatchArm)
                  })
                {
                  flattened.insert(range.start as usize);
                }
                statements.push((
                  if matches!(
                    node.kind(),
                    SyntaxKind::LetStmt
                      | SyntaxKind::ExprStmt
                      | SyntaxKind::MacroRules
                      | SyntaxKind::MacroCall
                  ) {
                    crate::format::items::header_start(node).unwrap_or(range.start as usize)
                  } else if node.kind() == SyntaxKind::Use {
                    crate::format::imports::body_start(node).unwrap_or(range.start as usize)
                  } else {
                    range.start as usize
                  },
                  end,
                ))
              }
            }
          }
          // A `use` from its visibility or `use` on, and an item's header, after their
          // attributes.
          if node.kind() == SyntaxKind::Use
            && let Some(start) = crate::format::imports::body_start(node)
          {
            roles.layable.insert(start, node.clone());
          } else if (is_item(node.kind())
            || node.kind() == SyntaxKind::MacroCall && !braced_macro(node))
            && !skipped
            && let Some(start) = crate::format::items::header_start(node)
          {
            roles.layable.insert(start, node.clone());
          } else if (matches!(node.kind(), SyntaxKind::ExprStmt | SyntaxKind::LetStmt)
            || crate::format::stmt::is_tail(node))
            && node
              .parent()
              .is_some_and(|p| p.kind() == SyntaxKind::BlockExpr)
            && let Some(start) = crate::format::items::header_start(node)
          {
            roles.layable.insert(start, node.clone());
          }
        }
        // A block comment on a line in a binding pattern (`ref /* c */ x`, `a @ /* c */ b`),
        // after a pattern list's opening delimiter (`Some(/* c */ x)`) or after a `let`'s
        // `=`, one space from what follows it, and from what is before it but the delimiter,
        // as the standard formatter joins them (`combine_strs_with_missing_comments`).
        let patterned = matches!(
          node.kind(),
          SyntaxKind::IdentPat
            | SyntaxKind::TupleStructPat
            | SyntaxKind::TuplePat
            | SyntaxKind::SlicePat
            | SyntaxKind::LetExpr
            | SyntaxKind::LetStmt
        );
        let elements: Vec<SyntaxElement> = if patterned {
          node.children_with_tokens_iter().collect()
        } else {
          Vec::new()
        };
        let comment = |e: &SyntaxElement| matches!(e, SyntaxElement::Token(t) if t.kind() == SyntaxKind::BlockComment && !t.text().contains('\n'));
        let mut i = 0;
        while i < elements.len() {
          if !comment(&elements[i]) {
            i += 1;
            continue;
          }
          // A run of comments, with only spaces among them (kept as written).
          let first = i;
          let mut last = i;
          let mut j = i + 1;
          while j < elements.len() {
            match &elements[j] {
              SyntaxElement::Token(t) if t.kind() == SyntaxKind::Whitespace => j += 1,
              e if comment(e) => {
                last = j;
                j += 1;
              }
              _ => break,
            }
          }
          i = last + 1;
          let before = elements[..first]
            .iter()
            .rev()
            .find(|e| e.kind() != SyntaxKind::Whitespace);
          let after = elements[last + 1..]
            .iter()
            .find(|e| e.kind() != SyntaxKind::Whitespace);
          let (Some(before), Some(after)) = (before, after) else {
            continue;
          };
          if matches!(
            after.kind(),
            SyntaxKind::LineComment | SyntaxKind::BlockComment
          ) || before.kind() == SyntaxKind::LineComment
          {
            continue;
          }
          let opener = matches!(before.kind(), SyntaxKind::LParen | SyntaxKind::LBrack);
          let placed = match node.kind() {
            SyntaxKind::IdentPat => true,
            SyntaxKind::TupleStructPat | SyntaxKind::TuplePat | SyntaxKind::SlicePat => opener,
            _ => before.kind() == SyntaxKind::Eq,
          };
          if !placed {
            continue;
          }
          let at = elements[first].text_range().start as usize;
          if opener {
            roles.tight.insert(at);
          } else {
            roles.spaced.insert(at);
          }
          roles.spaced.insert(after.text_range().start as usize);
        }
        // Comments between a struct's, enum's or union's visibility and its keyword, one
        // space from each, as the standard formatter's `format_header` combines them; code
        // after a line comment goes on the next line.
        if matches!(
          node.kind(),
          SyntaxKind::Struct | SyntaxKind::Enum | SyntaxKind::Union
        ) && let Some(vis) = node
          .children_iter()
          .find(|c| c.kind() == SyntaxKind::Visibility)
        {
          let mut previous_block = false;
          for element in node
            .children_with_tokens_iter()
            .filter(|e| e.text_range().start >= vis.text_range().end)
          {
            match element.kind() {
              SyntaxKind::Whitespace => continue,
              SyntaxKind::LineComment | SyntaxKind::BlockComment => {
                roles.spaced.insert(element.text_range().start as usize);
                previous_block = element.kind() == SyntaxKind::BlockComment;
              }
              _ => {
                if previous_block {
                  roles.spaced.insert(element.text_range().start as usize);
                }
                break;
              }
            }
          }
        }
        // An `else if`, from its `if`; and the `else` before it, or before a block, one
        // space after the block, with one space after it.
        if node.kind() == SyntaxKind::IfExpr {
          if node.parent().is_some_and(|p| {
            p.kind() == SyntaxKind::IfExpr && p.children_iter().nth(2).as_ref() == Some(node)
          }) {
            roles.layable.insert(range.start as usize, node.clone());
          }
          let elements: Vec<SyntaxElement> = node.children_with_tokens_iter().collect();
          if let Some(at) = elements.iter().position(|e| e.kind() == SyntaxKind::ElseKw) {
            let only_space =
              |part: &[SyntaxElement]| part.iter().all(|e| e.kind() == SyntaxKind::Whitespace);
            let before = elements[..at]
              .iter()
              .rposition(|e| e.kind() != SyntaxKind::Whitespace);
            let after = elements[at + 1..]
              .iter()
              .position(|e| e.kind() != SyntaxKind::Whitespace)
              .map(|i| at + 1 + i);
            if let (Some(before), Some(after)) = (before, after)
              && elements[before].kind() == SyntaxKind::BlockExpr
              && only_space(&elements[before + 1..at])
              && only_space(&elements[at + 1..after])
            {
              roles
                .spaced
                .insert(elements[at].text_range().start as usize);
              // A comment after `else` keeps its line (one there, before the block, went on
              // a line of its own before the pass).
              if !elements[after].kind().is_trivia() {
                roles
                  .spaced
                  .insert(elements[after].text_range().start as usize);
              }
            }
          }
        }
        // A block's `{` after its `unsafe`, `async`, `move`, `const` or label, one space
        // apart, on their line.
        if node.kind() == SyntaxKind::BlockExpr {
          let elements: Vec<SyntaxElement> = node.children_with_tokens_iter().collect();
          let prefixed = elements
            .iter()
            .find(|e| e.kind() != SyntaxKind::Whitespace)
            .is_some_and(|e| {
              matches!(
                e.kind(),
                SyntaxKind::UnsafeKw
                  | SyntaxKind::AsyncKw
                  | SyntaxKind::ConstKw
                  | SyntaxKind::Label
              )
            });
          // Whitespace alone between the last prefix and `{`.
          if prefixed
            && let Some(open) = elements.iter().position(|e| e.kind() == SyntaxKind::LBrace)
            && open >= 2
            && elements[open - 1].kind() == SyntaxKind::Whitespace
            && !elements[open - 2].kind().is_trivia()
          {
            roles
              .spaced
              .insert(elements[open].text_range().start as usize);
          }
        }
        // A block's head (`async move {`, `unsafe {`, `loop {`) on one line, one space
        // apart, when no comment is in it.
        if matches!(node.kind(), SyntaxKind::BlockExpr | SyntaxKind::LoopExpr) {
          let head: Vec<SyntaxElement> = node
            .children_with_tokens_iter()
            .take_while(|e| !matches!(e.kind(), SyntaxKind::LBrace | SyntaxKind::BlockExpr))
            .collect();
          let opener = node
            .children_with_tokens_iter()
            .find(|e| matches!(e.kind(), SyntaxKind::LBrace | SyntaxKind::BlockExpr));
          let keywords = head.iter().all(|e| {
            e.kind() == SyntaxKind::Whitespace
              || matches!(
                e.kind(),
                SyntaxKind::AsyncKw
                  | SyntaxKind::MoveKw
                  | SyntaxKind::UnsafeKw
                  | SyntaxKind::ConstKw
                  | SyntaxKind::LoopKw
              )
          });
          if keywords
            && head.iter().any(|e| e.kind() != SyntaxKind::Whitespace)
            && let Some(opener) = opener
          {
            let after: Vec<usize> = head
              .iter()
              .skip(1)
              .filter(|e| e.kind() != SyntaxKind::Whitespace)
              .map(|e| e.text_range().start as usize)
              .chain(std::iter::once(opener.text_range().start as usize))
              .collect();
            for at in after {
              roles.spaced.insert(at);
            }
          }
        }
        // A macro call's name and its arguments: `name! {` one space apart, `name!(` and
        // `name![` touching.
        if node.kind() == SyntaxKind::MacroCall {
          let elements: Vec<SyntaxElement> = node.children_with_tokens_iter().collect();
          if let Some(bang) = elements.iter().position(|e| e.kind() == SyntaxKind::Bang)
            && let Some(tree) = elements[bang + 1..]
              .iter()
              .find(|e| e.kind() != SyntaxKind::Whitespace)
            && tree.kind() == SyntaxKind::TokenTree
            && let SyntaxElement::Node(tree) = tree
          {
            let at = tree.text_range().start as usize;
            if tree
              .first_token()
              .is_some_and(|t| t.kind() == SyntaxKind::LBrace)
            {
              roles.spaced.insert(at);
            } else {
              roles.tight.insert(at);
            }
          }
        }
        // Inner attributes in a row, and the comments between them, follow one another with
        // no blank line around the comments; between them, the comments' own blank lines stay
        // (`recover_missing_comment_in_span` rewrites them as one comment).
        if matches!(node.kind(), SyntaxKind::SourceFile | SyntaxKind::ItemList) {
          let mut after_inner = false;
          let mut pending: Vec<usize> = Vec::new();
          for element in node.children_with_tokens_iter() {
            match &element {
              SyntaxElement::Node(n) if n.kind() == SyntaxKind::Attr => {
                let inner = Attr::cast(n.clone()).is_some_and(|a| a.is_inner());
                if inner && after_inner {
                  roles.joined.extend(pending.first().copied());
                  roles.joined.insert(n.text_range().start as usize);
                }
                pending.clear();
                after_inner = inner;
              }
              SyntaxElement::Token(t)
                if matches!(t.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment) =>
              {
                pending.push(t.text_range().start as usize);
              }
              SyntaxElement::Token(t) if t.kind() == SyntaxKind::Whitespace => {}
              _ => {
                after_inner = false;
                pending.clear();
              }
            }
          }
        }
        // A match arm, from after its attributes.
        if node.kind() == SyntaxKind::MatchArm
          && !skipped
          && let Some(start) = crate::format::items::header_start(node)
        {
          roles.layable.insert(start, node.clone());
        }
        // A field of a struct definition or a variant, from after its attributes.
        if crate::format::items::is_field(node)
          && !skipped
          && let Some(start) = crate::format::items::header_start(node)
        {
          roles.layable.insert(start, node.clone());
        }
        let unparsed_attr =
          node.kind() == SyntaxKind::Attr && (!meta_like(node) || skipped_attribute(node));
        // Attributes of files, bodies, items, statements, fields, variants and match arms;
        // those in expressions are laid out with them.
        let attr_owner = node.parent().is_some_and(|p| {
          is_statement(p.kind())
            || matches!(
              p.kind(),
              SyntaxKind::SourceFile
                | SyntaxKind::ItemList
                | SyntaxKind::AssocItemList
                | SyntaxKind::ExternItemList
                | SyntaxKind::RecordField
                | SyntaxKind::TupleField
                | SyntaxKind::Variant
                | SyntaxKind::MatchArm
            )
            || p.kind() == SyntaxKind::BlockExpr
              && p.parent().is_some_and(|f| f.kind() == SyntaxKind::Fn)
            // A block's or a `match`'s inner attribute, which starts a line of its own in it.
            || matches!(p.kind(), SyntaxKind::BlockExpr | SyntaxKind::MatchArmList)
              && node
                .children_with_tokens_iter()
                .any(|e| e.kind() == SyntaxKind::Bang)
        });
        // Not one of a skipped item's, which keeps its attributes as written too.
        let owner_skipped = node.kind() == SyntaxKind::Attr
          && node.parent().is_some_and(|p| {
            p.kind() != SyntaxKind::SourceFile
              && p
                .children_of_kind(SyntaxKind::Attr)
                .filter_map(Attr::cast)
                .any(|a| a.is_rustfmt_skip())
          });
        if node.kind() == SyntaxKind::Attr
          && !unparsed_attr
          && !skipped
          && !owner_skipped
          && attr_owner
        {
          roles.layable.insert(range.start as usize, node.clone());
        }
        let kept = matches!(
          node.kind(),
          SyntaxKind::TokenTree | SyntaxKind::MacroRules | SyntaxKind::MacroDef
        );
        // A braced macro call's arguments, with no string spanning lines in them (whose
        // lines the standard formatter treats apart); one in parentheses or brackets that
        // cannot be read as code, when its last line holds only closing delimiters (the
        // standard formatter's `return_macro_parse_failure_fallback`).
        if node.kind() == SyntaxKind::TokenTree
          && node
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::MacroCall && !skipped_macro(&p))
          && (node
            .first_token()
            .is_some_and(|t| t.kind() == SyntaxKind::LBrace)
            || node.text().to_string().contains('\n')
              && node
                .text()
                .to_string()
                .rsplit('\n')
                .next()
                .is_some_and(|last| {
                  !last.trim().is_empty()
                    && last.trim().chars().all(|c| matches!(c, ')' | ']' | '}'))
                })
              && node
                .parent()
                .is_some_and(|call| crate::format::expr::unread_macro(&call)))
          && !config.hard_tabs
        {
          roles
            .preserved
            .insert(range.start as usize, range.end as usize);
        }
        // A `macro_rules!` definition from after its attributes, which are laid out.
        let rules_start = (node.kind() == SyntaxKind::MacroRules && !skipped)
          .then(|| crate::format::items::header_start(node))
          .flatten();
        if let Some(start) = rules_start {
          roles.macro_rules.insert(start, node.clone());
        }
        let member = (skipped
          && node
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::AssocItemList))
        .then(|| skipped_member(node, text))
        .flatten();
        if let Some((ranges, spaced, indented)) = member {
          roles.verbatim.extend(ranges);
          roles.spaced.extend(spaced);
          roles.after_skipped_mod.extend(indented);
        } else if kept || skipped || unparsed_attr {
          // A skipped item from its first attribute or doc comment: the comments before it
          // are laid out.
          let first_attribute = skipped
            .then(|| {
              node
                .children_with_tokens_iter()
                .find(|e| e.kind() == SyntaxKind::Attr || crate::format::items::is_doc(e.kind()))
            })
            .flatten()
            .map(|a| a.text_range().start as usize);
          let start = rules_start
            .or(first_attribute)
            .unwrap_or(range.start as usize);
          // A definition laid out by the pass is noted there when it is not.
          if skipped || unparsed_attr || node.kind() == SyntaxKind::MacroDef {
            let shown = &text[start..range.end as usize];
            crate::format::explain::note(node.kind().name(), shown, || {
              if skipped {
                "#[rustfmt::skip]".to_string()
              } else if unparsed_attr {
                "an attribute the standard formatter keeps as written (not name-value \
                 lists, or a skipped attribute)"
                  .to_string()
              } else {
                "a macro definition, kept as written".to_string()
              }
            });
          }
          // A definition's comment after its code is not part of it.
          let end = if node.kind() == SyntaxKind::MacroRules && !skipped {
            crate::format::imports::code_end(node)
          } else {
            range.end as usize
          };
          roles.verbatim.push((start, end));
        }
        // A field's default value (`f: T = 0`), which the standard formatter leaves as
        // written, from after its attributes.
        if node.kind() == SyntaxKind::RecordField
          && node
            .children_with_tokens_iter()
            .any(|e| e.kind() == SyntaxKind::Eq)
        {
          let start = crate::format::items::header_start(node).unwrap_or(range.start as usize);
          explain(root, start, &text[start..range.end as usize], || {
            "a field's default value, which the standard formatter keeps as written".to_string()
          });
          roles.verbatim.push((start, range.end as usize));
        }
        if is_container(node.kind()) {
          container(node, text, &mut roles, config);
        }
      }
      SyntaxElement::Token(token) => {
        let start = token.text_range().start as usize;
        if matches!(
          token.kind(),
          SyntaxKind::LineComment | SyntaxKind::OuterLineDoc | SyntaxKind::InnerLineDoc
        ) {
          line_comments.push(start);
        }
        if token.kind() == SyntaxKind::BlockComment && token.text().contains('\n') {
          block_comments.push((start, token.text_range().end as usize));
        }
        if token.kind() == SyntaxKind::BlockComment
          && !token.text().contains('\n')
          && matches!(
            token.parent().kind(),
            SyntaxKind::ArgList | SyntaxKind::ArrayExpr | SyntaxKind::TupleExpr
          )
        {
          let mut next = token.next_sibling_or_token();
          if let Some(SyntaxElement::Token(t)) = &next
            && t.kind() == SyntaxKind::Whitespace
            && !t.text().contains('\n')
          {
            next = t.next_sibling_or_token();
          }
          if let Some(SyntaxElement::Node(item)) = next {
            after_pre_comments.insert(item.text_range().start as usize);
          }
        }
        // A raw string's later lines are kept whatever their width.
        if matches!(
          token.kind(),
          SyntaxKind::RawStr | SyntaxKind::RawByteStr | SyntaxKind::RawCStr
        ) && token.text().contains('\n')
        {
          raw_strings.push((start, token.text_range().end as usize));
        }
        if config.style_edition >= 2024
          && token.kind() == SyntaxKind::Str
          && token.text().contains('\n')
          && token
            .text()
            .lines()
            .rev()
            .skip(1)
            .all(|line| line.ends_with('\\'))
        {
          continued_strings.push((start, token.text_range().end as usize));
        }
        // A comment after an opening `{` on its line starts the next line, as the first
        // thing inside; one in a block or module holding no code stays on the line.
        if matches!(
          token.kind(),
          SyntaxKind::LineComment | SyntaxKind::BlockComment
        ) {
          let mut before = token.prev_sibling_or_token();
          while let Some(SyntaxElement::Token(t)) = &before
            && t.kind() == SyntaxKind::Whitespace
            && !t.text().contains('\n')
          {
            before = t.prev_sibling_or_token();
          }
          let parent = token.parent();
          let no_code = matches!(parent.kind(), SyntaxKind::BlockExpr | SyntaxKind::ItemList)
            && !parent.children_with_tokens_iter().any(|e| {
              !e.kind().is_trivia() && !matches!(e.kind(), SyntaxKind::LBrace | SyntaxKind::RBrace)
            });
          // A block comment only where code follows it in the block.
          let block_comment = token.kind() == SyntaxKind::BlockComment;
          if before.is_some_and(|b| b.kind() == SyntaxKind::LBrace)
            && !no_code
            && (!block_comment
              || matches!(parent.kind(), SyntaxKind::BlockExpr | SyntaxKind::ItemList))
          {
            roles.breaks.insert(start);
          }
        }
        // A comment between a binary operator's operands is one the standard formatter
        // cannot place: it leaves the statement around it as written; but a match arm's
        // guard, kept as written itself.
        if matches!(
          token.kind(),
          SyntaxKind::LineComment | SyntaxKind::BlockComment
        ) && token.parent().kind() == SyntaxKind::BinExpr
          && !token
            .parent()
            .ancestors_iter()
            .any(|a| a.kind() == SyntaxKind::MatchGuard)
        {
          stranded.push(start);
        }
        // So is one among a pattern's alternatives, or in a `for` loop's head: what stands
        // in the block around it (a statement, or the block's last expression) is left as
        // written.
        if matches!(
          token.kind(),
          SyntaxKind::LineComment | SyntaxKind::BlockComment
        ) && matches!(
          token.parent().kind(),
          SyntaxKind::OrPat | SyntaxKind::ForExpr
        ) && let Some(standing) = token.parent().ancestors_iter().find(|a| {
          a.parent()
            .is_some_and(|p| p.kind() == SyntaxKind::BlockExpr)
        }) {
          let start = crate::format::items::header_start(&standing)
            .unwrap_or(standing.text_range().start as usize);
          explain(
            root,
            start,
            &text[start..standing.text_range().end as usize],
            || {
              format!(
                "a comment the standard formatter cannot place: {}",
                token.text().lines().next().unwrap_or("").trim()
              )
            },
          );
          roles
            .verbatim
            .push((start, standing.text_range().end as usize));
        }
        // A closure with a comment before its body the standard formatter returns as
        // written (`rewrite_closure` keeps its snippet).
        if matches!(
          token.kind(),
          SyntaxKind::LineComment | SyntaxKind::BlockComment
        ) && token.parent().kind() == SyntaxKind::ClosureExpr
        {
          let closure = token.parent();
          let (from, to) = (
            closure.text_range().start as usize,
            closure.text_range().end as usize,
          );
          explain(root, from, &text[from..to], || {
            format!(
              "a comment before a closure's body: {}",
              token.text().lines().next().unwrap_or("").trim()
            )
          });
          roles.verbatim.push((from, to));
        }
        if token.kind() == SyntaxKind::LineComment {
          let marker = token.text().trim();
          if marker == "// mto: off" && region.is_none() {
            region = Some(start);
          } else if marker == "// mto: on"
            && let Some(from) = region.take()
          {
            explain(
              root,
              from,
              &text[from..token.text_range().end as usize],
              || "// mto: off".to_string(),
            );
            roles.verbatim.push((from, token.text_range().end as usize));
          }
        }
        roles.tokens.push(token.clone());
      }
    }
  }
  if let Some(from) = region {
    explain(root, from, &text[from..], || "// mto: off".to_string());
    roles.verbatim.push((from, text.len()));
  }
  // Where lines start, which the measuring below splits written lines at.
  line_starts(root, &mut roles, config);
  control_comment_lines(root, &mut roles);
  else_close_comments(root, &mut roles, config);
  comment_runs(root, &mut roles);
  // A skipped `mod name;` keeps the indentation its first line is written at.
  for module in root.descendant_nodes_where(|kind| kind == SyntaxKind::Module) {
    if !module
      .children_iter()
      .any(|c| c.kind() == SyntaxKind::ItemList)
      && module
        .children_iter()
        .filter_map(Attr::cast)
        .any(|a| a.is_rustfmt_skip())
      && let Some(first) = module.first_token()
    {
      roles.kept_indent.insert(first.text_range().start as usize);
      // Written after other code on its line: the space before it as written too (the
      // standard formatter writes nothing for it but what it finds).
      if let Some(SyntaxElement::Token(space)) = module.prev_sibling_or_token()
        && space.kind() == SyntaxKind::Whitespace
        && !space.text().contains('\n')
        && space.prev_sibling_or_token().is_some()
      {
        // From the end of the token before, so that the space itself is inside.
        roles.verbatim.push((
          space.text_range().start as usize - 1,
          space.text_range().end as usize,
        ));
      }
      // An item written after it on its line follows it there, after the line's
      // indentation (the standard formatter writes no newline before it).
      let item = |e: &SyntaxElement| {
        matches!(e, SyntaxElement::Node(n) if is_item(n.kind())
          || matches!(n.kind(), SyntaxKind::Use | SyntaxKind::MacroCall | SyntaxKind::MacroRules))
      };
      let mut reindent_last = true;
      let followed = match module.next_sibling_or_token() {
        Some(SyntaxElement::Token(space))
          if space.kind() == SyntaxKind::Whitespace
            && !space.text().contains('\n')
            && space.next_sibling_or_token().is_some_and(|e| item(&e)) =>
        {
          // Another skipped item: the space between them as written, as all between is.
          let skipped_next = space.next_sibling_or_token().is_some_and(|e| {
            matches!(&e, SyntaxElement::Node(n) if n
              .children_iter()
              .filter_map(Attr::cast)
              .any(|a| a.is_rustfmt_skip()))
          });
          if skipped_next {
            roles.verbatim.push((
              space.text_range().start as usize - 1,
              space.text_range().end as usize,
            ));
            reindent_last = false;
          } else {
            roles
              .after_skipped_mod
              .insert(space.text_range().start as usize);
          }
          reindent_last
        }
        // Written touching it: the space the pass would put before it.
        Some(next) if item(&next) => {
          roles
            .after_skipped_mod
            .insert(next.text_range().start as usize);
          true
        }
        _ => false,
      };
      // Its last line, the item's, then at the block's indentation (the standard formatter
      // re-indents the last line of code it could not format).
      let (from, to) = (
        module.text_range().start as usize,
        module.text_range().end as usize,
      );
      if followed
        && let Some(newline) = text[from..to].rfind('\n').map(|at| from + at)
        && let Some(container) = roles.starts.get(&from).copied()
        && let Some(at) = roles
          .verbatim
          .iter()
          .position(|&(start, end)| start <= from && to <= end && start <= newline)
      {
        let (start, end) = roles.verbatim[at];
        let code =
          newline + 1 + (text[newline + 1..to].len() - text[newline + 1..to].trim_start().len());
        roles.verbatim[at] = (start, newline);
        roles.verbatim.push((code, end));
        roles.starts.insert(code, container);
        roles.breaks.insert(code);
      }
    }
  }
  // An item's header with a line too wide is laid out when it can be, else left as written
  // (decided as the pass reaches it).
  let spans = Spans {
    statements: &statements,
    breaks: &roles.breaks,
    chains: &chains,
    raw_strings: &raw_strings,
    continued_strings: &continued_strings,
    attrs: &attrs,
    line_comments: &line_comments,
    block_comments: &block_comments,
    after_pre_comments: &after_pre_comments,
    flattened: &flattened,
  };
  for (range, width, line) in overflowing(&spans, text, config) {
    if braced_macros.iter().any(|&(_, end)| end == range.1) {
      continue;
    }
    let reason = || {
      format!(
        "a written line is {} columns, past max_width {}: {}",
        width,
        config.max_width,
        text[line..].lines().next().unwrap_or("").trim()
      )
    };
    match headers.get(&range) {
      Some(&start) => {
        roles.wide_headers.insert(start, range.1);
        if crate::format::explain::active() {
          roles.wide_reasons.insert(start, reason());
        }
      }
      // Noted as the pass reaches it, when it is not laid out whole there.
      None => {
        if crate::format::explain::active() {
          roles.wide_reasons.insert(range.0, reason());
        }
        roles.verbatim.push(range)
      }
    }
  }
  // The innermost statement around each stranded comment.
  for comment in stranded {
    if let Some(&around) = statements
      .iter()
      .filter(|&&(start, end)| start < comment && comment < end)
      .min_by_key(|&&(start, end)| end - start)
    {
      explain(root, around.0, &text[around.0..around.1], || {
        format!(
          "a comment the standard formatter cannot place: {}",
          text[comment..].lines().next().unwrap_or("").trim()
        )
      });
      roles.verbatim.push(around);
    }
  }
  roles.statements = statements.iter().copied().collect();
  roles.line_comments = line_comments;
  // Sorted and merged, for looking up by binary search.
  roles.verbatim.sort_unstable();
  let mut merged: Vec<(usize, usize)> = Vec::with_capacity(roles.verbatim.len());
  for (start, end) in roles.verbatim.drain(..) {
    match merged.last_mut() {
      Some(last) if start < last.1 => last.1 = last.1.max(end),
      _ => merged.push((start, end)),
    }
  }
  roles.verbatim = merged;
  roles
}

/// Whether an attribute is one a `#[rustfmt::skip::attributes(..)]` around it (or a file's
/// `#![..]`) names: left exactly as written.
fn skipped_attribute(attr: &SyntaxNode) -> bool {
  let Some(path) = attr
    .descendant_nodes_where(|kind| kind == SyntaxKind::Path)
    .next()
  else {
    return false;
  };
  let name = path.text().to_string();
  let name = name.split("::").next().unwrap_or("").trim().to_string();
  if CRATE_SKIPS.with(|names| names.borrow().1.contains(&name)) {
    return true;
  }
  attr.ancestors_iter().skip(1).any(|owner| {
    owner
      .children_iter()
      .filter(|c| c.kind() == SyntaxKind::Attr && c != attr)
      .any(|other| {
        let text: String = other.text().to_string().split_whitespace().collect();
        text
          .split_once("rustfmt::skip::attributes(")
          .and_then(|(_, rest)| rest.split_once(')'))
          .is_some_and(|(names, _)| names.split(',').any(|n| n == name))
      })
  })
}

/// What of a skipped impl or trait member is left as written: the standard formatter writes
/// its attributes as it writes text it has not formatted, the member from its first token
/// after them as written. Between the comments there (indented as comments are), each line
/// of code ending its line is kept as written, the blank lines before it too; code a
/// comment follows on its line is indented as the member, its spacing kept, and the comment
/// follows it after one space. A verbatim range keeps what starts after its own start: each
/// starts one byte early. The ranges and where those comments start; `None` when code
/// shares the member's first line.
fn skipped_member(node: &SyntaxNode, text: &str) -> Option<(Ranges, Vec<usize>, Vec<usize>)> {
  let head = crate::format::items::header_start(node)?;
  let range = node.text_range();
  let start = match node.prev_sibling_or_token() {
    Some(SyntaxElement::Token(t)) if t.kind() == SyntaxKind::Whitespace => t.text_range().start,
    _ => range.start,
  } as usize;
  let comments = node.children_with_tokens_iter().filter_map(|e| match e {
    SyntaxElement::Token(t)
      if t.kind().is_trivia() && t.kind() != SyntaxKind::Whitespace
        || crate::format::items::is_doc(t.kind()) =>
    {
      Some(t.text_range())
    }
    _ => None,
  });
  let mut slices = Vec::new();
  let mut at = start;
  for comment in comments.filter(|c| (c.end as usize) <= head) {
    slices.push((at, comment.start as usize));
    at = comment.end as usize;
  }
  slices.push((at, head));
  let mut ranges = vec![(head, range.end as usize)];
  let mut spaced = Vec::new();
  // The space between attributes and the code on their line: the line's indentation
  // (`process_last_snippet` pushes the block's after them).
  let mut indented = Vec::new();
  for (from, to) in slices {
    let slice = &text[from..to];
    if slice.trim().is_empty() {
      continue;
    }
    let rest = match slice.rfind('\n') {
      Some(newline) => {
        let lines = slice[..newline].trim_end();
        if !lines.trim().is_empty() {
          ranges.push((from.saturating_sub(1), from + lines.len()));
        }
        from + newline + 1
      }
      None => from,
    };
    let code = text[rest..to].trim();
    if code.is_empty() {
      continue;
    }
    let code_start = rest + (text[rest..to].len() - text[rest..to].trim_start().len());
    ranges.push((code_start, code_start + code.len()));
    if to == head {
      if code_start + code.len() == head {
        return None;
      }
      indented.push(code_start + code.len());
    } else {
      spaced.push(to);
    }
  }
  Some((ranges, spaced, indented))
}

/// A file with `#![rustfmt::skip]` is left alone.
fn skips_itself(root: &SyntaxNode) -> bool {
  root
    .children_iter()
    .filter_map(Attr::cast)
    .any(|a| a.is_inner() && a.is_rustfmt_skip())
}

/// Whether a block of its own (no `unsafe`, `const`, `async` or label) stands as a statement
/// of a block.
fn standing_block(node: &SyntaxNode) -> bool {
  node
    .children_with_tokens_iter()
    .take_while(|e| e.kind() != SyntaxKind::LBrace)
    .all(|e| e.kind().is_trivia())
    && node.children_iter().any(|c| c.kind() != SyntaxKind::Label)
    && node.parent().is_some_and(|statement| {
      statement.kind() == SyntaxKind::ExprStmt
        && statement
          .parent()
          .is_some_and(|p| p.kind() == SyntaxKind::BlockExpr)
    })
}

/// Whether an attribute's arguments are what the standard formatter can format: names and
/// paths, `name = literal`, and `name(...)` lists of the same, separated by commas. Anything
/// else (a closure, a path as a value) is left as written.
/// Whether a literal's suffix is one it can take: none on a string or character, a
/// number's type (`1u8`, `2.0f32`) on a number.
fn valid_suffix(kind: SyntaxKind, text: &str) -> bool {
  match kind {
    SyntaxKind::IntNumber | SyntaxKind::FloatNumber => {
      let digits = if text.starts_with("0x") || text.starts_with("0X") {
        text.trim_end_matches(|c: char| !c.is_ascii_hexdigit() && c != '_')
      } else {
        text.trim_end_matches(|c: char| c.is_ascii_alphabetic() && c != 'e' && c != 'E')
      };
      let suffix = &text[digits.len()..];
      suffix.is_empty()
        || matches!(
          suffix,
          "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
        )
    }
    _ => {
      // The closing quote ends a string or character with no suffix.
      text.ends_with(['"', '\'', '#'])
    }
  }
}

fn width_of(indent: &str, tab: usize) -> usize {
  indent
    .chars()
    .map(|c| if c == '\t' { tab } else { char_width(c) })
    .sum()
}

#[cfg(test)]
mod tests;
