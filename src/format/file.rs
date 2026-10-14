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
//! Left as written, as the standard formatter leaves them: macro bodies, `macro_rules!`
//! definitions, attributes it cannot read as names and values, anything marked
//! `#[rustfmt::skip]`, `// mto: off` regions, files that skip themselves, files with syntax
//! errors, files `disable_all_formatting` or `format_generated_files` turn away, a
//! statement with a comment between a binary operator's operands, and a statement or item
//! with a line of code longer than `max_width`, which the standard formatter gives up on.
//! The lines of a closure, an argument or a match arm count toward the statement around it,
//! since the standard formatter formats them as part of its expression. Lines after a
//! statement's first are measured as written; its first line is measured as the pass reaches
//! it, at its new indentation, so that a second run decides the same.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

use crate::format::attrs::rewrite_attr;
use crate::format::imports::rewrite_use;
use crate::format::items::{is_item, rewrite_header};
use crate::format::space::{spacing, wants_space};
use crate::format::stmt::rewrite_statement;
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
  /// Containers whose blank lines before the closing delimiter stay (extern bodies), by
  /// their opening offset.
  keeps_trailing_blank: OffsetSet,
  /// Ranges left exactly as written.
  verbatim: Vec<(usize, usize)>,
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

/// The file formatted; the input itself when it cannot be formatted safely.
pub fn format_file(source: &str, edition: Edition, config: &FormatConfig) -> String {
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

/// Where a container's child (of `kind`, at `range`, a node or not) starts, if it is
/// something that starts a line: a node, or a comment standing among them.
fn construct_start((kind, range, node): (SyntaxKind, TextRange, bool)) -> Option<usize> {
  let starts = node
    || matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment)
    || crate::format::items::is_doc(kind);
  starts.then_some(range.start as usize)
}

fn container(node: &SyntaxNode, roles: &mut Roles, config: &FormatConfig) {
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
  // The comments between the last code and the closing delimiter.
  if let Some(close_at) = children.iter().rposition(|&(kind, _, _)| is_close(kind)) {
    for &(kind, range, node) in children[..close_at].iter().rev() {
      if !node && comment(kind) {
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

/// Whether a block's child is a statement, which makes the block one of lines.
fn is_statement(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    SyntaxKind::LetStmt
      | SyntaxKind::ExprStmt
      | SyntaxKind::Fn
      | SyntaxKind::Struct
      | SyntaxKind::Enum
      | SyntaxKind::Union
      | SyntaxKind::Use
      | SyntaxKind::Const
      | SyntaxKind::Static
      | SyntaxKind::Impl
      | SyntaxKind::Trait
      | SyntaxKind::TypeAlias
      | SyntaxKind::Module
      | SyntaxKind::MacroRules
      | SyntaxKind::MacroDef
      | SyntaxKind::ExternCrate
      | SyntaxKind::ExternBlock
      | SyntaxKind::MacroCall
  )
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
    )
  };
  for node in root.descendant_nodes_where(lines) {
    // Fields of a struct or union definition and an enum's variants are one per line too.
    let items = matches!(
      node.kind(),
      SyntaxKind::SourceFile
        | SyntaxKind::ItemList
        | SyntaxKind::AssocItemList
        | SyntaxKind::ExternItemList
        | SyntaxKind::VariantList
    ) || node.kind() == SyntaxKind::RecordFieldList
      && node
        .parent()
        .is_some_and(|p| matches!(p.kind(), SyntaxKind::Struct | SyntaxKind::Union));
    // A function's body with anything in it is one of lines too (`fn_single_line` off).
    let block = node.kind() == SyntaxKind::BlockExpr
      && (node.children_iter().any(|c| is_statement(c.kind()))
        || node.parent().is_some_and(|p| p.kind() == SyntaxKind::Fn)
          && node.children_iter().any(|c| c.kind() != SyntaxKind::Label));
    if !items && !block {
      continue;
    }
    let mut any = false;
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
      let mut joined = Vec::new();
      for element in child.children_with_tokens_iter() {
        match &element {
          SyntaxElement::Node(n) if n.kind() == SyntaxKind::Attr => {
            mark(n.text_range().start as usize, true);
            if after_attr {
              joined.push(n.text_range().start as usize);
            }
            after_attr = true;
          }
          SyntaxElement::Token(t)
            if crate::format::items::is_doc(t.kind())
              || matches!(t.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment) =>
          {
            mark(t.text_range().start as usize, false);
            after_attr = false;
          }
          SyntaxElement::Token(t) if t.kind() == SyntaxKind::Whitespace => {}
          SyntaxElement::Token(t)
            if t.kind().is_trivia() || crate::format::items::is_doc(t.kind()) =>
          {
            mark(t.text_range().start as usize, false);
          }
          other => {
            mark(other.text_range().start as usize, after_attr);
            // A statement's, field's, variant's or foreign item's attributes join it with no
            // blank line, as the standard formatter combines them; an item's in a file or
            // module keep one.
            let combined = node.kind() == SyntaxKind::ExternItemList
              || matches!(
                child.kind(),
                SyntaxKind::LetStmt
                  | SyntaxKind::ExprStmt
                  | SyntaxKind::RecordField
                  | SyntaxKind::Variant
              );
            if after_attr && combined {
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
    if !allowed || (previous == Some(SyntaxKind::Eq) && !kind.is_literal()) {
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
  let mut comment_column: Option<(usize, usize)> = None;
  let mut out_line_start = 0usize;
  let mut text_line_start = 0usize;
  let mut code_on_line = false;
  // The end of a statement written out whole; its tokens are passed over.
  let mut laid_out_to = 0usize;
  for (i, tok) in tokens.iter().enumerate() {
    let next = tokens.get(i + 1);
    if tok.start < laid_out_to {
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
      let aligned = comment_column.filter(|&(column, _)| {
        newlines == 1 && next.kind == SyntaxKind::LineComment && written == column
      });
      let indent = if let Some((_, column)) = aligned {
        column
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
            let step = if roles.unindented.contains(open) {
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
      if aligned.is_none() {
        comment_column = None;
      }
      for _ in 0..newlines {
        out.push('\n');
      }
      out_line_start = out.len();
      out.push_str(&indentation(indent, config));
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
    if let Some(node) = roles.layable.get(&tok.start)
      && !in_verbatim(tok.start)
      && out[out_line_start..].chars().all(|c| c == ' ' || c == '\t')
      && let Some((text, to)) = {
        let indent = width_of(&out[out_line_start..], tab);
        let whole = |text: Option<String>| text.map(|t| (t, node.text_range().end as usize));
        match node.kind() {
          SyntaxKind::Use => whole(rewrite_use(node, indent, config)),
          SyntaxKind::Attr => whole(rewrite_attr(node, indent, config)),
          // Not an item given up on whole.
          SyntaxKind::RecordField | SyntaxKind::Variant => {
            crate::format::items::rewrite_field(node, indent, config)
          }
          // Not an item given up on whole: a region left as written that covers all of it
          // (a verbatim attribute inside one is not that).
          kind if is_item(kind) => {
            let (start, end) = (
              node.text_range().start as usize,
              node.text_range().end as usize,
            );
            let covered = roles
              .verbatim
              .iter()
              .chain(gave_up.borrow().iter())
              .any(|&(s, e)| s <= start && end <= e);
            if covered {
              None
            } else {
              rewrite_header(node, indent, config)
            }
          }
          _ => whole(rewrite_statement(node, indent, config)),
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
      comment_column = Some((written, output));
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
          if !in_expression() {
            statements.push((range.start as usize, range.end as usize));
          }
          // A `use` from its visibility or `use` on, and an item's header, after their
          // attributes.
          if node.kind() == SyntaxKind::Use
            && let Some(start) = crate::format::imports::body_start(node)
          {
            roles.layable.insert(start, node.clone());
          } else if is_item(node.kind())
            && let Some(start) = crate::format::items::header_start(node)
          {
            roles.layable.insert(start, node.clone());
          } else if matches!(node.kind(), SyntaxKind::ExprStmt | SyntaxKind::LetStmt)
            && node
              .parent()
              .is_some_and(|p| p.kind() == SyntaxKind::BlockExpr)
            && let Some(start) = crate::format::items::header_start(node)
          {
            roles.layable.insert(start, node.clone());
          }
        }
        // A field of a struct definition or a variant, from after its attributes.
        if crate::format::items::is_field(node)
          && !skipped
          && let Some(start) = crate::format::items::header_start(node)
        {
          roles.layable.insert(start, node.clone());
        }
        let unparsed_attr = node.kind() == SyntaxKind::Attr && !meta_like(node);
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
        if kept || skipped || unparsed_attr {
          roles
            .verbatim
            .push((range.start as usize, range.end as usize));
        }
        if is_container(node.kind()) {
          container(node, &mut roles, config);
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
  roles.verbatim.extend(overflowing(
    &statements,
    &attrs,
    &line_comments,
    text,
    config,
  ));
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
