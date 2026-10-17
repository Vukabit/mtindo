use super::*;
use crate::verify::format::verify_format;

fn formatted(source: &str) -> String {
  formatted_with(source, &FormatConfig::default())
}

fn formatted_with(source: &str, config: &FormatConfig) -> String {
  let config = config.clone();
  let once = format_file(source, Edition::E2021, &config);
  // Every output keeps the program's meaning and is stable.
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
fn test_arm_comments_aligned() {
  // Comments after arms aligned in their group, an arm's comma counted in its width; one
  // after a block's `}` one space after it.
  assert_eq!(
    formatted(
      "fn f() {\n    match x {\n        A => {\n            a();\n        }\t// Pop\n        EMPTY => return,  // w\n        NOTIFIED => return, // u\n        PARKED => {}   // g\n        _ => panic!(),\n    }\n}\n"
    ),
    "fn f() {\n    match x {\n        A => {\n            a();\n        } // Pop\n        EMPTY => return,    // w\n        NOTIFIED => return, // u\n        PARKED => {}        // g\n        _ => panic!(),\n    }\n}\n"
  );
}

#[test]
fn test_attribute_arguments_and_doc_comments() {
  // Keywords are names among an attribute's arguments, `true` a value; a doc comment after
  // an attribute starts its own line.
  assert_eq!(
    formatted(
      "#[salsa::tracked(lru\n\n=\t512,\treturns(ref))]\n#[t(a\n=\ntrue)] /// Doc.\nfn f() {}\n"
    ),
    "#[salsa::tracked(lru = 512, returns(ref))]\n#[t(a = true)]\n/// Doc.\nfn f() {}\n"
  );
  // Attributes a `rustfmt::skip::attributes` names are left as written.
  let skipped =
    "#[rustfmt::skip::attributes(other)]\nfn main() {\n    #[other(a,\nb)]\n    struct S {}\n}\n";
  assert_eq!(formatted(skipped), skipped);
}

#[test]
fn test_attributes_extern_crates_and_macro_calls() {
  // Inner attributes in a row with no blank line between; a `use`'s and an `extern crate`'s
  // attributes joined to it, another item's keeping a blank line; `extern crate` on one
  // line; `name! {` one space apart, `name!(` and `name![` touching; `vec![x; n]` on one
  // line.
  assert_eq!(
    formatted(
      "#![feature(test)]\n\n#![warn(rust_2018_idioms)]\n\n// c\n#![allow(x)]\n\n#[macro_use]\n\nextern\n    crate bitflags;\nextern  crate  alloc as std2 ;\n#[cfg(x)]\n\nuse a::b;\n#[derive(Debug)]\n\nstruct S;\nbitflags!\n{ pub struct A: u8 {} }\nfoo!\n(a, b);\nbar!   [1];\nfn f() {\n    m!\n    { x };\n    let x = n!\n    (1);\n    let vec\n=\nvec![33u8;\n    1024];\n}\n"
    ),
    "#![feature(test)]\n#![warn(rust_2018_idioms)]\n// c\n#![allow(x)]\n\n#[macro_use]\nextern crate bitflags;\nextern crate alloc as std2;\n#[cfg(x)]\nuse a::b;\n#[derive(Debug)]\n\nstruct S;\nbitflags! { pub struct A: u8 {} }\nfoo!(a, b);\nbar![1];\nfn f() {\n    m! { x };\n    let x = n!(1);\n    let vec = vec![33u8; 1024];\n}\n"
  );
}

#[test]
fn test_blank_lines_and_trailing_whitespace() {
  assert_eq!(
    formatted("\n\nfn a() {   \n\n\n    x();  \n\n}\n\n\n\nfn b() {} // note   \n\n\n"),
    "fn a() {\n    x();\n}\n\nfn b() {} // note\n"
  );
}

#[test]
fn test_blank_lines_between_variant_fields() {
  // A blank line between a struct-like variant's fields stays.
  assert_eq!(
    formatted(
      "enum E {\n    Item {\n        meta: Vec<syn::Attribute>,\n\n        vis: syn::Visibility,\n        tail: proc_macro2::TokenStream,\n    },\n    A { x: u8,\n\n y: u8 },\n}\n"
    ),
    "enum E {\n    Item {\n        meta: Vec<syn::Attribute>,\n\n        vis: syn::Visibility,\n        tail: proc_macro2::TokenStream,\n    },\n    A {\n        x: u8,\n\n        y: u8,\n    },\n}\n"
  );
}

#[test]
fn test_braced_macros_holding_strings_across_lines() {
  // Re-indented around the string: a line continuing it after `\` moves, one starting
  // inside it otherwise is the string's own and stays.
  assert_eq!(
    formatted(
      "cfg_if!\n\t{\n\tif #[cfg(x)] {\n            compile_error!(\n                \"one \\\n            two\"  );\n}\n}\nfn g() {\n  m! {\n     let s = \"a\n  b\";\n        x\n  }\n}\n"
    ),
    "cfg_if! {\n    if #[cfg(x)] {\n            compile_error!(\n                \"one \\\n            two\"  );\n}\n}\nfn g() {\n    m! {\n       let s = \"a\n  b\";\n          x\n    }\n}\n"
  );
}

#[test]
fn test_comments_after_a_trailing_comment_and_before_a_brace() {
  // Comment lines after a statement's trailing comment align under it, before the 2024
  // style; from it, they take the statement's indentation.
  let messy = "fn f() {\n    let b = g(); // one\n// two\n    if x {\n        y();\n// three\n    } else {\n        z();\n    }\n}\n";
  let tidy = "fn f() {\n    let b = g(); // one\n                 // two\n    if x {\n        y();\n    // three\n    } else {\n        z();\n    }\n}\n";
  assert_eq!(formatted(messy), tidy);
  let config = FormatConfig {
    style_edition: 2024,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(messy, &config),
    "fn f() {\n    let b = g(); // one\n    // two\n    if x {\n        y();\n    // three\n    } else {\n        z();\n    }\n}\n"
  );
  // At the end of a block, a comment takes the block's indentation.
  assert_eq!(
    formatted("fn f() {\n    g();\n// end\n}\n"),
    "fn f() {\n    g();\n    // end\n}\n"
  );
}

#[test]
fn test_comments_after_arms_in_nested_matches() {
  // Comments after arms aligned as a list's, in a match inside a statement.
  assert_eq!(
    formatted(
      "fn f() {\n    let target_indent = match target_indent {\n        Some(x) => x, // in all other cases, take previous indentation and indent once\n        None => IndentLevel::from_node(&parent) + 1,\n    };\n    let y = match a {\n        Aaaa => 1, // one\n        B => 22222, // two\n        C => { foo(); bar() }\n        D => 4, // four\n    };\n}\n"
    ),
    "fn f() {\n    let target_indent = match target_indent {\n        Some(x) => x, // in all other cases, take previous indentation and indent once\n        None => IndentLevel::from_node(&parent) + 1,\n    };\n    let y = match a {\n        Aaaa => 1,  // one\n        B => 22222, // two\n        C => {\n            foo();\n            bar()\n        }\n        D => 4, // four\n    };\n}\n"
  );
}

#[test]
fn test_comments_among_items_follow_the_items() {
  assert_eq!(
    formatted("fn f() {\n        // first\n  a();\n      /* second */\n  b();\n}\n"),
    "fn f() {\n    // first\n    a();\n    /* second */\n    b();\n}\n"
  );
}

#[test]
fn test_comments_inside_macro_arguments() {
  // Comments inside a macro's arguments placed as code's.
  assert_eq!(
    formatted(
      "fn f() {\n    try_likely_ok!(self.format_two_digits(\n        // Safety: `month` is guaranteed to be in the range `1..=12`.\n    unsafe\n\t{\tru8::new_unchecked(u8::from(value.month(state)))\t},\nPadding::Zero,\n));\n    assert_eq!( *s2.get_or_insert_with( 7,  || {\n    *called2.lock().unwrap()\n        =  true;\n\n// allow main thread to run before we return result\n        std::thread::sleep(std::time::Duration::from_secs(4));\n70  },\n\tguard,\n\t)\n\n.value(), 700\n);\n}\n"
    ),
    "fn f() {\n    try_likely_ok!(self.format_two_digits(\n        // Safety: `month` is guaranteed to be in the range `1..=12`.\n        unsafe { ru8::new_unchecked(u8::from(value.month(state))) },\n        Padding::Zero,\n    ));\n    assert_eq!(\n        *s2.get_or_insert_with(\n            7,\n            || {\n                *called2.lock().unwrap() = true;\n\n                // allow main thread to run before we return result\n                std::thread::sleep(std::time::Duration::from_secs(4));\n                70\n            },\n            guard,\n        )\n        .value(),\n        700\n    );\n}\n"
  );
}

#[test]
fn test_comments_kept_where_the_standard_formatter_keeps_them() {
  // Just before a closing delimiter: where written, relative to the container.
  let before_close = "fn f() {\n    if a {\n        b();\n    // c\n    } else {\n    }\n}\n";
  assert_eq!(formatted(before_close), before_close);
  // Under a comment that ends a line of code.
  let aligned = "fn f() {\n    let a = 1; // one\n               // two\n    a;\n}\n";
  assert_eq!(formatted(aligned), aligned);
  // Doc comments keep trailing whitespace; module bodies keep a leading blank line.
  let kept = "mod m {\n\n    /// a   \n    fn f() {}\n}\n";
  assert_eq!(formatted(kept), kept);
}

#[test]
fn test_contents_follow_the_line_their_owner_starts_on() {
  // A closure whose parameters span lines: its body follows the closure's first line.
  assert_eq!(
    formatted(
      "fn f() {\n    let g = |S { aaaaaaaa, bbbbbbbbbbbb }: S,\n             b: u8| {\n  a\n    };\n}\n"
    ),
    "fn f() {\n    let g = |S { aaaaaaaa, bbbbbbbbbbbb }: S,\n             b: u8| {\n        a\n    };\n}\n"
  );
  // A comment after an argument: the arguments one per line, the comment after its item.
  assert_eq!(
    formatted("fn f() {\n    x\n        .g(\n  1, // c\n  );\n}\n"),
    "fn f() {\n    x.g(\n        1, // c\n    );\n}\n"
  );
  // A comment between links is the chain layout's.
  assert_eq!(
    formatted("fn f() {\n    x // c\n        .g(\n  1,\n  );\n}\n"),
    "fn f() {\n    x // c\n        .g(1);\n}\n"
  );
  // A block standing as a statement owns itself.
  assert_eq!(
    formatted("fn f() {\n    {\n  a();\n    }\n    unsafe {\n b();\n    }\n}\n"),
    "fn f() {\n    {\n        a();\n    }\n    unsafe {\n        b();\n    }\n}\n"
  );
}

#[test]
fn test_continuation_lines_move_with_their_construct() {
  assert_eq!(
    formatted("fn f() {\n  let x = foo // c\n      .bar()\n      .baz();\n}\n"),
    "fn f() {\n    let x = foo // c\n        .bar()\n        .baz();\n}\n"
  );
  // A comment after `(` goes above the first argument; short ones are packed.
  assert_eq!(
    formatted("fn f() {\n    g( // c\n  a,\n          b,\n  );\n}\n"),
    "fn f() {\n    g(\n        // c\n        a, b,\n    );\n}\n"
  );
}

#[test]
fn test_default_field_values_left_as_written() {
  // A field's default value is left as written.
  assert_eq!(
    formatted(
      "#![feature(default_struct_values)]\n\n// Test for now that nightly default field values are left alone for now.\n\nstruct Foo {\n    default_field:    Spacing =    /* uwu */ 0,\n}\n\nstruct Foo2 {\n    #[rustfmt::skip]\n    default_field:    Spacing =    /* uwu */ 0,\n}\n\na_macro!(\n    struct Foo2 {\n        default_field:    Spacing =    /* uwu */ 0,\n    }\n);\n"
    ),
    "#![feature(default_struct_values)]\n\n// Test for now that nightly default field values are left alone for now.\n\nstruct Foo {\n    default_field:    Spacing =    /* uwu */ 0,\n}\n\nstruct Foo2 {\n    #[rustfmt::skip]\n    default_field:    Spacing =    /* uwu */ 0,\n}\n\na_macro!(\n    struct Foo2 {\n        default_field:    Spacing =    /* uwu */ 0,\n    }\n);\n"
  );
}

#[test]
fn test_derives_with_comments() {
  // Comments among derived names: on one line when they fit and none ends its line, else
  // one name per line.
  assert_eq!(
    formatted(
      "#[derive(\n/* ---------- Some really important comment that just had to go inside the derive --------- */\nDebug, Clone,/* Another comment */Eq, PartialEq,\n)]\nstruct Foo {\n    a: i32,\n    b: T,\n}\n"
    ),
    "#[derive(\n    /* ---------- Some really important comment that just had to go inside the derive --------- */\n    Debug,\n    Clone,\n    /* Another comment */ Eq,\n    PartialEq,\n)]\nstruct Foo {\n    a: i32,\n    b: T,\n}\n"
  );
  assert_eq!(
    formatted("#[derive(/*Debug, */Clone)]\nstruct Foo;\n"),
    "#[derive(/*Debug, */ Clone)]\nstruct Foo;\n"
  );
}

#[test]
fn test_doc_comments_and_foreign_items() {
  // Inner doc comments at the top take the file's indentation; a foreign item joins its
  // attribute, an item in a file keeps the blank line after its attribute.
  assert_eq!(
    formatted(
      "//! a\n    //! b\n        //! c\nextern \"C\" {\n    #[a]\n\n    pub fn f();\n}\n#[b]\n\nfn g() {}\n"
    ),
    "//! a\n//! b\n//! c\nextern \"C\" {\n    #[a]\n    pub fn f();\n}\n#[b]\n\nfn g() {}\n"
  );
}

#[test]
fn test_doc_comments_consts_and_macro_bodies() {
  // A doc comment joins a field, variant, statement or `use` with no blank line, as an
  // attribute does; a long const's value goes to the next line; a braced macro call's
  // arguments are re-indented keeping their layout (tabs counting `tab_spaces`), unless a
  // `rustfmt::skip::macros` around it names the macro.
  assert_eq!(
    formatted(
      "/// doc\n\nstruct S {\n    /// a\n\n    a: u8,\n}\nenum E {\n    /// b\n\n    B,\n}\n/// f\n\nfn f() {\n    /// c\n\n    let x = 1;\n}\n/// u\n\nuse a;\ntrait T: Clone + Send {}\npub trait OsStrExt: private::Sealed {}\nconst A_BILLION: i64 = 1_000_000_000; // Maximum\npub const TLS_1_2_VERSION_LONG_NAME_XXXXXXXXXXXXXXXXXXXXX: __u16 = TLS_VERSION_NUMBER_LONG(TLS_1_2_VERSION_MAJOR);\nmod m {\n    bitflags! {\n            pub struct Flags: u32 {\n        \tconst A = 1;\n\n                    const B = 2;\n            }\n    }\n}\n#[rustfmt::skip::macros(html)]\nfn main() {\n    let x = html! { <div>\nthis stays</div>\n    };\n}\n"
    ),
    "/// doc\n\nstruct S {\n    /// a\n    a: u8,\n}\nenum E {\n    /// b\n    B,\n}\n/// f\n\nfn f() {\n    /// c\n    let x = 1;\n}\n/// u\nuse a;\ntrait T: Clone + Send {}\npub trait OsStrExt: private::Sealed {}\nconst A_BILLION: i64 = 1_000_000_000; // Maximum\npub const TLS_1_2_VERSION_LONG_NAME_XXXXXXXXXXXXXXXXXXXXX: __u16 =\n    TLS_VERSION_NUMBER_LONG(TLS_1_2_VERSION_MAJOR);\nmod m {\n    bitflags! {\n            pub struct Flags: u32 {\n            const A = 1;\n\n                    const B = 2;\n            }\n    }\n}\n#[rustfmt::skip::macros(html)]\nfn main() {\n    let x = html! { <div>\nthis stays</div>\n    };\n}\n"
  );
}

#[test]
fn test_dots_kept_apart_in_chains() {
  // A link after a range keeps its space (`...` would read as one token; the standard
  // formatter puts the closure in parentheses); a float before a link is parenthesized.
  assert_eq!(
    formatted(
      "struct S;\nimpl S {\n    fn method(&self) {}\n}\nfn f() {\n    || .. .method();\n    || 1.. .method();\n}\n"
    ),
    "struct S;\nimpl S {\n    fn method(&self) {}\n}\nfn f() {\n    || .. .method();\n    || 1.. .method();\n}\n"
  );
  assert_eq!(
    formatted("fn f() {\n    0. .to_string();\n}\n"),
    "fn f() {\n    (0.).to_string();\n}\n"
  );
}

#[test]
fn test_empty_bodies_after_long_heads() {
  // An empty body keeps its braces apart after `else if`, after a head on one line with
  // `{` on the next, and when `{}` would pass the line width.
  assert_eq!(
    formatted(
      "fn main() {\n    while loooooooooooooooooooooooooooooooooong_variable_name + another_value > some_other_value_xxxxxxx {}\n    if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa || bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb || cccccccccc {}\n    if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa || bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb || cccccccccc {} else {}\n    for xxxxxxxxxxxxxxxxxxxxxxx in yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy.zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz() {}\n}\n"
    ),
    "fn main() {\n    while loooooooooooooooooooooooooooooooooong_variable_name + another_value\n        > some_other_value_xxxxxxx\n    {}\n    if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n        || bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n        || cccccccccc\n    {}\n    if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n        || bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n        || cccccccccc\n    {\n    } else {\n    }\n    for xxxxxxxxxxxxxxxxxxxxxxx in\n        yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy.zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz()\n    {}\n}\n"
  );
  assert_eq!(
    formatted(
      "fn main() {\n    'a: while loooooooooooooooooooooooooooooooooong_variable_name + another_value > some_other_value {}\n    while aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa > bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb {}\n    let x = if a {\n        1\n    } else if condition__no_alignment_for_later_else__0 || condition__no_alignment_for_later_else__1 || condition__no_alignment_for_later_else__2 {};\n}\n"
    ),
    "fn main() {\n    'a: while loooooooooooooooooooooooooooooooooong_variable_name + another_value > some_other_value\n    {\n    }\n    while aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa > bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb {\n    }\n    let x = if a {\n        1\n    } else if condition__no_alignment_for_later_else__0\n        || condition__no_alignment_for_later_else__1\n        || condition__no_alignment_for_later_else__2\n    {\n    };\n}\n"
  );
}

#[test]
fn test_empty_braced_macro() {
  // A braced macro call holding nothing closes up.
  assert_eq!(
    formatted("lazy_static::lazy_static!{\n}\n"),
    "lazy_static::lazy_static! {}\n"
  );
}

#[test]
fn test_empty_where_clauses() {
  // An empty `where` goes, but a function's.
  assert_eq!(
    formatted("trait Foo where {}\nstruct S where;\n"),
    "trait Foo {}\nstruct S;\n"
  );
  assert_eq!(
    formatted("fn f() -> C\nwhere\n{\n    1\n}\nstruct S where {}\nimpl A where {}\n"),
    "fn f() -> C\nwhere\n{\n    1\n}\nstruct S {}\nimpl A {}\n"
  );
}

#[test]
fn test_failed_macro_statement_kept() {
  // A macro statement that does not lay out is kept as written; the rest is laid out.
  assert_eq!(
    formatted(
      "fn f() {\n    match name.as_str() {\n        \"threaded_scheduler\" | \"multi_thread\" =>\n {  format!(\n\n\"Set the runtime flavor with #[{macro_name}(flavor = \\\"multi_thread\\\")]. xxxxxxxxxxxxxx\"\n )\n        }\n        _ => 1,\n    };\n}\n"
    ),
    "fn f() {\n    match name.as_str() {\n        \"threaded_scheduler\" | \"multi_thread\" => {\n            format!(\n\n\"Set the runtime flavor with #[{macro_name}(flavor = \\\"multi_thread\\\")]. xxxxxxxxxxxxxx\"\n )\n        }\n        _ => 1,\n    };\n}\n"
  );
}

#[test]
fn test_field_comments_aligned_and_joined() {
  // Comments after fields aligned in groups a blank line ends; a comment above a field
  // joins it; one after a statement's `;` one space after it.
  assert_eq!(
    formatted(
      "struct S {\n    aliases: Vec<(Str, bool)>,\t// (name, visible)\n    a: u8,  // x\n\n    c: u8,   // z\n        // Hash buckets\n\nentries: u8,\n}\nconst A: u8 = 1;  /* one */\n"
    ),
    "struct S {\n    aliases: Vec<(Str, bool)>, // (name, visible)\n    a: u8,                     // x\n\n    c: u8, // z\n    // Hash buckets\n    entries: u8,\n}\nconst A: u8 = 1; /* one */\n"
  );
}

#[test]
fn test_files_left_alone_by_their_settings() {
  let messy = "// @generated by a tool\nfn  f() {\nx();\n}\n";
  let skip_generated = FormatConfig {
    format_generated_files: false,
    ..FormatConfig::default()
  };
  assert_eq!(format_file(messy, Edition::E2021, &skip_generated), messy);
  assert_ne!(
    format_file(messy, Edition::E2021, &FormatConfig::default()),
    messy
  );
  let disabled = FormatConfig {
    disable_all_formatting: true,
    ..FormatConfig::default()
  };
  assert_eq!(format_file(messy, Edition::E2021, &disabled), messy);
}

#[test]
fn test_for_head_comment_keeps_statement() {
  // A comment in a loop's head leaves the statement as written.
  assert_eq!(
    formatted(
      "fn main() {\n    for variable_in_x /* ... */ in 0..1 {\n        if false {\n\n        } else {\n\n        }\n    }\n}\n"
    ),
    "fn main() {\n    for variable_in_x /* ... */ in 0..1 {\n        if false {\n\n        } else {\n\n        }\n    }\n}\n"
  );
}

#[test]
fn test_for_head_with_commented_array() {
  // A loop's iterator may hold comments its list places.
  assert_eq!(
    formatted(
      "fn f() {\n    for\n  f in [\n            // these drift without `float_roundtrip`\n            -0.20221894534048165,\n            6.178787134922198e305, // c\n        ] {\n        g(f);\n    }\n}\n"
    ),
    "fn f() {\n    for f in [\n        // these drift without `float_roundtrip`\n        -0.20221894534048165,\n        6.178787134922198e305, // c\n    ] {\n        g(f);\n    }\n}\n"
  );
}

#[test]
fn test_index_expressions_and_ranges() {
  // An index after what it indexes, else on the next line; a range breaks as a binary
  // expression does; a string spanning lines under `&` and indexed.
  assert_eq!(
    formatted(
      "fn f() {\n    match x {\n        PushParam => {\n            stack.push(mparams[match cur.to_digit(10) { Some(d) => d as usize - 1, None => return Err(\"bad param number\".to_owned()) }])\n        }\n    }\n    foo(mparams[match cur.to_digit(10) { Some(d) => d as usize - 1, None => return Err(\"bad param number\".to_owned()) }]);\n}\n"
    ),
    "fn f() {\n    match x {\n        PushParam => stack.push(\n            mparams[match cur.to_digit(10) {\n                Some(d) => d as usize - 1,\n                None => return Err(\"bad param number\".to_owned()),\n            }],\n        ),\n    }\n    foo(mparams[match cur.to_digit(10) {\n        Some(d) => d as usize - 1,\n        None => return Err(\"bad param number\".to_owned()),\n    }]);\n}\n"
  );
  assert_eq!(
    formatted(
      "fn params_in_repr(db: &dyn HirDatabase, def_id: AdtId) -> Box<[bool]> {\n    let mut params_in_repr = (0..generics.len_early_bound_lifetimes() + generics.len_type_or_consts()).map(|_| false).collect::<Box<[bool]>>();\n}\n"
    ),
    "fn params_in_repr(db: &dyn HirDatabase, def_id: AdtId) -> Box<[bool]> {\n    let mut params_in_repr = (0..generics.len_early_bound_lifetimes()\n        + generics.len_type_or_consts())\n        .map(|_| false)\n        .collect::<Box<[bool]>>();\n}\n"
  );
  assert_eq!(
    formatted(
      "fn parse_category_test() {\n  let   ini_file =\n  &b\"[category]\n\nparameter=value\nkey = value2\"[..];\n  let x = foo(&b\"[category]\n\nparameter=value\nkey = value2\"[..], bar);\n}\n"
    ),
    "fn parse_category_test() {\n    let ini_file = &b\"[category]\n\nparameter=value\nkey = value2\"[..];\n    let x = foo(\n        &b\"[category]\n\nparameter=value\nkey = value2\"[..],\n        bar,\n    );\n}\n"
  );
}

#[test]
fn test_item_alone_as_macro_argument() {
  // An item alone in a macro's arguments on a line of its own.
  assert_eq!(
    formatted(
      "cfg_net_unix!(\n    impl ReadyEvent {\n        pub(crate) fn with_ready(&self, ready: Ready) -> Self {\n            Self {\n                ready,\n                tick: self.tick,\n            }\n        }\n    }\n);\nm!(  impl\n\nA\n{  fn f()\n{ 1 }\n}\n);\nm!(  struct\n\nA\n{  x: u8\n}\n);\nm!(fn f() { 1 });\nm!(fn f() {\n   1 });\n"
    ),
    "cfg_net_unix!(\n    impl ReadyEvent {\n        pub(crate) fn with_ready(&self, ready: Ready) -> Self {\n            Self {\n                ready,\n                tick: self.tick,\n            }\n        }\n    }\n);\nm!(  impl\n\nA\n{  fn f()\n{ 1 }\n}\n);\nm!(\n    struct A {\n        x: u8,\n    }\n);\nm!(\n    fn f() {\n        1\n    }\n);\nm!(\n    fn f() {\n        1\n    }\n);\n"
  );
}

#[test]
fn test_leading_pipes_dropped() {
  // A `|` before a pattern's first alternative goes.
  assert_eq!(
    formatted(
      "fn f() {\n    let (| a | a, | b) = ((), ());\n    let R { a: | 1 | 2 } = 0;\n    let [| a, ..] = [];\n    let S(| a) = ();\n    match x { | A | B => 1, | C => 2 }\n}\n"
    ),
    "fn f() {\n    let (a | a, b) = ((), ());\n    let R { a: 1 | 2 } = 0;\n    let [a, ..] = [];\n    let S(a) = ();\n    match x {\n        A | B => 1,\n        C => 2,\n    }\n}\n"
  );
}

#[test]
fn test_line_endings_and_tabs() {
  assert_eq!(
    formatted("fn f() {\r\nx();\r\n}\r\n"),
    "fn f() {\r\n    x();\r\n}\r\n"
  );
  let tabs = FormatConfig {
    hard_tabs: true,
    ..FormatConfig::default()
  };
  assert_eq!(
    format_file("fn f() {\n    x();\n}\n", Edition::E2021, &tabs),
    "fn f() {\n\tx();\n}\n"
  );
  let unix = FormatConfig {
    newline_style: NewlineStyle::Unix,
    ..FormatConfig::default()
  };
  assert_eq!(
    format_file("fn f() {}\r\n", Edition::E2021, &unix),
    "fn f() {}\n"
  );
  // A string spanning lines keeps the file's endings, which are part of it.
  let crlf_string = "fn f() {\r\n    g(\"a\r\nb\");\r\n}\r\n";
  assert_eq!(format_file(crlf_string, Edition::E2021, &unix), crlf_string);
  let windows = FormatConfig {
    newline_style: NewlineStyle::Windows,
    tab_spaces: 2,
    ..FormatConfig::default()
  };
  assert_eq!(
    format_file("fn f() {\nx();\n}\n", Edition::E2021, &windows),
    "fn f() {\r\n  x();\r\n}\r\n"
  );
  let native = FormatConfig {
    newline_style: NewlineStyle::Native,
    ..FormatConfig::default()
  };
  let ending = if cfg!(windows) { "\r\n" } else { "\n" };
  assert_eq!(
    format_file("fn f() {}\r\n", Edition::E2021, &native),
    format!("fn f() {{}}{}", ending)
  );
}

#[test]
fn test_lines_starting_constructs_follow_their_block() {
  assert_eq!(
    formatted("fn f() {\nlet a = 1;\n        if a {\n  g();\n            }\n}\n"),
    "fn f() {\n    let a = 1;\n    if a {\n        g();\n    }\n}\n"
  );
  assert_eq!(
    formatted("mod m {\nstruct S {\na: u8,\n  b: u8,\n}\n}\n"),
    "mod m {\n    struct S {\n        a: u8,\n        b: u8,\n    }\n}\n"
  );
}

#[test]
fn test_long_attribute_values_and_inner_docs() {
  // An attribute's `name = "literal"` keeps the literal however long, its arguments on
  // lines when they do not fit.
  assert_eq!(
    formatted(
      "#[deprecated(\n\tnote\t=\n \"Use [std::iter::FromFn](https://doc.rust-lang.org/std/iter/struct.FromFn.html) instead\",  since = \"0.13.0\")]\nfn g() {}\n"
    ),
    "#[deprecated(\n    note = \"Use [std::iter::FromFn](https://doc.rust-lang.org/std/iter/struct.FromFn.html) instead\",\n    since = \"0.13.0\"\n)]\nfn g() {}\n"
  );
  // A file's doc comment after its attribute starts its own line.
  assert_eq!(
    formatted("#![allow(x)] //! Docs.\n\nfn f() {}\n"),
    "#![allow(x)]\n//! Docs.\n\nfn f() {}\n"
  );
}

#[test]
fn test_match_arm_indent_off_keeps_arms_at_the_match() {
  let config = FormatConfig {
    match_arm_indent: false,
    ..FormatConfig::default()
  };
  let source = "fn f() {\n    match x {\n        0 => {\n            a();\n        }\n        _ => b(),\n    }\n}\n";
  let want = "fn f() {\n    match x {\n    0 => {\n        a();\n    }\n    _ => b(),\n    }\n}\n";
  let once = format_file(source, Edition::E2021, &config);
  assert_eq!(once, want);
  assert_eq!(format_file(&once, Edition::E2021, &config), once);
}

#[test]
fn test_nested_parentheses_dropped() {
  // Parentheses directly inside parentheses go, but around a comment.
  assert_eq!(
    formatted(
      "fn main() {\n    let x = (((1)));\n    foo((((a + b))), c);\n    let z = ((a)).b();\n}\n"
    ),
    "fn main() {\n    let x = (1);\n    foo((a + b), c);\n    let z = (a).b();\n}\n"
  );
}

#[test]
fn test_nested_parentheses_kept_by_setting() {
  // `remove_nested_parens = false` keeps them.
  let config = FormatConfig {
    remove_nested_parens: false,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with("fn f() {\n    let x = ((1));\n}\n", &config),
    "fn f() {\n    let x = ((1));\n}\n"
  );
}

#[test]
fn test_parameter_and_tuple_field_attributes() {
  // An attribute too long for the parameter's line goes on a line of its own; one
  // breaking across lines puts the field on the next.
  assert_eq!(
    formatted(
      "fn a1(\n    #[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa] a: u8,\n) {\n}\n"
    ),
    "fn a1(\n    #[aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]\n    a: u8,\n) {\n}\n"
  );
  assert_eq!(
    formatted(
      "struct Fields(\n    #[cfg_attr(feature = \"serde_derive\", serde(state_with = \"::base::serialization::shared\"))] Arc<Vec<InternedStr>>,\n);\npub enum State {\n    Closure(#[cfg_attr(feature = \"serde_derive\", serde(state_with = \"::serialization::closure\"))] GcPtr<ClosureData>),\n}\n"
    ),
    "struct Fields(\n    #[cfg_attr(\n        feature = \"serde_derive\",\n        serde(state_with = \"::base::serialization::shared\")\n    )]\n    Arc<Vec<InternedStr>>,\n);\npub enum State {\n    Closure(\n        #[cfg_attr(\n            feature = \"serde_derive\",\n            serde(state_with = \"::serialization::closure\")\n        )]\n        GcPtr<ClosureData>,\n    ),\n}\n"
  );
}

#[test]
fn test_parameters_with_block_comments() {
  // Block comments among parameters on the signature's line when it fits.
  assert_eq!(
    formatted(
      "fn foo(  /* comment */\n) {}\nfn foo_one_pre(/* Important comment1 */\n    self) {}\nfn foo_one_post(self\n    /* Important comment1 */) {}\nfn two(/*a*/ a: u8, b: u8 /*b*/) {}\n"
    ),
    "fn foo(/* comment */) {}\nfn foo_one_pre(/* Important comment1 */ self) {}\nfn foo_one_post(self /* Important comment1 */) {}\nfn two(/*a*/ a: u8, b: u8 /*b*/) {}\n"
  );
}

#[test]
fn test_raw_string_arguments() {
  // Lines inside a raw string argument are not measured.
  assert_eq!(
    formatted(
      "fn overloaded_binop() {\n    check_number(r#\"\n    const GOAL: bool = {\n        x == Yellow && y == Red && Red + Green == Yellow && Red + Red == Yellow && Yellow + Green == Yellow\n    };\n    \"#, 1);\n    check_number(r#\"\n    const GOAL: bool = {\n        x == Yellow\n    };\n    \"#, 1);\n}\n"
    ),
    "fn overloaded_binop() {\n    check_number(\n        r#\"\n    const GOAL: bool = {\n        x == Yellow && y == Red && Red + Green == Yellow && Red + Red == Yellow && Yellow + Green == Yellow\n    };\n    \"#,\n        1,\n    );\n    check_number(\n        r#\"\n    const GOAL: bool = {\n        x == Yellow\n    };\n    \"#,\n        1,\n    );\n}\n"
  );
}

#[test]
fn test_skipped_macro_invocations() {
  // The calls of the macros `skip_macro_invocations` names, by their path as written or all
  // for `*`, are left as written.
  let named = FormatConfig {
    skip_macro_invocations: vec!["items".to_string(), "self::m".to_string()],
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with(
      "items!(\n        const _: u8 = 0;\n);\nother!(\n        const _: u8 = 0;\n);\nself::m!(\n        const _: u8 = 0;\n);\n",
      &named
    ),
    "items!(\n        const _: u8 = 0;\n);\nother!(\n    const _: u8 = 0;\n);\nself::m!(\n        const _: u8 = 0;\n);\n"
  );
  let all = FormatConfig {
    skip_macro_invocations: vec!["*".to_string()],
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted_with("fn f() {\n    foo!(a,   b);\n}\n", &all),
    "fn f() {\n    foo!(a,   b);\n}\n"
  );
}

#[test]
fn test_statements_the_standard_formatter_gives_up_on() {
  // A line past `max_width` after the first: the statement is left as written.
  let long = format!(
    "fn f() {{\n    g(\n  \"{}\",\n      );\n}}\n",
    "x".repeat(100)
  );
  assert_eq!(formatted(&long), long);
  // A statement marked to be skipped.
  let skipped = "fn f() {\n    #[rustfmt::skip]\n    let a = [1,0,\n               0,1];\n}\n";
  assert_eq!(formatted(skipped), skipped);
}

#[test]
fn test_string_receivers_spanning_lines() {
  // A string spanning lines as a method's receiver.
  assert_eq!(
    formatted(
      "fn f() {\n        let mut deps\n        =\n        \"wasm-bindgen = { path = '{root}' }\\n\\\n             wasm-bindgen-futures = { path = '{root}/crates/futures' }\\n\"\n            .to_owned();\n}\n"
    ),
    "fn f() {\n    let mut deps = \"wasm-bindgen = { path = '{root}' }\\n\\\n             wasm-bindgen-futures = { path = '{root}/crates/futures' }\\n\"\n        .to_owned();\n}\n"
  );
}

#[test]
fn test_trailing_comment_after_use_attribute() {
  // A use after an attribute ending in a comment joins it.
  assert_eq!(
    formatted(
      "#[cfg(feature = \"tokio\")]\n#[allow(unreachable_pub, unused_imports)] // https://github.com/rust-lang/rust/issues/57411\n\npub use self::tokio::*;\n#[allow(x)] // c\n\nfn f() {}\n"
    ),
    "#[cfg(feature = \"tokio\")]\n#[allow(unreachable_pub, unused_imports)] // https://github.com/rust-lang/rust/issues/57411\npub use self::tokio::*;\n#[allow(x)] // c\n\nfn f() {}\n"
  );
}

#[test]
fn test_try_blocks_on_one_line() {
  // A `try` block of one expression on one line wherever it stands.
  assert_eq!(
    formatted(
      "fn main() {\n    let _x: Option<_> = try {\n        4\n    };\n    let y = try bikeshed Option<_> {\n        4\n    };\n    try {\n        foo()?\n    }\n    try {}\n}\n"
    ),
    "fn main() {\n    let _x: Option<_> = try { 4 };\n    let y = try bikeshed Option<_> { 4 };\n    try { foo()? }\n    try {}\n}\n"
  );
}

#[test]
fn test_tuple_field_attributes() {
  // A tuple field's attributes on its line when one line, else above it.
  assert_eq!(
    formatted(
      "pub struct NewUser(#[diesel(column_name\n    =\tname)] String);\npub struct NewComment<'a>(#[diesel(column_name\t=\tpost_id)] pub\n    i32,\n    #[diesel(column_name\n    =\ttext)]\n    &'a str);\n#[diesel(table_name\n  = users)]\nstruct S;\n"
    ),
    "pub struct NewUser(\n    #[diesel(column_name\n    =\tname)]\n    String,\n);\npub struct NewComment<'a>(\n    #[diesel(column_name\t=\tpost_id)] pub i32,\n    #[diesel(column_name\n    =\ttext)]\n    &'a str,\n);\n#[diesel(table_name\n  = users)]\nstruct S;\n"
  );
}

#[test]
fn test_tuple_struct_with_where() {
  // `struct S<T>(..)` with no space before `(`.
  assert_eq!(
    formatted(
      "struct Exactly100CharsToSemicolon<A, B, C, D, E>\n    (LongLongTypename, i32, i32)\n    where A: LongTrait1234;\nstruct S<A>(A) where A: T;\n"
    ),
    "struct Exactly100CharsToSemicolon<A, B, C, D, E>(LongLongTypename, i32, i32)\nwhere\n    A: LongTrait1234;\nstruct S<A>(A)\nwhere\n    A: T;\n"
  );
}

#[test]
fn test_type_alias_where_before_value() {
  // A `where` before a type alias's value: the clause, then `= value;` on its line.
  assert_eq!(
    formatted(
      "pub type Adder<F, T> where T: Clone, F: Copy = impl Fn(T) -> T;\ntype A<T> where T: Clone = Vec<T>;\ntype B<T> = Vec<T> where T: Clone;\ntrait X { type C<T> where T: Clone = Vec<T>; type D<'a> where Self: 'a; }\n"
    ),
    "pub type Adder<F, T>\nwhere\n    T: Clone,\n    F: Copy,\n= impl Fn(T) -> T;\ntype A<T>\nwhere\n    T: Clone,\n= Vec<T>;\ntype B<T>\n    = Vec<T>\nwhere\n    T: Clone;\ntrait X {\n    type C<T>\n    where\n        T: Clone,\n    = Vec<T>;\n    type D<'a>\n    where\n        Self: 'a;\n}\n"
  );
}

#[test]
fn test_use_trees_normalized() {
  // Braces around one item go, `a::self` is `a`, nested lists are sorted.
  assert_eq!(
    formatted(
      "use super::{auxvec};\nuse crate::lexer::self;\nuse a::{l::{self, m, n::o, p::*}};\nuse std::{*};\nuse outer::tree::{inner::tree};\nuse a::{b as c, self as d};\npub use x::{y::self as z};\nuse ::log::{error, info, log};\n"
    ),
    "use super::auxvec;\nuse crate::lexer;\nuse ::log::{error, info, log};\nuse a::l::{self, m, n::o, p::*};\nuse a::{self as d, b as c};\nuse outer::tree::inner::tree;\nuse std::*;\npub use x::y as z;\n"
  );
}

#[test]
fn test_variant_fields_only_a_comment() {
  // Fields that are only a block comment on one line close up around it.
  assert_eq!(
    formatted(
      "enum E {\n    A {\n        /* c */\n    } = 1,\n    B(\n        /* c */\n    ) = 2,\n}\n"
    ),
    "enum E {\n    A {/* c */} = 1,\n    B(/* c */) = 2,\n}\n"
  );
}

#[test]
fn test_visibility_paths() {
  // `pub(in crate)` is `pub(crate)`; a path from the root loses its `::`.
  assert_eq!(
    formatted("pub(in   crate) struct S;\npub(in self) fn f() {}\npub(in crate::a) struct T;\n"),
    "pub(crate) struct S;\npub(self) fn f() {}\npub(in crate::a) struct T;\n"
  );
  assert_eq!(
    formatted("pub(in ::global::path) fn f() {}\npub(in   ::a) struct S;\n"),
    "pub(in global::path) fn f() {}\npub(in a) struct S;\n"
  );
}

#[test]
fn test_what_f5b_leaves_as_the_standard_formatter_does() {
  // Extern bodies keep a blank line at either end.
  let extern_block = "extern \"C\" {\n\n    fn f();\n\n}\n";
  assert_eq!(formatted(extern_block), extern_block);
  // A closure's lines count toward the statement around it: one line too long and the
  // whole statement is left as written.
  let long = format!(
    "fn f() {{\n    let v = x.map(|a| {{\n\n          g(\"{}\")\n    }});\n}}\n",
    "y".repeat(100)
  );
  assert_eq!(formatted(&long), long);
  // A block of one expression that is a binary operator's operand goes on one line.
  let operand = "fn f() {\n    a\n        && {\n            b\n        };\n}\n";
  assert_eq!(formatted(operand), "fn f() {\n    a && { b };\n}\n");
}

#[test]
fn test_what_is_left_as_written() {
  // A macro definition whose transcriber repeats (`$(..)`).
  let source = "macro_rules! m {\n  ($($a:expr),*) => {\n        $($a)*\n  };\n}\n#[rustfmt::skip]\nfn  f() {\n  x( );\n}\n// mto: off\nfn   g() {\n x();\n}\n// mto: on\n";
  assert_eq!(formatted(source), source);
  let skipped = "#![rustfmt::skip]\nfn  f() {\n  x();\n}\n";
  assert_eq!(formatted(skipped), skipped);
  let broken = "fn f( {\n  x();\n";
  assert_eq!(
    format_file(broken, Edition::E2021, &FormatConfig::default()),
    broken
  );
}
