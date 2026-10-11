//! The order the standard formatter gives imports and module declarations, which layout
//! keeps (so the standard formatter leaves its output alone) and formatting applies. Ported
//! from the standard formatter's `imports.rs`, `reorder.rs` and `sort.rs` (MIT OR
//! Apache-2.0, like mtindo).
//!
//! The standard formatter sorts a run of `use` items by their use trees after normalizing
//! them: a sole braced item loses its braces (`a::{b}` is `a::b`), `a::self` is `a`, and
//! nested lists are sorted. Visibility and attributes play no part. Segments order as
//! `self`, `super`, `crate`, names, `*`, then `{...}` lists. Names compare, before style
//! edition 2024, snake_case first, then CamelCase, then UPPER_SNAKE_CASE, each by bytes;
//! from 2024 by [`version_sort`], ignoring `r#`. Module declarations compare by bytes
//! through style edition 2024 and by [`version_sort`] after it.

use std::cmp::Ordering;

use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

#[derive(Clone, Copy, Debug)]
enum Chunk<'a> {
  Underscore,
  Str(&'a str),
  /// Value, leading zeros, text.
  Number(usize, usize, &'a str),
}

impl<'a> Chunk<'a> {
  fn text(self) -> &'a str {
    match self {
      Chunk::Underscore => "_",
      Chunk::Str(s) | Chunk::Number(_, _, s) => s,
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
  /// A name and its alias (`a as b`); a leading `::` is part of the name.
  Ident(String, Option<String>),
  Slf(Option<String>),
  Super(Option<String>),
  Crate(Option<String>),
  Glob,
  List(Vec<UseTree>),
}

impl Segment {
  fn compare(&self, other: &Segment, version: bool) -> Ordering {
    use Segment::*;
    let names = |a: &str, b: &str| {
      if version {
        version_sort(a.trim_start_matches("r#"), b.trim_start_matches("r#"))
      } else {
        a.cmp(b)
      }
    };
    match (self, other) {
      (Slf(a), Slf(b)) | (Super(a), Super(b)) | (Crate(a), Crate(b)) => match (a, b) {
        (Some(a), Some(b)) => names(a, b),
        _ => a.cmp(b),
      },
      (Glob, Glob) => Ordering::Equal,
      (Ident(a, alias_a), Ident(b, alias_b)) => {
        let order = if version {
          version_sort(a.trim_start_matches("r#"), b.trim_start_matches("r#"))
        } else {
          // snake_case < CamelCase < UPPER_SNAKE_CASE.
          let key = |s: &str| {
            let upper_snake = s
              .chars()
              .all(|c| c.is_uppercase() || c == '_' || c.is_numeric());
            (upper_snake, s.starts_with(char::is_uppercase))
          };
          key(a).cmp(&key(b)).then_with(|| a.cmp(b))
        };
        if order != Ordering::Equal {
          return order;
        }
        match (alias_a, alias_b) {
          (None, Some(_)) => Ordering::Less,
          (Some(_), None) => Ordering::Greater,
          (Some(a), Some(b)) => names(a, b),
          (None, None) => Ordering::Equal,
        }
      }
      (List(a), List(b)) => {
        for (x, y) in a.iter().zip(b.iter()) {
          let order = x.compare(y, version);
          if order != Ordering::Equal {
            return order;
          }
        }
        a.len().cmp(&b.len())
      }
      (Slf(_), _) => Ordering::Less,
      (_, Slf(_)) => Ordering::Greater,
      (Super(_), _) => Ordering::Less,
      (_, Super(_)) => Ordering::Greater,
      (Crate(_), _) => Ordering::Less,
      (_, Crate(_)) => Ordering::Greater,
      (Ident(..), _) => Ordering::Less,
      (_, Ident(..)) => Ordering::Greater,
      (Glob, _) => Ordering::Less,
      (_, Glob) => Ordering::Greater,
    }
  }

  fn without_alias(&self) -> Segment {
    match self {
      Segment::Ident(name, _) => Segment::Ident(name.clone(), None),
      Segment::Slf(_) => Segment::Slf(None),
      Segment::Super(_) => Segment::Super(None),
      Segment::Crate(_) => Segment::Crate(None),
      other => other.clone(),
    }
  }
}

struct Chunks<'a> {
  rest: &'a str,
}

impl<'a> Chunks<'a> {
  fn new(text: &'a str) -> Self {
    Chunks { rest: text }
  }
}

impl<'a> Iterator for Chunks<'a> {
  type Item = Chunk<'a>;

  fn next(&mut self) -> Option<Chunk<'a>> {
    let first = self.rest.chars().next()?;
    if first == '_' {
      self.rest = &self.rest[1..];
      return Some(Chunk::Underscore);
    }
    if first.is_ascii_digit() {
      let end = self
        .rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(self.rest.len());
      let (digits, rest) = self.rest.split_at(end);
      self.rest = rest;
      let zeros = digits.chars().take_while(|c| *c == '0').count();
      // The standard formatter stops at a number too big for usize.
      let value = digits.parse::<usize>().ok()?;
      return Some(Chunk::Number(value, zeros, digits));
    }
    let end = self
      .rest
      .find(|c: char| c == '_' || c.is_ascii_digit())
      .unwrap_or(self.rest.len());
    let (text, rest) = self.rest.split_at(end);
    self.rest = rest;
    Some(Chunk::Str(text))
  }
}

/// A use tree as the standard formatter models it for sorting.
#[derive(Clone, Debug)]
pub struct UseTree {
  path: Vec<Segment>,
  /// A comment sits beside this tree inside its list's braces; the standard formatter
  /// then keeps the braces of a list holding only this tree.
  has_comment: bool,
}

impl UseTree {
  /// The normalized tree of a `USE_TREE` node. `global_paths` is whether a leading `::`
  /// names the root (the 2018 edition and later), as the standard formatter reads it;
  /// `version` is the order nested lists are sorted in, as for [`UseTree::compare`].
  pub fn from_syntax(tree: &SyntaxNode, global_paths: bool, version: bool) -> UseTree {
    read_tree(tree, global_paths, true).normalize(version)
  }

  /// The standard formatter's ordering of two trees; `version` for style edition 2024 and later.
  pub fn compare(&self, other: &UseTree, version: bool) -> Ordering {
    for (a, b) in self.path.iter().zip(other.path.iter()) {
      let ord = a.compare(b, version);
      // Compared without aliases too, so `a::b` does not sort after `a as c`.
      if ord != Ordering::Equal
        && a.without_alias().compare(&b.without_alias(), version) != Ordering::Equal
      {
        return ord;
      }
    }
    self.path.len().cmp(&other.path.len())
  }

  /// The standard formatter's `UseTree::normalize`.
  fn normalize(mut self, version: bool) -> UseTree {
    let Some(mut last) = self.path.pop() else {
      return self;
    };
    match &last {
      Segment::List(list) if list.is_empty() => {
        self.path.clear();
        return self;
      }
      // `use self;` at the top level.
      Segment::Slf(None) if self.path.is_empty() => {
        self.path.clear();
        return self;
      }
      _ => {}
    }
    // `a::self` -> `a`.
    if last == Segment::Slf(None)
      && let Some(second_last) = self.path.last()
      && !matches!(second_last, Segment::Slf(_))
    {
      return self;
    }
    // `a::self as b` -> `a as b`.
    if let Segment::Slf(Some(rename)) = &last
      && let Some(Segment::Ident(_, alias @ None)) = self.path.last_mut()
    {
      *alias = Some(rename.clone());
      return self;
    }
    // `a::{b}` -> `a::b`.
    if let Segment::List(list) = &last
      && list.len() == 1
      && list[0].path != [Segment::Slf(None)]
      && !list[0].has_comment
    {
      let only = list[0].clone();
      self.path.extend(only.path);
      return self.normalize(version);
    }
    if let Segment::List(list) = last {
      let mut list: Vec<UseTree> = list.into_iter().map(|t| t.normalize(version)).collect();
      list.sort_by(|a, b| a.compare(b, version));
      list.dedup();
      last = Segment::List(list);
    }
    self.path.push(last);
    self
  }
}

impl Eq for UseTree {}

/// Trees are equal when their paths are, as in the standard formatter.
impl PartialEq for UseTree {
  fn eq(&self, other: &UseTree) -> bool {
    self.path == other.path
  }
}

/// The version sort of the Rust style guide, as the standard formatter's `sort.rs`
/// implements it: names split into runs of digits, other text and single underscores;
/// underscores first, digit runs by value (then fewer leading zeros last), text by bytes.
pub fn version_sort(a: &str, b: &str) -> Ordering {
  let mut more_zeros = Ordering::Equal;
  let (mut left, mut right) = (Chunks::new(a), Chunks::new(b));
  loop {
    let (x, y) = match (left.next(), right.next()) {
      (None, None) => return more_zeros,
      (Some(_), None) => return Ordering::Greater,
      (None, Some(_)) => return Ordering::Less,
      (Some(x), Some(y)) => (x, y),
    };
    match (x, y) {
      (Chunk::Underscore, Chunk::Underscore) => {}
      (Chunk::Underscore, _) => return Ordering::Less,
      (_, Chunk::Underscore) => return Ordering::Greater,
      (Chunk::Number(va, za, _), Chunk::Number(vb, zb, _)) => match va.cmp(&vb) {
        Ordering::Equal => {
          if more_zeros == Ordering::Equal && za != zb {
            // More leading zeros sorts first.
            more_zeros = zb.cmp(&za);
          }
        }
        order => return order,
      },
      (x, y) => match x.text().cmp(y.text()) {
        Ordering::Equal => {}
        order => return order,
      },
    }
  }
}

fn first_token_is_colons(node: &SyntaxNode) -> bool {
  node
    .children_with_tokens()
    .iter()
    .find(|e| !e.kind().is_trivia())
    .is_some_and(|e| e.kind() == SyntaxKind::Colon2)
}

/// The segment names of a `PATH`, in order, and whether it starts with `::`.
fn path_segments(path: &SyntaxNode) -> (Vec<String>, bool) {
  let mut names = Vec::new();
  for child in path.children() {
    match child.kind() {
      SyntaxKind::PathSegment => names.push(significant_text(&child)),
      // A qualified prefix, if the parser nests one.
      SyntaxKind::Path => names.extend(path_segments(&child).0),
      _ => {}
    }
  }
  (names, first_token_is_colons(path))
}

/// The standard formatter's `UseTree::from_ast`: the path's segments, then `*`, a list, or
/// the last name with its alias.
fn read_tree(tree: &SyntaxNode, global_paths: bool, top: bool) -> UseTree {
  let mut path = Vec::new();
  let (segments, leading_colons) = match tree
    .children()
    .iter()
    .find(|c| c.kind() == SyntaxKind::Path)
  {
    Some(prefix) => path_segments(prefix),
    None => (Vec::new(), first_token_is_colons(tree)),
  };
  let modsep = top && global_paths && leading_colons;
  for (idx, name) in segments.iter().enumerate() {
    path.push(segment(name, modsep && idx == 0, None));
  }
  let list = tree
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::UseTreeList);
  let glob = tree
    .children_with_tokens()
    .iter()
    .any(|e| e.kind() == SyntaxKind::Star);
  // `::*` and `::{...}` at the root keep an empty first name, as in the standard formatter.
  if (glob || list.is_some()) && segments.is_empty() && modsep {
    path.push(Segment::Ident(String::new(), None));
  }
  if glob {
    path.push(Segment::Glob);
  } else if let Some(list) = list {
    let commented = list
      .children_with_tokens()
      .iter()
      .any(|e| matches!(e.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment));
    let items = list
      .children()
      .into_iter()
      .filter(|c| c.kind() == SyntaxKind::UseTree)
      .map(|t| UseTree {
        has_comment: commented,
        ..read_tree(&t, global_paths, false)
      })
      .collect();
    path.push(Segment::List(items));
  } else if let Some(last) = segments.last() {
    let alias = tree
      .children()
      .into_iter()
      .find(|c| c.kind() == SyntaxKind::Rename)
      .and_then(|r| rename_of(&r))
      .filter(|alias| alias != last);
    path.pop();
    path.push(segment(last, modsep && segments.len() == 1, alias));
  }
  UseTree {
    path,
    has_comment: false,
  }
}

/// The name after `as`, or `_`.
fn rename_of(rename: &SyntaxNode) -> Option<String> {
  significant_text(rename)
    .strip_prefix("as")
    .map(str::to_string)
}

fn segment(name: &str, modsep: bool, alias: Option<String>) -> Segment {
  match name {
    "self" => Segment::Slf(alias),
    "super" => Segment::Super(alias),
    "crate" => Segment::Crate(alias),
    _ if modsep => Segment::Ident(format!("::{}", name), alias),
    _ => Segment::Ident(name.to_string(), alias),
  }
}

fn significant_text(node: &SyntaxNode) -> String {
  node
    .descendants_with_tokens()
    .into_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t.text().to_string()),
      _ => None,
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::syntax::{Edition, parse};

  fn sorted(items: &[&str], version: bool) -> Vec<String> {
    let tree = |text: &str| {
      let parsed = parse(&format!("use {};", text), Edition::E2021).syntax_node();
      let item = parsed.children().into_iter().next().unwrap();
      let node = item
        .children()
        .into_iter()
        .find(|c| c.kind() == SyntaxKind::UseTree)
        .unwrap();
      UseTree::from_syntax(&node, true, version)
    };
    let mut items: Vec<&str> = items.to_vec();
    items.sort_by(|a, b| tree(a).compare(&tree(b), version));
    items.into_iter().map(str::to_string).collect()
  }

  #[test]
  fn test_normalization() {
    assert_eq!(tree("a::{b}"), tree("a::b"));
    assert_eq!(tree("a::{c, b}"), tree("a::{b, c}"));
    assert_eq!(tree("a::self"), tree("a"));
    assert_eq!(tree("a::{self as x}"), tree("a as x"));
    assert_ne!(tree("a::{self}"), tree("a"));
    assert_ne!(
      tree("a::{\n// note\nb}"),
      tree("a::b"),
      "a comment keeps the braces"
    );
  }

  #[test]
  fn test_style_edition_2015_order() {
    assert_eq!(
      sorted(
        &[
          "Zeta",
          "ZETA",
          "alpha",
          "self::x",
          "crate::y",
          "super::z",
          "a::*",
          "a::{b, c}",
          "a::b"
        ],
        false
      ),
      [
        "self::x",
        "super::z",
        "crate::y",
        "a::b",
        "a::*",
        "a::{b, c}",
        "alpha",
        "Zeta",
        "ZETA"
      ]
    );
    assert_eq!(sorted(&["a10", "a2"], false), ["a10", "a2"]);
    assert_eq!(sorted(&["a as c", "a::b"], false), ["a as c", "a::b"]);
  }

  #[test]
  fn test_style_edition_2024_order() {
    assert_eq!(
      sorted(&["a10", "a2", "B", "a"], true),
      ["B", "a", "a2", "a10"]
    );
    assert_eq!(
      sorted(&["u16", "u8", "r#u32"], true),
      ["u8", "u16", "r#u32"]
    );
  }

  #[test]
  fn test_version_sort() {
    // From the style guide's examples.
    let mut names = vec![
      "x86_128", "x86_32", "x86_64", "x86", "X86_64", "x86_64x", "x86x",
    ];
    names.sort_by(|a, b| version_sort(a, b));
    assert_eq!(
      names,
      [
        "X86_64", "x86", "x86_32", "x86_64", "x86_64x", "x86_128", "x86x"
      ]
    );
    let mut zeros = vec!["v001", "v01", "v1", "v009", "v10"];
    zeros.sort_by(|a, b| version_sort(a, b));
    assert_eq!(zeros, ["v001", "v01", "v1", "v009", "v10"]);
  }

  fn tree(text: &str) -> UseTree {
    let parsed = parse(&format!("use {};", text), Edition::E2021).syntax_node();
    let item = parsed.children().into_iter().next().unwrap();
    let node = item
      .children()
      .into_iter()
      .find(|c| c.kind() == SyntaxKind::UseTree)
      .unwrap();
    UseTree::from_syntax(&node, true, false)
  }
}
