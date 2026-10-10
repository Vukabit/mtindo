use super::*;
use crate::verify::rewrites::REWRITES;

fn rules(input: &str, output: &str) -> Vec<Rule> {
    verify_format(input, output, Edition::E2021)
        .into_iter()
        .map(|v| v.rule)
        .collect()
}

fn same(input: &str, output: &str) {
    let found = verify_format(input, output, Edition::E2021);
    assert!(found.is_empty(), "{:?}\n{}\n---\n{}", found, input, output);
    // Every rewrite is allowed in both directions.
    let back = verify_format(output, input, Edition::E2021);
    assert!(back.is_empty(), "{:?}\n{}\n---\n{}", back, output, input);
}

#[test]
fn test_broken_output_is_reported_alone() {
    assert_eq!(rules("fn f() {}\n", "fn f( {}\n"), vec![Rule::OutputBroken]);
}

#[test]
fn test_changes_of_meaning_are_caught() {
    let changed = |input: &str, output: &str| {
        assert!(
            rules(input, output).contains(&Rule::MeaningChanged),
            "{}\n---\n{}",
            input,
            output
        );
    };
    changed("fn f() { g(1); }\n", "fn f() { g(2); }\n");
    changed("fn f() { g(a, b); }\n", "fn f() { g(b, a); }\n");
    // A one-element tuple keeps its comma.
    changed("fn f() { let t = (a,); }\n", "fn f() { let t = (a); }\n");
    // The comma before a block argument is not a trailing comma.
    changed("fn f() { g(a, { b }); }\n", "fn f() { g(a { b }); }\n");
    // A block with a statement is not its expression.
    changed(
        "fn f() { let c = || { g(); x }; }\n",
        "fn f() { let c = || x; }\n",
    );
    // A semicolon after a plain expression ending a block changes the block's value.
    changed("fn f() -> u8 { g(); }\n", "fn f() -> u8 { g() }\n");
    // Derives are compared as a list.
    changed(
        "#[derive(Debug)]\nstruct S;\n",
        "#[derive(Clone)]\nstruct S;\n",
    );
    // Imports compare by what they import.
    changed("use a::b;\n", "use a::c;\n");
    // Order matters outside import and module runs.
    changed("fn a() {}\nfn b() {}\n", "fn b() {}\nfn a() {}\n");
    // Doc comments are attributes: their text counts.
    changed("/// One.\nfn a() {}\n", "/// Two.\nfn a() {}\n");
    // A float is not an integer.
    changed("fn f() { g(1.); }\n", "fn f() { g(1); }\n");
    // An import moved into a different set of paths.
    changed("use a::{b, c};\n", "use a::b;\nuse a::d;\n");
    changed("use a::b;\nuse a::c;\n", "use a::b;\n");
    // Whitespace inside a string without a continuation is part of it.
    changed("fn f() { g(\"a  b\"); }\n", "fn f() { g(\"a b\"); }\n");
    // A `loop` yielding a value through `break` keeps the meaning of its `;`.
    changed(
        "fn f() { loop { break 5 }; }\n",
        "fn f() { loop { break 5 } }\n",
    );
    // A block-like expression ending a block is its value.
    changed(
        "fn f() -> u8 { if a { 1 } else { 2 }; }\n",
        "fn f() -> u8 { if a { 1 } else { 2 } }\n",
    );
}

#[test]
fn test_comment_words_must_all_survive() {
    assert_eq!(
        rules("fn f() {} // one two\n", "fn f() {} // one\n"),
        vec![Rule::CommentLost]
    );
}

#[test]
fn test_comments_must_all_survive() {
    assert_eq!(
        rules("fn f() {} // gone\n", "fn f() {}\n"),
        vec![Rule::CommentLost]
    );
    assert_eq!(
        rules("fn f() {}\n", "fn f() {} // new\n"),
        vec![Rule::CommentAdded]
    );
}

#[test]
fn test_each_allowed_rewrite() {
    // trailing-comma
    same("fn f(a: u8,) { g(a, b,); }\n", "fn f(a: u8) { g(a, b); }\n");
    same(
        "struct S<T,> where T: Copy, { a: T, }\n",
        "struct S<T> where T: Copy { a: T }\n",
    );
    same("fn f() { vec![1, 2,]; }\n", "fn f() { vec![1, 2]; }\n");
    // closure-body-braces
    same(
        "fn f() { let c = |x| { x + 1 }; }\n",
        "fn f() { let c = |x| x + 1; }\n",
    );
    // match-arm-body and the arm comma
    same(
        "fn f() { match a { A => { 1 }, B => 2 } }\n",
        "fn f() { match a { A => 1, B => 2, } }\n",
    );
    // match-arm-leading-pipe
    same(
        "fn f() { match a { | A | B => 1 } }\n",
        "fn f() { match a { A | B => 1 } }\n",
    );
    // nested-parens
    same("fn f() { let a = ((b)); }\n", "fn f() { let a = (b); }\n");
    // trailing-semicolon
    same(
        "fn f() -> u8 { return 1; }\nfn g() { loop { break; } }\n",
        "fn f() -> u8 { return 1 }\nfn g() { loop { break } }\n",
    );
    // stray-semicolon: an empty statement in a block either way; among items, where a lone
    // `;` is an error, only its removal.
    same("fn f() { a(); ; }\n", "fn f() { a(); }\n");
    assert!(
        verify_format(
            "fn f() {};\nstruct S;\n",
            "fn f() {}\nstruct S;\n",
            Edition::E2021
        )
        .is_empty()
    );
    // explicit-abi
    same(
        "extern { fn g(); }\nextern fn h() {}\n",
        "extern \"C\" { fn g(); }\nextern \"C\" fn h() {}\n",
    );
    // range-pattern
    same(
        "fn f() { match a { 1...5 => 0, _ => 1 } }\n",
        "fn f() { match a { 1..=5 => 0, _ => 1 } }\n",
    );
    // visibility-in
    same(
        "pub(in crate) fn f() {}\npub(in super) fn g() {}\n",
        "pub(crate) fn f() {}\npub(super) fn g() {}\n",
    );
    // merge-derives
    same(
        "#[derive(Debug)]\n#[derive(Clone, Copy)]\nstruct S;\n",
        "#[derive(Debug, Clone, Copy)]\nstruct S;\n",
    );
    // use-trees
    same(
        "use a::{b};\nuse c::{self, d::{e, f}};\n",
        "use a::b;\nuse c::{d::{f, e}, self};\n",
    );
    // import-and-module-order
    same(
        "use b::x;\nuse a::y;\nmod z;\nmod m;\n",
        "use a::y;\nuse b::x;\nmod m;\nmod z;\n",
    );
    // use-trees across items: merged, split and emptied.
    same("use a::b;\nuse a::c;\nuse d::{};\n", "use a::{b, c};\n");
    // import-and-module-order for extern crates.
    same(
        "extern crate b;\nextern crate a;\n",
        "extern crate a;\nextern crate b;\n",
    );
    // impl-item-order
    same(
        "impl T for S { fn f() {} type X = u8; }\n",
        "impl T for S { type X = u8; fn f() {} }\n",
    );
    // doc-text: rewrapped, and `#[doc]` as `///`.
    same(
        "/// A long line that\n/// wraps.\nfn a() {}\n#[doc = \"Two words.\"]\nfn b() {}\n",
        "/// A long line\n/// that wraps.\nfn a() {}\n/// Two words.\nfn b() {}\n",
    );
    // comment-text: rewrapped and re-marked.
    same(
        "fn f() {} /* one two three */\n",
        "fn f() {} // one two\n// three\n",
    );
    // number-literals
    same("fn f() { g(1., 0xAB); }\n", "fn f() { g(1.0, 0xab); }\n");
    // empty-closure-params and the closure trailing comma
    same(
        "fn f() { let c = | | 1; let d = |a, b,| a; }\n",
        "fn f() { let c = || 1; let d = |a, b| a; }\n",
    );
    // wildcard-suffix
    same(
        "fn f() { let (a, _, _) = t; }\n",
        "fn f() { let (a, ..) = t; }\n",
    );
    // field-init-shorthand
    same(
        "fn f() { let s = S { x: x, y }; }\n",
        "fn f() { let s = S { x, y }; }\n",
    );
    // try-shorthand
    same(
        "fn f() { let a = try!(g(1)); }\n",
        "fn f() { let a = g(1)?; }\n",
    );
    // match-arm-body with a block-like expression.
    same(
        "fn f() { match a { A => { loop {} } } }\n",
        "fn f() { match a { A => loop {} } }\n",
    );
    // block-semicolon
    same(
        "fn f() { while a {}; g(); loop {}; }\n",
        "fn f() { while a {} g(); loop {} }\n",
    );
    same(
        "fn f() { if a {} else {}; g(); }\n",
        "fn f() { if a {} else {} g(); }\n",
    );
    // empty-generics
    same(
        "struct S<>;\nstruct B<T:>;\nfn f() { String::<>::from(1); }\ntype F = Box<Fn::(i32)>;\n",
        "struct S;\nstruct B<T>;\nfn f() { String::from(1); }\ntype F = Box<Fn(i32)>;\n",
    );
    // A leading `|` anywhere in a pattern; nested blocks in a closure.
    same(
        "fn f() { let (| a, b) = t; let c = |x| { { x } }; }\n",
        "fn f() { let (a, b) = t; let c = |x| x; }\n",
    );
    // Doc comments on a `use` count as doc text too.
    same(
        "/// Re-exported\n/// here.\npub use a::b;\n",
        "/// Re-exported here.\npub use a::b;\n",
    );
    same(
        "fn f(x: Item::<T>) where {}\nstruct S6<'a:, T: Copy +>;\n",
        "fn f(x: Item<T>) {}\nstruct S6<'a, T: Copy>;\n",
    );
    same("use ::log::info;\n", "use log::info;\n");
    same(
        "fn f() { (1.).abs(); match a { A => { return 1; } } }\n",
        "fn f() { 1.0.abs(); match a { A => return 1 } }\n",
    );
    same(
        "fn f() { g(\"a \\\n        b\"); let v = vec!(1, 2); }\n",
        "fn f() { g(\"a \\\n    b\"); let v = vec![1, 2]; }\n",
    );
    same(
        "macro_rules! m { () => (); ($a:expr) => { $a } }\n",
        "macro_rules! m { () => {}; ($a:expr) => { $a }; }\n",
    );
    same(
        "fn f() { g(\"a long string\"); }\n",
        "fn f() { g(\"a long \\\n         string\"); }\n",
    );
    same(
        "fn f() { let m = 7f32; }\npub(in self) fn g() {}\n",
        "fn f() { let m = 7.0f32; }\npub(self) fn g() {}\n",
    );
    same(
        "fn f() { let S(x, _, ..) = s; }\n",
        "fn f() { let S(x, ..) = s; }\n",
    );
    same(
        "fn f() { S { #[a] z: z }; }\n",
        "fn f() { S { #[a] z }; }\n",
    );
    same(
        "#[doc = \"say \\\"hi\\\"\"]\nfn f() {}\n",
        "/// say \"hi\"\nfn f() {}\n",
    );
    assert_eq!(REWRITES.len(), 26);
}

#[test]
fn test_whitespace_and_comment_placement_are_free() {
    same(
        "fn f(a:u8,b:u8)->u8{a+b}// sum\n",
        "fn f(a: u8, b: u8) -> u8 {\n    a + b\n} // sum\n",
    );
    same(
        "fn f() {\n        /* note\n           more */\n    g();\n}\n",
        "fn f() {\n    /* note\n    more */\n    g();\n}\n",
    );
}
