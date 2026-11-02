//! Layer 0: text handling that knows nothing about Rust.

pub mod color;
#[cfg(windows)]
pub mod console;
pub mod line_index;
pub mod source;
pub mod terminal;
pub mod unicode;
