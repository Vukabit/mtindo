//! The formatting settings the formatter reads, resolved from the settings table.

/// Where a control expression's `{` and `else` go (`control_brace_style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ControlBraceStyle {
  /// `if a {` and `} else {`.
  #[default]
  AlwaysSameLine,
  /// `}` and `else {` on lines of their own.
  ClosingNextLine,
  /// `{` and `else` on lines of their own, as `}` is.
  AlwaysNextLine,
}

/// Whether a float literal ends in `.0` (`float_literal_trailing_zero`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FloatTrailingZero {
  /// As written.
  #[default]
  Preserve,
  /// Always: `1.0`, `1.0e3`, `1.0f32`.
  Always,
  /// Unless an exponent or a suffix follows: `1.0`, `1e3`, `1f32`.
  IfNoPostfix,
  /// Never: `1.`, `1e3`, `1f32`.
  Never,
}

/// How a function's parameters are laid out when they do not fit on its line
/// (`fn_params_layout`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FnParamsLayout {
  /// On the line when they fit there, else one per line.
  #[default]
  Tall,
  /// As many on each line as fit.
  Compressed,
  /// One per line, but a lone one.
  Vertical,
}

/// How the items of a `use` list are laid out when they do not fit on one line
/// (`imports_layout`).
/// How a hex literal's digits are written (`hex_literal_case`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HexLiteralCase {
  /// As written.
  #[default]
  Preserve,
  /// In uppercase.
  Upper,
  /// In lowercase.
  Lower,
}

/// How imports are merged or split (`imports_granularity`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImportsGranularity {
  /// As written.
  #[default]
  Preserve,
  /// One `use` per crate.
  Crate,
  /// One `use` per module.
  Module,
  /// One `use` per name imported.
  Item,
  /// One `use` for all.
  One,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImportsLayout {
  /// Items fill lines.
  #[default]
  Mixed,
  /// One line, or one item per line.
  HorizontalVertical,
  /// One item per line.
  Vertical,
  /// One line.
  Horizontal,
}

/// Whether a match arm's pattern starts with `|` (`match_arm_leading_pipes`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LeadingPipes {
  /// None.
  #[default]
  Never,
  /// Every arm's.
  Always,
  /// As written.
  Preserve,
}

/// How lines end in the output (`newline_style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewlineStyle {
  /// As the file's first line ends.
  #[default]
  Auto,
  /// The platform's: `\r\n` on Windows, `\n` elsewhere.
  Native,
  Unix,
  Windows,
}

/// The settings formatting uses, at their defaults until the configuration says otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatConfig {
  pub max_width: usize,
  pub tab_spaces: usize,
  pub hard_tabs: bool,
  pub newline_style: NewlineStyle,
  pub blank_lines_upper_bound: usize,
  pub blank_lines_lower_bound: usize,
  /// `a .. b` rather than `a..b`.
  pub spaces_around_ranges: bool,
  /// `a : T` rather than `a: T`.
  pub space_before_colon: bool,
  /// `a: T` rather than `a:T`.
  pub space_after_colon: bool,
  /// `A+B` and `T=U` in types and bounds rather than `A + B` and `T = U`.
  pub compressed_type_punctuation: bool,
  /// The standard formatter's `disable_all_formatting`: files are left as written.
  pub disable_all_formatting: bool,
  /// The widths above which an argument list, chain, struct literal or array is no longer
  /// kept on one line, and the widest item of a packed list: `use_small_heuristics` scaled
  /// to `max_width`, with any width set explicitly over it.
  pub fn_call_width: usize,
  pub chain_width: usize,
  pub struct_lit_width: usize,
  pub array_width: usize,
  pub short_array_element_width_threshold: usize,
  /// Whether match arms are indented one level past their `match`.
  pub match_arm_indent: bool,
  /// Whether files marked `@generated` are formatted.
  pub format_generated_files: bool,
  /// How many lines from the top are searched for the `@generated` mark.
  pub generated_marker_line_search_limit: usize,
  /// The texts that mark a file as generated (`generated_markers` in `mto.toml`).
  pub generated_markers: Vec<String>,
  /// `format_macro_bodies`: a `macro_rules!` rule's transcriber is formatted as code;
  /// without it, written as it is.
  pub format_macro_bodies: bool,
  /// `match_block_trailing_comma`: a match arm whose body is a block ends with a comma.
  pub match_block_trailing_comma: bool,
  /// `match_arm_leading_pipes`.
  pub leading_pipes: LeadingPipes,
  /// `use_field_init_shorthand`: `S { a: a }` written `S { a }`.
  pub field_init_shorthand: bool,
  /// `struct_lit_single_line`: a short struct literal or pattern kept on one line.
  pub struct_lit_single_line: bool,
  /// `hex_literal_case`.
  pub hex_literal_case: HexLiteralCase,
  /// `float_literal_trailing_zero`.
  pub float_trailing_zero: FloatTrailingZero,
  /// `use_try_shorthand`: `try!(e)` written `e?`.
  pub try_shorthand: bool,
  /// `condense_wildcard_suffixes`: `(a, _, _)` written `(a, ..)`.
  pub condense_wildcards: bool,
  /// `normalize_doc_attributes`: `#[doc = "a"]` written `///a`.
  pub normalize_doc_attributes: bool,
  /// `normalize_comments`: `/* a */` ending a line written `// a`, `/** a */` written `/// a`.
  pub normalize_comments: bool,
  /// `wrap_comments`: a comment's lines wider than `comment_width` broken between words.
  pub wrap_comments: bool,
  /// `comment_width`: the widest a wrapped comment's lines go.
  pub comment_width: usize,
  /// `control_brace_style`.
  pub control_brace_style: ControlBraceStyle,
  /// `fn_params_layout`.
  pub fn_params_layout: FnParamsLayout,
  /// `binop_separator = "Back"`: a binary operator ends the line it breaks after.
  pub binop_back: bool,
  /// `match_arm_blocks`: an arm's body on the next line is put in a block.
  pub match_arm_blocks: bool,
  /// `force_multiline_blocks`: an arm's or closure's body on lines is put in a block.
  pub force_multiline_blocks: bool,
  /// `overflow_delimited_expr`: a struct literal, array, `match` or bracketed macro call
  /// overflows the last of any number of arguments.
  pub overflow_delimited_expr: bool,
  /// `imports_granularity`.
  pub imports_granularity: ImportsGranularity,
  /// `empty_item_single_line`: an empty function, impl or trait body written `{}`.
  pub empty_item_single_line: bool,
  /// `fn_single_line`: a function of one simple statement on one line.
  pub fn_single_line: bool,
  /// `where_single_line`: a function's `where` of one predicate on one line.
  pub where_single_line: bool,
  /// `inline_attribute_width`: the widest `use`, `extern crate` or `mod` written on its
  /// attribute's line.
  pub inline_attribute_width: usize,
  /// `struct_field_align_threshold`: fields' types (or values) aligned in groups whose
  /// names' widths differ by at most this.
  pub struct_field_align_threshold: usize,
  /// `enum_discrim_align_threshold`: variants' `=` lined up after names at most this wide.
  pub enum_discrim_align_threshold: usize,
  pub imports_layout: ImportsLayout,
  /// `attr_fn_like_width`: the widest attribute argument list kept on one line.
  pub attr_fn_like_width: usize,
  /// The style edition in effect (2015, 2018, 2021, 2024...), for the orders that differ.
  pub style_edition: u16,
  /// Whether runs of `use` declarations are sorted.
  pub reorder_imports: bool,
  /// Whether runs of `mod name;` declarations are sorted.
  pub reorder_modules: bool,
  /// Whether `extern` alone is written `extern "C"`.
  pub force_explicit_abi: bool,
  /// Whether runs of `#[derive(..)]` attributes are merged into one.
  pub merge_derives: bool,
  /// Whether parentheses directly inside parentheses are dropped: `((a))` is `(a)`.
  pub remove_nested_parens: bool,
  /// The macros whose calls are left as written, by their path as written (`self::m`), or
  /// all of them for `*`.
  pub skip_macro_invocations: Vec<String>,
  /// For a file a crate root reaches through its modules, the macros the root's
  /// `#![rustfmt::skip::macros(..)]` names, left as written there too; not a setting.
  pub crate_skipped_macros: Vec<String>,
  /// The same for the root's `#![rustfmt::skip::attributes(..)]`.
  pub crate_skipped_attributes: Vec<String>,
  /// `brace_style = "AlwaysNextLine"`: an item's `{` on the line after its header.
  pub brace_next_line: bool,
  /// `brace_style = "PreferSameLine"`: an item's `{` after its `where` clause's last
  /// predicate.
  pub brace_prefer_same_line: bool,
  /// `struct_variant_width`: the widest struct-like variant's fields kept on one line.
  pub struct_variant_width: usize,
  /// `single_line_if_else_max_width`: the widest `if ... { .. } else { .. }` kept on one
  /// line.
  pub single_line_if_else_max_width: usize,
  /// `single_line_let_else_max_width`: the widest `let ... else { .. }` kept on one line.
  pub single_line_let_else_max_width: usize,
  /// `trailing_comma = "Never"`: no comma is added after the last field or variant.
  pub trailing_comma_never: bool,
  /// `trailing_semicolon`: a `;` after a block's last `return`, `break` or `continue`.
  pub trailing_semicolon: bool,
  /// `combine_control_expr`: an `if`, `for`, `loop` or `while` alone in a call's arguments
  /// may overflow onto the call's line.
  pub combine_control_expr: bool,
}

impl Default for FormatConfig {
  fn default() -> Self {
    FormatConfig {
      max_width: 100,
      tab_spaces: 4,
      hard_tabs: false,
      newline_style: NewlineStyle::Auto,
      blank_lines_upper_bound: 1,
      blank_lines_lower_bound: 0,
      spaces_around_ranges: false,
      space_before_colon: false,
      space_after_colon: true,
      compressed_type_punctuation: false,
      disable_all_formatting: false,
      fn_call_width: 60,
      chain_width: 60,
      struct_lit_width: 18,
      array_width: 60,
      short_array_element_width_threshold: 10,
      match_arm_indent: true,
      format_generated_files: true,
      generated_marker_line_search_limit: 5,
      generated_markers: vec!["@generated".to_string()],
      format_macro_bodies: true,
      match_block_trailing_comma: false,
      leading_pipes: LeadingPipes::Never,
      field_init_shorthand: false,
      struct_lit_single_line: true,
      hex_literal_case: HexLiteralCase::Preserve,
      float_trailing_zero: FloatTrailingZero::Preserve,
      try_shorthand: false,
      condense_wildcards: false,
      normalize_comments: false,
      wrap_comments: false,
      comment_width: 80,
      normalize_doc_attributes: false,
      control_brace_style: ControlBraceStyle::AlwaysSameLine,
      fn_params_layout: FnParamsLayout::Tall,
      binop_back: false,
      match_arm_blocks: true,
      force_multiline_blocks: false,
      overflow_delimited_expr: false,
      imports_granularity: ImportsGranularity::Preserve,
      empty_item_single_line: true,
      fn_single_line: false,
      where_single_line: false,
      inline_attribute_width: 0,
      struct_field_align_threshold: 0,
      enum_discrim_align_threshold: 0,
      imports_layout: ImportsLayout::Mixed,
      attr_fn_like_width: 70,
      style_edition: 2015,
      reorder_imports: true,
      reorder_modules: true,
      force_explicit_abi: true,
      merge_derives: true,
      remove_nested_parens: true,
      skip_macro_invocations: Vec::new(),
      crate_skipped_macros: Vec::new(),
      crate_skipped_attributes: Vec::new(),
      brace_next_line: false,
      brace_prefer_same_line: false,
      struct_variant_width: 35,
      single_line_if_else_max_width: 50,
      single_line_let_else_max_width: 50,
      trailing_comma_never: false,
      trailing_semicolon: true,
      combine_control_expr: true,
    }
  }
}
