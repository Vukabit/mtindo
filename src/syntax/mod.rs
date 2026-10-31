//! Layer 1: Rust syntax, which layout runs on.
//! [`ast`] gives typed views over the tree for the parts layout needs.

pub mod ast;
pub mod edition;
pub mod kind;
pub mod lexer;
pub mod marker;
pub mod metavariables;
pub mod parser;
pub mod tree;

use std::cell::RefCell;

pub use edition::Edition;
pub use kind::SyntaxKind;
pub use tree::builder::SyntaxError;
pub use tree::{SyntaxElement, SyntaxNode, SyntaxToken, TextRange};

/// How deep a tree may nest, in nodes. Layout, formatting and the verifiers walk the tree
/// recursively, each level taking stack; a file nested deeper is left as written, with the
/// error [`TOO_DEEP`], rather than running out of stack.
pub const MAX_DEPTH: usize = 1024;

/// The error a file nested deeper than [`MAX_DEPTH`] gets.
pub const TOO_DEEP: &str = "nested too deeply";

/// How many recent parses a [`ParseCache`] keeps.
const CACHED_PARSES: usize = 4;

/// The result of parsing a file: a tree that always reproduces the input exactly, and the
/// errors found while lexing and parsing it.
#[derive(Debug, Clone)]
pub struct Parse {
  green: tree::GreenNode,
  errors: Vec<SyntaxError>,
  depth: usize,
}

impl Parse {
  /// How deep the tree nests, in nodes, counting the root.
  pub fn depth(&self) -> usize {
    self.depth
  }

  pub fn errors(&self) -> &[SyntaxError] {
    &self.errors
  }

  pub fn syntax_node(&self) -> SyntaxNode {
    SyntaxNode::new_root(self.green.clone())
  }
}

/// While alive, [`parse_cached`] on this thread reuses the tree of any recent parse of the
/// same text in the same edition. A file is parsed by layout, by formatting and by both
/// verifiers, mostly from the same few texts (a file layout leaves alone is formatted as it
/// was read), and a parse depends on nothing but its text and edition. Dropping it frees
/// every kept tree.
pub struct ParseCache {
  /// Whether this guard opened the cache, rather than finding one open.
  opened: bool,
}

impl ParseCache {
  pub fn open() -> Self {
    let opened = PARSES.with(|parses| {
      let mut parses = parses.borrow_mut();
      let closed = parses.is_none();
      if closed {
        *parses = Some(Vec::with_capacity(CACHED_PARSES));
      }
      closed
    });
    ParseCache { opened }
  }
}

thread_local! {
  /// This thread's recent parses by edition (and text, each tree's own), while a
  /// [`ParseCache`] is open.
  static PARSES: RefCell<Option<Vec<(Edition, Parse)>>> = const { RefCell::new(None) };
}

impl Drop for ParseCache {
  fn drop(&mut self) {
    if self.opened {
      // Taken out first, so the trees are not dropped while the cache is borrowed.
      let kept = PARSES.with(|parses| parses.borrow_mut().take());
      drop(kept);
    }
  }
}

/// Parses a whole source file.
pub fn parse(text: &str, edition: Edition) -> Parse {
  let tokens = lexer::split_tuple_field_floats(text, lexer::tokenize(text, edition));
  let parsed = parser::parse_events(text, &tokens, edition);
  let (green, errors, depth) = tree::builder::build_tree(text, &tokens, parsed);
  Parse {
    green,
    errors,
    depth,
  }
}

/// [`parse`], reusing a recent parse of exactly this text and edition while a [`ParseCache`]
/// is open on this thread.
pub fn parse_cached(text: &str, edition: Edition) -> Parse {
  let hit = PARSES.with(|parses| {
    let parses = parses.borrow();
    parses
      .as_ref()?
      .iter()
      .find(|(e, parse)| *e == edition && parse.green.text() == text)
      .map(|(_, parse)| parse.clone())
  });
  if let Some(parse) = hit {
    return parse;
  }
  let parsed = parse(text, edition);
  PARSES.with(|parses| {
    if let Some(parses) = parses.borrow_mut().as_mut() {
      if parses.len() == CACHED_PARSES {
        parses.remove(0);
      }
      parses.push((edition, parsed.clone()));
    }
  });
  parsed
}

/// The tokens of [`parse`]'s tree for `text`, in order, as kind and text, without building
/// the tree: for comparing what a snippet parses to, where only its tokens matter.
pub fn parse_tokens(text: &str, edition: Edition) -> Vec<(SyntaxKind, &str)> {
  let tokens = lexer::split_tuple_field_floats(text, lexer::tokenize(text, edition));
  let events = parser::parse_events(text, &tokens, edition).events;
  let mut out = Vec::with_capacity(tokens.len());
  let (mut pos, mut offset) = (0, 0);
  // As the tree builder places them: every raw token in order, trivia and anything the
  // parser left as themselves, and the parser's tokens each joined from its raw ones.
  let raw = |pos: usize, offset: usize| {
    (
      tokens[pos].kind,
      &text[offset..offset + tokens[pos].len as usize],
    )
  };
  for event in events {
    if let parser::event::Event::Token { kind, n_raw_tokens } = event {
      while pos < tokens.len() && tokens[pos].kind.is_trivia() {
        out.push(raw(pos, offset));
        offset += tokens[pos].len as usize;
        pos += 1;
      }
      let start = offset;
      for _ in 0..n_raw_tokens {
        offset += tokens[pos].len as usize;
        pos += 1;
      }
      out.push((kind, &text[start..offset]));
    }
  }
  while pos < tokens.len() {
    out.push(raw(pos, offset));
    offset += tokens[pos].len as usize;
    pos += 1;
  }
  out
}

#[cfg(test)]
mod tests {
  use proptest::prelude::*;

  use super::*;

  #[test]
  fn test_a_tree_nested_too_deeply_is_refused_and_kept_whole() {
    // The stack mto gives the stages that walk the tree, which the parser runs within.
    crate::with_stack(|| {
      let parens = |n: usize| format!("fn f() {{ {}1{} }}", "(".repeat(n), ")".repeat(n));
      let fits = parse(&parens(MAX_DEPTH - 10), Edition::E2021);
      assert!(fits.errors().is_empty(), "{:?}", &fits.errors()[..1]);
      assert!(fits.depth() <= MAX_DEPTH);
      // The parser stops recursing (parentheses), and the builder refuses what nests
      // without recursion in the parser (a chain, each call wrapping the last).
      let chain = format!("fn f() {{ x{}; }}", ".a()".repeat(50_000));
      for text in [parens(MAX_DEPTH), parens(50_000), chain] {
        let parsed = parse(&text, Edition::E2021);
        assert!(parsed.errors().iter().any(|e| e.message == TOO_DEEP));
        assert_eq!(parsed.syntax_node().text(), text);
      }
    });
  }

  #[test]
  fn test_comparisons_do_not_chain() {
    for text in [
      "fn f() { a < b < c; }",
      "fn f() { a == b != c; }",
      "fn f() { f(a) < g(b) < h; }",
    ] {
      assert!(!parse(text, Edition::E2021).errors().is_empty(), "{}", text);
    }
    for text in [
      "fn f() { a < b && c < d; }",
      "fn f() { (a < b) < c; }",
      "fn f() { a < b + c; (a as u8) < b; f::<u8>() < g::<i8>(); }",
    ] {
      assert!(parse(text, Edition::E2021).errors().is_empty(), "{}", text);
    }
  }

  #[test]
  fn test_is_node_tells_nodes_from_tokens() {
    for text in [include_str!("mod.rs"), include_str!("tree/red.rs")] {
      let root = parse(text, Edition::E2021).syntax_node();
      for element in root.descendants_with_tokens() {
        let node = matches!(element, SyntaxElement::Node(_));
        assert_eq!(element.kind().is_node(), node, "{:?}", element.kind());
      }
    }
  }

  #[test]
  fn test_parse_cache_reuses_only_the_same_text_and_edition() {
    let fresh = parse_cached("fn f() {}", Edition::E2021);
    let _cache = ParseCache::open();
    let first = parse_cached("fn f() {}", Edition::E2021);
    assert!(
      !first
        .syntax_node()
        .green()
        .ptr_eq(fresh.syntax_node().green())
    );
    let again = parse_cached("fn f() {}", Edition::E2021);
    assert!(
      again
        .syntax_node()
        .green()
        .ptr_eq(first.syntax_node().green())
    );
    let other_edition = parse_cached("fn f() {}", Edition::E2018);
    assert!(
      !other_edition
        .syntax_node()
        .green()
        .ptr_eq(first.syntax_node().green())
    );
    let other_text = parse_cached("fn f() { }", Edition::E2021);
    assert_eq!(other_text.syntax_node().text(), "fn f() { }");
    {
      // A cache opened inside another leaves it open when it closes.
      let _inner = ParseCache::open();
    }
    let still = parse_cached("fn f() {}", Edition::E2021);
    assert!(
      still
        .syntax_node()
        .green()
        .ptr_eq(first.syntax_node().green())
    );
    drop(_cache);
    let closed = parse_cached("fn f() {}", Edition::E2021);
    assert!(
      !closed
        .syntax_node()
        .green()
        .ptr_eq(first.syntax_node().green())
    );
  }

  #[test]
  fn test_parse_reports_lex_errors_with_ranges() {
    let parse = parse("fn a() { \"open", Edition::E2021);
    let lex_error = parse
      .errors()
      .iter()
      .find(|e| e.message == "string is not closed")
      .expect("the lexer error is reported");
    assert_eq!(lex_error.range, TextRange::new(9, 14));
  }

  #[test]
  fn test_parse_tokens_are_the_tree_tokens() {
    for text in [
      include_str!("mod.rs"),
      include_str!("tree/red.rs"),
      include_str!("parser/mod.rs"),
      "fn f() { a >>= b; x.0.1; let v: Vec<Vec<u8>> = c; } fn",
      "#![a] // c\n/** d */ struct S<'a>(&'a [u8]); )) } {",
      "",
    ] {
      for edition in [Edition::E2015, Edition::E2021, Edition::E2024] {
        assert_eq!(parse_tokens(text, edition), tree_tokens(text, edition));
      }
    }
  }

  #[test]
  fn test_three_dots_are_no_range_in_an_expression() {
    for text in [
      "fn f() { a...b; }",
      "fn f() { ...b; }",
      "fn f() { a.........b; }",
    ] {
      assert!(!parse(text, Edition::E2021).errors().is_empty(), "{}", text);
    }
    for text in [
      "fn f() { a..b; ..; a..; a..=b; }",
      "fn f(x: u8, ...) {}",
      "fn f() { match x { 1...2 => {} } }",
      "m!(a...b);",
    ] {
      assert!(parse(text, Edition::E2015).errors().is_empty(), "{}", text);
    }
  }

  /// The tree's tokens, which [`parse_tokens`] must give exactly, kinds and all; sliced
  /// from `text` by their ranges, to outlive the tree.
  fn tree_tokens(text: &str, edition: Edition) -> Vec<(SyntaxKind, &str)> {
    let tree = parse(text, edition).syntax_node();
    tree
      .token_texts()
      .map(|(kind, token, range)| {
        let at = &text[range.start as usize..range.end as usize];
        assert_eq!(token, at);
        (kind, at)
      })
      .collect()
  }

  proptest! {
      #![proptest_config(ProptestConfig::with_cases(2000))]

      #[test]
      fn prop_parse_tokens_are_the_tree_tokens(text in any::<String>()) {
          for edition in [Edition::E2015, Edition::E2021] {
              prop_assert_eq!(parse_tokens(&text, edition), tree_tokens(&text, edition));
          }
      }

      #[test]
      fn prop_tree_text_is_the_input(text in any::<String>()) {
          for edition in [Edition::E2015, Edition::E2018, Edition::E2021, Edition::E2024] {
              let tree = parse(&text, edition).syntax_node();
              prop_assert_eq!(tree.text(), text.clone());
              prop_assert_eq!(tree.text_range(), TextRange::new(0, text.len() as u32));
          }
      }
  }
}
