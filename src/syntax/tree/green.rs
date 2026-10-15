//! Immutable tree data: each node knows its kind, total length and children; positions and
//! parents are the red tree's.
//!
//! A parse's whole tree is one [`Arena`]: its text, and every node's children side by side
//! in one vector. Building it is appending, not allocating per node, and dropping it frees
//! a few vectors, however deep it nests. A [`GreenNode`] is the arena and one node in it,
//! cloned by a count.

use std::rc::Rc;

use crate::syntax::SyntaxKind;

/// One parse's tree.
#[derive(Debug)]
pub struct Arena {
  /// The text parsed: every token's text is a slice of it.
  text: Box<str>,
  /// Each node's children: where they start in `elements`, how many, and where the node
  /// starts in `text`; indexed by node id.
  nodes: Vec<NodeRecord>,
  elements: Vec<GreenElement>,
  /// Every token, in the order of the text: a node's are a slice of them, read without
  /// going down the tree.
  tokens: Vec<GreenElement>,
}

/// Builds an [`Arena`] bottom-up: every open node's children sit on one shared stack, and a
/// finished node's are moved to the arena side by side.
#[derive(Default)]
pub struct ArenaBuilder {
  nodes: Vec<NodeRecord>,
  elements: Vec<GreenElement>,
  /// The children of the open nodes, innermost last.
  open: Vec<GreenElement>,
  tokens: Vec<GreenElement>,
  /// Where the next token starts.
  offset: u32,
}

impl ArenaBuilder {
  /// Adds a token of `len` bytes, the next in the text.
  pub fn token(&mut self, kind: SyntaxKind, len: u32) {
    let token = GreenElement {
      kind,
      node: false,
      text_len: len,
      at: self.offset,
    };
    self.open.push(token);
    self.tokens.push(token);
    self.offset += len;
  }

  /// Where a node opened now starts.
  pub fn mark(&self) -> Mark {
    Mark {
      children: self.open.len(),
      tokens: self.tokens.len(),
    }
  }

  /// Closes a node of `kind` over the children added since `mark`.
  pub fn node(&mut self, kind: SyntaxKind, mark: Mark) {
    let children = &self.open[mark.children..];
    let text_len = children.iter().map(|c| c.text_len).sum();
    let start = children.first().map_or(self.offset, |c| self.start_of(c));
    let id = self.nodes.len() as u32;
    self.nodes.push(NodeRecord {
      first: self.elements.len() as u32,
      count: children.len() as u32,
      start,
      first_token: mark.tokens as u32,
      token_count: (self.tokens.len() - mark.tokens) as u32,
    });
    self.elements.extend(self.open.drain(mark.children..));
    self.open.push(GreenElement {
      kind,
      node: true,
      text_len,
      at: id,
    });
  }

  /// The tree, whose root is the one node left open, over `text`.
  pub fn finish(mut self, text: &str) -> GreenNode {
    assert_eq!(self.open.len(), 1, "one root");
    assert_eq!(
      self.offset as usize,
      text.len(),
      "the tokens are not the text"
    );
    let root = self.open.pop().expect("a root");
    assert!(root.node, "the root is a node");
    GreenNode {
      arena: Rc::new(Arena {
        text: text.into(),
        nodes: self.nodes,
        elements: self.elements,
        tokens: self.tokens,
      }),
      element: root,
    }
  }

  fn start_of(&self, element: &GreenElement) -> u32 {
    if element.node {
      self.nodes[element.at as usize].start
    } else {
      element.at
    }
  }
}

/// A node or a token inside a [`GreenNode`]: its kind and length, and the node's id or where
/// the token's text starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GreenElement {
  kind: SyntaxKind,
  node: bool,
  text_len: u32,
  /// The node's id, or the token's start in the text.
  at: u32,
}

impl GreenElement {
  pub fn is_node(&self) -> bool {
    self.node
  }

  pub fn kind(&self) -> SyntaxKind {
    self.kind
  }

  pub fn text_len(&self) -> u32 {
    self.text_len
  }

  /// Where a token starts in the text (a token's offset in the tree is its own).
  pub fn start(&self) -> u32 {
    debug_assert!(!self.node, "a node's start is its record's");
    self.at
  }
}

/// A node of an [`Arena`].
#[derive(Debug, Clone)]
pub struct GreenNode {
  arena: Rc<Arena>,
  element: GreenElement,
}

impl GreenNode {
  pub fn children(&self) -> &[GreenElement] {
    self.children_of(self.element)
  }

  /// The children of `node`, a node of this tree, without a handle for it.
  pub fn children_of(&self, node: GreenElement) -> &[GreenElement] {
    debug_assert!(node.node, "a token has no children");
    let record = self.arena.nodes[node.at as usize];
    &self.arena.elements[record.first as usize..(record.first + record.count) as usize]
  }

  /// The node `element`, one of this node's children (or below them).
  pub fn child_node(&self, element: GreenElement) -> GreenNode {
    debug_assert!(element.node, "a token is not a node");
    GreenNode {
      arena: self.arena.clone(),
      element,
    }
  }

  /// This node as one of its parent's children.
  pub fn element(&self) -> GreenElement {
    self.element
  }

  pub fn kind(&self) -> SyntaxKind {
    self.element.kind
  }

  /// Whether `other` is a node of the same tree.
  pub fn same_tree(&self, other: &GreenNode) -> bool {
    Rc::ptr_eq(&self.arena, &other.arena)
  }

  /// This very node, among every node alive: its tree's and its place in it, for a cache to
  /// key on while it keeps the tree alive.
  pub fn identity(&self) -> (usize, u32) {
    (Rc::as_ptr(&self.arena) as usize, self.element.at)
  }

  /// Whether `other` is this very node (a clone of it), not just an equal one.
  pub fn ptr_eq(&self, other: &GreenNode) -> bool {
    Rc::ptr_eq(&self.arena, &other.arena) && self.element.at == other.element.at
  }

  /// The node's source text.
  pub fn text(&self) -> &str {
    let start = self.arena.nodes[self.element.at as usize].start as usize;
    &self.arena.text[start..start + self.element.text_len as usize]
  }

  pub fn text_len(&self) -> u32 {
    self.element.text_len
  }

  /// Every token below this node, in order.
  pub fn tokens(&self) -> &[GreenElement] {
    let record = self.arena.nodes[self.element.at as usize];
    let first = record.first_token as usize;
    &self.arena.tokens[first..first + record.token_count as usize]
  }

  /// The text of `token`, one of the tokens below this node.
  pub fn token_text(&self, token: GreenElement) -> &str {
    debug_assert!(!token.node, "a node is not a token");
    &self.arena.text[token.at as usize..(token.at + token.text_len) as usize]
  }
}

/// Where a node opened now starts, among the children and the tokens added so far. See
/// [`ArenaBuilder::mark`].
#[derive(Clone, Copy, Debug)]
pub struct Mark {
  children: usize,
  tokens: usize,
}

#[derive(Debug, Clone, Copy)]
struct NodeRecord {
  first: u32,
  count: u32,
  start: u32,
  /// The node's tokens: where they start in `tokens`, and how many.
  first_token: u32,
  token_count: u32,
}
