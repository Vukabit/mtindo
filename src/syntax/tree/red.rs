//! Navigable handles over the green tree. A [`SyntaxNode`] knows its offset in the file and
//! its parent; both are worked out while walking down from the root.

use std::fmt;
use std::rc::Rc;

use crate::syntax::SyntaxKind;
use crate::syntax::tree::green::{GreenElement, GreenNode, GreenToken};

/// A node or a token in the tree.
#[derive(Clone, PartialEq, Eq)]
pub enum SyntaxElement {
    Node(SyntaxNode),
    Token(SyntaxToken),
}

impl fmt::Debug for SyntaxElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyntaxElement::Node(node) => node.fmt(f),
            SyntaxElement::Token(token) => token.fmt(f),
        }
    }
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
}

struct NodeData {
    green: Rc<GreenNode>,
    parent: Option<SyntaxNode>,
    index: u32,
    offset: u32,
}

#[derive(Clone)]
pub struct SyntaxNode(Rc<NodeData>);

impl fmt::Debug for SyntaxNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.kind().name(), self.text_range())
    }
}

impl fmt::Display for SyntaxNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

impl Eq for SyntaxNode {}

impl PartialEq for SyntaxNode {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0.green, &other.0.green) && self.0.offset == other.0.offset
    }
}

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

    pub fn children(&self) -> Vec<SyntaxNode> {
        self.children_with_tokens()
            .into_iter()
            .filter_map(|element| match element {
                SyntaxElement::Node(node) => Some(node),
                SyntaxElement::Token(_) => None,
            })
            .collect()
    }

    pub fn children_with_tokens(&self) -> Vec<SyntaxElement> {
        let mut out = Vec::with_capacity(self.0.green.children().len());
        let mut offset = self.0.offset;
        for (index, child) in self.0.green.children().iter().enumerate() {
            out.push(self.make_child(child, index as u32, offset));
            offset += child.text_len();
        }
        out
    }

    /// This node and every node below it, in source order.
    pub fn descendants(&self) -> Vec<SyntaxNode> {
        let mut out = vec![self.clone()];
        for child in self.children() {
            out.extend(child.descendants());
        }
        out
    }

    /// Every node and token below this node (and the node itself), in source order.
    pub fn descendants_with_tokens(&self) -> Vec<SyntaxElement> {
        let mut out = vec![SyntaxElement::Node(self.clone())];
        for child in self.children_with_tokens() {
            match child {
                SyntaxElement::Node(node) => out.extend(node.descendants_with_tokens()),
                token => out.push(token),
            }
        }
        out
    }

    pub fn first_token(&self) -> Option<SyntaxToken> {
        for child in self.children_with_tokens() {
            match child {
                SyntaxElement::Token(token) => return Some(token),
                SyntaxElement::Node(node) => {
                    if let Some(token) = node.first_token() {
                        return Some(token);
                    }
                }
            }
        }
        None
    }

    pub fn green(&self) -> &Rc<GreenNode> {
        &self.0.green
    }

    pub fn kind(&self) -> SyntaxKind {
        self.0.green.kind()
    }

    pub fn last_token(&self) -> Option<SyntaxToken> {
        for child in self.children_with_tokens().into_iter().rev() {
            match child {
                SyntaxElement::Token(token) => return Some(token),
                SyntaxElement::Node(node) => {
                    if let Some(token) = node.last_token() {
                        return Some(token);
                    }
                }
            }
        }
        None
    }

    pub fn new_root(green: Rc<GreenNode>) -> Self {
        SyntaxNode(Rc::new(NodeData {
            green,
            parent: None,
            index: 0,
            offset: 0,
        }))
    }

    pub fn next_sibling_or_token(&self) -> Option<SyntaxElement> {
        let parent = self.parent()?;
        parent
            .children_with_tokens()
            .into_iter()
            .nth(self.0.index as usize + 1)
    }

    pub fn parent(&self) -> Option<SyntaxNode> {
        self.0.parent.clone()
    }

    pub fn prev_sibling_or_token(&self) -> Option<SyntaxElement> {
        let parent = self.parent()?;
        let index = (self.0.index as usize).checked_sub(1)?;
        parent.children_with_tokens().into_iter().nth(index)
    }

    /// The node's source text.
    pub fn text(&self) -> String {
        let mut out = String::with_capacity(self.0.green.text_len() as usize);
        self.0.green.write_text(&mut out);
        out
    }

    pub fn text_range(&self) -> TextRange {
        TextRange::new(self.0.offset, self.0.offset + self.0.green.text_len())
    }

    fn make_child(&self, child: &GreenElement, index: u32, offset: u32) -> SyntaxElement {
        match child {
            GreenElement::Node(green) => SyntaxElement::Node(SyntaxNode(Rc::new(NodeData {
                green: green.clone(),
                parent: Some(self.clone()),
                index,
                offset,
            }))),
            GreenElement::Token(green) => SyntaxElement::Token(SyntaxToken {
                green: green.clone(),
                parent: self.clone(),
                index,
                offset,
            }),
        }
    }
}

#[derive(Clone)]
pub struct SyntaxToken {
    green: Rc<GreenToken>,
    parent: SyntaxNode,
    index: u32,
    offset: u32,
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
        Rc::ptr_eq(&self.green, &other.green) && self.offset == other.offset
    }
}

impl SyntaxToken {
    pub fn kind(&self) -> SyntaxKind {
        self.green.kind()
    }

    pub fn next_sibling_or_token(&self) -> Option<SyntaxElement> {
        self.parent
            .children_with_tokens()
            .into_iter()
            .nth(self.index as usize + 1)
    }

    pub fn parent(&self) -> SyntaxNode {
        self.parent.clone()
    }

    pub fn prev_sibling_or_token(&self) -> Option<SyntaxElement> {
        let index = (self.index as usize).checked_sub(1)?;
        self.parent.children_with_tokens().into_iter().nth(index)
    }

    pub fn text(&self) -> &str {
        self.green.text()
    }

    pub fn text_range(&self) -> TextRange {
        TextRange::new(self.offset, self.offset + self.green.text_len())
    }
}

/// A byte range `start..end` in the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl fmt::Display for TextRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
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
