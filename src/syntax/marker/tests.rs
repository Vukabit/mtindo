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
  assert_eq!(parse("// mto: section"), Some(Marker::Section));
  assert_eq!(parse("// mto: section -- helpers"), Some(Marker::Section));
  assert_eq!(parse("// mto: off"), Some(Marker::Off));
  assert_eq!(parse("// mto: on  "), Some(Marker::On));
  assert_eq!(parse("// mto: pin -- keeps main first"), Some(Marker::Pin));
  assert_eq!(parse("// mto: off -- aligned by hand"), Some(Marker::Off));
  assert_eq!(parse("// mto: first"), Some(Marker::First));
  assert_eq!(
    parse("// mto: last -- the deprecated shims"),
    Some(Marker::Last)
  );
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
fn test_a_file_mentions_mto_in_any_case() {
  assert!(mentions("// mTo: off") && mentions("x mto"));
  assert!(!mentions("fn m() { to(); }") && !mentions("mt") && !mentions(""));
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
fn test_an_ignore_counts_only_in_the_header() {
  assert_eq!(parse("// mto: ignore"), Some(Marker::Ignore(Ignore::All)));
  assert_eq!(
    parse("// mto: ignore=layout -- order kept by hand"),
    Some(Marker::Ignore(Ignore::Layout))
  );
  assert_eq!(
    parse("// mto: ignore=format"),
    Some(Marker::Ignore(Ignore::Format))
  );
  assert_eq!(parse("// mto: ignore=both"), None);
  // After comments, inner doc comments, a shebang and inner attributes, over their lines.
  let text = "#!/usr/bin/env run\n\u{feff}//! Docs.\n/* a\n b */\n#![cfg_attr(\n  x,\n  no_std\n)]\n// mto: ignore\nfn a() {}\n";
  let at = text.find("// mto: ignore").unwrap();
  assert_eq!(ignored(text), Some((Ignore::All, at)));
  assert!(
    directives(text).warnings.is_empty(),
    "{:?}",
    directives(text).warnings
  );
  // Not after an outer doc comment, an attribute or an item: reported, not followed.
  for text in [
    "/// a\n// mto: ignore\nfn a() {}\n",
    "#[cfg(x)]\n// mto: ignore\nfn a() {}\n",
    "fn a() {}\n// mto: ignore\n",
    "fn a() {\n  // mto: ignore\n}\n",
  ] {
    assert_eq!(ignored(text), None, "{}", text);
    assert_eq!(directives(text).warnings.len(), 1, "{}", text);
  }
  // A second one, and a scope it does not know, are reported.
  assert_eq!(
    directives("// mto: ignore\n// mto: ignore\n")
      .warnings
      .len(),
    1
  );
  let found = directives("// mto: ignore=both\nfn a() {}\n");
  assert!(
    found.warnings[0].1.contains("`layout` or `format`"),
    "{:?}",
    found.warnings
  );
  // `mto pins` lists it, with its reason.
  let text = "// mto: ignore=format -- tables\nfn a() {}\n";
  let listed = exceptions(&parse_source(text, Edition::E2021).syntax_node(), text);
  assert_eq!(listed.len(), 1);
  assert_eq!(
    (listed[0].kind, listed[0].reason.as_deref()),
    ("ignore=format", Some("tables"))
  );
}

/// `// mto: off` after an item's doc comment or attribute opens a region of the item's
/// container, from the item's start: an `// mto: on` after the item closes it, and neither
/// is warned of (bug hunt, 2026-11-04).
#[test]
fn test_an_off_after_a_doc_comment_starts_with_its_item() {
  let text = "/// doc\n// mto: off\nfn  a(){}\n// mto: on\nfn  b(){}\n";
  let found = directives(text);
  assert!(found.warnings.is_empty(), "{:?}", found.warnings);
  assert_eq!(
    regions(text),
    ["/// doc\n// mto: off\nfn  a(){}\n// mto: on"]
  );
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
    "// Mto: off\nfn a() {}\n",
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
