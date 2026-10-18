//! Layer 3: the formatter. It sits beside `layout` and never imports it.
//!
//! Built on the width-budget design: [`expr`] rewrites an
//! expression into the [`shape::Shape`] left for it, trying layouts in the standard
//! formatter's order.

pub mod arms;
pub mod attrs;
pub mod block;
pub mod explain;
pub mod expr;
pub mod file;
pub mod imports;
pub mod items;
pub mod lists;
pub mod macros;
pub mod reorder;
pub mod shape;
pub mod space;
pub mod stmt;
pub mod types;
