//! The space an expression is printed into, and the width limits of the Rust style at the
//! default settings.

/// The widest array kept on one line (`array_width`).
pub const ARRAY_WIDTH: usize = 60;

/// The widest chain kept on one line (`chain_width`).
pub const CHAIN_WIDTH: usize = 60;

/// The widest argument list kept on one line (`fn_call_width`).
pub const FN_CALL_WIDTH: usize = 60;

/// The line width.
pub const MAX_WIDTH: usize = 100;

/// The widest item in a packed list of simple items (`short_array_element_width_threshold`).
pub const SHORT_ITEM: usize = 10;

/// The widest struct literal body kept on one line (`struct_lit_width`).
pub const STRUCT_LIT_WIDTH: usize = 18;

/// One level of indentation.
pub const TAB: usize = 4;

/// The space an expression is printed into: its line's indentation, how far into the line it
/// starts past that, and the width left on its first line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub indent: usize,
    pub offset: usize,
    pub width: usize,
}

impl Shape {
    /// The shape one block level in, on a line of its own, its width up to the line width.
    pub fn block_indent(self, extra: usize) -> Shape {
        let indent = self.indent + extra;
        Shape {
            indent,
            offset: 0,
            width: MAX_WIDTH.saturating_sub(indent),
        }
    }

    pub fn offset_left(self, n: usize) -> Option<Shape> {
        Some(Shape {
            offset: self.offset + n,
            width: self.width.checked_sub(n)?,
            ..self
        })
    }

    pub fn sub_width(self, n: usize) -> Option<Shape> {
        Some(Shape {
            width: self.width.checked_sub(n)?,
            ..self
        })
    }

    /// The width reserved after the shape on its line (a `;` after an expression).
    pub fn rhs_overhead(self) -> usize {
        MAX_WIDTH.saturating_sub(self.indent + self.offset + self.width)
    }
}
