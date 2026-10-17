use crate::format::file::format_file;
use crate::options::format::FormatConfig;
use crate::syntax::Edition;
use crate::verify::format::verify_format;

/// `source` formatted, checked for meaning and stability.
fn formatted(source: &str) -> String {
  let config = FormatConfig::default();
  let once = format_file(source, Edition::E2021, &config);
  let found = verify_format(source, &once, Edition::E2021);
  assert!(found.is_empty(), "{:?}\n{}", found, once);
  assert_eq!(
    format_file(&once, Edition::E2021, &config),
    once,
    "not stable"
  );
  once
}

#[test]
fn test_attributes_and_comments_before_are_laid_out_on_their_own() {
  assert_eq!(
    formatted(
      "// why
#[cfg( test )]
pub use
    a::{
    b,
    c,
};
"
    ),
    "// why
#[cfg(test)]
pub use a::{b, c};
"
  );
}

#[test]
fn test_imports_layout() {
  use crate::options::format::ImportsLayout;
  let long = "use lists::{definitive_tactic, itemize_list, shape_for_tactic, struct_lit_formatting, struct_lit_shape};\n";
  let short = "use a::{b, c};\n";
  let under = |layout| FormatConfig {
    imports_layout: layout,
    ..FormatConfig::default()
  };
  let vertical = "use lists::{\n    definitive_tactic,\n    itemize_list,\n    shape_for_tactic,\n    struct_lit_formatting,\n    struct_lit_shape,\n};\n";
  let horizontal_vertical = under(ImportsLayout::HorizontalVertical);
  assert_eq!(
    format_file(long, Edition::E2021, &horizontal_vertical),
    vertical
  );
  assert_eq!(
    format_file(short, Edition::E2021, &horizontal_vertical),
    short
  );
  let one_per_line = under(ImportsLayout::Vertical);
  assert_eq!(format_file(long, Edition::E2021, &one_per_line), vertical);
  assert_eq!(
    format_file(short, Edition::E2021, &one_per_line),
    "use a::{\n    b,\n    c,\n};\n"
  );
  // Too wide for its line under `Horizontal`: left as written.
  let horizontal = under(ImportsLayout::Horizontal);
  assert_eq!(format_file(long, Edition::E2021, &horizontal), long);
}

#[test]
fn test_lists_holding_lists_take_a_line_per_path() {
  assert_eq!(
    formatted("use a::{b::{c, d}, e};\n"),
    "use a::{\n    b::{c, d},\n    e,\n};\n"
  );
  assert_eq!(
    formatted("use a::{b, c::d, e, f, g::{h, i}, j, k};\n"),
    "use a::{\n    b,\n    c::d,\n    e, f,\n    g::{h, i},\n    j, k,\n};\n"
  );
}

#[test]
fn test_lists_join_onto_one_line_when_they_fit() {
  assert_eq!(
    formatted("pub(crate) use a::{b,\n c};\nuse\n  a::b\n  ;\n"),
    "use a::b;\npub(crate) use a::{b, c};\n"
  );
}

#[test]
fn test_long_lists_fill_lines() {
  assert_eq!(
    formatted(
      "use std::collections::{Arc, BTreeMap, BTreeSet, BinaryHeap, Cell, HashMap, HashSet, LinkedList, Rc, RefCell, VecDeque};\n"
    ),
    "use std::collections::{\n    Arc, BTreeMap, BTreeSet, BinaryHeap, Cell, HashMap, HashSet, LinkedList, Rc, RefCell, VecDeque,\n};\n"
  );
}

#[test]
fn test_trees_normalized() {
  // As the standard formatter normalizes them when it reorders: a list of one loses its
  // braces, `a::self` is `a`, `self as x` alone keeps its braces, `::{a}` is left.
  assert_eq!(formatted("use a::{\n    b,\n};\n"), "use a::b;\n");
  assert_eq!(
    formatted("use crate::lexer::self;\n"),
    "use crate::lexer;\n"
  );
  assert_eq!(
    formatted("use a::{l::{self, m, p::*}};\n"),
    "use a::l::{self, m, p::*};\n"
  );
  assert_eq!(
    formatted("use super::unix::{self as os};\n"),
    "use super::unix::{self as os};\n"
  );
}

#[test]
fn test_what_is_left_as_written() {
  // A comment stays where it is.
  let kept = "use a::{\n    b, // why\n    c,\n};\n";
  assert_eq!(formatted(kept), kept);
}
