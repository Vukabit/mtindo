//! The parser's output: a flat list of events that the tree builder turns into a tree.
//! Keeping the parser free of tree code lets it reorder nodes cheaply (see `forward_parent`).

use std::num::NonZeroU32;

use crate::syntax::SyntaxKind;

/// Small and `Copy` (8 bytes): a file makes about three per token, written by the parser
/// and read back by the tree builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
  /// Opens a node. If `forward_parent` is set, the event that many positions later is a
  /// `Start` for this node's parent: the parser decided to wrap this node after parsing it.
  Start {
    kind: SyntaxKind,
    forward_parent: Option<NonZeroU32>,
  },
  /// Consumes the next `n_raw_tokens` significant tokens as one tree token of `kind`,
  /// such as two `:` tokens forming `::`.
  Token { kind: SyntaxKind, n_raw_tokens: u8 },
  /// Closes the innermost open node.
  Finish,
  /// Reports a syntax error at the current position: the message is this one of the
  /// parse's messages (see `parser::Parsed`).
  Error { message: u32 },
  /// A `Start` that the parser abandoned; ignored.
  Tombstone,
}
