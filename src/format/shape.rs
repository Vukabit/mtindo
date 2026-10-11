//! The space an expression is printed into, and the width limits it is laid out under.

use crate::options::format::FormatConfig;

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
  /// The settings tokens are spaced by.
  pub spacing: Spacing,
  /// Laying out attribute arguments: an overflowing nested list is not held to the list
  /// width, and stops a column short.
  pub attribute: bool,
  /// A list keeps a trailing comma only where written, on one line too (attribute and macro
  /// arguments).
  pub comma_as_written: bool,
  /// Style edition 2024 or later, for the choices that differ.
  pub style_2024: bool,
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
      spacing: Spacing::default(),
      attribute: false,
      comma_as_written: false,
      style_2024: false,
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

/// The settings spacing within a line reads: `spaces_around_ranges`, `space_before_colon`,
/// `space_after_colon` and `type_punctuation_density`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spacing {
  pub spaces_around_ranges: bool,
  pub space_before_colon: bool,
  pub space_after_colon: bool,
  pub compressed_type_punctuation: bool,
}

impl Spacing {
  pub fn of(config: &FormatConfig) -> Spacing {
    Spacing {
      spaces_around_ranges: config.spaces_around_ranges,
      space_before_colon: config.space_before_colon,
      space_after_colon: config.space_after_colon,
      compressed_type_punctuation: config.compressed_type_punctuation,
    }
  }

  /// The defaults with these settings, for the spacing rules to read.
  pub fn config(self) -> FormatConfig {
    FormatConfig {
      spaces_around_ranges: self.spaces_around_ranges,
      space_before_colon: self.space_before_colon,
      space_after_colon: self.space_after_colon,
      compressed_type_punctuation: self.compressed_type_punctuation,
      ..FormatConfig::default()
    }
  }
}

impl Default for Spacing {
  fn default() -> Self {
    Spacing::of(&FormatConfig::default())
  }
}
