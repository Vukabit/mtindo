use super::*;
use crate::syntax::{Edition, parse as parse_source};

fn directives(text: &str) -> Directives {
  scan(&parse_source(text, Edition::E2021).syntax_node(), text)
}

/// The text of each region.
fn regions(text: &str) -> Vec<&str> {
  directives(text)
    .regions
    .iter()
    .map(|&(s, e)| &text[s..e])
    .collect()
}

#[test]
fn test_a_directive_is_the_whole_comment_or_a_reason_after_dashes() {
  assert_eq!(parse("// mto: off"), Some(Marker::Off));
  assert_eq!(parse("// mto: on  "), Some(Marker::On));
  assert_eq!(parse("// mto: pin -- keeps main first"), Some(Marker::Pin));
  assert_eq!(parse("// mto: off -- aligned by hand"), Some(Marker::Off));
  for not in [
    "//mto: off",
    "// mto:off",
    "// MTO: off",
    "// mto: of",
    "// mto: on    This is an example",
    "// mto: off --",
    "// see // mto: off",
  ] {
    assert_eq!(parse(not), None, "{}", not);
  }
}

#[test]
fn test_a_directive_stands_alone_on_its_line() {
  let text = "fn f() {} // mto: off\n    // mto: off\n";
  assert_eq!(at(text, 10, 21), None);
  assert_eq!(at(text, 26, 37), Some(Marker::Off));
}

#[test]
fn test_a_pin_is_found_where_it_stands() {
  let text = "fn z() {}\n// mto: pin\n// about m\nfn m() {}\n";
  assert_eq!(directives(text).pins, [10]);
}

#[test]
fn test_a_region_among_a_statements_parts_takes_the_statement() {
  let text = "fn f() {\n    let v = vec![\n        // mto: off\n        1,  0,\n        // mto: on\n        2,\n    ];\n}\n";
  let found = directives(text);
  assert_eq!(found.regions.len(), 1);
  let (s, e) = found.regions[0];
  assert!(
    text[s..e].starts_with("let v") && text[s..e].ends_with("];"),
    "{}",
    &text[s..e]
  );
}

#[test]
fn test_a_region_ends_with_its_block() {
  // No `on`: to the end of the block, before its brace, with a warning.
  let text = "fn f() {\n    // mto: off\n    x( );\n}\nfn  g() {}\n";
  let found = directives(text);
  assert_eq!(regions(text), ["// mto: off\n    x( );\n"]);
  assert_eq!(found.warnings.len(), 1, "{:?}", found.warnings);
  // An `on` in another block closes nothing.
  let text = "// mto: off\nfn f() {\n    // mto: on\n}\n";
  let found = directives(text);
  assert_eq!(regions(text), [text]);
  assert_eq!(found.warnings.len(), 2, "{:?}", found.warnings);
}

#[test]
fn test_markers_without_a_partner_are_reported_and_do_nothing() {
  let found =
    directives("// mto: on\nfn a() {}\n// mto: off\n// mto: off\nfn b() {}\n// mto: on\n");
  assert_eq!(found.regions.len(), 1);
  assert_eq!(found.warnings.len(), 2, "{:?}", found.warnings);
}

#[test]
fn test_near_misses_are_reported() {
  for text in [
    "//mto: off\nfn a() {}\n",
    "// MTO: on\nfn a() {}\n",
    "// mto: of\nfn a() {}\n",
    "// mto: on    This is an example\nfn a() {}\n",
    "fn a() {} // mto: off\n",
    "/* mto: off */\nfn a() {}\n",
    "/// mto: pin\nfn a() {}\n",
  ] {
    let found = directives(text);
    assert!(
      found.regions.is_empty() && found.pins.is_empty(),
      "{}",
      text
    );
    assert_eq!(found.warnings.len(), 1, "{}: {:?}", text, found.warnings);
  }
  // Prose that mentions a directive is not one, nor a near miss; neither is a string.
  for text in [
    "// an `// mto: off` region is kept\nfn a() {}\n",
    "// mtools on the side\nfn a() {}\n",
    "fn a() { let s = \"// mto: off\"; }\n",
  ] {
    let found = directives(text);
    assert!(
      found.warnings.is_empty() && found.regions.is_empty(),
      "{}: {:?}",
      text,
      found.warnings
    );
  }
}

#[test]
fn test_regions_run_from_off_to_on_in_one_block() {
  let text = "fn a() {}\n// mto: off\nfn  b() {}\n// mto: on\nfn c() {}\n";
  assert_eq!(regions(text), ["// mto: off\nfn  b() {}\n// mto: on"]);
  assert!(directives(text).warnings.is_empty());
  // Inside a function body, and a second region after the first.
  let text = "fn f() {\n    // mto: off\n    x( );\n    // mto: on\n    // mto: off\n    y( );\n    // mto: on\n}\n";
  assert_eq!(
    regions(text),
    [
      "// mto: off\n    x( );\n    // mto: on",
      "// mto: off\n    y( );\n    // mto: on"
    ]
  );
}
