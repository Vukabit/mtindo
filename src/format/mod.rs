//! Layer 3: the formatter (stage 3). It sits beside `layout` and never imports it.
//!
//! Built on the width-budget design the F3 spike chose (S3-10): [`expr`] rewrites an
//! expression into the [`shape::Shape`] left for it, trying layouts in the standard
//! formatter's order.

pub mod expr;
pub mod shape;
