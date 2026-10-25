//! The lossless syntax tree: immutable "green" nodes that hold the data, and "red"
//! [`SyntaxNode`] handles that walk them by position and parent. Printing a tree
//! reproduces its source text exactly.

mod attach;
pub mod builder;
mod dump;
pub mod green;
pub mod red;

pub use dump::debug_dump;
pub use green::{GreenElement, GreenNode};
pub use red::{SyntaxElement, SyntaxNode, SyntaxToken, TextRange};
