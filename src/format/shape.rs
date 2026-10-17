//! The space an expression is printed into, and the width limits it is laid out under.

use crate::options::format::FormatConfig;

/// The width limits of the Rust style: the line width, one indentation level, and the widths
/// above which a list, chain or literal is no longer kept on one line. [`Limits::default`]
/// is the standard formatter's at its defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
  /// `single_line_if_else_max_width`.
  pub single_line_if_else: usize,
  /// `single_line_let_else_max_width`.
  pub single_line_let_else: usize,
  /// `blank_lines_lower_bound` and `blank_lines_upper_bound`: the blank lines kept between
  /// the statements of a block.
  pub blank_lines: (usize, usize),
  /// `match_arm_indent`.
  pub match_arm_indent: bool,
  /// `trailing_semicolon`.
  pub trailing_semicolon: bool,
  /// `combine_control_expr`.
  pub combine_control_expr: bool,
  /// Laying out a macro's arguments: a closure's body keeps or lacks its block as written.
  pub inside_macro: bool,
  /// Laying out a block's last expression as its statement: an expression from style
  /// edition 2024, rather than one taken out of a match arm's or closure's block.
  pub block_tail: bool,
  /// Inside a method chain overflowing the list it ends: every chain in it stays on one
  /// line, but for its last link's arguments (the standard formatter's
  /// `force_one_line_chain`).
  pub one_line_chains: bool,
}

impl Limits {
  /// The limits a statement is laid out under with these settings.
  pub fn of(config: &FormatConfig) -> Limits {
    Limits {
      max_width: config.max_width,
      tab: config.tab_spaces,
      fn_call: config.fn_call_width,
      chain: config.chain_width,
      struct_lit: config.struct_lit_width,
      array: config.array_width,
      short_item: config.short_array_element_width_threshold,
      spacing: Spacing::of(config),
      attribute: false,
      comma_as_written: false,
      style_2024: config.style_edition >= 2024,
      one_line_chains: false,
      single_line_if_else: config.single_line_if_else_max_width,
      single_line_let_else: config.single_line_let_else_max_width,
      blank_lines: (
        config.blank_lines_lower_bound,
        config.blank_lines_upper_bound,
      ),
      match_arm_indent: config.match_arm_indent,
      trailing_semicolon: config.trailing_semicolon,
      combine_control_expr: config.combine_control_expr,
      inside_macro: false,
      block_tail: false,
    }
  }

  /// The settings these limits were read from, for the statement and arm layout a block
  /// inside an expression lays its insides out with.
  pub fn config(self) -> FormatConfig {
    FormatConfig {
      max_width: self.max_width,
      tab_spaces: self.tab,
      fn_call_width: self.fn_call,
      chain_width: self.chain,
      struct_lit_width: self.struct_lit,
      array_width: self.array,
      short_array_element_width_threshold: self.short_item,
      style_edition: if self.style_2024 { 2024 } else { 2021 },
      single_line_if_else_max_width: self.single_line_if_else,
      single_line_let_else_max_width: self.single_line_let_else,
      blank_lines_lower_bound: self.blank_lines.0,
      blank_lines_upper_bound: self.blank_lines.1,
      match_arm_indent: self.match_arm_indent,
      trailing_semicolon: self.trailing_semicolon,
      combine_control_expr: self.combine_control_expr,
      ..self.spacing.config()
    }
  }
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
      one_line_chains: false,
      single_line_if_else: 50,
      single_line_let_else: 50,
      blank_lines: (0, 1),
      match_arm_indent: true,
      trailing_semicolon: true,
      combine_control_expr: true,
      inside_macro: false,
      block_tail: false,
    }
  }
}

/// The space an expression is printed into: its line's indentation, how far into the line it
/// starts past that, and the width left on its first line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shape {
  pub indent: usize,
  /// Columns past `indent` a type's later lines line up to (the standard formatter's
  /// alignment): a list nested inside drops it, bounds broken over lines keep it.
  pub align: usize,
  pub offset: usize,
  pub width: usize,
  pub limits: Limits,
}

impl Shape {
  /// The shape one block level in, on a line of its own, its width up to the line width.
  pub fn block_indent(self, extra: usize) -> Shape {
    let indent = self.indent + extra;
    Shape {
      align: 0,
      indent,
      offset: 0,
      width: self.limits.max_width.saturating_sub(indent),
      limits: self.limits,
    }
  }

  /// The shape a block's insides start from, one level in: what holds the block no longer
  /// constrains them (the standard formatter's visitor starts a context of its own).
  pub fn block_inside(self) -> Shape {
    let inner = self.block_indent(self.limits.tab);
    Shape {
      limits: Limits {
        one_line_chains: false,
        ..inner.limits
      },
      ..inner
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
/// `space_after_colon` and `type_punctuation_density`; `remove_nested_parens`, for the
/// tokens a line keeps; and the style edition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Spacing {
  pub spaces_around_ranges: bool,
  pub space_before_colon: bool,
  pub space_after_colon: bool,
  pub compressed_type_punctuation: bool,
  pub remove_nested_parens: bool,
  pub style_edition: u16,
}

impl Spacing {
  pub fn of(config: &FormatConfig) -> Spacing {
    Spacing {
      spaces_around_ranges: config.spaces_around_ranges,
      space_before_colon: config.space_before_colon,
      space_after_colon: config.space_after_colon,
      compressed_type_punctuation: config.compressed_type_punctuation,
      remove_nested_parens: config.remove_nested_parens,
      style_edition: config.style_edition,
    }
  }

  /// The defaults with these settings, for the spacing rules to read.
  pub fn config(self) -> FormatConfig {
    FormatConfig {
      spaces_around_ranges: self.spaces_around_ranges,
      space_before_colon: self.space_before_colon,
      space_after_colon: self.space_after_colon,
      compressed_type_punctuation: self.compressed_type_punctuation,
      remove_nested_parens: self.remove_nested_parens,
      style_edition: self.style_edition,
      ..FormatConfig::default()
    }
  }
}

impl Default for Spacing {
  fn default() -> Self {
    Spacing::of(&FormatConfig::default())
  }
}
