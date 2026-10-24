use super::*;
use crate::options::layout::{NameSort, TraitImplSort};

fn arranged(source: &str) -> String {
  layout(source, &LayoutConfig::default()).unwrap().text
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

#[test]
fn test_code_only_valid_in_an_older_edition_still_parses() {
  // `async` and `dyn` are plain names in 2015.
  let source = "fn dyn() {}\nfn async() {}\n";
  let (edition, parsed) = parse_any_edition(source);
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
