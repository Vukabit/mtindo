//! Layer 3: the formatter. It sits beside `layout` and never imports it.
//!
//! Built on the width-budget design: [`expr`] rewrites an
//! expression into the [`shape::Shape`] left for it, trying layouts in the standard
//! formatter's order.

pub mod attrs;
pub mod expr;
pub mod file;
pub mod imports;
pub mod shape;
pub mod space;
pub mod stmt;
