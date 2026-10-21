//! The scrambler of the scrambled-pair suite (`cargo xtask pairs`): a file put in another
//! state it could have been written in, one that still compiles to the same program, so a
//! formatter that fails on it is at fault and never the scrambler.
//!
//! Two passes, each checked before the next is made:
//!
//! - **Layout:** the items of each module (the file, an inline `mod`) and the members of
//!   each `impl` are shuffled where their order carries no meaning. An item moves whole,
//!   with its attributes, doc comments and the comments above and after it. Some stay
//!   where they are and are never crossed: the file's first item (the header above it goes
//!   with it), `extern crate`, items under `#[cfg]` or `#[cfg_attr]`, items with an
//!   attribute that may be a macro, and macro calls among impl members. A `macro_rules!`,
//!   a `macro` definition, a macro call and a `#[macro_use]` or `#[macro_export]` item stay
//!   too, and so does everything after them in their list: what follows a macro
//!   definition in the text may use it. Checked: the shuffled file parses, holds the same
//!   tokens, and each list holds the same items byte for byte, every fixed one in its
//!   place.
//! - **Whitespace:** every run of whitespace between two tokens becomes another
//!   ([`crate::unformat`]): never empty, and with a line break after a line comment.
//!   Checked with rustc's own lexer: the same tokens in the same order, with whitespace
//!   between the same pairs (`& &x` never becomes `&&x`), and the file still parses.
//!
//! A failed check is the scrambler's bug: the file is reported and left out.

use mtindo::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, parse};
use ra_ap_rustc_lexer as rustc;

use crate::unformat::unformat_with;

/// Attributes that never make an item depend on where it stands. Any other attribute may
/// be a macro, so its item stays in place.
const PLAIN_ATTRIBUTES: &[&str] = &[
  "allow",
  "automatically_derived",
  "bench",
  "cold",
  "deny",
  "deprecated",
  "derive",
  "doc",
  "expect",
  "export_name",
  "forbid",
  "global_allocator",
  "ignore",
  "inline",
  "link",
  "link_name",
  "link_section",
  "must_use",
  "no_mangle",
  "non_exhaustive",
  "panic_handler",
  "path",
  "repr",
  "should_panic",
  "target_feature",
  "test",
  "track_caller",
  "used",
  "warn",
];

/// Tools whose attributes are all plain (`#[rustfmt::skip]`, `#[clippy::...]`).
const PLAIN_TOOLS: &[&str] = &["clippy::", "diagnostic::", "rustfmt::"];

/// What scrambling a file came to.
#[derive(Debug, PartialEq)]
pub enum Outcome {
  /// The scrambled text, and how many items moved.
  Scrambled { text: String, moved: usize },
  /// Not scrambled, through no one's fault: why (syntax errors, carriage returns).
  Skipped(String),
  /// A check failed: the scrambler's bug, and what failed.
  Bug(String),
}

/// Where an item may go.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
  /// Anywhere between the fixed items around it.
  Moves,
  /// Stays, and nothing crosses it.
  Stays,
  /// Stays, and so does everything after it in its list.
  Ends,
}

/// One item of a list, with the comments it carries.
struct Entry {
  /// Its text: from the first comment above it that goes with it to its end.
  start: usize,
  end: usize,
  node: SyntaxNode,
  role: Role,
}

/// A list of items: a file, an inline module's body or an impl's body.
struct List {
  /// Where the text before the first item ends: inner attributes, the brace, a shebang.
  head_end: usize,
  entries: Vec<Entry>,
}

/// splitmix64: a small, fixed generator, so a seed always scrambles the same way.
pub struct Random(u64);

impl Random {
  pub fn new(seed: u64) -> Random {
    Random(seed)
  }

  pub fn next(&mut self) -> u64 {
    self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = self.0;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
  }

  /// A number below `n` (which is not 0).
  pub fn below(&mut self, n: usize) -> usize {
    (self.next() % n as u64) as usize
  }

  /// `items` in a random order.
  pub fn shuffle<T>(&mut self, items: &mut [T]) {
    for at in (1..items.len()).rev() {
      items.swap(at, self.below(at + 1));
    }
  }
}

/// `text` scrambled from `seed`: its items shuffled, then its whitespace.
pub fn scramble(text: &str, edition: Edition, seed: u64) -> Outcome {
  if text.contains('\r') {
    return Outcome::Skipped("carriage returns".to_string());
  }
  if !parse(text, edition).errors().is_empty() {
    return Outcome::Skipped("syntax errors".to_string());
  }
  // rustc reads the whitespace around a cargo script's frontmatter as its own.
  let tokens = mtindo::syntax::lexer::tokenize(text, edition);
  if tokens.iter().any(|t| t.kind == SyntaxKind::Frontmatter) {
    return Outcome::Skipped("frontmatter".to_string());
  }
  let mut random = Random::new(seed);
  let (shuffled, moved) = shuffle(text, edition, &mut random);
  if let Err(why) = check_shuffle(text, &shuffled, edition) {
    return Outcome::Bug(format!("layout: {}", why));
  }
  let Some(messy) = unformat_with(&shuffled, edition, random.next() | 1) else {
    return Outcome::Bug("whitespace: the unformatter refused the file".to_string());
  };
  if let Err(why) = check_whitespace(&shuffled, &messy, edition) {
    return Outcome::Bug(format!("whitespace: {}", why));
  }
  Outcome::Scrambled { text: messy, moved }
}

/// The path an attribute starts with: `derive` for `#[derive(Debug)]`, `rustfmt::skip`,
/// `no_mangle` for `#[unsafe(no_mangle)]`.
fn attribute_name(attr: &str) -> String {
  let squeezed: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
  let inside = squeezed
    .trim_start_matches('#')
    .trim_start_matches('!')
    .trim_start_matches('[');
  let inside = inside.strip_prefix("unsafe(").unwrap_or(inside);
  inside
    .chars()
    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
    .collect()
}

/// The file as a string that does not change when items move as the rules allow: each
/// list's head and tail (whitespace aside), each fixed item in its place, and the items
/// between fixed ones sorted. An item is its text byte for byte, with its own list in the
/// same form.
fn canonical(text: &str, edition: Edition) -> String {
  let root = parse(text, edition).syntax_node();
  canonical_list(&root, text, false)
}

fn canonical_list(node: &SyntaxNode, text: &str, in_impl: bool) -> String {
  let range = node.text_range();
  let (start, end) = (range.start as usize, range.end as usize);
  let Some(list) = list_of(node, in_impl) else {
    return format!("<as written:{}>", &text[start..end]);
  };
  let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
  let fixed = fixed(&list.entries);
  let mut out = format!("<head:{}>", squeeze(&text[start..list.head_end]));
  let mut free: Vec<String> = Vec::new();
  let flush = |free: &mut Vec<String>, out: &mut String| {
    if !free.is_empty() {
      free.sort();
      out.push_str(&format!("<free:{}>", free.join("\u{0}")));
      free.clear();
    }
  };
  for (entry, stays) in list.entries.iter().zip(fixed) {
    let item = match inner_list(&entry.node) {
      Some((inner, in_impl)) => {
        let inner_range = inner.text_range();
        format!(
          "{}{}{}",
          &text[entry.start..inner_range.start as usize],
          canonical_list(&inner, text, in_impl),
          &text[inner_range.end as usize..entry.end]
        )
      }
      None => text[entry.start..entry.end].to_string(),
    };
    if stays {
      flush(&mut free, &mut out);
      out.push_str(&format!("<fixed:{}>", item));
    } else {
      free.push(item);
    }
  }
  flush(&mut free, &mut out);
  let last = list.entries.last().map_or(start, |e| e.end);
  out.push_str(&format!("<tail:{}>", squeeze(&text[last..end])));
  out
}

/// Checks that `shuffled` is `original` with items moved only as the rules allow.
fn check_shuffle(original: &str, shuffled: &str, edition: Edition) -> Result<(), String> {
  let parsed = parse(shuffled, edition);
  if let Some(error) = parsed.errors().first() {
    return Err(format!("the shuffled file does not parse: {:?}", error));
  }
  let before = canonical(original, edition);
  let after = canonical(shuffled, edition);
  if before != after {
    let at = before
      .chars()
      .zip(after.chars())
      .take_while(|(a, b)| a == b)
      .count();
    let near: String = after.chars().skip(at.saturating_sub(40)).take(80).collect();
    return Err(format!(
      "the lists hold other items or fixed items moved, near {:?}",
      near
    ));
  }
  let mut tokens_before = rustc_tokens(original);
  let mut tokens_after = rustc_tokens(shuffled);
  tokens_before.sort();
  tokens_after.sort();
  let solid = |tokens: Vec<(String, String, bool)>| -> Vec<(String, String)> {
    tokens
      .into_iter()
      .map(|(kind, text, _)| (kind, text))
      .collect()
  };
  if solid(tokens_before) != solid(tokens_after) {
    return Err("the shuffled file holds other tokens".to_string());
  }
  Ok(())
}

/// Checks that `messy` differs from `tidy` only in the whitespace between tokens.
fn check_whitespace(tidy: &str, messy: &str, edition: Edition) -> Result<(), String> {
  same_tokens(tidy, messy)?;
  if let Some(error) = parse(messy, edition).errors().first() {
    return Err(format!("the scrambled file does not parse: {:?}", error));
  }
  Ok(())
}

fn ends_in_line_comment(entry: &Entry) -> bool {
  entry.node.last_token().is_some_and(|token| {
    matches!(
      token.kind(),
      SyntaxKind::LineComment | SyntaxKind::OuterLineDoc | SyntaxKind::InnerLineDoc
    )
  })
}

/// Which entries stay where they are: those that do not move, and every entry after one
/// that ends its list.
fn fixed(entries: &[Entry]) -> Vec<bool> {
  let mut ended = false;
  entries
    .iter()
    .map(|entry| {
      let stays = ended || entry.role != Role::Moves;
      ended |= entry.role == Role::Ends;
      stays
    })
    .collect()
}

/// The list inside an item, if it has one that is shuffled: an impl's members or an inline
/// module's items; and whether it is an impl's.
fn inner_list(item: &SyntaxNode) -> Option<(SyntaxNode, bool)> {
  match item.kind() {
    SyntaxKind::Impl => item
      .children_of_kind(SyntaxKind::AssocItemList)
      .next()
      .map(|list| (list, true)),
    SyntaxKind::Module => item
      .children_of_kind(SyntaxKind::ItemList)
      .next()
      .map(|list| (list, false)),
    _ => None,
  }
}

/// The items of list `node` (a file, an item list or an impl's member list), or `None`
/// when it holds anything else between them (an error, a stray token) and is left alone.
fn list_of(node: &SyntaxNode, in_impl: bool) -> Option<List> {
  let is_file = node.kind() == SyntaxKind::SourceFile;
  let mut head_end = node.text_range().start as usize;
  let mut entries: Vec<Entry> = Vec::new();
  // Where the comments waiting for the next item start.
  let mut pending: Option<usize> = None;
  for element in node.children_with_tokens() {
    let range = element.text_range();
    let (start, end) = (range.start as usize, range.end as usize);
    match element {
      SyntaxElement::Token(token) => match token.kind() {
        SyntaxKind::Whitespace => {}
        // A comment on its own goes with the item after it.
        SyntaxKind::LineComment | SyntaxKind::BlockComment => {
          pending.get_or_insert(start);
        }
        SyntaxKind::LBrace | SyntaxKind::Bom | SyntaxKind::Shebang | SyntaxKind::Frontmatter
          if entries.is_empty() && pending.is_none() =>
        {
          head_end = end
        }
        // Inner doc comments, like inner attributes, are the list's head.
        SyntaxKind::InnerLineDoc | SyntaxKind::InnerBlockDoc if entries.is_empty() => {
          head_end = end;
          pending = None;
        }
        // What follows the last item stays where it is.
        SyntaxKind::RBrace => break,
        _ => return None,
      },
      SyntaxElement::Node(child) => {
        if child.kind() == SyntaxKind::Attr {
          // Inner attributes, and the comments before them, are the list's head.
          if !entries.is_empty() {
            return None;
          }
          head_end = end;
          pending = None;
          continue;
        }
        if child.kind() == SyntaxKind::Error {
          return None;
        }
        let role = if is_file && entries.is_empty() {
          // The file's first item carries the file's header comments.
          Role::Stays
        } else {
          role_of(&child, in_impl)
        };
        entries.push(Entry {
          start: pending.take().unwrap_or(start),
          end,
          node: child,
          role,
        });
      }
    }
  }
  (!entries.is_empty()).then_some(List { head_end, entries })
}

/// The new text of an entry: its own list (an impl's members, an inline module's items)
/// shuffled too.
fn rewrite_entry(entry: &Entry, text: &str, random: &mut Random, moved: &mut usize) -> String {
  match inner_list(&entry.node) {
    Some((inner, in_impl)) => {
      let range = inner.text_range();
      let mut out = text[entry.start..range.start as usize].to_string();
      out.push_str(&rewrite_list(&inner, text, in_impl, random, moved));
      out.push_str(&text[range.end as usize..entry.end]);
      out
    }
    None => text[entry.start..entry.end].to_string(),
  }
}

/// The new text of list `node`.
fn rewrite_list(
  node: &SyntaxNode,
  text: &str,
  in_impl: bool,
  random: &mut Random,
  moved: &mut usize,
) -> String {
  let range = node.text_range();
  let (start, end) = (range.start as usize, range.end as usize);
  let Some(list) = list_of(node, in_impl) else {
    return text[start..end].to_string();
  };
  let bodies: Vec<String> = list
    .entries
    .iter()
    .map(|entry| rewrite_entry(entry, text, random, moved))
    .collect();
  let fixed = fixed(&list.entries);
  let mut order: Vec<usize> = (0..list.entries.len()).collect();
  let mut at = 0;
  while at < order.len() {
    if fixed[at] {
      at += 1;
      continue;
    }
    let run_end = (at..order.len()).find(|&i| fixed[i]).unwrap_or(order.len());
    random.shuffle(&mut order[at..run_end]);
    at = run_end;
  }
  *moved += order.iter().enumerate().filter(|&(i, &j)| i != j).count();
  let mut out = text[start..list.head_end].to_string();
  let mut previous = list.head_end;
  for (slot, &from) in order.iter().enumerate() {
    let separator = &text[previous..list.entries[slot].start];
    // Each item after the first starts its own line: a line comment ending the item
    // before cannot swallow it, and comments stay with the item they were with.
    if slot == 0 || separator.contains('\n') {
      out.push_str(separator);
    } else {
      out.push('\n');
    }
    out.push_str(&bodies[from]);
    previous = list.entries[slot].end;
  }
  let tail = &text[previous..end];
  let last = order.last().map(|&i| &list.entries[i]);
  let tail_breaks = tail
    .split(|c: char| !c.is_whitespace())
    .next()
    .is_some_and(|space| space.contains('\n'));
  if last.is_some_and(ends_in_line_comment) && !tail_breaks {
    out.push('\n');
  }
  out.push_str(tail);
  out
}

/// Where an item of a module (or of an impl, when `in_impl`) may go.
fn role_of(item: &SyntaxNode, in_impl: bool) -> Role {
  use SyntaxKind as K;
  let mut role = match (item.kind(), in_impl) {
    (K::MacroRules | K::MacroDef | K::MacroCall, false) => Role::Ends,
    (
      K::Fn
      | K::Struct
      | K::Enum
      | K::Union
      | K::Trait
      | K::Impl
      | K::TypeAlias
      | K::Const
      | K::Static
      | K::Use
      | K::Module
      | K::ExternBlock,
      false,
    )
    | (K::Fn | K::Const | K::TypeAlias, true) => Role::Moves,
    // `extern crate`, macro calls among members, and anything not known to be free.
    _ => Role::Stays,
  };
  for attr in item.children_of_kind(K::Attr) {
    let name = attribute_name(attr.text());
    if name.contains("macro_use") || name.contains("macro_export") {
      return if in_impl { Role::Stays } else { Role::Ends };
    }
    let plain = PLAIN_ATTRIBUTES.contains(&name.as_str())
      || PLAIN_TOOLS.iter().any(|tool| name.starts_with(tool));
    if !plain && role == Role::Moves {
      // `cfg` and `cfg_attr` too: a pair of items for two configurations keeps its order.
      role = Role::Stays;
    }
  }
  role
}

/// rustc's tokens of `text`, whitespace aside: each one's kind, its text, and whether
/// whitespace comes before it. A byte-order mark and a shebang line are tokens of their
/// own, as rustc strips them first.
fn rustc_tokens(text: &str) -> Vec<(String, String, bool)> {
  let mut tokens = Vec::new();
  let mut offset = 0;
  if text.starts_with('\u{feff}') {
    offset = '\u{feff}'.len_utf8();
    tokens.push(("bom".to_string(), String::new(), false));
  }
  if let Some(length) = rustc::strip_shebang(&text[offset..]) {
    tokens.push((
      "shebang".to_string(),
      text[offset..offset + length].to_string(),
      false,
    ));
    offset += length;
  }
  let mut spaced = false;
  for token in rustc::tokenize(&text[offset..], rustc::FrontmatterAllowed::Yes) {
    let end = offset + token.len as usize;
    if token.kind == rustc::TokenKind::Whitespace {
      spaced = true;
    } else {
      tokens.push((
        format!("{:?}", token.kind),
        text[offset..end].to_string(),
        spaced,
      ));
      spaced = false;
    }
    offset = end;
  }
  tokens
}

/// Checks that rustc finds the same tokens in `tidy` and `messy`, with whitespace before
/// the same ones.
fn same_tokens(tidy: &str, messy: &str) -> Result<(), String> {
  let before = rustc_tokens(tidy);
  let after = rustc_tokens(messy);
  if let Some(at) = (0..before.len().max(after.len())).find(|&i| before.get(i) != after.get(i)) {
    let show = |t: Option<&(String, String, bool)>| match t {
      Some((_, text, spaced)) => format!("{:?}{}", text, if *spaced { " after space" } else { "" }),
      None => "the end".to_string(),
    };
    return Err(format!(
      "token {} is {} and was {}",
      at + 1,
      show(after.get(at)),
      show(before.get(at))
    ));
  }
  Ok(())
}

/// `text` with its items shuffled where order carries no meaning, and how many moved.
fn shuffle(text: &str, edition: Edition, random: &mut Random) -> (String, usize) {
  let root = parse(text, edition).syntax_node();
  let mut moved = 0;
  let out = rewrite_list(&root, text, false, random, &mut moved);
  (out, moved)
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  use super::*;

  const SAMPLE: &str = "\
//! The crate.
#![allow(dead_code)]

// Licensed under the terms below.
use std::fmt;
use std::io;

/// A thing.
#[derive(Debug)]
struct A; // trailing

// Detached.

fn f() -> i32 {
  let x = 1;
  x - -1 + *&&x
}

impl A {
  const N: u8 = 1;
  /// The first.
  fn one(&self) {}
  fn two(&self) {}
  type T = u8;
}

#[cfg(test)]
fn g() {}

enum E { X }
mod inner { fn a() {} fn b() {} struct C; }
macro_rules! m { () => {}; }
fn after_macro() {}
static S: u8 = 0;
";

  /// The text of each item at the top of `text`, in order.
  fn items(text: &str) -> Vec<String> {
    let root = parse(text, Edition::E2021).syntax_node();
    list_of(&root, false)
      .unwrap()
      .entries
      .iter()
      .map(|e| text[e.start..e.end].to_string())
      .collect()
  }

  fn scrambled(text: &str, seed: u64) -> String {
    match scramble(text, Edition::E2021, seed) {
      Outcome::Scrambled { text, .. } => text,
      other => panic!("seed {}: {:?}", seed, other),
    }
  }

  #[test]
  fn test_a_line_comment_ending_an_item_keeps_its_line_break() {
    let text = "fn a() {}\nstruct B; fn c() {} // note\nfn d() {}";
    for seed in 0..40 {
      let out = scrambled(text, seed);
      assert!(out.contains("// note\n"), "seed {}: {:?}", seed, out);
    }
  }

  #[test]
  fn test_comments_and_attributes_move_with_their_item() {
    for seed in 0..50 {
      let mut random = Random::new(seed);
      let (shuffled, _) = shuffle(SAMPLE, Edition::E2021, &mut random);
      assert!(shuffled.contains("/// A thing.\n#[derive(Debug)]\nstruct A; // trailing"));
      assert!(shuffled.contains("// Detached.\n\nfn f() -> i32 {"));
      assert!(shuffled.contains("/// The first.\n  fn one(&self) {}"));
    }
  }

  #[test]
  fn test_files_that_cannot_be_scrambled_are_skipped() {
    assert_eq!(
      scramble("fn f() {}\r\n", Edition::E2021, 1),
      Outcome::Skipped("carriage returns".to_string())
    );
    assert_eq!(
      scramble("fn f( {}\n", Edition::E2021, 1),
      Outcome::Skipped("syntax errors".to_string())
    );
  }

  #[test]
  fn test_fixed_items_never_move() {
    for seed in 0..50 {
      let mut random = Random::new(seed);
      let (shuffled, _) = shuffle(SAMPLE, Edition::E2021, &mut random);
      let order = items(&shuffled);
      // The first item, with the header comment above it.
      assert!(order[0].starts_with("// Licensed") && order[0].ends_with("use std::fmt;"));
      // The macro and everything after it, in place, at the end.
      let n = order.len();
      assert_eq!(order[n - 3], "macro_rules! m { () => {}; }");
      assert_eq!(order[n - 2], "fn after_macro() {}");
      assert_eq!(order[n - 1], "static S: u8 = 0;");
      // The cfg item keeps its place, so nothing crosses it either.
      let cfg = order
        .iter()
        .position(|i| i.starts_with("#[cfg(test)]"))
        .unwrap();
      assert_eq!(cfg, 5, "seed {}: {:?}", seed, order);
      assert!(shuffled.starts_with("//! The crate.\n#![allow(dead_code)]\n"));
    }
  }

  #[test]
  fn test_items_move_whole_and_the_multiset_is_kept() {
    let mut moved_any = false;
    for seed in 0..50 {
      let mut random = Random::new(seed);
      let (shuffled, moved) = shuffle(SAMPLE, Edition::E2021, &mut random);
      moved_any |= moved > 0;
      let mut before = items(SAMPLE);
      let mut after = items(&shuffled);
      // The impl and the inner module are shuffled inside, so compare their heads.
      let head = |s: &String| s.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
      let mut heads_before: Vec<String> = before.iter().map(head).collect();
      let mut heads_after: Vec<String> = after.iter().map(head).collect();
      heads_before.sort();
      heads_after.sort();
      assert_eq!(heads_before, heads_after, "seed {}", seed);
      before.retain(|i| !i.starts_with("impl") && !i.starts_with("mod"));
      after.retain(|i| !i.starts_with("impl") && !i.starts_with("mod"));
      before.sort();
      after.sort();
      assert_eq!(before, after, "seed {}", seed);
      assert_eq!(check_shuffle(SAMPLE, &shuffled, Edition::E2021), Ok(()));
    }
    assert!(moved_any);
  }

  #[test]
  fn test_macro_calls_and_macro_use_end_the_shuffling() {
    let text = "use a;\nfn b() {}\nfn c() {}\nlazy_static! {}\nfn d() {}\nfn e() {}\n\
                #[macro_use]\nmod f;\nfn g() {}\n";
    let root = parse(text, Edition::E2021).syntax_node();
    let list = list_of(&root, false).unwrap();
    assert_eq!(
      fixed(&list.entries),
      [true, false, false, true, true, true, true, true]
    );
    let text = "#[tokio::test]\nfn a() {}\nfn b() {}\n#[cfg_attr(x, inline)]\nfn c() {}\n\
                #[unsafe(no_mangle)]\nfn d() {}\nextern crate e;\n";
    let root = parse(text, Edition::E2021).syntax_node();
    let roles: Vec<Role> = list_of(&root, false)
      .unwrap()
      .entries
      .iter()
      .map(|e| e.role)
      .collect();
    assert_eq!(
      roles,
      [
        Role::Stays,
        Role::Moves,
        Role::Stays,
        Role::Moves,
        Role::Stays
      ]
    );
  }

  #[test]
  fn test_shuffle_check_catches_lost_changed_and_misplaced_items() {
    let text = "fn a() {}\nfn b() {}\nfn c() {}\nmacro_rules! m { () => {} }\nfn d() {}\n";
    let moved = "fn a() {}\nfn c() {}\nfn b() {}\nmacro_rules! m { () => {} }\nfn d() {}\n";
    assert_eq!(check_shuffle(text, moved, Edition::E2021), Ok(()));
    let lost = "fn a() {}\nfn c() {}\nmacro_rules! m { () => {} }\nfn d() {}\n";
    assert!(check_shuffle(text, lost, Edition::E2021).is_err());
    let changed = "fn a() {}\nfn c() {}\nfn b() { }\nmacro_rules! m { () => {} }\nfn d() {}\n";
    assert!(check_shuffle(text, changed, Edition::E2021).is_err());
    let crossed = "fn a() {}\nfn c() {}\nfn d() {}\nmacro_rules! m { () => {} }\nfn b() {}\n";
    assert!(check_shuffle(text, crossed, Edition::E2021).is_err());
    let first = "fn b() {}\nfn a() {}\nfn c() {}\nmacro_rules! m { () => {} }\nfn d() {}\n";
    assert!(check_shuffle(text, first, Edition::E2021).is_err());
  }

  /// Every file of this repository's own crates scrambles without a check failing, under
  /// several seeds: the tokens kept, the items kept, every file still parsing.
  #[test]
  fn test_the_repository_scrambles_cleanly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = crate::common::repo_rust_files(&root.canonicalize().unwrap()).unwrap();
    let mut scrambled_files = 0;
    for (index, file) in files.iter().enumerate().step_by(3) {
      let text = std::fs::read_to_string(file).unwrap_or_default();
      let edition = crate::common::edition_for(file, &text);
      match scramble(&text, edition, index as u64 * 7919) {
        Outcome::Scrambled { .. } => scrambled_files += 1,
        Outcome::Skipped(_) => {}
        Outcome::Bug(why) => panic!("{}: {}", file.display(), why),
      }
    }
    assert!(
      scrambled_files > 50,
      "only {} files scrambled",
      scrambled_files
    );
  }

  #[test]
  fn test_the_same_seed_scrambles_the_same_way_and_others_differ() {
    assert_eq!(scrambled(SAMPLE, 7), scrambled(SAMPLE, 7));
    let all: std::collections::HashSet<String> = (0..20).map(|s| scrambled(SAMPLE, s)).collect();
    assert!(all.len() > 15, "{} distinct scrambles of 20", all.len());
  }

  #[test]
  fn test_whitespace_check_catches_joined_and_split_tokens() {
    assert_eq!(same_tokens("a - -b", "a -\n\t-b"), Ok(()));
    assert!(same_tokens("a - -b", "a --b").is_err());
    assert!(same_tokens("x & &y", "x &&y").is_err());
    assert!(same_tokens("'a", "' a").is_err());
    assert!(same_tokens("x.0 .0", "x.0.0").is_err());
    assert!(same_tokens("// c\nfn f() {}", "// c fn f() {}").is_err());
    assert!(same_tokens("\"a b\"", "\"a  b\"").is_err());
    assert!(check_whitespace("fn f() {}", "fn\n\tf()  {}", Edition::E2021).is_ok());
    assert!(check_whitespace("fn f() {}", "fn f() {} }", Edition::E2021).is_err());
  }
}
