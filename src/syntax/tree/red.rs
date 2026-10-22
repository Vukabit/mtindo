//! Navigable handles over the green tree. A [`SyntaxNode`] knows its offset in the file and
//! its parent; both are worked out while walking down from the root.

use std::fmt;
use std::rc::Rc;

use crate::syntax::SyntaxKind;
use crate::syntax::tree::green::{GreenElement, GreenNode};

/// A node or a token in the tree.
#[derive(Clone, PartialEq, Eq)]
pub enum SyntaxElement {
  Node(SyntaxNode),
  Token(SyntaxToken),
}

impl SyntaxElement {
  pub fn kind(&self) -> SyntaxKind {
    match self {
      SyntaxElement::Node(node) => node.kind(),
      SyntaxElement::Token(token) => token.kind(),
    }
  }

  pub fn text_range(&self) -> TextRange {
    match self {
      SyntaxElement::Node(node) => node.text_range(),
      SyntaxElement::Token(token) => token.text_range(),
    }
  }

  /// The kinds of the siblings after this element, nearest first, read from the green tree
  /// without making handles; none for a root.
  pub fn kinds_after(&self) -> impl Iterator<Item = SyntaxKind> + '_ {
    let (siblings, at) = self.siblings();
    siblings
      .get(at + 1..)
      .unwrap_or_default()
      .iter()
      .map(|e| e.kind())
  }

  /// [`Self::kinds_after`] for the siblings before this element, nearest first.
  pub fn kinds_before(&self) -> impl Iterator<Item = SyntaxKind> + '_ {
    let (siblings, at) = self.siblings();
    siblings[..at].iter().rev().map(|e| e.kind())
  }

  /// The children of this element's parent and where it sits among them; none for a root.
  fn siblings(&self) -> (&[GreenElement], usize) {
    match self {
      SyntaxElement::Token(token) => (token.parent.0.green.children(), token.index as usize),
      SyntaxElement::Node(node) => match &node.0.parent {
        Some(parent) => (parent.0.green.children(), node.0.index as usize),
        None => (&[], 0),
      },
    }
  }
}

impl fmt::Debug for SyntaxElement {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      SyntaxElement::Node(node) => node.fmt(f),
      SyntaxElement::Token(token) => token.fmt(f),
    }
  }
}

/// A node's child nodes, all or those of one kind, made as they are reached. See
/// [`SyntaxNode::children_iter`] and [`SyntaxNode::children_of_kind`].
#[derive(Clone)]
pub struct ChildNodes {
  children: Children,
  kind: Option<SyntaxKind>,
}

impl ChildNodes {
  fn wanted(&self) -> impl Fn(&GreenElement) -> bool + use<> {
    let kind = self.kind;
    move |child| child.is_node() && kind.is_none_or(|kind| child.kind() == kind)
  }
}

impl DoubleEndedIterator for ChildNodes {
  fn next_back(&mut self) -> Option<SyntaxNode> {
    let wanted = self.wanted();
    match self.children.next_back_wanted(wanted)? {
      SyntaxElement::Node(node) => Some(node),
      SyntaxElement::Token(_) => unreachable!("a token among child nodes"),
    }
  }
}

impl Iterator for ChildNodes {
  type Item = SyntaxNode;

  fn next(&mut self) -> Option<SyntaxNode> {
    let wanted = self.wanted();
    match self.children.next_wanted(wanted)? {
      SyntaxElement::Node(node) => Some(node),
      SyntaxElement::Token(_) => unreachable!("a token among child nodes"),
    }
  }
}

/// A node's children, made as they are reached. See [`SyntaxNode::children_with_tokens_iter`].
#[derive(Clone)]
pub struct Children {
  parent: SyntaxNode,
  /// The unvisited children are `front..back`, starting at `front_offset` and ending at
  /// `back_offset`.
  front: usize,
  back: usize,
  front_offset: u32,
  back_offset: u32,
}

impl Children {
  /// The next child `wanted` takes, made; those before it are only measured.
  fn next_wanted(&mut self, wanted: impl Fn(&GreenElement) -> bool) -> Option<SyntaxElement> {
    while self.front < self.back {
      let child = self.parent.0.green.children()[self.front];
      let (index, offset) = (self.front, self.front_offset);
      self.front += 1;
      self.front_offset += child.text_len();
      if wanted(&child) {
        return Some(self.parent.make_child(child, index as u32, offset));
      }
    }
    None
  }

  /// [`Self::next_wanted`] from the back.
  fn next_back_wanted(&mut self, wanted: impl Fn(&GreenElement) -> bool) -> Option<SyntaxElement> {
    while self.front < self.back {
      self.back -= 1;
      let child = self.parent.0.green.children()[self.back];
      self.back_offset -= child.text_len();
      if wanted(&child) {
        return Some(
          self
            .parent
            .make_child(child, self.back as u32, self.back_offset),
        );
      }
    }
    None
  }
}

impl DoubleEndedIterator for Children {
  fn next_back(&mut self) -> Option<SyntaxElement> {
    self.next_back_wanted(|_| true)
  }
}

impl ExactSizeIterator for Children {}

impl Iterator for Children {
  type Item = SyntaxElement;

  fn next(&mut self) -> Option<SyntaxElement> {
    self.next_wanted(|_| true)
  }

  fn size_hint(&self) -> (usize, Option<usize>) {
    let len = self.back - self.front;
    (len, Some(len))
  }

  fn nth(&mut self, n: usize) -> Option<SyntaxElement> {
    // Skipped children are only measured, never made.
    let skip = n.min(self.back - self.front);
    for child in &self.parent.0.green.children()[self.front..self.front + skip] {
      self.front_offset += child.text_len();
    }
    self.front += skip;
    self.next()
  }
}

/// An open node of a [`Preorder`] walk.
struct Frame {
  green: GreenElement,
  /// Its handle, once something at or below it has been given.
  handle: Option<SyntaxNode>,
  /// Where it sits in its parent, and where it starts.
  index: u32,
  start: u32,
  /// The index and offset of its next child.
  next: usize,
  offset: u32,
}

struct NodeData {
  green: GreenNode,
  parent: Option<SyntaxNode>,
  index: u32,
  offset: u32,
}

/// A node and everything below it in source order, as many of them as `wanted` takes. Only
/// those, and the nodes on the way down to them, are made: a walk looking for a few kinds
/// costs no allocation for the rest. See [`SyntaxNode::descendants_with_tokens_iter`] and
/// [`SyntaxNode::descendants_where`].
pub struct Preorder<F = fn(SyntaxKind) -> bool> {
  /// The node the walk starts at, until it is given.
  root: Option<SyntaxNode>,
  /// The tree walked, for the children of each node.
  tree: GreenNode,
  /// Each open node, outermost first.
  stack: Vec<Frame>,
  /// Whether tokens are given at all; without them, only nodes are asked about.
  tokens: bool,
  wanted: F,
}

impl<F: FnMut(SyntaxKind) -> bool> Preorder<F> {
  fn new(root: SyntaxNode, tokens: bool, wanted: F) -> Self {
    Preorder {
      tree: root.0.green.clone(),
      root: Some(root),
      stack: Vec::new(),
      tokens,
      wanted,
    }
  }

  /// The handle of the open node at `depth`, made (with any of its ancestors') if need be.
  fn handle(&mut self, depth: usize) -> SyntaxNode {
    if let Some(handle) = &self.stack[depth].handle {
      return handle.clone();
    }
    let parent = self.handle(depth - 1);
    let frame = &mut self.stack[depth];
    let handle = parent.make_node(self.tree.child_node(frame.green), frame.index, frame.start);
    frame.handle = Some(handle.clone());
    handle
  }
}

impl<F: FnMut(SyntaxKind) -> bool> Iterator for Preorder<F> {
  type Item = SyntaxElement;

  fn next(&mut self) -> Option<SyntaxElement> {
    if let Some(root) = self.root.take() {
      self.stack.push(Frame {
        green: root.0.green.element(),
        handle: Some(root.clone()),
        index: root.0.index,
        start: root.0.offset,
        next: 0,
        offset: root.0.offset,
      });
      if (self.wanted)(root.kind()) {
        return Some(SyntaxElement::Node(root));
      }
    }
    loop {
      let top = self.stack.last_mut()?;
      let Some(&child) = self.tree.children_of(top.green).get(top.next) else {
        self.stack.pop();
        continue;
      };
      let (index, start) = (top.next as u32, top.offset);
      top.next += 1;
      top.offset += child.text_len();
      if !child.is_node() {
        if self.tokens && (self.wanted)(child.kind()) {
          let parent = self.handle(self.stack.len() - 1);
          return Some(SyntaxElement::Token(SyntaxToken {
            green: child,
            parent,
            index,
            offset: start,
          }));
        }
        continue;
      }
      let wanted = (self.wanted)(child.kind());
      self.stack.push(Frame {
        green: child,
        handle: None,
        index,
        start,
        next: 0,
        offset: start,
      });
      if wanted {
        return Some(SyntaxElement::Node(self.handle(self.stack.len() - 1)));
      }
    }
  }
}

#[derive(Clone)]
pub struct SyntaxNode(Rc<NodeData>);

impl SyntaxNode {
  /// Every ancestor, starting with this node itself.
  pub fn ancestors(&self) -> Vec<SyntaxNode> {
    let mut out = vec![self.clone()];
    let mut current = self.parent();
    while let Some(node) = current {
      current = node.parent();
      out.push(node);
    }
    out
  }

  /// [`Self::ancestors`] one at a time.
  pub fn ancestors_iter(&self) -> impl Iterator<Item = SyntaxNode> + use<> {
    std::iter::successors(Some(self.clone()), SyntaxNode::parent)
  }

  pub fn children(&self) -> Vec<SyntaxNode> {
    self.children_iter().collect()
  }

  /// [`Self::children`] one at a time: tokens are skipped without being made, and nothing is
  /// made past where the caller stops.
  pub fn children_iter(&self) -> ChildNodes {
    ChildNodes {
      children: self.children_with_tokens_iter(),
      kind: None,
    }
  }

  /// The child nodes of `kind`, in order; the others are skipped without being made.
  pub fn children_of_kind(&self, kind: SyntaxKind) -> ChildNodes {
    ChildNodes {
      children: self.children_with_tokens_iter(),
      kind: Some(kind),
    }
  }

  /// Each child's kind and range, and whether it is a node, read from the green tree
  /// without making handles.
  pub fn child_spans(&self) -> impl Iterator<Item = (SyntaxKind, TextRange, bool)> + '_ {
    let mut offset = self.0.offset;
    self.0.green.children().iter().map(move |child| {
      let start = offset;
      offset += child.text_len();
      (child.kind(), TextRange::new(start, offset), child.is_node())
    })
  }

  pub fn children_with_tokens(&self) -> Vec<SyntaxElement> {
    self.children_with_tokens_iter().collect()
  }

  /// [`Self::children_with_tokens`] one at a time, from either end.
  pub fn children_with_tokens_iter(&self) -> Children {
    Children {
      parent: self.clone(),
      front: 0,
      back: self.0.green.children().len(),
      front_offset: self.0.offset,
      back_offset: self.0.offset + self.0.green.text_len(),
    }
  }

  /// This node and every node below it, in source order.
  pub fn descendants(&self) -> Vec<SyntaxNode> {
    // Into one buffer: a vector per level, copied into its parent's, would copy each node
    // once for every ancestor.
    self.descendants_iter().collect()
  }

  /// [`Self::descendants`] one at a time, for walks that stop early or only look.
  pub fn descendants_iter(&self) -> impl Iterator<Item = SyntaxNode> + use<> {
    self.descendant_nodes_where(|_| true)
  }

  /// The nodes and tokens of [`Self::descendants_with_tokens`] whose kind is `wanted`, in
  /// source order. The rest are only looked at, never made.
  pub fn descendants_where<F: FnMut(SyntaxKind) -> bool>(&self, wanted: F) -> Preorder<F> {
    Preorder::new(self.clone(), true, wanted)
  }

  /// [`Self::descendants_where`] for node kinds only.
  pub fn descendant_nodes_where<F: FnMut(SyntaxKind) -> bool>(
    &self,
    wanted: F,
  ) -> impl Iterator<Item = SyntaxNode> + use<F> {
    Preorder::new(self.clone(), false, wanted).map(|element| match element {
      SyntaxElement::Node(node) => node,
      SyntaxElement::Token(_) => unreachable!("a walk without tokens gave a token"),
    })
  }

  /// Whether this node, or any node or token below it, has a kind `wanted` takes. Looks at
  /// the green tree only.
  pub fn subtree_has(&self, mut wanted: impl FnMut(SyntaxKind) -> bool) -> bool {
    fn walk(
      tree: &GreenNode,
      node: GreenElement,
      wanted: &mut impl FnMut(SyntaxKind) -> bool,
    ) -> bool {
      tree
        .children_of(node)
        .iter()
        .any(|&child| wanted(child.kind()) || child.is_node() && walk(tree, child, wanted))
    }
    let green = &self.0.green;
    wanted(self.kind()) || walk(green, green.element(), &mut wanted)
  }

  /// Every token below this node, in source order, as its kind, text and range: for reading
  /// tokens without making handles for them or their parents.
  pub fn token_texts(&self) -> TokenTexts<'_> {
    TokenTexts {
      tree: &self.0.green,
      tokens: self.0.green.tokens().iter(),
    }
  }

  /// [`Self::token_texts`] for the tokens starting at `from` or later and before `to`: the
  /// tokens are in source order, so these are one run of them, found by binary search.
  pub fn token_texts_between(&self, from: u32, to: u32) -> TokenTexts<'_> {
    let tokens = self.0.green.tokens();
    let low = tokens.partition_point(|t| t.start() < from);
    let high = low + tokens[low..].partition_point(|t| t.start() < to);
    TokenTexts {
      tree: &self.0.green,
      tokens: tokens[low..high].iter(),
    }
  }

  /// Every node and token below this node (and the node itself), in source order.
  pub fn descendants_with_tokens(&self) -> Vec<SyntaxElement> {
    self.descendants_with_tokens_iter().collect()
  }

  /// [`Self::descendants_with_tokens`] one at a time.
  pub fn descendants_with_tokens_iter(&self) -> Preorder {
    Preorder::new(self.clone(), true, |_| true)
  }

  pub fn first_token(&self) -> Option<SyntaxToken> {
    // Down the green tree first: only the nodes on the way to the token get handles.
    let mut offset = self.0.offset;
    for (index, &child) in self.0.green.children().iter().enumerate() {
      if has_token(&self.0.green, child) {
        return match self.make_child(child, index as u32, offset) {
          SyntaxElement::Token(token) => Some(token),
          SyntaxElement::Node(node) => node.first_token(),
        };
      }
      offset += child.text_len();
    }
    None
  }

  /// This very node, among every node alive (see [`GreenNode::identity`]).
  pub fn identity(&self) -> (usize, u32) {
    self.0.green.identity()
  }

  pub fn green(&self) -> &GreenNode {
    &self.0.green
  }

  pub fn kind(&self) -> SyntaxKind {
    self.0.green.kind()
  }

  pub fn last_token(&self) -> Option<SyntaxToken> {
    let mut end = self.0.offset + self.0.green.text_len();
    for (index, &child) in self.0.green.children().iter().enumerate().rev() {
      end -= child.text_len();
      if has_token(&self.0.green, child) {
        return match self.make_child(child, index as u32, end) {
          SyntaxElement::Token(token) => Some(token),
          SyntaxElement::Node(node) => node.last_token(),
        };
      }
    }
    None
  }

  pub fn new_root(green: GreenNode) -> Self {
    SyntaxNode(Rc::new(NodeData {
      green,
      parent: None,
      index: 0,
      offset: 0,
    }))
  }

  pub fn next_sibling_or_token(&self) -> Option<SyntaxElement> {
    let parent = self.parent()?;
    let end = self.0.offset + self.0.green.text_len();
    parent.child_after(self.0.index, end)
  }

  pub fn parent(&self) -> Option<SyntaxNode> {
    self.0.parent.clone()
  }

  pub fn prev_sibling_or_token(&self) -> Option<SyntaxElement> {
    self.parent()?.child_before(self.0.index, self.0.offset)
  }

  /// The node's source text.
  pub fn text(&self) -> &str {
    self.0.green.text()
  }

  pub fn text_range(&self) -> TextRange {
    TextRange::new(self.0.offset, self.0.offset + self.0.green.text_len())
  }

  /// The child after the one at `index`, which ends at `end`; only that child is made.
  fn child_after(&self, index: u32, end: u32) -> Option<SyntaxElement> {
    let child = *self.0.green.children().get(index as usize + 1)?;
    Some(self.make_child(child, index + 1, end))
  }

  /// The child before the one at `index`, which starts at `start`.
  fn child_before(&self, index: u32, start: u32) -> Option<SyntaxElement> {
    let index = index.checked_sub(1)?;
    let child = self.0.green.children()[index as usize];
    Some(self.make_child(child, index, start - child.text_len()))
  }

  fn make_child(&self, child: GreenElement, index: u32, offset: u32) -> SyntaxElement {
    if child.is_node() {
      let green = self.0.green.child_node(child);
      SyntaxElement::Node(self.make_node(green, index, offset))
    } else {
      SyntaxElement::Token(SyntaxToken {
        green: child,
        parent: self.clone(),
        index,
        offset,
      })
    }
  }

  fn make_node(&self, green: GreenNode, index: u32, offset: u32) -> SyntaxNode {
    SyntaxNode(Rc::new(NodeData {
      green,
      parent: Some(self.clone()),
      index,
      offset,
    }))
  }
}

impl fmt::Debug for SyntaxNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}@{}", self.kind().name(), self.text_range())
  }
}

impl fmt::Display for SyntaxNode {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(self.text())
  }
}

impl Eq for SyntaxNode {}

impl PartialEq for SyntaxNode {
  fn eq(&self, other: &Self) -> bool {
    self.0.green.ptr_eq(&other.0.green) && self.0.offset == other.0.offset
  }
}

#[derive(Clone)]
pub struct SyntaxToken {
  green: GreenElement,
  parent: SyntaxNode,
  index: u32,
  offset: u32,
}

impl SyntaxToken {
  pub fn kind(&self) -> SyntaxKind {
    self.green.kind()
  }

  pub fn next_sibling_or_token(&self) -> Option<SyntaxElement> {
    let end = self.offset + self.green.text_len();
    self.parent.child_after(self.index, end)
  }

  pub fn parent(&self) -> SyntaxNode {
    self.parent.clone()
  }

  pub fn prev_sibling_or_token(&self) -> Option<SyntaxElement> {
    self.parent.child_before(self.index, self.offset)
  }

  pub fn text(&self) -> &str {
    self.parent.0.green.token_text(self.green)
  }

  pub fn text_range(&self) -> TextRange {
    TextRange::new(self.offset, self.offset + self.green.text_len())
  }
}

impl fmt::Debug for SyntaxToken {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      f,
      "{}@{} {:?}",
      self.kind().name(),
      self.text_range(),
      self.text()
    )
  }
}

impl Eq for SyntaxToken {}

impl PartialEq for SyntaxToken {
  fn eq(&self, other: &Self) -> bool {
    self.parent.0.green.same_tree(&other.parent.0.green)
      && self.offset == other.offset
      && self.green == other.green
  }
}

/// A byte range `start..end` in the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextRange {
  pub start: u32,
  pub end: u32,
}

impl TextRange {
  pub fn contains(&self, offset: u32) -> bool {
    self.start <= offset && offset < self.end
  }

  pub fn is_empty(&self) -> bool {
    self.start == self.end
  }

  pub fn len(&self) -> u32 {
    self.end - self.start
  }

  pub fn new(start: u32, end: u32) -> Self {
    assert!(start <= end, "range {}..{} is reversed", start, end);
    TextRange { start, end }
  }
}

impl fmt::Display for TextRange {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}..{}", self.start, self.end)
  }
}

/// The tokens below a node as kind, text and range, straight from the green tree. See
/// [`SyntaxNode::token_texts`].
pub struct TokenTexts<'a> {
  tree: &'a GreenNode,
  tokens: std::slice::Iter<'a, GreenElement>,
}

impl<'a> Iterator for TokenTexts<'a> {
  type Item = (SyntaxKind, &'a str, TextRange);

  fn next(&mut self) -> Option<Self::Item> {
    let &token = self.tokens.next()?;
    let text = self.tree.token_text(token);
    let start = token.start();
    Some((
      token.kind(),
      text,
      TextRange {
        start,
        end: start + token.text_len(),
      },
    ))
  }
}

/// Whether `element` is a token or has one somewhere below it.
fn has_token(tree: &GreenNode, element: GreenElement) -> bool {
  !element.is_node()
    || tree
      .children_of(element)
      .iter()
      .any(|&c| has_token(tree, c))
}

#[cfg(test)]
mod tests;
