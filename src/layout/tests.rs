use super::*;

fn arranged(source: &str) -> String {
    layout(source, &LayoutConfig::default()).unwrap().text
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
fn test_imports_group_and_declarations_stay_together() {
    let source = "use crate::x;\nuse serde::Y;\nuse std::io;\nmod b;\nmod a;\n";
    assert_eq!(
        arranged(source),
        "mod a;\nmod b;\n\nuse std::io;\n\nuse serde::Y;\n\nuse crate::x;\n"
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
fn test_pinned_items_keep_their_slot() {
    let source = "fn b() {}\nextern crate alloc;\nfn a() {}\n";
    assert_eq!(
        arranged(source),
        "fn a() {}\n\nextern crate alloc;\n\nfn b() {}\n"
    );
}

#[test]
fn test_section_comments_move_with_their_item() {
    let source = "fn b() {}\n\n// Section A\n\n// about a\nfn a() {}\n";
    assert_eq!(
        arranged(source),
        "// Section A\n\n// about a\nfn a() {}\n\nfn b() {}\n"
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
