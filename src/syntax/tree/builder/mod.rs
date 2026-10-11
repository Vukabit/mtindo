//! Builds a green tree from parser events, putting every trivia token back in place and
//! deciding which node owns each comment.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use crate::syntax::SyntaxKind;
use crate::syntax::lexer::Token;
use crate::syntax::parser::event::Event;
use crate::syntax::tree::attach;
use crate::syntax::tree::green::{GreenElement, GreenNode, GreenToken};
use crate::syntax::tree::red::TextRange;

/// Tokens up to this many bytes are shared through the builder's cache.
const CACHED_TOKEN_LEN: usize = 16;

/// FNV-1a: a small, fast hash for short keys. The token cache lives for one file and its
/// keys come from that file, so a hash built to resist attackers is not needed here.
#[derive(Default)]
struct Fnv(u64);

impl Hasher for Fnv {
  fn finish(&self) -> u64 {
    self.0
  }

  fn write(&mut self, bytes: &[u8]) {
    let mut hash = if self.0 == 0 {
      0xcbf2_9ce4_8422_2325
    } else {
      self.0
    };
    for byte in bytes {
      hash ^= u64::from(*byte);
      hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    self.0 = hash;
  }
}

/// Replays events over the token stream.
struct Sink<'a> {
  text: &'a str,
  tokens: &'a [Token],
  starts: Vec<usize>,
  pos: usize,
  builder: TreeBuilder<'a>,
  errors: Vec<SyntaxError>,
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

  /// Records a parser error at the start of the next significant token.
  fn error(&mut self, message: String) {
    let mut idx = self.pos;
    while idx < self.tokens.len() && self.tokens[idx].kind.is_trivia() {
      idx += 1;
    }
    let offset = self.starts[idx] as u32;
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
      // Code just before the trivia (not a line break or the byte-order mark) means the
      // trivia starts on that code's line.
      let after_code = self.pos > 0
        && !matches!(
          self.tokens[self.pos - 1].kind,
          SyntaxKind::Whitespace | SyntaxKind::Bom
        );
      attach::leading(&trivia, after_code)
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

/// A problem found while lexing or parsing, with where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
  pub message: String,
  pub range: TextRange,
}

/// Assembles green nodes bottom-up.
///
/// Allocation dominates building, so: every open node's children sit on one shared stack
/// and a finished node takes its own with a single, exactly sized allocation; and short
/// tokens with the same kind and text (`(`, a space, a newline and indentation, `fn`) share
/// one allocation, which is safe because green tokens never change.
#[derive(Default)]
pub struct TreeBuilder<'a> {
  /// Each open node: its kind and where its children start on `children`.
  parents: Vec<(SyntaxKind, usize)>,
  children: Vec<GreenElement>,
  tokens: HashMap<(SyntaxKind, &'a str), Rc<GreenToken>, BuildHasherDefault<Fnv>>,
}

impl<'a> TreeBuilder<'a> {
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
    let (kind, first) = self.parents.pop().unwrap();
    GreenNode::new(kind, self.children.split_off(first))
  }

  pub fn finish_node(&mut self) {
    assert!(self.parents.len() > 1, "finish_node would close the root");
    let (kind, first) = self.parents.pop().unwrap();
    let children: Vec<GreenElement> = self.children.drain(first..).collect();
    let node = GreenNode::new(kind, children);
    self.children.push(GreenElement::Node(Rc::new(node)));
  }

  pub fn start_node(&mut self, kind: SyntaxKind) {
    self.parents.push((kind, self.children.len()));
  }

  pub fn token(&mut self, kind: SyntaxKind, text: &'a str) {
    assert!(!self.parents.is_empty(), "token outside any node");
    let token = if text.len() <= CACHED_TOKEN_LEN {
      self
        .tokens
        .entry((kind, text))
        .or_insert_with(|| Rc::new(GreenToken::new(kind, text)))
        .clone()
    } else {
      Rc::new(GreenToken::new(kind, text))
    };
    self.children.push(GreenElement::Token(token));
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
