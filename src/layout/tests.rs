use super::*;
use crate::options::layout::{
  CfgGroups, Flow, ItemKind, MethodOrder, NameSort, OrderKey, Reexports, Sections, TraitImplSort,
};

fn arranged(source: &str) -> String {
  layout(source, &LayoutConfig::default()).unwrap().text
}

fn arranged_with(source: &str, config: &LayoutConfig) -> String {
  layout(source, config).unwrap().text
}

/// A comment between two items on one line belongs to neither, so it gets a line of its
/// own with a blank line after it; at the start of the next item's line it would head that
/// item. Found by the comment-insertion fuzz target.
#[test]
fn test_a_comment_between_items_on_one_line_stays_loose() {
  let source = "use b::r; /* c */ use crate::b::ba;\n";
  let laid = layout(source, &LayoutConfig::default()).unwrap();
  assert_eq!(laid.text, "use b::r;\n\n/* c */\n\nuse crate::b::ba;\n");
  let violations = crate::verify::verify_layout(source, &laid.text, laid.edition);
  assert!(violations.is_empty(), "{:?}", violations);
  assert_eq!(arranged(&laid.text), laid.text);
}

#[test]
fn test_a_comment_touching_its_item_still_heads_it_with_sections() {
  let config = with_sections(Sections::Comments);
  let source = "fn b() {}\n\n// about a\nfn a() {}\n";
  assert_eq!(
    arranged_with(source, &config),
    "// about a\nfn a() {}\n\nfn b() {}\n"
  );
}

#[test]
fn test_a_comment_touching_the_first_item_heads_it() {
  // `macro` items do not take attached comments, so the comment above is loose.
  let source = "// licence\n\n// about m\nmacro m() {}\nfn a() {}\n";
  let once = arranged(source);
  assert_eq!(
    once,
    "// licence\n\n// about m\nmacro m() {}\n\nfn a() {}\n"
  );
  assert_eq!(arranged(&once), once);
}

#[test]
fn test_a_comment_trailing_the_header_stays_in_it() {
  let source = "#![allow(unsafe_code)] // module uses ffi\nuse b;\nuse a;\n";
  assert_eq!(
    arranged(source),
    "#![allow(unsafe_code)] // module uses ffi\n\nuse a;\nuse b;\n"
  );
}

/// A file ending in a carriage return, the last character of its last line comment, keeps
/// its end: a line break after it would make that a CRLF line ending, taking the character
/// from the comment and mixing the file's line endings. Found by the layout fuzz target.
#[test]
fn test_a_final_carriage_return_is_kept_as_written() {
  let source = "\n  //\0 ?/\n//\t\r";
  let out = arranged(source);
  assert_eq!(out, "//\0 ?/\n//\t\r");
  assert!(crate::verify::verify_layout(source, &out, crate::syntax::Edition::E2021).is_empty());
  assert_eq!(arranged(&out), out);
}

#[test]
fn test_a_header_ending_in_a_comment_stays_apart() {
  // Its last line holds a block comment, then a line comment: still a comment, so the blank
  // line after it stays, or the comment would head the item.
  let config = LayoutConfig {
    blank_lines_between_blocks: 0,
    ..LayoutConfig::default()
  };
  let source = "/* a */ // b\n\nfn f() {}\n";
  assert_eq!(layout(source, &config).unwrap().text, source);
}

/// A comment right under an item, a blank line after it, is judged the same on the next
/// run, when it has a blank line above too. Found on the corpus (`// generated end`).
#[test]
fn test_a_heading_right_under_an_item_settles() {
  let config = with_sections(Sections::Headings);
  let source = "fn d() {}\n// generated end\n\nfn c() {}\n\nfn a() {}\n";
  let once = arranged_with(source, &config);
  assert_eq!(
    once,
    "fn d() {}\n\n// generated end\n\nfn a() {}\n\nfn c() {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
}

/// A helper both `#[cfg]` variants of one item use, written apart, has one user: once the
/// variants sort together they are one unit, and the helper must already be beside them,
/// below the last as it was written below. Found on the corpus (syn's two `atom_expr`).
#[test]
fn test_a_helper_of_cfg_variants_settles_beside_them() {
  let config = with_helpers();
  let source = "#[cfg(a)]\nfn atom() {\n    tuple();\n}\n\nfn b() {}\n\n#[cfg(not(a))]\nfn atom() {\n    tuple();\n}\n\nfn tuple() {}\n";
  let once = arranged_with(source, &config);
  assert_eq!(arranged_with(&once, &config), once);
  assert!(
    once.contains("#[cfg(not(a))]\nfn atom() {\n    tuple();\n}\n\nfn tuple() {}"),
    "{}",
    once
  );
}

#[test]
fn test_a_helper_stays_next_to_its_one_user_on_its_written_side() {
  let config = with_helpers();
  // Written above its user: sorted alone `zz` would go last; it stays above `beta`.
  let above = "fn zz() {}\n\nfn beta() {\n    zz();\n}\n\nfn alpha() {}\n";
  let once = arranged_with(above, &config);
  assert_eq!(
    once,
    "fn alpha() {}\n\nfn zz() {}\n\nfn beta() {\n    zz();\n}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
  assert!(arranged(above).ends_with("fn zz() {}\n"));
  // Written below its user: sorted alone `aa` would go first; it stays below `zeta`.
  let below = "fn zeta() {\n    aa();\n}\n\nfn mid() {}\n\nfn aa() {}\n";
  let once = arranged_with(below, &config);
  assert_eq!(
    once,
    "fn mid() {}\n\nfn zeta() {\n    aa();\n}\n\nfn aa() {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
  // A helper of a helper stays next to the item it helps; two helpers sort among
  // themselves, each on its own side.
  let chain =
    "pub fn run() {\n    b();\n    a();\n}\n\nfn c() {}\n\nfn a() {\n    c();\n}\n\nfn b() {}\n";
  let laid = arranged_with(chain, &config);
  assert_eq!(
    laid,
    "pub fn run() {\n    b();\n    a();\n}\n\nfn c() {}\n\nfn a() {\n    c();\n}\n\nfn b() {}\n"
  );
  assert_eq!(arranged_with(&laid, &config), laid);
}

/// A private type with impls of its own, used by one item, moves with its impls.
#[test]
fn test_a_helper_type_moves_with_its_impls() {
  let config = with_helpers();
  let source = "pub fn table() -> Vec<Entry> {\n    vec![Entry::new()]\n}\n\nfn alpha() {}\n\nstruct Entry;\n\nimpl Entry {\n    fn new() -> Self {\n        Entry\n    }\n}\n";
  let once = arranged_with(source, &config);
  assert_eq!(
    once,
    "pub fn table() -> Vec<Entry> {\n    vec![Entry::new()]\n}\n\nstruct Entry;\n\nimpl Entry {\n    fn new() -> Self {\n        Entry\n    }\n}\n\nfn alpha() {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
}

#[test]
fn test_a_helper_whose_user_is_in_another_section_stays_in_its_own() {
  let config = LayoutConfig {
    keep_helpers_near: true,
    sections: Sections::Comments,
    ..LayoutConfig::default()
  };
  let source = "fn helper() {}\n\nfn b() {}\n\n// Rest\n\nfn a() {\n    helper();\n}\n";
  assert_eq!(
    arranged_with(source, &config),
    source.replace("fn helper() {}\n\nfn b() {}", "fn b() {}\n\nfn helper() {}")
  );
}

/// `// mto: hold` keeps the item under it between its neighbours, and nothing moves across
/// it, as a build guard's hold does: stronger than `// mto: pin`, whose slot others pass.
#[test]
fn test_a_hold_directive_keeps_the_item_between_its_neighbours() {
  let source = "fn z() {}\n\n// mto: hold -- moving it breaks the build: E0433\nmod generated {}\n\nfn a() {}\n";
  let once = arranged(source);
  assert_eq!(once, source);
  let pinned = source.replace("hold", "pin");
  // A pin keeps its slot, but `a` and `z` pass it.
  assert_eq!(
    arranged(&pinned),
    "fn a() {}\n\n// mto: pin -- moving it breaks the build: E0433\nmod generated {}\n\nfn z() {}\n"
  );
}

/// A `macro_rules!` shadows a macro of its name (std's `vec!`, one before it) for what
/// comes after it: an item written before it that calls that name may not move below it,
/// nor may the definition move above that item. Found by the bug hunt of 2026-11-04.
#[test]
fn test_a_macro_definition_never_changes_which_macro_a_call_reaches() {
  let config = LayoutConfig::default();
  // `fn count` calls std's `vec!`; the sort puts macros before functions.
  let shadow = "fn count() -> usize {\n    vec![1, 2, 3].len()\n}\n\nmacro_rules! vec {\n    ($($t:tt)*) => {\n        [0u8; 5]\n    };\n}\n\nfn main() {\n    let _ = vec![9];\n}\n";
  let laid = arranged_with(shadow, &config);
  assert!(
    laid.find("fn count").unwrap() < laid.find("macro_rules! vec").unwrap(),
    "{}",
    laid
  );
  assert_eq!(arranged_with(&laid, &config), laid);
  // A definition whose name nothing before it calls still sorts first.
  let free = "fn count() -> usize {\n    3\n}\n\nmacro_rules! other {\n    () => {};\n}\n";
  let laid = arranged_with(free, &config);
  assert!(laid.starts_with("macro_rules! other"), "{}", laid);
}

#[test]
fn test_a_paragraph_ends_at_a_blank_line_even_below_a_comment() {
  let config = LayoutConfig {
    keep_paragraphs: true,
    ..LayoutConfig::default()
  };
  // The comment touches `z`, but a blank line parts it from `a`: two units.
  let source = "fn z() {}\n// note\n\nfn a() {}\n";
  assert_eq!(
    arranged_with(source, &config),
    "// note\n\nfn a() {}\n\nfn z() {}\n"
  );
  // A comment heading the second item, with no blank line anywhere, is inside it.
  let tight = "fn z() {}\n// note\nfn a() {}\n";
  assert_eq!(arranged_with(tight, &config), tight);
}

/// A pinned unit after a held one keeps its slot in its run: slots were matched by the
/// unit's place in the whole file, so in a later run a pinned unit counted as free and was
/// dropped (the verifier stopped the write). Found by `sections`, which cuts runs too.
#[test]
fn test_a_pinned_unit_after_a_held_one_keeps_its_slot() {
  let source = "fn z() {}\n\nfn b() {}\n\nm!();\n\nfn a() {}\n";
  let held: HashSet<usize> = [source.find("fn b").unwrap()].into();
  let kept = layout_holding(source, &LayoutConfig::default(), &held).unwrap();
  assert_eq!(kept.text, source);
  assert!(crate::verify::verify_layout(source, &kept.text, kept.edition).is_empty());
}

#[test]
fn test_a_test_module_goes_last_in_its_section() {
  let config = with_sections(Sections::Comments);
  let source = "#[cfg(test)]\nmod tests {}\n\nfn b() {}\n\n// Rest\n\nfn a() {}\n";
  assert_eq!(
    arranged_with(source, &config),
    "fn b() {}\n\n#[cfg(test)]\nmod tests {}\n\n// Rest\n\nfn a() {}\n"
  );
}

/// An impl for a name two types share goes with the one placed first, whichever was
/// written first, so a second run leaves it there. Found by the generated-code fuzz target.
#[test]
fn test_an_impl_of_a_shared_name_settles() {
  for source in [
    "impl Display for Foo {}\n\nstruct Foo;\n\nenum Foo {\n    A,\n}\n",
    "enum Foo {\n    A,\n}\n\nstruct Foo;\n\nimpl Display for Foo {}\n",
  ] {
    let once = arranged(source);
    assert_eq!(
      once,
      "enum Foo {\n    A,\n}\n\nimpl Display for Foo {}\n\nstruct Foo;\n"
    );
    assert_eq!(arranged(&once), once);
  }
}

/// `cfg_groups`: items under the same `#[cfg(...)]` move as one: written with no blank
/// line between them, or, with "gather", anywhere in their section, following the first
/// to sort; never above a macro definition they use.
#[test]
fn test_cfg_groups_move_items_under_one_predicate_as_one() {
  let together = LayoutConfig {
    cfg_groups: CfgGroups::Together,
    ..LayoutConfig::default()
  };
  let tight = "fn z() {}\n\n#[cfg(unix)]\nfn y() {}\n#[cfg( unix )]\nstruct U;\n\nfn b() {}\n\n#[cfg(unix)]\nfn a() {}\n";
  let once = arranged_with(tight, &together);
  assert_eq!(
    once,
    "#[cfg(unix)]\nfn a() {}\n\nfn b() {}\n\n#[cfg(unix)]\nfn y() {}\n#[cfg( unix )]\nstruct U;\n\nfn z() {}\n"
  );
  assert_eq!(arranged_with(&once, &together), once);
  let gather = LayoutConfig {
    cfg_groups: CfgGroups::Gather,
    ..LayoutConfig::default()
  };
  let source = "fn z() {}\n\n#[cfg(unix)]\nfn y() {}\n\n#[cfg( unix )]\nstruct U;\n\nfn b() {}\n\n#[cfg(windows)]\nfn w() {}\n\n#[cfg(unix)]\nfn a() {}\n";
  let once = arranged_with(source, &gather);
  assert_eq!(
    once,
    "#[cfg( unix )]\nstruct U;\n\n#[cfg(unix)]\nfn a() {}\n\n#[cfg(unix)]\nfn y() {}\n\nfn b() {}\n\n#[cfg(windows)]\nfn w() {}\n\nfn z() {}\n"
  );
  assert_eq!(arranged_with(&once, &gather), once);
  assert!(crate::verify::verify_layout(source, &once, crate::syntax::Edition::E2021).is_empty());
  // Never above the macro a member uses: `a` calls `m!`, defined after `y`.
  let macros = "#[cfg(unix)]\nfn y() {}\n\nmacro_rules! m {\n    () => {};\n}\n\nfn b() {}\n\n#[cfg(unix)]\nfn a() {\n    m!();\n}\n";
  let laid = arranged_with(macros, &gather);
  assert!(crate::verify::verify_layout(macros, &laid, crate::syntax::Edition::E2021).is_empty());
  assert!(
    laid.find("macro_rules").unwrap() < laid.find("fn a").unwrap(),
    "{}",
    laid
  );
  assert_eq!(arranged_with(&laid, &gather), laid);
}

#[test]
fn test_cfg_variants_move_together() {
  let source = "#[cfg(unix)]\nfn z() {}\n#[cfg(windows)]\nfn z() {}\nfn a() {}\n";
  assert_eq!(
    arranged(source),
    "fn a() {}\n\n#[cfg(unix)]\nfn z() {}\n#[cfg(windows)]\nfn z() {}\n"
  );
  // Duplicate macros are fine when they are cfg variants of each other.
  let macros =
    "#[cfg(a)]\nmacro_rules! m { () => {} }\n#[cfg(not(a))]\nmacro_rules! m { () => {} }\n";
  assert_eq!(arranged(macros), macros);
}

/// `#[cfg]` variants of a helper written apart come out together above their user, and
/// stay there: once adjacent they are one unit, which must land in the same place.
#[test]
fn test_cfg_variants_of_a_helper_settle_together() {
  let config = with_helpers();
  let source = "#[cfg(a)]\nconst S: u8 = 1;\n\n#[cfg(a)]\nconst H: u8 = 1;\n\n#[cfg(b)]\nconst S: u8 = 2;\n\n#[cfg(b)]\nconst H: u8 = 2;\n\nfn z() {}\n\nfn uses() -> u8 {\n    S + H\n}\n";
  let once = arranged_with(source, &config);
  assert_eq!(arranged_with(&once, &config), once);
  // Written apart, each variant names the other, which is not a use.
  assert_eq!(
    layout(source, &config)
      .unwrap()
      .before
      .iter()
      .filter(|u| u.caller.is_some())
      .count(),
    4
  );
  assert!(
    once.contains("const H: u8 = 1;\n\n#[cfg(b)]\nconst H: u8 = 2;")
      && once.contains("const S: u8 = 2;\n\nfn uses()"),
    "{}",
    once
  );
}

#[test]
fn test_code_only_valid_in_an_older_edition_still_parses() {
  // `async` and `dyn` are plain names in 2015.
  let source = "fn dyn() {}\nfn async() {}\n";
  let (edition, parsed) = parse_any_edition(source, Edition::E2021);
  assert!(parsed.errors().is_empty());
  assert_eq!(edition, Edition::E2015);
  assert_eq!(arranged(source), "fn async() {}\n\nfn dyn() {}\n");
}

/// Comments ending an item's line move with it, the ones the tree leaves out too (a block
/// comment followed by another), in an impl, a module and the file.
#[test]
fn test_comments_ending_an_items_line_move_with_it() {
  let config = LayoutConfig {
    reorder_impl_items: true,
    ..LayoutConfig::default()
  };
  let cases = [
    (
      "impl S {\n    const B: u8 = 0;\n    const A: u8 = 0; /* x */ // y\n}\n",
      "impl S {\n    const A: u8 = 0; /* x */ // y\n    const B: u8 = 0;\n}\n",
    ),
    (
      "mod m {\n    const B: u8 = 0;\n    const A: u8 = 0; /* x */ // y\n}\n",
      "mod m {\n    const A: u8 = 0; /* x */ // y\n\n    const B: u8 = 0;\n}\n",
    ),
    (
      "const B: u8 = 0; /* x */ /* z */\nconst A: u8 = 0;\n",
      "const A: u8 = 0;\n\nconst B: u8 = 0; /* x */ /* z */\n",
    ),
  ];
  for (source, expected) in cases {
    let laid = layout(source, &config).unwrap();
    assert_eq!(laid.text, expected);
    let violations = crate::verify::verify_layout(source, &laid.text, laid.edition);
    assert!(violations.is_empty(), "{:?}", violations);
  }
}

#[test]
fn test_crlf_and_bom_come_back() {
  let source = "\u{feff}fn b() {}\r\nfn a() {}\r\n";
  assert_eq!(arranged(source), "\u{feff}fn a() {}\r\n\r\nfn b() {}\r\n");
}

#[test]
fn test_deprecated_and_hidden_items_sink_to_the_bottom_of_their_kind() {
  let config = with_keys(vec![
    OrderKey::Kind,
    OrderKey::Deprecated,
    OrderKey::Hidden,
    OrderKey::Visibility,
    OrderKey::Name,
  ]);
  let source = "#[deprecated]\npub fn a() {}\n\n#[doc(hidden)]\npub fn b() {}\n\nfn c() {}\n\n#[deprecated]\npub struct Old;\n\nimpl Old {}\n\npub struct New;\n\nimpl New {}\n";
  let once = arranged_with(source, &config);
  // A deprecated type's impl sinks with it.
  assert_eq!(
    once,
    "pub struct New;\n\nimpl New {}\n\n#[deprecated]\npub struct Old;\n\nimpl Old {}\n\nfn c() {}\n\n#[doc(hidden)]\npub fn b() {}\n\n#[deprecated]\npub fn a() {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
  assert!(crate::verify::verify_layout(source, &once, crate::syntax::Edition::E2021).is_empty());
}

/// A directive after an item's doc comment or an attribute reads as written above the item:
/// a pin keeps its slot, nothing crosses a hold, an off region starts with the item (bug
/// hunt, 2026-11-04).
#[test]
fn test_directives_after_a_doc_comment_or_attribute_count() {
  let config = LayoutConfig::default();
  let source = "fn zeta() {}\n\n/// Must stay second.\n// mto: pin\nfn mike() {}\n\n#[inline]\n// mto: hold\nfn charlie() {}\n\nfn alpha() {}\n";
  assert_eq!(arranged_with(source, &config), source);
  let off = "fn zeta() {}\n\n/// Doc.\n// mto: off\nfn mike( a:u8 ) {}\nfn   bravo() {}\n// mto: on\n\nfn alpha() {}\n";
  let laid = arranged_with(off, &config);
  assert!(
    laid.contains("// mto: off\nfn mike( a:u8 ) {}\nfn   bravo() {}\n// mto: on"),
    "{}",
    laid
  );
}

#[test]
fn test_entry_puts_main_at_the_top_of_the_functions() {
  let config = with_keys(vec![
    OrderKey::Kind,
    OrderKey::Entry,
    OrderKey::Visibility,
    OrderKey::Name,
  ]);
  let source = "pub fn api() {}\n\nfn helper() {}\n\nfn main() {}\n\nstruct S;\n";
  let once = arranged_with(source, &config);
  assert_eq!(
    once,
    "struct S;\n\nfn main() {}\n\npub fn api() {}\n\nfn helper() {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
}

#[test]
fn test_every_item_kind_has_its_place() {
  let source =
    "fn f() {}\nextern \"C\" {}\nunion U { a: u8 }\nstruct S;\ntype T = u8;\nmacro m() {}\n";
  assert_eq!(
    arranged(source),
    "macro m() {}\n\ntype T = u8;\n\nstruct S;\n\nunion U { a: u8 }\n\nextern \"C\" {}\n\nfn f() {}\n"
  );
}

#[test]
fn test_every_spelling_of_rustfmt_skip_is_honored() {
  for attr in [
    "#![rustfmt::skip]",
    "#![cfg_attr(rustfmt, rustfmt_skip)]",
    "#![cfg_attr(rustfmt, rustfmt::skip)]",
  ] {
    let source = format!("{}\nfn b() {{}}\nfn a() {{}}\n", attr);
    assert_eq!(arranged(&source), source, "{}", attr);
  }
  let item = "fn z() {}\n#[cfg_attr(rustfmt, rustfmt_skip)]\nfn m() {}\nfn a() {}\n";
  assert_eq!(
    arranged(item),
    "fn a() {}\n\n#[cfg_attr(rustfmt, rustfmt_skip)]\nfn m() {}\n\nfn z() {}\n"
  );
  // Any other `cfg_attr` condition is not a skip.
  let other = "#![cfg_attr(feature = \"x\", rustfmt_skip)]\nfn b() {}\nfn a() {}\n";
  assert_ne!(arranged(other), other);
}

#[test]
fn test_files_that_cannot_be_laid_out_say_why() {
  let config = LayoutConfig::default();
  assert!(
    layout("fn a( {}\n", &config)
      .err()
      .unwrap()
      .contains("syntax errors")
  );
  let twice = "macro_rules! m { () => {} }\nmacro_rules! m { () => {} }\n";
  assert!(
    layout(twice, &config)
      .err()
      .unwrap()
      .contains("defined more than once")
  );
}

/// `// mto: first` and `// mto: last` put the item under them at the top or the bottom of
/// its section, the others sorted between; in an impl, among its items.
#[test]
fn test_first_and_last_put_an_item_at_an_end_of_its_section() {
  let source =
    "// mto: last -- the shims\nfn a() {}\n\nstruct S;\n\nfn b() {}\n\n// mto: first\nfn z() {}\n";
  let once = arranged(source);
  assert_eq!(
    once,
    "// mto: first\nfn z() {}\n\nstruct S;\n\nfn b() {}\n\n// mto: last -- the shims\nfn a() {}\n"
  );
  assert_eq!(arranged(&once), once);
  assert!(crate::verify::verify_layout(source, &once, crate::syntax::Edition::E2021).is_empty());
  // A section's top, not the file's: the marker under `// mto: section` stays below it.
  let config = with_sections(Sections::Markers);
  let sectioned = "fn b() {}\n\n// mto: section\n\nfn y() {}\n\n// mto: first\nfn z() {}\n";
  assert_eq!(
    arranged_with(sectioned, &config),
    "fn b() {}\n\n// mto: section\n\n// mto: first\nfn z() {}\n\nfn y() {}\n"
  );
  // Imports are an item too: one marked last goes below them all.
  let first_use = "use b::B;\n\n// mto: first\nfn z() {}\n";
  assert_eq!(
    arranged(first_use),
    "// mto: first\nfn z() {}\n\nuse b::B;\n"
  );
  let in_impl = "impl S {\n    // mto: last\n    fn new() -> Self {\n        S\n    }\n\n    const A: u8 = 1;\n\n    type T = u8;\n}\n";
  let laid = arranged(in_impl);
  assert!(laid.trim_end().ends_with("S\n    }\n}"), "{}", laid);
}

/// A type moves with its impls under `flow`, and items of other kinds stay in their rank.
#[test]
fn test_flow_moves_a_type_with_its_impls() {
  let source =
    "struct A;\n\nimpl A {\n    fn f() {}\n}\n\nstruct B {\n    a: A,\n}\n\nfn main() {}\n";
  let config = LayoutConfig {
    flow: Flow::TopDown,
    ..LayoutConfig::default()
  };
  let once = arranged_with(source, &config);
  assert_eq!(
    once,
    "struct B {\n    a: A,\n}\n\nstruct A;\n\nimpl A {\n    fn f() {}\n}\n\nfn main() {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
}

/// `flow` puts a function above the ones it uses, or below them; the sort keys decide
/// where no use does, and a circle of uses keeps them.
#[test]
fn test_flow_orders_items_by_their_uses() {
  let source = "fn a() {}\n\nfn b() {\n    c();\n}\n\nfn c() {\n    a();\n}\n\nfn d() {}\n\nfn odd() {\n    even();\n}\n\nfn even() {\n    odd();\n}\n";
  let top_down = LayoutConfig {
    flow: Flow::TopDown,
    ..LayoutConfig::default()
  };
  let once = arranged_with(source, &top_down);
  assert_eq!(
    once,
    "fn b() {\n    c();\n}\n\nfn c() {\n    a();\n}\n\nfn a() {}\n\nfn d() {}\n\nfn even() {\n    odd();\n}\n\nfn odd() {\n    even();\n}\n"
  );
  assert_eq!(arranged_with(&once, &top_down), once);
  let bottom_up = LayoutConfig {
    flow: Flow::BottomUp,
    ..LayoutConfig::default()
  };
  let once = arranged_with(source, &bottom_up);
  assert_eq!(
    once,
    "fn a() {}\n\nfn c() {\n    a();\n}\n\nfn b() {\n    c();\n}\n\nfn d() {}\n\nfn even() {\n    odd();\n}\n\nfn odd() {\n    even();\n}\n"
  );
  assert_eq!(arranged_with(&once, &bottom_up), once);
}

/// With `order_keys = []` a type and its impls may stand apart; `flow` takes them as one
/// node all the same, so a second run finds what the first did (bug hunt, 2026-11-04).
#[test]
fn test_flow_settles_with_the_written_order() {
  let source = "fn main() {\n    let _ = Args::parse();\n}\n\nstruct Args {\n    layer: Layer,\n}\n\nenum Layer {\n    Fg,\n}\n\nimpl Args {\n    fn parse() -> Args {\n        Args { layer: Layer::Fg }\n    }\n}\n";
  for flow in [Flow::TopDown, Flow::BottomUp] {
    let config = LayoutConfig {
      flow,
      order_keys: Vec::new(),
      ..LayoutConfig::default()
    };
    let once = arranged_with(source, &config);
    assert_eq!(arranged_with(&once, &config), once, "{:?}", flow);
  }
}

/// Blank lines between a section comment and its item follow the configured spacing, so a
/// second run cannot change them when the comment ends up at the top of the file.
#[test]
fn test_gaps_inside_a_unit_follow_the_spacing_and_are_stable() {
  let source = "fn z() {}\n// note\n\n\n\nfn a() {}\n";
  let once = arranged(source);
  assert_eq!(once, "// note\n\nfn a() {}\n\nfn z() {}\n");
  assert_eq!(arranged(&once), once);
}

#[test]
fn test_header_stays_first_byte_for_byte() {
  let source = "#!/usr/bin/env rust\n//! Docs with a trailing space \n#![allow(x)]\n\n// Licence\n\nfn b() {}\nfn a() {}\n";
  assert_eq!(
    arranged(source),
    "#!/usr/bin/env rust\n//! Docs with a trailing space \n#![allow(x)]\n\n// Licence\n\nfn a() {}\n\nfn b() {}\n"
  );
}

#[test]
fn test_headings_start_sections_and_other_loose_comments_keep_their_place() {
  let config = with_sections(Sections::Headings);
  let source =
    "fn d() {}\n\n// TODO: split this\n\nfn c() {}\n\n// Parsing\n\nfn b() {}\n\nfn a() {}\n";
  let once = arranged_with(source, &config);
  // The TODO keeps its slot among the first section's units; `// Parsing` cuts.
  assert_eq!(
    once,
    "fn c() {}\n\n// TODO: split this\n\nfn d() {}\n\n// Parsing\n\nfn a() {}\n\nfn b() {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
  for not_heading in [
    "// let x = 1;",
    "// SAFETY: checked above",
    "// one\n// two\n// three",
  ] {
    let text = format!("fn b() {{}}\n\n{}\n\nfn a() {{}}\n", not_heading);
    let laid = arranged_with(&text, &config);
    assert!(
      laid.starts_with("fn a()"),
      "{:?} cut a section: {}",
      not_heading,
      laid
    );
  }
}

/// A held unit keeps its place, as if pinned, while what is inside it is still laid out;
/// holding every move gives back the order as written.
#[test]
fn test_held_units_keep_their_places() {
  let source = "struct A;\nfn b() {}\nmod m {\n    fn z() {}\n    struct Y;\n}\n";
  let config = LayoutConfig::default();
  let laid = layout(source, &config).unwrap();
  assert_eq!(
    laid
      .moves
      .iter()
      .map(|m| m.identifier.as_str())
      .collect::<Vec<_>>(),
    ["m", "Y"]
  );
  let module = &laid.moves[0];
  assert_eq!(module.kind, ItemKind::ModDecl);
  assert_eq!(module.before, Some((ItemKind::StructDecl, "A".to_string())));
  assert!(source[module.span.0..module.span.1].starts_with("mod m {"));
  let held: HashSet<usize> = [module.span.0].into();
  let kept = layout_holding(source, &config, &held).unwrap();
  assert_eq!(
    kept.text,
    "struct A;\n\nfn b() {}\n\nmod m {\n    struct Y;\n\n    fn z() {}\n}\n"
  );
  assert!(crate::verify::verify_layout(source, &kept.text, kept.edition).is_empty());
  let all: HashSet<usize> = laid.moves.iter().map(|m| m.span.0).collect();
  let none = layout_holding(source, &config, &all).unwrap();
  assert!(none.moves.is_empty());
  assert_eq!(
    none.text,
    "struct A;\n\nfn b() {}\n\nmod m {\n    fn z() {}\n\n    struct Y;\n}\n"
  );
}

#[test]
fn test_impl_functions_follow_the_trait() {
  let source = "trait T {\n    fn a();\n    fn b();\n}\nimpl T for X {\n    fn extra() {}\n\n    const C: u8 = 0;\n    fn b() {}\n    // pinned\n    // mto: pin\n    fn z() {}\n    fn a() {}\n}\n";
  let config = LayoutConfig {
    impl_method_order_trait: true,
    ..LayoutConfig::default()
  };
  let once = layout(source, &config).unwrap().text;
  // Functions trade places in the functions' slots; the const, the pinned function and
  // the blank lines stay.
  assert_eq!(
    once,
    "trait T {\n    fn a();\n    fn b();\n}\n\nimpl T for X {\n    fn a() {}\n\n    const C: u8 = 0;\n    fn b() {}\n    // pinned\n    // mto: pin\n    fn z() {}\n    fn extra() {}\n}\n"
  );
  assert_eq!(layout(&once, &config).unwrap().text, once, "stable");
  assert!(crate::verify::verify_layout(source, &once, Edition::E2021).is_empty());
}

#[test]
fn test_impl_items_rank_by_their_own_code() {
  let config = LayoutConfig {
    reorder_impl_items: true,
    ..LayoutConfig::default()
  };
  // An `impl Trait` alias goes after a plain one, whatever `=` a comment above it holds.
  let source = "impl S {\n    // a = b\n    type A = impl X;\n\n    type B = u8;\n}\n";
  let once = layout(source, &config).unwrap().text;
  assert_eq!(
    once,
    "impl S {\n    type B = u8;\n\n    // a = b\n    type A = impl X;\n}\n"
  );
  assert_eq!(layout(&once, &config).unwrap().text, once);
  // A body closed on its last item's line is left as written: a comment ending an item
  // moved there would no longer end its line.
  let closed = "impl S {\n    fn f() {} // c\n    type A = u8;}\n";
  assert_eq!(layout(closed, &config).unwrap().text, closed);
}

/// Reordering impl items must not lift a const above a `macro_rules!` in the body.
#[test]
fn test_impl_items_stay_below_a_macro_definition() {
  let config = LayoutConfig {
    reorder_impl_items: true,
    ..LayoutConfig::default()
  };
  let source = "impl S {\n    fn f() {}\n\n    macro_rules! m {\n        () => {};\n    }\n\n    const A: u8 = 0;\n}\n";
  let laid = layout(source, &config).unwrap();
  assert!(
    laid.text.find("const A").unwrap() > laid.text.find("macro_rules!").unwrap(),
    "{}",
    laid.text
  );
  let violations = crate::verify::verify_layout(source, &laid.text, laid.edition);
  assert!(violations.is_empty(), "{:?}", violations);
}

#[test]
fn test_impls_under_a_type() {
  let source = "impl Display for Foo {}\nimpl Foo { fn b() {} }\nimpl Clone for Foo {}\nimpl Foo { fn a() {} }\nstruct Foo;\nimpl fmt::Debug for Foo {}\n";
  let order = |config: &LayoutConfig| {
    layout(source, config)
      .unwrap()
      .text
      .lines()
      .filter(|l| !l.is_empty())
      .map(str::to_string)
      .collect::<Vec<_>>()
  };
  // Own impls first, as written; then trait impls by name.
  assert_eq!(
    order(&LayoutConfig::default()),
    [
      "struct Foo;",
      "impl Foo { fn b() {} }",
      "impl Foo { fn a() {} }",
      "impl Clone for Foo {}",
      "impl fmt::Debug for Foo {}",
      "impl Display for Foo {}",
    ]
  );
  let listed = LayoutConfig {
    trait_impl_order: vec!["Display".to_string()],
    trait_impl_sort: TraitImplSort::Original,
    ..LayoutConfig::default()
  };
  assert_eq!(
    order(&listed)[3..],
    [
      "impl Display for Foo {}",
      "impl Clone for Foo {}",
      "impl fmt::Debug for Foo {}",
    ]
  );
}

#[test]
fn test_imports_declarations_tests_and_pins_never_join_a_paragraph() {
  let config = LayoutConfig {
    keep_paragraphs: true,
    ..LayoutConfig::default()
  };
  let imports = "use b::B;\nuse a::A;\n";
  assert_eq!(arranged_with(imports, &config), "use a::A;\nuse b::B;\n");
  let modules = "mod b;\nmod a;\n";
  assert_eq!(arranged_with(modules, &config), "mod a;\nmod b;\n");
  // Two definitions of one macro still refuse the file, written together or not.
  let macros = "macro_rules! m { () => {} }\nmacro_rules! m { () => {} }\nfn a() {}\n";
  assert!(layout(macros, &config).is_err());
  // A pinned item keeps its slot; the item after it is free to move.
  let pinned = "fn z() {}\n// mto: pin\nfn y() {}\nfn a() {}\n";
  let once = arranged_with(pinned, &config);
  assert!(once.starts_with("fn a() {}"), "{}", once);
  assert_eq!(arranged_with(&once, &config), once);
}

#[test]
fn test_imports_group_and_declarations_stay_together() {
  let source = "use crate::x;\nuse serde::Y;\nuse std::io;\nmod b;\nmod a;\n";
  assert_eq!(
    arranged(source),
    "mod a;\nmod b;\n\nuse std::io;\n\nuse serde::Y;\n\nuse crate::x;\n"
  );
}

#[test]
fn test_imports_sort_as_the_standard_formatter_does_for_the_style_edition() {
  let source =
    "use x::u16;\nuse x::u8;\nuse x::Z;\nuse x::a;\nuse super::*;\nuse super::m::{Y, Z};\n";
  let with = |style_edition: u16, import_sort: Option<u16>| {
    let config = LayoutConfig {
      style_edition,
      import_sort,
      ..LayoutConfig::default()
    };
    layout(source, &config).unwrap().text
  };
  let legacy =
    "use x::a;\nuse x::u16;\nuse x::u8;\nuse x::Z;\n\nuse super::m::{Y, Z};\nuse super::*;\n";
  let version =
    "use x::Z;\nuse x::a;\nuse x::u8;\nuse x::u16;\n\nuse super::m::{Y, Z};\nuse super::*;\n";
  assert_eq!(with(2021, None), legacy);
  assert_eq!(with(2024, None), version);
  // `import_sort` overrides the style edition for imports.
  assert_eq!(with(2024, Some(2015)), legacy);
  assert_eq!(with(2015, Some(2024)), version);
}

/// `inherent_method_order` orders the functions of an impl with no trait; a trait impl
/// keeps its own.
#[test]
fn test_inherent_impl_functions_follow_the_method_order() {
  let source = "struct A;\n\nimpl A {\n    fn a(&self) {}\n\n    fn b(&mut self) {}\n\n    fn c(self) {}\n\n    pub fn new() -> Self {\n        A\n    }\n\n    fn helper() {}\n}\n\nimpl Clone for A {\n    fn clone_from(&mut self, _: &Self) {}\n\n    fn clone() -> Self {\n        A\n    }\n}\n";
  let order = |method_order| {
    let config = LayoutConfig {
      inherent_method_order: method_order,
      ..LayoutConfig::default()
    };
    let once = arranged_with(source, &config);
    assert_eq!(arranged_with(&once, &config), once);
    let names: Vec<String> = once
      .lines()
      .filter_map(|line| line.trim().split("fn ").nth(1))
      .map(|rest| rest.split('(').next().unwrap().to_string())
      .collect();
    names.join(" ")
  };
  assert_eq!(
    order(MethodOrder::ConstructorsFirst),
    "new a b c helper clone_from clone"
  );
  assert_eq!(
    order(MethodOrder::ByReceiver),
    "new helper a b c clone_from clone"
  );
  assert_eq!(
    order(MethodOrder::Written),
    "a b c new helper clone_from clone"
  );
}

#[test]
fn test_inline_modules_are_laid_out_at_every_level() {
  let source = "mod outer {\n    //! Docs.\n\n    fn b() {}\n    // about a\n    fn a() {}\n    mod inner {\n        use std::io;\n        fn z() {}\n        use std::fmt;\n    }\n}\nfn top() {}\n";
  let expected = "mod outer {\n    //! Docs.\n\n    mod inner {\n        use std::fmt;\n        use std::io;\n\n        fn z() {}\n    }\n\n    // about a\n    fn a() {}\n\n    fn b() {}\n}\n\nfn top() {}\n";
  let once = arranged(source);
  assert_eq!(once, expected);
  assert_eq!(arranged(&once), once, "stable");
  let verified = crate::verify::verify_layout(source, &once, Edition::E2021);
  assert!(verified.is_empty(), "{:?}", verified);
  // A body on one line with its braces, and a skipped module, are left as written.
  let one_line = "mod m { fn b() {} fn a() {} }\n";
  assert_eq!(arranged(one_line), one_line);
  let skipped = "#[rustfmt::skip]\nmod m {\n    fn b() {}\n    fn a() {}\n}\n";
  assert_eq!(arranged(skipped), skipped);
  // So is a body closed on its last item's line: a comment ending an item moved there would
  // no longer end its line.
  let closed = "mod m {\n    fn b() {} /* c */\n\n    fn a() {\n}  }\n";
  assert_eq!(arranged(closed), closed);
}

#[test]
fn test_inspect_names_the_rule_behind_each_move() {
  let source = "fn b() {}\n\nstruct A;\n\npub fn c() {}\n";
  let seen = inspect(source, &LayoutConfig::default()).unwrap();
  let moves: Vec<(&str, Rule)> = seen
    .moves
    .iter()
    .map(|m| (m.name.as_str(), m.rule))
    .collect();
  // The fewest units out of order move: `b`, below `pub fn c` by visibility.
  assert_eq!(moves, [("b", Rule::Visibility)]);
  let helper = LayoutConfig {
    keep_helpers_near: true,
    ..LayoutConfig::default()
  };
  // Only `zz` is out of order: the rest keep theirs.
  let used = "fn zz() {}\n\nfn a1() {}\n\nfn a2() {}\n\nfn a3() {}\n\nfn beta() {\n    zz();\n}\n";
  let moves: Vec<(String, Rule)> = inspect(used, &helper)
    .unwrap()
    .moves
    .iter()
    .map(|m| (m.name.clone(), m.rule))
    .collect();
  assert_eq!(moves, [("zz".to_string(), Rule::Helper)]);
}

#[test]
fn test_inspect_says_why_each_unit_was_cut_so() {
  let config = LayoutConfig {
    keep_paragraphs: true,
    keep_helpers_near: true,
    sections: Sections::Comments,
    ..LayoutConfig::default()
  };
  let source = "use std::fmt;\n\n// Parsing\n\nfn parse() {\n    step();\n}\nfn parse_all() {}\n\nfn step() {}\n\nfn shared() {}\n\nfn one() {\n    shared();\n}\n\nfn two() {\n    shared();\n}\n\n// mto: pin\nfn kept() {}\n\nmod inner {\n    fn b() {}\n\n    fn a() {}\n}\n";
  let seen = inspect(source, &config).unwrap();
  assert_eq!(seen.containers.len(), 2);
  assert_eq!(seen.containers[1].path, "mod inner");
  let units = &seen.containers[0].units;
  let notes_of = |name: &str| {
    units
      .iter()
      .find(|u| u.name == name)
      .unwrap_or_else(|| panic!("no unit {}", name))
      .notes
      .join("; ")
  };
  assert!(notes_of("std::fmt").contains("import group: standard library"));
  assert!(notes_of("// Parsing").contains("starts a section"));
  assert!(notes_of("parse").contains("a paragraph"));
  assert!(notes_of("step").contains("helper: stays below `parse`"));
  assert!(notes_of("shared").contains("not a helper: used by 2 items: one, two"));
  assert!(notes_of("kept").contains("pinned, keeps its slot: `// mto: pin`"));
  let parse = units.iter().find(|u| u.name == "parse").unwrap();
  assert_eq!((parse.kind, parse.lines), ("fn", (5, 8)));
}

/// Items compared under any `type_order` (empty, partial, full) and the settings that keep
/// some kinds as written are one order: never `a < b < c < a`, which made sorting panic. An
/// import among constants, below the others, with `type_order = []` and
/// `sort_imports = false` was one.
#[test]
fn test_items_compare_as_one_order_under_any_type_order() {
  use std::cmp::Ordering;
  let source = "use a::A;\nuse z::Z;\npub use m::M;\n\nconst B: u8 = 1;\npub const C: u8 = 2;\n\
                pub use crate::abi::STACK_ENTRY;\npub(crate) const D: u8 = 3;\nstatic E: u8 = 0;\n\
                fn f() {}\npub fn g() {}\nstruct S;\nimpl S {}\nmacro_rules! m { () => {} }\n\
                mod x {}\npub type T = u8;\n";
  let defaults = LayoutConfig::default();
  let orders = [
    vec![],
    vec![ItemKind::FnDecl],
    vec![ItemKind::ConstDecl, ItemKind::UseDecl],
    defaults.type_order.clone(),
  ];
  for type_order in orders {
    for sort_imports in [false, true] {
      for sort_alphabetically in [false, true] {
        for group_impls in [false, true] {
          let config = LayoutConfig {
            type_order: type_order.clone(),
            sort_imports,
            sort_alphabetically,
            group_impls,
            ..LayoutConfig::default()
          };
          let (split, _) = split(source, "\n\n", &config, &HashSet::new()).unwrap();
          let units: Vec<&Unit> = split.units.iter().filter(|u| !u.pinned).collect();
          let families = Vec::new();
          let cmp = |a: &Unit, b: &Unit| order::compare(a, b, &config, &families, false);
          for &a in &units {
            for &b in &units {
              assert_eq!(cmp(a, b), cmp(b, a).reverse(), "{} / {}", a.text, b.text);
              for &c in &units {
                if cmp(a, b) != Ordering::Greater && cmp(b, c) != Ordering::Greater {
                  assert_ne!(
                    cmp(a, c),
                    Ordering::Greater,
                    "{:?} sort_imports {}: {} <= {} <= {}",
                    config.type_order,
                    sort_imports,
                    a.text,
                    b.text,
                    c.text
                  );
                }
              }
            }
          }
        }
      }
    }
  }
}

#[test]
fn test_items_stay_in_their_section() {
  let source = "fn d() {}\n\nfn c() {}\n\n// Helpers\n\nfn b() {}\n\nfn a() {}\n";
  let config = with_sections(Sections::Comments);
  let once = arranged_with(source, &config);
  assert_eq!(
    once,
    "fn c() {}\n\nfn d() {}\n\n// Helpers\n\nfn a() {}\n\nfn b() {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
  // Off, the comment moves with the item under it.
  assert_eq!(
    arranged(source),
    "fn a() {}\n\n// Helpers\n\nfn b() {}\n\nfn c() {}\n\nfn d() {}\n"
  );
}

#[test]
fn test_items_that_are_not_clearly_helpers_sort_as_usual() {
  let config = with_helpers();
  for source in [
    // Used by two items.
    "fn z() {\n    h();\n}\n\nfn y() {\n    h();\n}\n\nfn h() {}\n",
    // Public.
    "fn z() {\n    h();\n}\n\npub fn h() {}\n",
    // Used by nothing but itself.
    "fn z() {}\n\nfn h() {\n    h();\n}\n",
  ] {
    assert_eq!(
      arranged_with(source, &config),
      arranged(source),
      "{}",
      source
    );
  }
}

/// `#[cfg_attr(.., macro_use)]` and an inner `#![macro_use]` export a module's macros as
/// `#[macro_use]` does, so its users stay below it and it keeps its place among modules.
#[test]
fn test_macro_use_in_cfg_attr_or_inside_counts() {
  let config = LayoutConfig {
    type_order: vec![ItemKind::FnDecl, ItemKind::ModDecl],
    ..LayoutConfig::default()
  };
  for attr in [
    "#[cfg_attr(all(), macro_use)]\nmod a {",
    "mod a {\n    #![macro_use]\n",
  ] {
    let source = format!(
      "{}\n    macro_rules! two {{\n        () => {{\n            2\n        }};\n    }}\n}}\n\nfn main() {{\n    let _ = two!();\n}}\n",
      attr
    );
    let laid = arranged_with(&source, &config);
    assert!(
      laid.find("mod a").unwrap() < laid.find("fn main").unwrap(),
      "{}",
      laid
    );
  }
}

#[test]
fn test_modules_sort_as_the_standard_formatter_does() {
  // Without `r#`, by bytes; a test module declared last stands apart from the others.
  let source = "#[cfg(test)]\nmod tests;\nmod r#type;\nmod b;\nmod Z;\n";
  assert_eq!(
    arranged(source),
    "mod Z;\nmod b;\nmod r#type;\n\n#[cfg(test)]\nmod tests;\n"
  );
}

/// The moves are the fewest units out of their written order: one item brought up from the
/// end moves alone, not every item it passed.
#[test]
fn test_moves_are_the_fewest_units_out_of_order() {
  let source = "struct A;\nstruct B;\nstruct C;\nuse x;\n";
  let laid = layout(source, &LayoutConfig::default()).unwrap();
  assert_eq!(laid.moves.len(), 1);
  assert_eq!(laid.moves[0].identifier, "x");
  assert_eq!(
    laid.moves[0].before,
    Some((ItemKind::StructDecl, "A".to_string()))
  );
  assert_eq!(longest_increasing(&[3, 0, 1, 2]), [1, 2, 3].into());
  assert_eq!(longest_increasing(&[]), HashSet::new());
}

#[test]
fn test_mto_off_regions_keep_their_slot() {
  let source = "fn d() {}\n\n// mto: off\nfn c() {}\nfn b() {}\n// mto: on\n\nfn a() {}\n";
  assert_eq!(
    arranged(source),
    "fn a() {}\n\n// mto: off\nfn c() {}\nfn b() {}\n// mto: on\n\nfn d() {}\n"
  );
  // A region with no end runs to the end of the file.
  let open = "fn b() {}\n\n// mto: off\nfn z() {}\nfn y() {}\n";
  assert_eq!(arranged(open), open);
}

#[test]
fn test_name_sort_and_visibility_order() {
  let source = "fn parse_u16() {}\nfn Beta() {}\nfn alpha() {}\nfn parse_u8() {}\n";
  let with = |name_sort: NameSort| {
    let config = LayoutConfig {
      name_sort,
      ..LayoutConfig::default()
    };
    let text = layout(source, &config).unwrap().text;
    text
      .lines()
      .filter(|l| !l.is_empty())
      .map(|l| {
        l.trim_start_matches("fn ")
          .trim_end_matches("() {}")
          .to_string()
      })
      .collect::<Vec<_>>()
  };
  assert_eq!(
    with(NameSort::Version),
    ["Beta", "alpha", "parse_u8", "parse_u16"]
  );
  assert_eq!(
    with(NameSort::Bytes),
    ["Beta", "alpha", "parse_u16", "parse_u8"]
  );
  assert_eq!(
    with(NameSort::CaseInsensitive),
    ["alpha", "Beta", "parse_u8", "parse_u16"]
  );

  let config = LayoutConfig {
    visibility_order: vec![Visibility::Private, Visibility::Pub],
    ..LayoutConfig::default()
  };
  let source = "pub(crate) fn c() {}\npub fn b() {}\nfn a() {}\n";
  assert_eq!(
    layout(source, &config).unwrap().text,
    "fn a() {}\n\npub fn b() {}\n\npub(crate) fn c() {}\n"
  );
}

#[test]
fn test_no_order_keys_keeps_the_written_order() {
  let config = with_keys(Vec::new());
  let source = "use z;\nuse a;\n\n#[cfg(test)]\nmod tests {}\n\nconst ZZ: u8 = 1;\n\nfn timer() {}\n\nstruct S;\n\nfn double_fault() {}\n";
  let once = arranged_with(source, &config);
  assert_eq!(
    once,
    "use a;\nuse z;\n\nconst ZZ: u8 = 1;\n\nfn timer() {}\n\nstruct S;\n\nfn double_fault() {}\n\n#[cfg(test)]\nmod tests {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
}

/// Functions first must not lift a function above the macro call that defines the macro
/// it uses (tokio's `cfg_trace! { macro_rules! trace { ... } }`).
#[test]
fn test_nothing_moves_above_a_macro_call_that_defines_macros() {
  let config = LayoutConfig {
    type_order: vec![ItemKind::FnDecl, ItemKind::MacroDecl],
    ..LayoutConfig::default()
  };
  let source = "cfg_trace! {\n    macro_rules! trace { () => {} }\n}\n\nfn b() { trace!(); }\n\nfn a() { trace!(); }\n";
  let laid = layout(source, &config).unwrap();
  let call = laid.text.find("cfg_trace!").unwrap();
  assert!(laid.text.find("fn a").unwrap() > call, "{}", laid.text);
  assert!(laid.text.find("fn b").unwrap() > call, "{}", laid.text);
  let violations = crate::verify::verify_layout(source, &laid.text, laid.edition);
  assert!(violations.is_empty(), "{:?}", violations);
}

#[test]
fn test_only_markers_start_sections_in_marker_mode() {
  let source =
    "fn d() {}\n\n// note\n\nfn c() {}\n\n// mto: section -- helpers\n\nfn b() {}\n\nfn a() {}\n";
  let once = arranged_with(source, &with_sections(Sections::Markers));
  assert_eq!(
    once,
    "// note\n\nfn c() {}\n\nfn d() {}\n\n// mto: section -- helpers\n\nfn a() {}\n\nfn b() {}\n"
  );
  assert_eq!(
    arranged_with(&once, &with_sections(Sections::Markers)),
    once
  );
}

#[test]
fn test_paragraphs_move_whole_with_keep_paragraphs() {
  let config = LayoutConfig {
    keep_paragraphs: true,
    ..LayoutConfig::default()
  };
  // `y` and `b` were written together: they stay so, placed by `y`, and keep their
  // tight spacing; `a` and `z` are apart from everything and sort as usual.
  let source = "fn z() {}\n\nfn y() {}\nfn b() {}\n\nfn a() {}\n";
  let once = arranged_with(source, &config);
  assert_eq!(once, "fn a() {}\n\nfn y() {}\nfn b() {}\n\nfn z() {}\n");
  assert_eq!(arranged_with(&once, &config), once);
  // Off, the same file sorts item by item.
  assert_eq!(
    arranged(source),
    "fn a() {}\n\nfn b() {}\n\nfn y() {}\n\nfn z() {}\n"
  );
}

#[test]
fn test_pinned_items_keep_their_place() {
  let skip = "fn z() {}\n#[rustfmt::skip]\nfn m() {}\nfn a() {}\n";
  assert_eq!(
    arranged(skip),
    "fn a() {}\n\n#[rustfmt::skip]\nfn m() {}\n\nfn z() {}\n"
  );
  let attached = "fn z() {}\n// mto: pin\nfn m() {}\nfn a() {}\n";
  assert_eq!(
    arranged(attached),
    "fn a() {}\n\n// mto: pin\nfn m() {}\n\nfn z() {}\n"
  );
  let loose = "fn z() {}\n\n// mto: pin\n\nfn m() {}\nfn a() {}\n";
  assert_eq!(
    arranged(loose),
    "fn a() {}\n\n// mto: pin\n\nfn m() {}\n\nfn z() {}\n"
  );
}

#[test]
fn test_pinned_items_keep_their_slot() {
  let source = "fn b() {}\nextern crate alloc;\nfn a() {}\n";
  assert_eq!(
    arranged(source),
    "fn a() {}\n\nextern crate alloc;\n\nfn b() {}\n"
  );
}

/// `reexports`: `pub use` as a group of its own after the other imports, or after every
/// other item of its section.
#[test]
fn test_reexports_go_apart_or_last() {
  let source = "pub use crate::b::B;\nuse std::fmt;\npub(crate) use crate::a::A;\nuse crate::c::C;\n\nfn f() {}\n";
  let apart = LayoutConfig {
    reexports: Reexports::Apart,
    ..LayoutConfig::default()
  };
  let once = arranged_with(source, &apart);
  assert_eq!(
    once,
    "use std::fmt;\n\nuse crate::c::C;\n\npub(crate) use crate::a::A;\npub use crate::b::B;\n\nfn f() {}\n"
  );
  assert_eq!(arranged_with(&once, &apart), once);
  let last = LayoutConfig {
    reexports: Reexports::Last,
    ..LayoutConfig::default()
  };
  let once = arranged_with(source, &last);
  assert_eq!(
    once,
    "use std::fmt;\n\nuse crate::c::C;\n\nfn f() {}\n\npub(crate) use crate::a::A;\npub use crate::b::B;\n"
  );
  assert_eq!(arranged_with(&once, &last), once);
  assert!(crate::verify::verify_layout(source, &once, crate::syntax::Edition::E2021).is_empty());
}

#[test]
fn test_reorder_impl_items_as_the_standard_formatter_does() {
  let source = "impl Iterator for D {\n    fn next(&mut self) {}\n    m!();\n    const B: u8 = 1;\n    type Z = impl Sized;\n    type Item = i32;\n    const A: u8 = 0;\n}\n";
  let config = LayoutConfig {
    reorder_impl_items: true,
    ..LayoutConfig::default()
  };
  let once = layout(source, &config).unwrap().text;
  assert_eq!(
    once,
    "impl Iterator for D {\n    type Item = i32;\n\n    type Z = impl Sized;\n\n    const A: u8 = 0;\n    const B: u8 = 1;\n\n    m!();\n\n    fn next(&mut self) {}\n}\n"
  );
  assert_eq!(layout(&once, &config).unwrap().text, once, "stable");
  assert!(crate::verify::verify_layout(source, &once, Edition::E2021).is_empty());
  // Off by default: impl bodies are left alone.
  assert_eq!(arranged(source), source);
}

#[test]
fn test_rustfmt_skip_on_the_file_leaves_it_alone() {
  let source = "#![rustfmt::skip]\nfn b() {}\nfn a() {}\n";
  assert_eq!(arranged(source), source);
}

#[test]
fn test_section_comments_move_with_their_item() {
  let source = "fn b() {}\n\n// Section A\n\n// about a\nfn a() {}\n";
  assert_eq!(
    arranged(source),
    "// Section A\n\n// about a\nfn a() {}\n\nfn b() {}\n"
  );
}

/// With no blank lines between items, a comment that stood apart from the item below it
/// still keeps one blank line, since touching the item would attach it there; so do import
/// groups, which the standard formatter would otherwise sort into one. Found by the race's
/// layout rows on the standard formatter's test inputs, which the verifier refused to write.
#[test]
fn test_spacing_zero_keeps_loose_comments_loose() {
  let config = LayoutConfig {
    blank_lines_between_blocks: 0,
    ..LayoutConfig::default()
  };
  let cases = [
    // A header comment.
    (
      "// licence\n\nfn b() {}\n\nfn a() {}\n",
      "// licence\n\nfn a() {}\nfn b() {}\n",
    ),
    // A section comment inside a unit.
    (
      "fn z() {}\n\n// section\n\nfn a() {}\n",
      "// section\n\nfn a() {}\nfn z() {}\n",
    ),
    // Import groups: the standard formatter would sort touching groups into one.
    (
      "use crate::b;\nuse std::a;\nfn f() {}\n",
      "use std::a;\n\nuse crate::b;\nfn f() {}\n",
    ),
    // A test module declaration sorted last, likewise.
    (
      "#[cfg(test)]\nmod tests;\nmod b;\nmod a;\n",
      "mod a;\nmod b;\n\n#[cfg(test)]\nmod tests;\n",
    ),
    // Comments after the last item.
    (
      "fn b() {}\n\nfn a() {}\n\n// the end\n",
      "fn a() {}\nfn b() {}\n\n// the end\n",
    ),
  ];
  for (source, expected) in cases {
    let laid = layout(source, &config).unwrap();
    assert_eq!(laid.text, expected);
    let violations = crate::verify::verify_layout(source, &laid.text, laid.edition);
    assert!(violations.is_empty(), "{:?}", violations);
    assert_eq!(layout(&laid.text, &config).unwrap().text, laid.text);
  }
}

/// `test_helpers = "last"`: `#[cfg(test)]` items outside a test module go to the end,
/// before the test modules; imports stay with the imports.
#[test]
fn test_test_helpers_go_with_the_tests() {
  let config = LayoutConfig {
    test_helpers_last: true,
    ..LayoutConfig::default()
  };
  let source = "use std::fmt;\n#[cfg(test)]\nuse std::io;\n\n#[cfg(test)]\nfn fixture() {}\n\n#[cfg(test)]\nmod tests {}\n\nfn z() {}\n\n#[cfg(test)]\nimpl S {}\n\nstruct S;\n\nfn a() {}\n";
  let once = arranged_with(source, &config);
  assert_eq!(
    once,
    "use std::fmt;\n#[cfg(test)]\nuse std::io;\n\nstruct S;\n\nfn a() {}\n\nfn z() {}\n\n#[cfg(test)]\nimpl S {}\n\n#[cfg(test)]\nfn fixture() {}\n\n#[cfg(test)]\nmod tests {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
  assert!(crate::verify::verify_layout(source, &once, crate::syntax::Edition::E2021).is_empty());
  // With the written order kept, the helpers still join the tests.
  let written = LayoutConfig {
    order_keys: Vec::new(),
    ..config
  };
  let laid = arranged_with(source, &written);
  assert!(
    laid.ends_with("fn a() {}\n\n#[cfg(test)]\nfn fixture() {}\n\n#[cfg(test)]\nimpl S {}\n\n#[cfg(test)]\nmod tests {}\n"),
    "{}",
    laid
  );
  assert_eq!(arranged_with(&laid, &written), laid);
}

#[test]
fn test_test_module_last_or_in_place() {
  let source = "fn b() {}\n#[cfg(test)]\nmod tests {}\nfn a() {}\n";
  assert_eq!(
    arranged(source),
    "fn a() {}\n\nfn b() {}\n\n#[cfg(test)]\nmod tests {}\n"
  );
  let config = LayoutConfig {
    test_module_in_place: true,
    ..LayoutConfig::default()
  };
  assert_eq!(
    layout(source, &config).unwrap().text,
    "fn a() {}\n\n#[cfg(test)]\nmod tests {}\n\nfn b() {}\n"
  );
}

#[test]
fn test_trailing_comments_stay_last() {
  let source = "fn b() {}\nfn a() {}\n\n// the end   \n";
  assert_eq!(
    arranged(source),
    "fn a() {}\n\nfn b() {}\n\n// the end   \n"
  );
}

#[test]
fn test_unsorted_imports_keep_their_written_order() {
  let config = LayoutConfig {
    sort_imports: false,
    ..LayoutConfig::default()
  };
  let source = "use foo::{bar};\nuse crate::x;\nuse foo::{\n    bar, // abc\n};\nuse std::a;\n";
  assert_eq!(layout(source, &config).unwrap().text, source);
}

#[test]
fn test_visibility_first_puts_the_public_items_first_with_their_impls() {
  let config = with_keys(vec![OrderKey::Visibility, OrderKey::Kind, OrderKey::Name]);
  let source = "use std::fmt;\n\nfn helper() {}\n\nstruct Inner;\n\npub fn api() {}\n\npub struct Outer;\n\nimpl Outer {}\n\nimpl Inner {}\n";
  let once = arranged_with(source, &config);
  assert_eq!(
    once,
    "use std::fmt;\n\npub struct Outer;\n\nimpl Outer {}\n\npub fn api() {}\n\nstruct Inner;\n\nimpl Inner {}\n\nfn helper() {}\n"
  );
  assert_eq!(arranged_with(&once, &config), once);
  // Without `kind`, imports and modules still come first; the rest by visibility, name.
  let no_kind = with_keys(vec![OrderKey::Visibility, OrderKey::Name]);
  let laid = arranged_with(source, &no_kind);
  // By name across kinds: `Outer` before `api`, as version sort orders them.
  assert!(
    laid.starts_with("use std::fmt;\n\npub struct Outer;\n\nimpl Outer {}\n\npub fn api() {}"),
    "{}",
    laid
  );
  assert_eq!(arranged_with(&laid, &no_kind), laid);
}

fn with_helpers() -> LayoutConfig {
  LayoutConfig {
    keep_helpers_near: true,
    ..LayoutConfig::default()
  }
}

fn with_keys(keys: Vec<OrderKey>) -> LayoutConfig {
  LayoutConfig {
    order_keys: keys,
    ..LayoutConfig::default()
  }
}

fn with_sections(sections: Sections) -> LayoutConfig {
  LayoutConfig {
    sections,
    ..LayoutConfig::default()
  }
}
