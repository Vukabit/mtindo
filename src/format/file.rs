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
  /// Every token of the file, in order: found by the same walk, which makes the handles of
  /// the nodes above them once.
  tokens: Vec<SyntaxToken>,
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
/// control expression's, or one holding a comment.
pub fn block_of_lines(node: &SyntaxNode) -> bool {
  node.kind() == SyntaxKind::BlockExpr && (lines_block(node) || control_block(node))
    || commented_block(node)
}

/// The file formatted; the input itself when it cannot be formatted safely.
pub fn format_file(source: &str, edition: Edition, config: &FormatConfig) -> String {
  let _memo = crate::format::expr::RewriteMemo::open();
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

/// A braced macro call's arguments as the standard formatter's `trim_left_preserve_layout`
/// writes them: the first line as written, without trailing whitespace; each later line
/// trimmed and indented from `indent` by as much as it was past the least indented of them
/// (tabs counting `tab_spaces`); a blank line empty.
pub(super) fn preserve_layout(snippet: &str, indent: usize, config: &FormatConfig) -> String {
  let tab = config.tab_spaces.max(1);
  let mut lines = snippet.split('\n');
  let first = lines.next().unwrap_or("").trim_end();
  let rest: Vec<&str> = lines.collect();
  let leading = |line: &str| {
    line
      .chars()
      .take_while(|c| *c == ' ' || *c == '\t')
      .map(|c| if c == '\t' { tab } else { 1 })
      .sum::<usize>()
  };
  let least = rest
    .iter()
    .filter(|l| !l.trim().is_empty())
    .map(|l| leading(l))
    .min()
    .unwrap_or(0);
  let mut out = first.to_string();
  for line in rest {
    out.push('\n');
    if !line.trim().is_empty() {
      out.push_str(&" ".repeat(indent + leading(line) - least));
      out.push_str(line.trim());
    }
  }
  out
}

/// Whether a macro call is one a `#[rustfmt::skip::macros(..)]` around it names: left
/// exactly as written.
pub(super) fn skipped_macro(call: &SyntaxNode) -> bool {
  let Some(path) = call.children_iter().find(|c| c.kind() == SyntaxKind::Path) else {
    return false;
  };
  let name = path.text().to_string();
  let name = name.rsplit("::").next().unwrap_or("").trim().to_string();
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
  if matches!(
    node.kind(),
    SyntaxKind::ItemList | SyntaxKind::ExternItemList
  ) {
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
    );
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

/// Whether a block is the body of an `if`, `else`, `while`, `for` or `loop` that is a
/// statement, a `let`'s value or a match arm's body, or the tail of a block of lines: the standard formatter writes it on lines
/// (an `if`'s may go on one line as a tail, where the statement layout writes it so).
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
  while let Some(up) = control.parent().filter(|p| p.kind() == SyntaxKind::IfExpr) {
    control = up;
  }
  control.parent().is_some_and(|p| {
    matches!(
      p.kind(),
      SyntaxKind::ExprStmt | SyntaxKind::LetStmt | SyntaxKind::MatchArm
    ) || p.kind() == SyntaxKind::BlockExpr && lines_block(&p)
  })
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

/// What the Rust style starts a line with wherever it is written: the items and attributes
/// of a file or a body, an item's or a statement's first token after its attributes, and in
/// a block holding a statement, or a function's body holding anything, each statement, the
/// tail and the closing brace. Each takes its container's indentation.
fn line_starts(root: &SyntaxNode, roles: &mut Roles) {
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
      && node
        .parent()
        .is_some_and(|p| matches!(p.kind(), SyntaxKind::Struct | SyntaxKind::Union));
    // A function's body with anything in it is one of lines too (`fn_single_line` off).
    let block = block_of_lines(&node);
    if !items && !block {
      continue;
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
    for child in node.children_iter() {
      if child.kind() == SyntaxKind::Label {
        continue;
      }
      any = true;
      let start = child.text_range().start as usize;
      roles.breaks.insert(start);
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
      // A statement's, field's, variant's, match arm's, foreign item's, `use`'s and
      // `extern crate`'s attributes and doc comments join it with no blank line, as the
      // standard formatter combines them; another item's in a file or module keep one.
      let combined = node.kind() == SyntaxKind::ExternItemList
        || matches!(
          child.kind(),
          SyntaxKind::LetStmt
            | SyntaxKind::ExprStmt
            | SyntaxKind::RecordField
            | SyntaxKind::Variant
            | SyntaxKind::MatchArm
            | SyntaxKind::Use
            | SyntaxKind::ExternCrate
        );
      let mut joined = Vec::new();
      for element in child.children_with_tokens_iter() {
        match &element {
          SyntaxElement::Node(n) if n.kind() == SyntaxKind::Attr => {
            mark(n.text_range().start as usize, true);
            // A blank line between a doc comment and an attribute stays.
            if after_attr {
              joined.push(n.text_range().start as usize);
            }
            after_attr = true;
            after_doc = false;
          }
          SyntaxElement::Token(t)
            if crate::format::items::is_doc(t.kind())
              || matches!(t.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment) =>
          {
            // A doc comment is an attribute: it starts a line.
            let doc = crate::format::items::is_doc(t.kind());
            mark(t.text_range().start as usize, doc);
            after_attr = false;
            after_doc = doc;
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
            mark(other.text_range().start as usize, after_attr || after_doc);
            if (after_attr || after_doc) && combined {
              joined.push(other.text_range().start as usize);
            }
            break;
          }
        }
      }
      roles.joined.extend(joined);
    }
    if any && node.kind() != SyntaxKind::SourceFile {
      for token in node.children_with_tokens_iter() {
        if token.kind() == SyntaxKind::RBrace {
          roles.breaks.insert(token.text_range().start as usize);
        }
      }
    }
  }
}

/// Whether an attribute's arguments are what the standard formatter can format: names and
/// paths, `name = literal`, and `name(...)` lists of the same, separated by commas. Anything
/// else (a closure, a path as a value) is left as written.
fn meta_like(attr: &SyntaxNode) -> bool {
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
    // name or a value.
    let allowed =
      allowed && (kind != SyntaxKind::Bang || token.parent().kind() == SyntaxKind::Attr);
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

/// The statements (`statements` in tree order) with a line of their own code wider than
/// `max_width`, as written: a line belongs to the innermost statement it starts in, and its
/// code ends where a line comment starts or the statement ends. A statement's first line is not measured: it is
/// indented anew, so measuring it would decide differently on a second run.
fn overflowing(
  statements: &[(usize, usize)],
  attrs: &[(usize, usize)],
  line_comments: &[usize],
  text: &str,
  config: &FormatConfig,
) -> Vec<(usize, usize)> {
  let tab = config.tab_spaces.max(1);
  let mut found = Vec::new();
  let mut open: Vec<(usize, usize, bool)> = Vec::new();
  let mut next = 0;
  let mut at = 0;
  for line in text.split('\n') {
    let line_end = at + line.len();
    let first = at + (line.len() - line.trim_start().len());
    // The statements that start before this line's first token are entered; those
    // ending before it are left.
    while next < statements.len() && statements[next].0 <= first {
      open.retain(|&(_, end, _)| end > statements[next].0);
      open.push((statements[next].0, statements[next].1, false));
      next += 1;
    }
    open.retain(|&(_, end, _)| end > first);
    // Attributes are formatted on their own; a long one does not stop its item.
    let in_attr = || {
      let idx = attrs.partition_point(|&(start, _)| start <= first);
      idx > 0 && first < attrs[idx - 1].1
    };
    if let Some(innermost) = open.last_mut()
      && innermost.0 < at
      && !innermost.2
      && !in_attr()
    {
      let comment = line_comments.partition_point(|&c| c < at);
      // Up to a line comment, or the statement's end when other code follows it.
      let code_end = line_comments
        .get(comment)
        .filter(|&&c| c < line_end)
        .copied()
        .unwrap_or(line_end)
        .min(innermost.1);
      if width_of(text[at..code_end].trim_end(), tab) > config.max_width {
        innermost.2 = true;
        found.push((innermost.0, innermost.1));
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
  // take its place.
  let mut spaced: Vec<Tok> = Vec::with_capacity(tokens.len());
  for tok in tokens {
    if roles.breaks.contains(&tok.start)
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
  let mut previous_significant: Option<usize> = None;
  // A line comment that ended a line of code: its column as written and as output.
  // A line comment after code: the column it was written at, the one it is written at,
  // whether it follows a statement's end, and the indentation of its line.
  let mut comment_column: Option<(usize, usize, bool, usize)> = None;
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
    // Space before what the Rust style starts a line with, written after other code: a
    // line break, the construct measured from the column it was written at.
    let breaks = tok.kind == SyntaxKind::Whitespace
      && !tok.text.contains('\n')
      && code_on_line
      && !in_verbatim(tok.start)
      && next.is_some_and(|n| roles.breaks.contains(&n.start) && !in_verbatim(n.start));
    if tok.synthetic && !breaks {
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
      let newlines = match after_open {
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
      let aligned = comment_column.filter(|&(column, _, after_statement, _)| {
        newlines == 1
          && next.kind == SyntaxKind::LineComment
          && (written == column || after_statement)
      });
      let indent = if let Some(open) = roles.closers.get(&next.start) {
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
        Some((_, _, true, block)) if config.style_edition >= 2024 => block,
        Some((_, column, _, _)) => column,
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
    // Spaces within a line: as the Rust style spaces the two tokens around them.
    if tok.kind == SyntaxKind::Whitespace && !in_verbatim(tok.start) && i > 0 {
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
        _ => false,
      };
      let spaced = match next {
        Some(n) if n.kind == SyntaxKind::LineComment && ends => Some(true),
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
      if let Some(laid) =
        crate::format::macros::rewrite_macro_rules(node, width_of(line, tab), edition, config)
      {
        out.push_str(&laid);
        if let Some(at) = out.rfind('\n') {
          out_line_start = at + 1;
        }
        laid_out_to = node.text_range().end as usize;
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
      out.push_str(&preserve_layout(&text[tok.start..end], indent, config));
      if let Some(at) = out.rfind('\n') {
        out_line_start = at + 1;
      }
      laid_out_to = end;
      previous_significant = None;
      code_on_line = true;
      comment_column = None;
      continue;
    }
    if let Some(node) = roles.layable.get(&tok.start)
      && !in_verbatim(tok.start)
      && !roles
        .kept_whole
        .contains(&(node.text_range().start as usize))
      // An `else if` follows `} else ` on its line.
      && (out[out_line_start..].chars().all(|c| c == ' ' || c == '\t')
        || node.kind() == SyntaxKind::IfExpr
          && node.parent().is_some_and(|p| p.kind() == SyntaxKind::IfExpr))
      && let Some((text, to)) = {
        let indent = width_of(&out[out_line_start..], tab);
        let whole = |text: Option<String>| text.map(|t| (t, node.text_range().end as usize));
        // A head laid out: its tail follows the block's `}` on its line.
        let take_head = |head: crate::format::stmt::Head| {
          for (at, kind) in head.tail {
            glued.borrow_mut().insert(at);
            if kind == SyntaxKind::Comma && !head.keep_comma {
              dropped.borrow_mut().insert(at);
            }
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
          SyntaxKind::Attr => whole(rewrite_attr(node, indent, config)),
          // Not an item given up on whole.
          SyntaxKind::RecordField | SyntaxKind::Variant => {
            crate::format::items::rewrite_field(node, indent, config)
          }
          // Not an item given up on whole.
          // A macro call standing as an item, in parentheses or brackets.
          SyntaxKind::MacroCall if !covered(node) => {
            whole(crate::format::items::rewrite_macro_item(node, indent, config))
          }
          kind if is_item(kind) => {
            if covered(node) {
              None
            } else {
              let laid = rewrite_header(node, indent, config);
              // An empty body after a header on one line closes up (`{}`); after one across
              // lines its `}` goes on a line of its own.
              if let Some((text, _)) = &laid
                && let Some(close) = empty_body_close(node)
              {
                if text.contains('\n') {
                  split.borrow_mut().insert(close);
                } else {
                  tight_items.borrow_mut().insert(close);
                }
              }
              laid
            }
          }
          _ => whole(rewrite_statement(node, indent, config)).or_else(|| {
            // A statement holding one block: its head, the block's insides left to this pass,
            // and its tail kept on the block's closing line. Not one given up on whole: its
            // head laid out would hide the long line on a second run.
            if covered(node) {
              return None;
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
              return control(&expression);
            };
            Some(take_head(head))
          }),
        }
      }
    {
      out.push_str(&text);
      if let Some(at) = out.rfind('\n') {
        out_line_start = at + 1;
      }
      laid_out_to = to;
      previous_significant = None;
      code_on_line = true;
      comment_column = None;
      continue;
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
      let width =
        width_of(&out[out_line_start..], tab) + width_of(text[tok.start..code_end].trim_end(), tab);
      if width > config.max_width {
        gave_up.borrow_mut().push((tok.start, end));
      }
    }
    if let Some(&base) = roles.openers.get(&tok.start) {
      let idx = lines.partition_point(|&(start, _, _)| start <= base);
      let (_, was, new) = lines[idx.saturating_sub(1)];
      open_indent.insert(tok.start, (was, new));
    }
    if tok.kind == SyntaxKind::LineComment && code_on_line {
      let written = width_of(&text[text_line_start..tok.start], tab);
      let output = width_of(&out[out_line_start..], tab);
      // After a statement's end, the comments below it are the standard formatter's
      // `missed_spans` comment: before the 2024 style, aligned under this one.
      let after_statement = tokens[..i]
        .iter()
        .rev()
        .find(|t| !t.kind.is_trivia())
        .is_some_and(|t| match t.kind {
          SyntaxKind::Semi => true,
          // The `}` ending a statement or item (`if x { .. }`, `fn f() { .. }`).
          SyntaxKind::RBrace => t
            .token
            .parent()
            .ancestors_iter()
            .find(|a| is_statement(a.kind()) || crate::format::stmt::is_tail(a))
            .is_some_and(|statement| {
              statement.text_range().end == t.token.text_range().end
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
        .and_then(|statement| crate::format::items::header_start(&statement));
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
      comment_column = Some((written, output, after_statement, block));
    }
    // An empty body's `}` kept apart from its `{`, written touching.
    if tok.kind == SyntaxKind::RBrace && split.borrow().contains(&tok.start) && out.ends_with('{') {
      let line = &out[out_line_start..];
      let indent = line[..line.len() - line.trim_start().len()].to_string();
      out.push('\n');
      out_line_start = out.len();
      out.push_str(&indent);
    }
    // Doc comments keep their trailing whitespace, as the standard formatter leaves it.
    let trimmed = tok.kind == SyntaxKind::LineComment && !in_verbatim(tok.start);
    out.push_str(if trimmed {
      tok.text.trim_end()
    } else {
      tok.text
    });
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
  for child in root.child_spans() {
    if let Some(start) = construct_start(child) {
      roles.starts.insert(start, None);
    }
  }
  let mut region: Option<usize> = None;
  let mut statements = Vec::new();
  let mut attrs = Vec::new();
  let mut stranded = Vec::new();
  let mut braced_macros: Vec<(usize, usize)> = Vec::new();
  let mut chained_const = Vec::new();
  let mut line_comments = Vec::new();
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
        // A closure's, an argument's or a match arm's block is formatted as part of
        // the expression it sits in: its lines count toward the statement around it,
        // which the standard formatter gives up on whole when one of them does not fit.
        let in_expression = || {
          node.ancestors_iter().skip(1).any(|a| {
            matches!(
              a.kind(),
              SyntaxKind::ClosureExpr | SyntaxKind::ArgList | SyntaxKind::MatchArmList
            )
          })
        };
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
            // A `use`'s comment after its `;` is not its own.
            _ if node.kind() == SyntaxKind::Use => crate::format::imports::code_end(node),
            _ => range.end as usize,
          };
          if !in_expression() {
            statements.push((range.start as usize, end));
          }
          // A `use` from its visibility or `use` on, and an item's header, after their
          // attributes.
          if node.kind() == SyntaxKind::Use
            && let Some(start) = crate::format::imports::body_start(node)
          {
            roles.layable.insert(start, node.clone());
          } else if (is_item(node.kind())
            || node.kind() == SyntaxKind::MacroCall && !braced_macro(node))
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
              roles
                .spaced
                .insert(elements[after].text_range().start as usize);
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
        // no blank line.
        if matches!(node.kind(), SyntaxKind::SourceFile | SyntaxKind::ItemList) {
          let mut after_inner = false;
          let mut pending: Vec<usize> = Vec::new();
          for element in node.children_with_tokens_iter() {
            match &element {
              SyntaxElement::Node(n) if n.kind() == SyntaxKind::Attr => {
                let inner = Attr::cast(n.clone()).is_some_and(|a| a.is_inner());
                if inner && after_inner {
                  roles.joined.extend(pending.drain(..));
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
        // Attributes of files, bodies, items, statements, fields and variants; those among
        // match arms or in expressions the standard formatter leaves as written.
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
            )
            || p.kind() == SyntaxKind::BlockExpr
              && p.parent().is_some_and(|f| f.kind() == SyntaxKind::Fn)
        });
        if node.kind() == SyntaxKind::Attr && !unparsed_attr && !skipped && attr_owner {
          roles.layable.insert(range.start as usize, node.clone());
        }
        let kept = matches!(
          node.kind(),
          SyntaxKind::TokenTree | SyntaxKind::MacroRules | SyntaxKind::MacroDef
        );
        // A braced macro call's arguments, with no string spanning lines in them (whose
        // lines the standard formatter treats apart).
        if node.kind() == SyntaxKind::TokenTree
          && node
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::MacroCall && !skipped_macro(&p))
          && node
            .first_token()
            .is_some_and(|t| t.kind() == SyntaxKind::LBrace)
          && !config.hard_tabs
          && !node
            .token_texts()
            .any(|(kind, text, _)| kind.is_literal() && text.contains('\n'))
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
        if kept || skipped || unparsed_attr {
          let start = rules_start.unwrap_or(range.start as usize);
          // A definition's comment after its code is not part of it.
          let end = if node.kind() == SyntaxKind::MacroRules && !skipped {
            crate::format::imports::code_end(node)
          } else {
            range.end as usize
          };
          roles.verbatim.push((start, end));
        }
        if is_container(node.kind()) {
          container(node, text, &mut roles, config);
        }
        // An inline `const` block a chain starts with the standard formatter leaves as
        // written, and the statement around it.
        if node.kind() == SyntaxKind::BlockExpr
          && node
            .first_token()
            .is_some_and(|t| t.kind() == SyntaxKind::ConstKw)
          && node.parent().is_some_and(|p| {
            matches!(
              p.kind(),
              SyntaxKind::MethodCallExpr
                | SyntaxKind::FieldExpr
                | SyntaxKind::AwaitExpr
                | SyntaxKind::TryExpr
            ) && p.children_iter().next().as_ref() == Some(node)
          })
        {
          chained_const.push(range.start as usize);
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
        // A line comment after an opening `{` on its line starts the next line, as the
        // first thing inside.
        if token.kind() == SyntaxKind::LineComment {
          let mut before = token.prev_sibling_or_token();
          while let Some(SyntaxElement::Token(t)) = &before
            && t.kind() == SyntaxKind::Whitespace
            && !t.text().contains('\n')
          {
            before = t.prev_sibling_or_token();
          }
          if before.is_some_and(|b| b.kind() == SyntaxKind::LBrace) {
            roles.breaks.insert(start);
          }
        }
        // A comment between a binary operator's operands is one the standard
        // formatter cannot place: it leaves the statement around it as written.
        if matches!(
          token.kind(),
          SyntaxKind::LineComment | SyntaxKind::BlockComment
        ) && token.parent().kind() == SyntaxKind::BinExpr
        {
          stranded.push(start);
        }
        if token.kind() == SyntaxKind::LineComment {
          let marker = token.text().trim();
          if marker == "// mto: off" && region.is_none() {
            region = Some(start);
          } else if marker == "// mto: on"
            && let Some(from) = region.take()
          {
            roles.verbatim.push((from, token.text_range().end as usize));
          }
        }
        roles.tokens.push(token.clone());
      }
    }
  }
  if let Some(from) = region {
    roles.verbatim.push((from, text.len()));
  }
  roles.verbatim.extend(
    overflowing(&statements, &attrs, &line_comments, text, config)
      .into_iter()
      .filter(|range| !braced_macros.contains(range)),
  );
  // The innermost statement around each stranded comment.
  for comment in stranded {
    if let Some(&around) = statements
      .iter()
      .filter(|&&(start, end)| start < comment && comment < end)
      .min_by_key(|&&(start, end)| end - start)
    {
      roles.verbatim.push(around);
    }
  }
  // The innermost statement starting with, or around, each chained `const` block: not laid
  // out, though it starts the region.
  for block in chained_const {
    if let Some(&around) = statements
      .iter()
      .filter(|&&(start, end)| start <= block && block < end)
      .min_by_key(|&&(start, end)| end - start)
    {
      roles.verbatim.push(around);
      roles.kept_whole.insert(around.0);
    }
  }
  roles.statements = statements.iter().copied().collect();
  roles.line_comments = line_comments;
  line_starts(root, &mut roles);
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

/// A file with `#![rustfmt::skip]` is left alone.
fn skips_itself(root: &SyntaxNode) -> bool {
  root
    .children_iter()
    .filter_map(Attr::cast)
    .any(|a| a.is_inner() && a.is_rustfmt_skip())
}

/// The display width of text: tabs count as `tab_spaces`, wide characters as two columns.
fn width_of(indent: &str, tab: usize) -> usize {
  indent
    .chars()
    .map(|c| if c == '\t' { tab } else { char_width(c) })
    .sum()
}

#[cfg(test)]
mod tests;
