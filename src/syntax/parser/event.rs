//! The parser's output: a flat list of events that the tree builder turns into a tree.
//! Keeping the parser free of tree code lets it reorder nodes cheaply (see `forward_parent`).

use crate::syntax::SyntaxKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
  /// Opens a node. If `forward_parent` is set, the event that many positions later is a
  /// `Start` for this node's parent: the parser decided to wrap this node after parsing it.
  Start {
    kind: SyntaxKind,
    forward_parent: Option<u32>,
  },
  /// Consumes the next `n_raw_tokens` significant tokens as one tree token of `kind`,
  /// such as two `:` tokens forming `::`.
  Token { kind: SyntaxKind, n_raw_tokens: u8 },
  /// Closes the innermost open node.
  Finish,
  /// Reports a syntax error at the current position.
  Error { message: String },
  /// A `Start` that the parser abandoned; ignored.
  Tombstone,
}
