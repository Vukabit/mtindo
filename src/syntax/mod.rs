//! Layer 1: Rust syntax. Built alongside the stage 0 engine; layout does not use it yet.

pub mod edition;
pub mod kind;
pub mod lexer;

pub use edition::Edition;
pub use kind::SyntaxKind;
