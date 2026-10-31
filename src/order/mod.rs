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
//! through style edition 2024 and by [`version_sort`] after it. [`regroup`] merges or
//! splits a run of imports first, as `imports_granularity` says.

use std::cmp::Ordering;

pub use crate::options::format::ImportsGranularity as Granularity;
use crate::syntax::{SyntaxKind, SyntaxNode};

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

  /// The segment as the standard formatter writes it.
  fn text(&self) -> String {
    let named = |name: &str, alias: &Option<String>| match alias {
      Some(alias) => format!("{} as {}", name, alias),
      None => name.to_string(),
    };
    match self {
      Segment::Ident(name, alias) => named(name, alias),
      Segment::Slf(alias) => named("self", alias),
      Segment::Super(alias) => named("super", alias),
      Segment::Crate(alias) => named("crate", alias),
      Segment::Glob => "*".to_string(),
      Segment::List(list) => {
        let items: Vec<String> = list.iter().map(UseTree::text).collect();
        format!("{{{}}}", items.join(", "))
      }
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

impl Segment {
  /// Equal but for their aliases.
  fn equal_except_alias(&self, other: &Segment) -> bool {
    use Segment::*;
    match (self, other) {
      (Ident(a, _), Ident(b, _)) => a == b,
      (Slf(_), Slf(_)) | (Super(_), Super(_)) | (Crate(_), Crate(_)) | (Glob, Glob) => true,
      (List(a), List(b)) => a == b,
      _ => false,
    }
  }

  fn alias(&self) -> Option<String> {
    match self {
      Segment::Ident(_, a) | Segment::Slf(a) | Segment::Super(a) | Segment::Crate(a) => a.clone(),
      _ => None,
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

/// One `use` item: its attributes as written (empty for none), its visibility with a space
/// after it (empty for none), and its tree; `written`, the item as written when a comment
/// is in it, which is kept as it is.
#[derive(Clone, Debug)]
pub struct Import {
  pub attributes: String,
  pub visibility: String,
  pub tree: UseTree,
  pub written: Option<String>,
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
    read_tree(tree, global_paths, true).normalize(version, true)
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

  /// The tree on one line, as the standard formatter writes it.
  pub fn text(&self) -> String {
    let segments: Vec<String> = self.path.iter().map(Segment::text).collect();
    segments.join("::")
  }

  /// Whether the tree imports nothing (`a::{}`), which normalizing leaves without a path.
  pub fn is_empty(&self) -> bool {
    self.path.is_empty()
  }

  /// Whether the tree is a name after a root `::` that was in braces (`::{a}` normalized),
  /// which read again as `::a` would sort apart from where it was sorted.
  pub fn unbraced_root(&self) -> bool {
    matches!(
      self.path.as_slice(),
      [Segment::Ident(root, _), second, ..]
        if root.is_empty() && !matches!(second, Segment::Glob | Segment::List(_))
    )
  }

  /// The standard formatter's `UseTree::normalize`; `top` for a `use`'s own tree, not one in
  /// a list.
  fn normalize(mut self, version: bool, top: bool) -> UseTree {
    let Some(mut last) = self.path.pop() else {
      return self;
    };
    match &last {
      Segment::List(list) if list.is_empty() => {
        self.path.clear();
        return self;
      }
      // `use self;` at the top level.
      Segment::Slf(None) if self.path.is_empty() && top => {
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
      // `self as a` reads as `self` here, as the standard formatter writes it to compare.
      && !matches!(list[0].path.as_slice(), [Segment::Slf(_)])
      && !list[0].has_comment
    {
      let only = list[0].clone();
      self.path.extend(only.path);
      return self.normalize(version, top);
    }
    if let Segment::List(list) = last {
      let mut list: Vec<UseTree> = list
        .into_iter()
        .map(|t| t.normalize(version, false))
        .collect();
      list.sort_by(|a, b| a.compare(b, version));
      list.dedup();
      last = Segment::List(list);
    }
    self.path.push(last);
    self
  }
}

impl UseTree {
  fn from_path(path: Vec<Segment>) -> UseTree {
    UseTree {
      path,
      has_comment: false,
    }
  }

  /// Whether `other` shares this tree's crate, module (all but its last segment, or its
  /// first), or anything (`share_prefix`).
  fn share_prefix(&self, other: &UseTree, by: Granularity) -> bool {
    if self.path.is_empty() || other.path.is_empty() || self.has_comment {
      return false;
    }
    let module = |path: &[Segment]| path[..(path.len() - 1).max(1)].to_vec();
    match by {
      Granularity::Crate => self.path[0] == other.path[0],
      Granularity::Module => module(&self.path) == module(&other.path),
      _ => true,
    }
  }

  /// The tree split at its last list into a tree per item, recursively (`flatten`); but a
  /// list holding `self` alone.
  fn flatten(self) -> Vec<UseTree> {
    if self.path.is_empty() || self.has_comment {
      return vec![self];
    }
    match self.path.last() {
      Some(Segment::List(list)) => {
        if let [only] = list.as_slice()
          && let [Segment::Slf(_)] = only.path.as_slice()
        {
          return vec![self];
        }
        let prefix = &self.path[..self.path.len() - 1];
        let mut result = Vec::new();
        for nested in list {
          for flattened in nested.clone().flatten() {
            let mut path = prefix.to_vec();
            path.extend(flattened.path);
            result.push(UseTree::from_path(path));
          }
        }
        result
      }
      _ => vec![self],
    }
  }

  /// A tree ending in `self` ending in `{self}` instead (`nest_trailing_self`).
  fn nest_trailing_self(mut self) -> UseTree {
    if let Some(Segment::Slf(_)) = self.path.last() {
      let last = self.path.pop().unwrap_or(Segment::Glob);
      self
        .path
        .push(Segment::List(vec![UseTree::from_path(vec![last])]));
    }
    self
  }

  /// `other` merged into this tree (`merge`); `false` when they differ in an alias alone,
  /// which the standard formatter then drops (`f` and `f as g` merged are `f`): that one
  /// is not merged, so nothing is lost.
  fn merge(&mut self, other: &UseTree, by: Granularity, version: bool) -> bool {
    let mut prefix = 0;
    for (a, b) in self.path.iter().zip(other.path.iter()) {
      // An alias set aside at the root only.
      if prefix == 0 && a.equal_except_alias(b) || a == b {
        prefix += 1;
      } else {
        break;
      }
    }
    match merge_rest(&self.path, &other.path, prefix, by, version) {
      Some(path) => {
        self.path = path;
        true
      }
      None => self.path == other.path,
    }
  }
}

impl Eq for UseTree {}

/// Trees are equal when their paths are, as in the standard formatter.
impl PartialEq for UseTree {
  fn eq(&self, other: &UseTree) -> bool {
    self.path == other.path
  }
}

/// The standard formatter's `normalize_use_trees_with_granularity`: `imports` merged into
/// trees sharing a crate, a module or nothing, or split into one per name, in the order the
/// first of each comes; one with attributes is kept apart, but for `Item`, which gives each
/// part the attributes, and one with a comment is kept as it is. `version` orders lists as
/// [`UseTree::compare`] does.
pub fn regroup(imports: Vec<Import>, by: Granularity, version: bool) -> Vec<Import> {
  if by == Granularity::Preserve {
    return imports;
  }
  if by == Granularity::Item {
    let mut result: Vec<Import> = Vec::new();
    for import in imports {
      if import.written.is_some() {
        result.push(import);
        continue;
      }
      for tree in import.tree.clone().flatten() {
        let tree = tree.nest_trailing_self();
        if !result.iter().any(|r| r.tree == tree) {
          result.push(Import {
            tree,
            ..import.clone()
          });
        }
      }
    }
    return result;
  }
  let mut result: Vec<Import> = Vec::new();
  for import in imports {
    if !import.attributes.is_empty() || import.written.is_some() {
      result.push(import);
      continue;
    }
    for mut flattened in import.tree.clone().flatten() {
      let merged = result
        .iter_mut()
        .find(|r| {
          r.attributes.is_empty()
            && r.written.is_none()
            && r.visibility == import.visibility
            && r.tree.share_prefix(&flattened, by)
        })
        .is_some_and(|r| r.tree.merge(&flattened, by, version));
      if !merged {
        if by == Granularity::Module {
          flattened = flattened.nest_trailing_self();
        }
        result.push(Import {
          tree: flattened,
          ..import.clone()
        });
      }
    }
  }
  result
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
    .children_with_tokens_iter()
    .find(|e| !e.kind().is_trivia())
    .is_some_and(|e| e.kind() == SyntaxKind::Colon2)
}

/// The standard formatter's `merge_use_trees_inner`: `tree` merged into the most alike of
/// `trees`, or added to them.
fn merge_into(trees: &mut Vec<UseTree>, tree: UseTree, by: Granularity, version: bool) {
  let similar: Vec<usize> = (0..trees.len())
    .filter(|&i| trees[i].share_prefix(&tree, by))
    .collect();
  if tree.path.len() == 1 && by == Granularity::Crate {
    if let Some(&i) = similar.iter().min_by_key(|&&i| trees[i].path.len())
      && trees[i].path.len() == 1
    {
      return;
    }
  } else if by == Granularity::One {
    let alike = |i: usize| {
      trees[i]
        .path
        .iter()
        .zip(&tree.path)
        .take_while(|(a, b)| a.equal_except_alias(b))
        .count()
    };
    // The last of the most alike, as `max_by_key` takes.
    if let Some(&i) = similar.iter().max_by_key(|&&i| alike(i))
      && alike(i) > 0
      && trees[i].merge(&tree, by, version)
    {
      return;
    }
  } else if let Some(&i) = similar.iter().max_by_key(|&&i| trees[i].path.len())
    && trees[i].path.len() > 1
    && trees[i].merge(&tree, by, version)
  {
    return;
  }
  trees.push(tree);
  trees.sort_by(|x, y| x.compare(y, version));
  trees.dedup();
}

/// The standard formatter's `merge_rest`: the merged path of `a` and `b` sharing their first
/// `len` segments, or `None` when they are the same.
fn merge_rest(
  a: &[Segment],
  b: &[Segment],
  mut len: usize,
  by: Granularity,
  version: bool,
) -> Option<Vec<Segment>> {
  if a.len() == len && b.len() == len {
    return None;
  }
  if a.len() != len && b.len() != len {
    if let Segment::List(list) = &a[len] {
      let mut list = list.clone();
      merge_into(
        &mut list,
        UseTree::from_path(b[len..].to_vec()),
        by,
        version,
      );
      let mut path = b[..len].to_vec();
      path.push(Segment::List(list));
      return Some(path);
    }
  } else if len == 1 {
    let (common, rest) = if a.len() == len {
      (&a[0], &b[1..])
    } else {
      (&b[0], &a[1..])
    };
    let mut list = vec![UseTree::from_path(vec![Segment::Slf(common.alias())])];
    match rest {
      [Segment::List(items)] => list.extend(items.iter().cloned()),
      _ => list.push(UseTree::from_path(rest.to_vec())),
    }
    return Some(vec![b[0].clone(), Segment::List(list)]);
  } else {
    len -= 1;
  }
  let mut list = vec![
    UseTree::from_path(a[len..].to_vec()),
    UseTree::from_path(b[len..].to_vec()),
  ];
  list.sort_by(|x, y| x.compare(y, version));
  list.dedup();
  let mut path = b[..len].to_vec();
  path.push(Segment::List(list));
  Some(path)
}

/// The segment names of a `PATH`, in order, and whether it starts with `::`.
fn path_segments(path: &SyntaxNode) -> (Vec<String>, bool) {
  let mut names = Vec::new();
  for child in path.children_iter() {
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
  let (segments, leading_colons) = match tree.children_iter().find(|c| c.kind() == SyntaxKind::Path)
  {
    Some(prefix) => path_segments(&prefix),
    None => (Vec::new(), first_token_is_colons(tree)),
  };
  let modsep = top && global_paths && leading_colons;
  for (idx, name) in segments.iter().enumerate() {
    path.push(segment(name, modsep && idx == 0, None));
  }
  let list = tree
    .children_iter()
    .find(|c| c.kind() == SyntaxKind::UseTreeList);
  let glob = tree
    .children_with_tokens_iter()
    .any(|e| e.kind() == SyntaxKind::Star);
  // `::*` and `::{...}` at the root keep an empty first name, as in the standard formatter.
  if (glob || list.is_some()) && segments.is_empty() && modsep {
    path.push(Segment::Ident(String::new(), None));
  }
  if glob {
    path.push(Segment::Glob);
  } else if let Some(list) = list {
    let commented = list
      .children_with_tokens_iter()
      .any(|e| matches!(e.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment));
    let items = list
      .children_iter()
      .filter(|c| c.kind() == SyntaxKind::UseTree)
      .map(|t| UseTree {
        has_comment: commented,
        ..read_tree(&t, global_paths, false)
      })
      .collect();
    path.push(Segment::List(items));
  } else if let Some(last) = segments.last() {
    let alias = tree
      .children_iter()
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
    .token_texts()
    .filter(|(kind, _, _)| !kind.is_trivia())
    .map(|(_, text, _)| text)
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::syntax::{Edition, parse};

  fn sorted(items: &[&str], version: bool) -> Vec<String> {
    let tree = |text: &str| {
      let parsed = parse(&format!("use {};", text), Edition::E2021).syntax_node();
      let item = parsed.children_iter().next().unwrap();
      let node = item
        .children_iter()
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
    // `self as x` alone keeps its braces (the standard formatter writes it `self` to decide).
    assert_ne!(tree("a::{self as x}"), tree("a as x"));
    assert_eq!(tree("a::self as x"), tree("a as x"));
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
    let item = parsed.children_iter().next().unwrap();
    let node = item
      .children_iter()
      .find(|c| c.kind() == SyntaxKind::UseTree)
      .unwrap();
    UseTree::from_syntax(&node, true, false)
  }
}
