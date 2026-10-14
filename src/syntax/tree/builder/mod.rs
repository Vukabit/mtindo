//! Builds a green tree from parser events, putting every trivia token back in place and
//! deciding which node owns each comment.

use crate::syntax::lexer::Token;
use crate::syntax::parser::Parsed;
use crate::syntax::parser::event::Event;
use crate::syntax::tree::attach;
use crate::syntax::tree::green::{ArenaBuilder, GreenNode, Mark};
use crate::syntax::tree::red::TextRange;
use crate::syntax::{MAX_DEPTH, SyntaxKind, TOO_DEEP};

/// Replays events over the token stream.
struct Sink<'a> {
  text: &'a str,
  tokens: &'a [Token],
  starts: Vec<usize>,
  pos: usize,
  builder: TreeBuilder,
  errors: Vec<SyntaxError>,
  /// [`Self::pending_trivia`]'s buffer, kept between calls: it runs before every node.
  trivia: Vec<(SyntaxKind, &'a str)>,
  /// Nodes not opened because the tree was already one deeper than [`MAX_DEPTH`]: their
  /// tokens go to the deepest node, so that no tree is deeper than that to walk or drop.
  flattened: usize,
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
      trivia: Vec::new(),
      flattened: 0,
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
    if self.flattened > 0 {
      self.flattened -= 1;
      return;
    }
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
      self.pending_trivia();
      let attached = attach::trailing(&self.trivia);
      self.emit_raw(attached);
    }
    self.builder.finish_node();
  }

  /// Fills `self.trivia` with the trivia tokens from the current position up to the next
  /// significant token.
  fn pending_trivia(&mut self) {
    self.trivia.clear();
    let mut idx = self.pos;
    while idx < self.tokens.len() && self.tokens[idx].kind.is_trivia() {
      self.trivia.push((
        self.tokens[idx].kind,
        &self.text[self.starts[idx]..self.starts[idx + 1]],
      ));
      idx += 1;
    }
  }

  /// How many trivia tokens there are from the current position up to the next significant
  /// token.
  fn pending_trivia_len(&self) -> usize {
    self.tokens[self.pos..]
      .iter()
      .take_while(|token| token.kind.is_trivia())
      .count()
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
    if self.builder.depth() > MAX_DEPTH {
      self.flattened += 1;
      return;
    }
    if self.builder.depth() == 0 {
      self.builder.start_node(kind);
      return;
    }
    if !kind.is_attachable() {
      let trivia = self.pending_trivia_len();
      self.emit_raw(trivia);
      self.builder.start_node(kind);
      return;
    }
    self.pending_trivia();
    // Code just before the trivia (not a line break or the byte-order mark) means the trivia
    // starts on that code's line.
    let after_code = self.pos > 0
      && !matches!(
        self.tokens[self.pos - 1].kind,
        SyntaxKind::Whitespace | SyntaxKind::Bom
      );
    let attached = attach::leading(&self.trivia, after_code);
    self.emit_raw(self.trivia.len() - attached);
    self.builder.start_node(kind);
  }

  /// Adds any pending trivia, then `n_raw_tokens` significant tokens joined into one token.
  fn token(&mut self, kind: SyntaxKind, n_raw_tokens: u8) {
    let trivia = self.pending_trivia_len();
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

/// Assembles green nodes bottom-up, into one arena for the whole tree.
#[derive(Default)]
pub struct TreeBuilder {
  /// Each open node: its kind and where its children start in the arena's open stack.
  parents: Vec<(SyntaxKind, Mark)>,
  arena: ArenaBuilder,
  /// The most nodes open at once: how deep the tree nests.
  deepest: usize,
}

impl TreeBuilder {
  /// The kind of the innermost open node.
  pub fn current_kind(&self) -> Option<SyntaxKind> {
    self.parents.last().map(|(kind, _)| *kind)
  }

  pub fn depth(&self) -> usize {
    self.parents.len()
  }

  /// Closes the root node and returns the tree, over `text`, which the tokens spell.
  pub fn finish(mut self, text: &str) -> GreenNode {
    assert_eq!(
      self.parents.len(),
      1,
      "unbalanced nodes: {} open",
      self.parents.len()
    );
    let (kind, first) = self.parents.pop().unwrap();
    self.arena.node(kind, first);
    self.arena.finish(text)
  }

  pub fn finish_node(&mut self) {
    assert!(self.parents.len() > 1, "finish_node would close the root");
    let (kind, first) = self.parents.pop().unwrap();
    self.arena.node(kind, first);
  }

  pub fn start_node(&mut self, kind: SyntaxKind) {
    self.parents.push((kind, self.arena.mark()));
    self.deepest = self.deepest.max(self.parents.len());
  }

  pub fn token(&mut self, kind: SyntaxKind, text: &str) {
    assert!(!self.parents.is_empty(), "token outside any node");
    self.arena.token(kind, text.len() as u32);
  }
}

/// Builds the tree for `text` from its tokens and the parser's events, with how deep it
/// nests. A tree deeper than [`MAX_DEPTH`] gets an error, [`TOO_DEEP`], as the stages after
/// the parser walk the tree recursively; it is built one level deeper and no more, still
/// holding every token.
pub fn build_tree(
  text: &str,
  tokens: &[Token],
  parsed: Parsed,
) -> (GreenNode, Vec<SyntaxError>, usize) {
  let Parsed {
    mut events,
    mut messages,
  } = parsed;
  let mut sink = Sink::new(text, tokens);
  // The kinds a start event opens, outermost last; reused, as nearly every event is a start.
  let mut kinds = Vec::new();
  for idx in 0..events.len() {
    match std::mem::replace(&mut events[idx], Event::Tombstone) {
      Event::Start {
        kind,
        forward_parent,
      } => {
        kinds.clear();
        kinds.push(kind);
        let mut parent_idx = idx;
        let mut next = forward_parent;
        while let Some(distance) = next {
          parent_idx += distance.get() as usize;
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
        for kind in kinds.drain(..).rev() {
          sink.start(kind);
        }
      }
      Event::Token { kind, n_raw_tokens } => sink.token(kind, n_raw_tokens),
      Event::Finish => sink.finish(),
      Event::Error { message } => sink.error(std::mem::take(&mut messages[message as usize])),
      Event::Tombstone => {}
    }
  }
  let depth = sink.builder.deepest;
  if depth > MAX_DEPTH && !sink.errors.iter().any(|e| e.message == TOO_DEEP) {
    sink.errors.push(SyntaxError {
      message: TOO_DEEP.to_string(),
      range: TextRange::new(0, 0),
    });
  }
  (sink.builder.finish(text), sink.errors, depth)
}

#[cfg(test)]
mod tests;
