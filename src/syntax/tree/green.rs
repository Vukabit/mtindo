//! Immutable tree data: each node knows its kind, total length and children, but not its
//! position or parent, so identical subtrees can be shared.

use std::rc::Rc;

use crate::syntax::SyntaxKind;

/// A node or a token inside a [`GreenNode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GreenElement {
  Node(Rc<GreenNode>),
  Token(Rc<GreenToken>),
}

impl GreenElement {
  pub fn kind(&self) -> SyntaxKind {
    match self {
      GreenElement::Node(node) => node.kind(),
      GreenElement::Token(token) => token.kind(),
    }
  }

  pub fn text_len(&self) -> u32 {
    match self {
      GreenElement::Node(node) => node.text_len(),
      GreenElement::Token(token) => token.text_len(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GreenNode {
  kind: SyntaxKind,
  text_len: u32,
  children: Vec<GreenElement>,
}

impl GreenNode {
  pub fn new(kind: SyntaxKind, children: Vec<GreenElement>) -> Self {
    let text_len = children.iter().map(GreenElement::text_len).sum();
    GreenNode {
      kind,
      text_len,
      children,
    }
  }

  pub fn children(&self) -> &[GreenElement] {
    &self.children
  }

  pub fn kind(&self) -> SyntaxKind {
    self.kind
  }

  pub fn text_len(&self) -> u32 {
    self.text_len
  }

  /// Appends the node's source text to `out`.
  pub fn write_text(&self, out: &mut String) {
    for child in self.children.iter() {
      match child {
        GreenElement::Node(node) => node.write_text(out),
        GreenElement::Token(token) => out.push_str(token.text()),
      }
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GreenToken {
  kind: SyntaxKind,
  text: Box<str>,
}

impl GreenToken {
  pub fn new(kind: SyntaxKind, text: &str) -> Self {
    GreenToken {
      kind,
      text: text.into(),
    }
  }

  pub fn kind(&self) -> SyntaxKind {
    self.kind
  }

  pub fn text(&self) -> &str {
    &self.text
  }

  pub fn text_len(&self) -> u32 {
    self.text.len() as u32
  }
}
