//! The formatting settings the formatter reads, resolved from the settings table.

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
    /// Whether match arms are indented one level past their `match`.
    pub match_arm_indent: bool,
    /// Whether files marked `@generated` are formatted.
    pub format_generated_files: bool,
    /// How many lines from the top are searched for the `@generated` mark.
    pub generated_marker_line_search_limit: usize,
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
            match_arm_indent: true,
            format_generated_files: true,
            generated_marker_line_search_limit: 5,
        }
    }
}
