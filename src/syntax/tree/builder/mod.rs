//! Builds a green tree from parser events, putting every trivia token back in place and
//! deciding which node owns each comment.

use std::rc::Rc;

use crate::syntax::SyntaxKind;
use crate::syntax::lexer::Token;
use crate::syntax::parser::event::Event;
use crate::syntax::tree::attach;
use crate::syntax::tree::green::{GreenElement, GreenNode, GreenToken};
use crate::syntax::tree::red::TextRange;

/// Replays events over the token stream.
struct Sink<'a> {
    text: &'a str,
    tokens: &'a [Token],
    starts: Vec<usize>,
    pos: usize,
    builder: TreeBuilder,
    errors: Vec<SyntaxError>,
}

/// A problem found while lexing or parsing, with where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    pub range: TextRange,
}

/// Assembles green nodes bottom-up.
#[derive(Default)]
pub struct TreeBuilder {
    parents: Vec<(SyntaxKind, Vec<GreenElement>)>,
}

impl TreeBuilder {
    /// The kind of the innermost open node.
    pub fn current_kind(&self) -> Option<SyntaxKind> {
        self.parents.last().map(|(kind, _)| *kind)
    }

    pub fn depth(&self) -> usize {
        self.parents.len()
    }

    /// Closes the root node and returns the tree.
    pub fn finish(mut self) -> GreenNode {
        assert_eq!(
            self.parents.len(),
            1,
            "unbalanced nodes: {} open",
            self.parents.len()
        );
        let (kind, children) = self.parents.pop().unwrap();
        GreenNode::new(kind, children)
    }

    pub fn finish_node(&mut self) {
        assert!(self.parents.len() > 1, "finish_node would close the root");
        let (kind, children) = self.parents.pop().unwrap();
        let node = GreenNode::new(kind, children);
        self.parents
            .last_mut()
            .unwrap()
            .1
            .push(GreenElement::Node(Rc::new(node)));
    }

    pub fn start_node(&mut self, kind: SyntaxKind) {
        self.parents.push((kind, Vec::new()));
    }

    pub fn token(&mut self, kind: SyntaxKind, text: &str) {
        let token = GreenToken::new(kind, text);
        self.parents
            .last_mut()
            .expect("token outside any node")
            .1
            .push(GreenElement::Token(Rc::new(token)));
    }
}

impl<'a> Sink<'a> {
    fn new(text: &'a str, tokens: &'a [Token]) -> Self {
        let mut starts = Vec::with_capacity(tokens.len() + 1);
        let mut offset = 0;
        for token in tokens.iter() {
            starts.push(offset);
            offset += token.len as usize;
        }
        starts.push(offset);
        Sink {
            text,
            tokens,
            starts,
            pos: 0,
            builder: TreeBuilder::default(),
            errors: Vec::new(),
        }
    }

    fn error(&mut self, message: String) {
        let offset = self.starts[self.pos] as u32;
        self.errors.push(SyntaxError {
            message,
            range: TextRange::new(offset, offset),
        });
    }

    /// Adds the next `count` raw tokens to the current node, one tree token each.
    fn emit_raw(&mut self, count: usize) {
        for _ in 0..count {
            let token = self.tokens[self.pos];
            self.record_lex_error(self.pos);
            let text = &self.text[self.starts[self.pos]..self.starts[self.pos + 1]];
            self.builder.token(token.kind, text);
            self.pos += 1;
        }
    }

    fn finish(&mut self) {
        if self.builder.depth() == 1 {
            let remaining = self.tokens.len() - self.pos;
            self.emit_raw(remaining);
            return;
        }
        if self
            .builder
            .current_kind()
            .is_some_and(SyntaxKind::is_attachable)
        {
            let trivia = self.pending_trivia();
            let attached = attach::trailing(&trivia);
            self.emit_raw(attached);
        }
        self.builder.finish_node();
    }

    /// The trivia tokens from the current position up to the next significant token.
    fn pending_trivia(&self) -> Vec<(SyntaxKind, &'a str)> {
        let mut trivia = Vec::new();
        let mut idx = self.pos;
        while idx < self.tokens.len() && self.tokens[idx].kind.is_trivia() {
            trivia.push((
                self.tokens[idx].kind,
                &self.text[self.starts[idx]..self.starts[idx + 1]],
            ));
            idx += 1;
        }
        trivia
    }

    fn record_lex_error(&mut self, idx: usize) {
        if let Some(error) = self.tokens[idx].error {
            self.errors.push(SyntaxError {
                message: error.message().to_string(),
                range: TextRange::new(self.starts[idx] as u32, self.starts[idx + 1] as u32),
            });
        }
    }

    fn start(&mut self, kind: SyntaxKind) {
        if self.builder.depth() == 0 {
            self.builder.start_node(kind);
            return;
        }
        let trivia = self.pending_trivia();
        let attached = if kind.is_attachable() {
            attach::leading(&trivia)
        } else {
            0
        };
        self.emit_raw(trivia.len() - attached);
        self.builder.start_node(kind);
    }

    /// Adds any pending trivia, then `n_raw_tokens` significant tokens joined into one token.
    fn token(&mut self, kind: SyntaxKind, n_raw_tokens: u8) {
        let trivia = self.pending_trivia().len();
        self.emit_raw(trivia);
        let start = self.starts[self.pos];
        let mut end = start;
        for _ in 0..n_raw_tokens {
            self.record_lex_error(self.pos);
            end = self.starts[self.pos + 1];
            self.pos += 1;
        }
        self.builder.token(kind, &self.text[start..end]);
    }
}

/// Builds the tree for `text` from its tokens and the parser's events.
pub fn build_tree(
    text: &str,
    tokens: &[Token],
    mut events: Vec<Event>,
) -> (GreenNode, Vec<SyntaxError>) {
    let mut sink = Sink::new(text, tokens);
    for idx in 0..events.len() {
        match std::mem::replace(&mut events[idx], Event::Tombstone) {
            Event::Start {
                kind,
                forward_parent,
            } => {
                let mut kinds = vec![kind];
                let mut parent_idx = idx;
                let mut next = forward_parent;
                while let Some(distance) = next {
                    parent_idx += distance as usize;
                    match std::mem::replace(&mut events[parent_idx], Event::Tombstone) {
                        Event::Start {
                            kind,
                            forward_parent,
                        } => {
                            kinds.push(kind);
                            next = forward_parent;
                        }
                        other => unreachable!("forward parent points at {:?}", other),
                    }
                }
                for kind in kinds.into_iter().rev() {
                    sink.start(kind);
                }
            }
            Event::Token { kind, n_raw_tokens } => sink.token(kind, n_raw_tokens),
            Event::Finish => sink.finish(),
            Event::Error { message } => sink.error(message),
            Event::Tombstone => {}
        }
    }
    (sink.builder.finish(), sink.errors)
}

#[cfg(test)]
mod tests;
