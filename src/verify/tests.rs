use super::*;

const INPUT: &str = "#!/usr/bin/env rust\n//! Crate docs.\n#![allow(dead_code)]\n\n// Section: helpers\n\n/// Adds.\nfn b() {} // trailing\n\nmacro_rules! first { () => {} }\nmacro_rules! second { () => {} }\n\n// about a\nfn a() {\n    // inside\n}\n";

/// The same items in another order, as layout may produce.
const REORDERED: &str = "#!/usr/bin/env rust\n//! Crate docs.\n#![allow(dead_code)]\n\nmacro_rules! first { () => {} }\nmacro_rules! second { () => {} }\n\n// about a\nfn a() {\n    // inside\n}\n\n// Section: helpers\n\n/// Adds.\nfn b() {} // trailing\n";

fn rules(input: &str, output: &str) -> Vec<Rule> {
    verify_layout(input, output, Edition::E2021)
        .into_iter()
        .map(|v| v.rule)
        .collect()
}

#[test]
fn test_a_reordering_passes() {
    assert_eq!(rules(INPUT, REORDERED), vec![]);
    assert_eq!(rules(INPUT, INPUT), vec![]);
}

#[test]
fn test_blank_lines_between_items_may_change() {
    let squeezed = REORDERED.replace("\n\n", "\n");
    // Squeezing the blank line under the section comment attaches it to `fn b`, which is
    // reported; everywhere else blank lines are free.
    assert_eq!(rules(INPUT, &squeezed), vec![Rule::CommentReattached]);
    let spaced = REORDERED.replace("}\n\nmacro_rules! second", "}\n\n\n\nmacro_rules! second");
    let spaced = spaced.replace(
        "macro_rules! second { () => {} }\n",
        "macro_rules! second { () => {} }\n\n\n",
    );
    assert_eq!(rules(INPUT, &spaced), vec![]);
}

#[test]
fn test_bug_report_names_rules_and_shows_the_input() {
    let violations = verify_layout(INPUT, &REORDERED.replace("// inside", ""), Edition::E2021);
    let report = report::bug_report(
        "src/lib.rs",
        &violations,
        Some("fn a() {\n    // inside\n}"),
    );
    assert!(report.starts_with("mto: internal error: the verifier stopped the write of src/lib.rs; the file is unchanged."), "{}", report);
    assert!(
        report.contains("  item-lost: 1 item(s) lost or changed: `fn a() {`"),
        "{}",
        report
    );
    assert!(
        report.contains("  comment-lost: 1 comment(s) lost: `// inside`"),
        "{}",
        report
    );
    assert!(
        report.contains("  ----\n  fn a() {\n      // inside\n  }\n  ----\n"),
        "{}",
        report
    );
}

#[test]
fn test_inline_modules_are_scopes_of_their_own() {
    let input =
        "mod a {\n    //! Docs.\n    fn y() {}\n    fn x() {}\n}\nmod b {\n    fn z() {}\n}\n";
    // Reordering inside a module, and modules among themselves, is fine.
    let reordered =
        "mod b {\n    fn z() {}\n}\nmod a {\n    //! Docs.\n    fn x() {}\n\n    fn y() {}\n}\n";
    assert_eq!(rules(input, reordered), vec![]);
    // Moving an item to another module is not.
    let moved =
        "mod a {\n    //! Docs.\n    fn y() {}\n}\nmod b {\n    fn x() {}\n    fn z() {}\n}\n";
    assert_eq!(rules(input, moved), vec![Rule::ItemLost, Rule::ItemAdded]);
    // Nor is changing an item inside a module, or moving a module's inner doc down.
    let changed = input.replace("fn x() {}", "fn x() { }");
    assert_eq!(
        rules(input, &changed),
        vec![Rule::ItemLost, Rule::ItemAdded]
    );
    let header =
        "mod a {\n    fn y() {}\n    //! Docs.\n    fn x() {}\n}\nmod b {\n    fn z() {}\n}\n";
    assert!(rules(input, header).contains(&Rule::HeaderChanged));
    // Macro order is kept within each module.
    let macros = "mod m {\n    macro_rules! p { () => {} }\n    macro_rules! q { () => {} }\n}\n";
    let swapped = "mod m {\n    macro_rules! q { () => {} }\n    macro_rules! p { () => {} }\n}\n";
    // `q` came after `p`, so it also moved above a definition it could use.
    assert_eq!(
        rules(macros, swapped),
        vec![Rule::MacroOrderChanged, Rule::MacroUserMoved]
    );
}

/// A macro defined inside an item macro call (`cfg_if! { macro_rules! m {...} }`) is as
/// order-sensitive as one written plainly: what came after the call may not move above it.
/// Found by laying out tokio with functions first, which then failed to compile.
#[test]
fn test_items_stay_below_macros_defined_in_macro_calls() {
    let input = "cfg_trace! {\n    macro_rules! trace { () => {} }\n}\nfn a() { trace!(); }\n";
    let moved = "fn a() { trace!(); }\ncfg_trace! {\n    macro_rules! trace { () => {} }\n}\n";
    assert_eq!(rules(input, moved), vec![Rule::MacroUserMoved]);
    // A call that defines nothing does not hold items back.
    let plain = "thread_local! { static X: u8 = 0; }\nfn a() {}\n";
    let swapped = "fn a() {}\nthread_local! { static X: u8 = 0; }\n";
    assert_eq!(rules(plain, swapped), vec![]);
}

#[test]
fn test_macro_use_modules_count_in_their_parent() {
    let input = "#[macro_use]\nmod macros {\n    macro_rules! m { () => {} }\n}\nmacro_rules! n { () => {} }\nmod a {\n    macro_rules! p { () => {} }\n}\nmod b {\n    macro_rules! q { () => {} }\n}\n";
    // Modules without `#[macro_use]` may swap; their macros stay local.
    let swapped = "#[macro_use]\nmod macros {\n    macro_rules! m { () => {} }\n}\nmacro_rules! n { () => {} }\nmod b {\n    macro_rules! q { () => {} }\n}\nmod a {\n    macro_rules! p { () => {} }\n}\n";
    assert_eq!(rules(input, swapped), vec![]);
    // A `#[macro_use]` module may not move below a macro defined after it.
    let moved = "macro_rules! n { () => {} }\n#[macro_use]\nmod macros {\n    macro_rules! m { () => {} }\n}\nmod a {\n    macro_rules! p { () => {} }\n}\nmod b {\n    macro_rules! q { () => {} }\n}\n";
    assert_eq!(
        rules(input, moved),
        vec![Rule::MacroOrderChanged, Rule::MacroUserMoved]
    );
}

#[test]
fn test_minimize_keeps_only_what_fails() {
    // "Fails" while the text still holds `fn bad`; everything else is dropped.
    let input = "use std::io;\n\n// note\nfn good() {}\n\nfn bad() {}\n\nstruct S;\n";
    let minimal = report::minimize(input, Edition::E2021, &|text| text.contains("fn bad"));
    assert_eq!(minimal.trim(), "fn bad() {}");
}

#[test]
fn test_mutation_attached_comment_came_loose() {
    // A blank line between `// about a` and `fn a` detaches the comment.
    let output = REORDERED.replace("// about a\nfn a()", "// about a\n\nfn a()");
    assert_eq!(rules(INPUT, &output), vec![Rule::CommentReattached]);
}

#[test]
fn test_mutation_bom_and_line_endings() {
    let bom_input = format!("\u{feff}{}", INPUT);
    assert_eq!(rules(&bom_input, REORDERED), vec![Rule::BomChanged]);
    assert_eq!(
        rules(INPUT, &format!("\u{feff}{}", REORDERED)),
        vec![Rule::BomChanged]
    );
    let crlf_input = INPUT.replace('\n', "\r\n");
    assert_eq!(rules(&crlf_input, &REORDERED.replace('\n', "\r\n")), vec![]);
    assert_eq!(
        rules(&crlf_input, REORDERED),
        vec![Rule::LineEndingsChanged]
    );
}

#[test]
fn test_mutation_changed_byte_in_item() {
    let output = REORDERED.replace("fn a() {", "fn a () {");
    assert_eq!(rules(INPUT, &output), vec![Rule::ItemLost, Rule::ItemAdded]);
}

#[test]
fn test_mutation_comment_inside_item_changed() {
    let output = REORDERED.replace("// inside", "// inside!");
    assert_eq!(
        rules(INPUT, &output),
        vec![
            Rule::ItemLost,
            Rule::ItemAdded,
            Rule::CommentLost,
            Rule::CommentAdded
        ]
    );
}

#[test]
fn test_mutation_dropped_item() {
    let output = REORDERED.replace("macro_rules! second { () => {} }\n", "");
    assert_eq!(
        rules(INPUT, &output),
        vec![Rule::ItemLost, Rule::MacroOrderChanged]
    );
}

#[test]
fn test_mutation_duplicated_item() {
    let output = format!("{}\nfn a() {{\n    // inside\n}}\n", REORDERED);
    assert_eq!(
        rules(INPUT, &output),
        vec![Rule::ItemAdded, Rule::CommentAdded]
    );
}

#[test]
fn test_mutation_header_moved_or_changed() {
    let moved = REORDERED.replace("#![allow(dead_code)]\n", "") + "#![allow(dead_code)]\n";
    assert!(rules(INPUT, &moved).contains(&Rule::HeaderChanged));
    let changed = REORDERED.replace("//! Crate docs.", "//! Crate docs!");
    assert_eq!(rules(INPUT, &changed), vec![Rule::HeaderChanged]);
    // Trimming trailing spaces off a doc comment is a change, and says so.
    let spaced = INPUT.replace("//! Crate docs.", "//! Crate docs.  ");
    let found = verify_layout(&spaced, REORDERED, Edition::E2021);
    assert_eq!(found.len(), 1);
    assert!(
        found[0]
            .detail
            .starts_with("header differs only in trailing whitespace"),
        "{}",
        found[0].detail
    );
}

#[test]
fn test_mutation_lost_loose_comment() {
    let output = REORDERED.replace("// Section: helpers\n\n", "");
    assert_eq!(rules(INPUT, &output), vec![Rule::CommentLost]);
}

#[test]
fn test_mutation_macro_order() {
    let output = REORDERED.replace(
        "macro_rules! first { () => {} }\nmacro_rules! second { () => {} }",
        "macro_rules! second { () => {} }\nmacro_rules! first { () => {} }",
    );
    assert_eq!(
        rules(INPUT, &output),
        vec![Rule::MacroOrderChanged, Rule::MacroUserMoved]
    );
}

#[test]
fn test_mutation_output_does_not_parse() {
    let output = REORDERED.replacen("fn a() {", "fn a() {{", 1);
    assert!(rules(INPUT, &output).contains(&Rule::OutputBroken));
}

#[test]
fn test_mutation_stray_text() {
    // A stray `;` is also a syntax error, so both rules see it.
    let output = format!("{};\n", REORDERED);
    assert_eq!(
        rules(INPUT, &output),
        vec![Rule::OutputBroken, Rule::OtherChanged]
    );
}
