//! The space an expression is printed into, and the width limits it is laid out under.

/// The width limits of the Rust style: the line width, one indentation level, and the widths
/// above which a list, chain or literal is no longer kept on one line. [`Limits::default`]
/// is the standard formatter's at its defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
  /// `max_width`.
  pub max_width: usize,
  /// `tab_spaces`: one level of indentation.
  pub tab: usize,
  /// `fn_call_width`: the widest argument list kept on one line.
  pub fn_call: usize,
  /// `chain_width`.
  pub chain: usize,
  /// `struct_lit_width`: the widest struct literal body kept on one line.
  pub struct_lit: usize,
  /// `array_width`.
  pub array: usize,
  /// `short_array_element_width_threshold`: the widest item in a packed list.
  pub short_item: usize,
}

impl Default for Limits {
  fn default() -> Self {
    Limits {
      max_width: 100,
      tab: 4,
      fn_call: 60,
      chain: 60,
      struct_lit: 18,
      array: 60,
      short_item: 10,
    }
  }
}

/// The space an expression is printed into: its line's indentation, how far into the line it
/// starts past that, and the width left on its first line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
  pub indent: usize,
  pub offset: usize,
  pub width: usize,
  pub limits: Limits,
}

impl Shape {
  /// The shape one block level in, on a line of its own, its width up to the line width.
  pub fn block_indent(self, extra: usize) -> Shape {
    let indent = self.indent + extra;
    Shape {
      indent,
      offset: 0,
      width: self.limits.max_width.saturating_sub(indent),
      limits: self.limits,
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
    self
      .limits
      .max_width
      .saturating_sub(self.indent + self.offset + self.width)
  }
}
