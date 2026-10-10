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
    // A condition other than `rustfmt` is not a skip.
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
fn test_imports_sort_as_rustfmt_does_for_the_style_edition() {
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
}

#[test]
fn test_modules_sort_as_rustfmt_does() {
    // Without `r#`, by bytes; a test module declared last stands apart from the others.
    let source = "#[cfg(test)]\nmod tests;\nmod r#type;\nmod b;\nmod Z;\n";
    assert_eq!(
        arranged(source),
        "mod Z;\nmod b;\nmod r#type;\n\n#[cfg(test)]\nmod tests;\n"
    );
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
        text.lines()
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
fn test_reorder_impl_items_as_rustfmt_does() {
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
/// groups, which rustfmt would otherwise sort into one. Found by the race's
/// layout rows on rustfmt's test inputs, which the verifier refused to write.
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
        // Import groups: rustfmt would sort touching groups into one.
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
