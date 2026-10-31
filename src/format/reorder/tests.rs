use super::{PassNodes, explicit_abi, strip_semicolons};
use crate::format::file::format_file;
use crate::options::format::FormatConfig;
use crate::syntax::Edition;
use crate::verify::format::verify_format;

/// `source` formatted, checked for meaning and stability.
fn formatted(source: &str, config: &FormatConfig) -> String {
  let once = format_file(source, Edition::E2021, config);
  let found = verify_format(source, &once, Edition::E2021);
  assert!(found.is_empty(), "{:?}\n{}", found, once);
  assert_eq!(
    format_file(&once, Edition::E2021, config),
    once,
    "not stable"
  );
  once
}

#[test]
fn test_a_comment_after_the_last_import_stays_last() {
  let config = FormatConfig::default();
  assert_eq!(
    formatted(
      "use syntax::{AstNode, SmolStr};\nuse crate::{AssistContext,\n    AssistId}; // Assist: reorder\n// more\n\nfn f() {}\n",
      &config
    ),
    "use crate::{AssistContext, AssistId};\nuse syntax::{AstNode, SmolStr}; // Assist: reorder\n                                // more\n\nfn f() {}\n"
  );
}

#[test]
fn test_comments_move_with_their_declarations() {
  // A comment after a declaration's `;` goes with it; the last one's stays after the run.
  assert_eq!(
    formatted("use c; // on c\nuse b;\n", &FormatConfig::default()),
    "use b;\nuse c; // on c\n"
  );
}

/// `control_brace_style`: `{` and `else` on lines of their own, an arm's block's and a
/// `let`'s `else` too, and a control expression no longer overflows a call; with
/// `ClosingNextLine` only `else` moves. One on one line stays there.
#[test]
fn test_control_brace_styles() {
  use crate::options::format::ControlBraceStyle;
  let source = "fn f() {\n    if a { b(); } else if c { d(); } else { e(); }\n    let x = if a { 1 } else { 2 };\n    match m { A => {}, B if g => { h(); i() } }\n    while w { k(); }\n    let Some(v) = o else { return };\n    foo(if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa { bbbbbbbbbbbbbbbbbbbbbbbb() } else { ccccccccccccccccccccccc() });\n}\n";
  let next = FormatConfig {
    control_brace_style: ControlBraceStyle::AlwaysNextLine,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(source, &next),
    "fn f() {\n    if a\n    {\n        b();\n    }\n    else if c\n    {\n        d();\n    }\n    else\n    {\n        e();\n    }\n    let x = if a { 1 } else { 2 };\n    match m\n    {\n        A =>\n        {}\n        B if g =>\n        {\n            h();\n            i()\n        }\n    }\n    while w\n    {\n        k();\n    }\n    let Some(v) = o\n    else\n    {\n        return;\n    };\n    foo(\n        if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n        {\n            bbbbbbbbbbbbbbbbbbbbbbbb()\n        }\n        else\n        {\n            ccccccccccccccccccccccc()\n        },\n    );\n}\n"
  );
  let closing = FormatConfig {
    control_brace_style: ControlBraceStyle::ClosingNextLine,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(source, &closing),
    "fn f() {\n    if a {\n        b();\n    }\n    else if c {\n        d();\n    }\n    else {\n        e();\n    }\n    let x = if a { 1 } else { 2 };\n    match m {\n        A => {}\n        B if g => {\n            h();\n            i()\n        }\n    }\n    while w {\n        k();\n    }\n    let Some(v) = o\n    else {\n        return;\n    };\n    foo(if aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa {\n        bbbbbbbbbbbbbbbbbbbbbbbb()\n    }\n    else {\n        ccccccccccccccccccccccc()\n    });\n}\n"
  );
}

#[test]
fn test_derives_merged_in_runs() {
  let source = "#[derive(A)] #[derive(B, C,)]\n#[cfg(x)]\n#[derive(D)]\nstruct S;\n#[derive(A)]\n\n#[derive(B)]\nstruct T;\nfn f() {\n    #[derive(A)]\n    #[derive(B)]\n    enum E {}\n}\n";
  assert_eq!(
    formatted(source, &FormatConfig::default()),
    "#[derive(A, B, C)]\n#[cfg(x)]\n#[derive(D)]\nstruct S;\n#[derive(A, B)]\nstruct T;\nfn f() {\n    #[derive(A, B)]\n    enum E {}\n}\n"
  );
  // A run with a derive of anything but meta items stays as written whole.
  let unread = "#[derive(Eq)]\n#[derive(Copy, C; mod m;)]\nenum E {}\n\n#[derive(A)]\n#[derive(B)]\n#[derive(x = 1)]\nenum F {}\n";
  assert_eq!(
    formatted(unread, &FormatConfig::default()),
    "#[derive(Eq)]\n#[derive(Copy, C; mod m;)]\nenum E {}\n\n#[derive(A, B, x = 1)]\nenum F {}\n"
  );
  let kept = FormatConfig {
    merge_derives: false,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted("#[derive(A)]\n#[derive(B)]\nstruct S;\n", &kept),
    "#[derive(A)]\n#[derive(B)]\nstruct S;\n"
  );
}

/// `enum_discrim_align_threshold`: variants' `=` lined up after the widest name at most that
/// wide; a wider one, and a variant without a discriminant, are left as they are.
#[test]
fn test_discriminants_aligned() {
  let config = FormatConfig {
    enum_discrim_align_threshold: 10,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(
      "enum E {\n    A = 1,\n    Bcdef = 2,\n    C,\n    LongerThanTen = 3,\n}\n",
      &config
    ),
    "enum E {\n    A     = 1,\n    Bcdef = 2,\n    C,\n    LongerThanTen = 3,\n}\n"
  );
}

/// `#[doc = "..."]` written as doc comments, a line each, `//!` for an inner one; code after
/// it on its line goes to the next. One with an escape doc comments cannot show plainly, or
/// with a comment, is left as written.
#[test]
fn test_doc_attributes_normalized() {
  let config = FormatConfig {
    normalize_doc_attributes: true,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(
      "#![doc = \" Crate\"]\n\n#[doc = \"One\\ntwo \\\"q\\\"\"]\n#[doc = \"\"]\n#[doc = \"\\u{1F600}\"]\nstruct A {\n    #[doc = \" field\"] b: i32,\n    #[doc = r#\"raw \"x\"\"#]\n    c: i32,\n}\n",
      &config
    ),
    "//! Crate\n\n///One\n///two \"q\"\n///\n#[doc = \"\\u{1F600}\"]\nstruct A {\n    /// field\n    b: i32,\n    ///raw \"x\"\n    c: i32,\n}\n"
  );
}

#[test]
fn test_empty_statements_dropped() {
  let source = "fn f() {;\n    x();;\n    struct S {};\n}\n";
  let root = crate::syntax::parse(source, crate::syntax::Edition::E2021).syntax_node();
  assert_eq!(
    strip_semicolons(&root, &mut PassNodes::default(), source).as_deref(),
    Some("fn f() {\n    x();\n    struct S {};\n}\n")
  );
}

#[test]
fn test_extern_given_its_abi() {
  let source = "extern {}\nextern fn f() {}\ntype F = extern fn();\nextern crate a;\nextern \"system\" fn g() {}\n";
  let root = crate::syntax::parse(source, crate::syntax::Edition::E2021).syntax_node();
  assert_eq!(
    explicit_abi(&root, source).as_deref(),
    Some(
      "extern \"C\" {}\nextern \"C\" fn f() {}\ntype F = extern \"C\" fn();\nextern crate a;\nextern \"system\" fn g() {}\n"
    )
  );
}

/// `inline_attribute_width`: an import, `extern crate` or `mod` and its one attribute on a
/// line when the two are at most that wide; two attributes stay apart.
#[test]
fn test_inline_attributes() {
  let config = FormatConfig {
    inline_attribute_width: 50,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(
      "#[cfg(feature = \"alloc\")]\nuse core::slice;\n\n#[cfg(feature = \"alloc\")]\nuse total_len_is::_51___;\n\n#[cfg(feature = \"alloc\")]\nextern crate len_is_50_;\n\n#[cfg(unix)]\nmod m;\n\n#[a]\n#[b]\nuse c;\n",
      &config
    ),
    "#[cfg(feature = \"alloc\")] use core::slice;\n\n#[cfg(feature = \"alloc\")]\nuse total_len_is::_51___;\n\n#[cfg(feature = \"alloc\")] extern crate len_is_50_;\n\n#[cfg(unix)] mod m;\n\n#[a]\n#[b]\nuse c;\n"
  );
}

#[test]
fn test_module_declarations_are_sorted() {
  assert_eq!(
    formatted("mod b;\nmod a;\nmod c {}\n", &FormatConfig::default()),
    "mod a;\nmod b;\nmod c {}\n"
  );
}

/// Number literals as `hex_literal_case` and `float_literal_trailing_zero` write them, in
/// macro arguments too; a tuple field's index is left alone, and a float left ending in `.`
/// before a method or a range is kept apart from it.
#[test]
fn test_number_literals_rewritten() {
  use crate::options::format::{FloatTrailingZero, HexLiteralCase};
  let source = "fn f() {\n    let a = [0xabu8, 1.0, 1f32, 1.5e3];\n    let b = 1.0.max(x.0.0);\n    m!(0xab, 1.0..2.0);\n}\n";
  let upper = FormatConfig {
    hex_literal_case: HexLiteralCase::Upper,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(source, &upper),
    "fn f() {\n    let a = [0xABu8, 1.0, 1f32, 1.5e3];\n    let b = 1.0.max(x.0 .0);\n    m!(0xAB, 1.0..2.0);\n}\n"
  );
  let always = FormatConfig {
    float_trailing_zero: FloatTrailingZero::Always,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(source, &always),
    "fn f() {\n    let a = [0xabu8, 1.0, 1.0f32, 1.5e3];\n    let b = 1.0.max(x.0 .0);\n    m!(0xab, 1.0..2.0);\n}\n"
  );
  let never = FormatConfig {
    float_trailing_zero: FloatTrailingZero::Never,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(source, &never),
    "fn f() {\n    let a = [0xabu8, 1., 1f32, 1.5e3];\n    let b = (1.).max(x.0 .0);\n    m!(0xab, 1. ..2.);\n}\n"
  );
}

#[test]
fn test_runs_left_in_their_order() {
  let config = FormatConfig::default();
  // A comment between them ends the run; the settings off; an `// mto: off` region.
  for kept in [
    "use c;\n// between\nuse b;\n",
    "// mto: off\nuse c;\nuse b;\n// mto: on\n",
  ] {
    assert_eq!(formatted(kept, &config), kept);
  }
  let off = FormatConfig {
    reorder_imports: false,
    reorder_modules: false,
    ..FormatConfig::default()
  };
  let kept = "use c;\nuse b;\nmod b;\nmod a;\n";
  assert_eq!(formatted(kept, &off), kept);
}

#[test]
fn test_runs_of_imports_are_sorted() {
  let config = FormatConfig::default();
  // A blank line ends a run; attributes and doc comments move with their import.
  assert_eq!(
    formatted(
      "use c;\nuse self::x;\nuse b;\n\nuse z;\nuse y;\nuse x; use w;\n#[cfg(test)]\nuse v;\n/// doc\nuse u;\n",
      &config
    ),
    "use self::x;\nuse b;\nuse c;\n\n/// doc\nuse u;\n#[cfg(test)]\nuse v;\nuse w;\nuse x;\nuse y;\nuse z;\n"
  );
  // `#[macro_use]` ends a run and stays.
  let kept = "#[macro_use]\nuse t;\nuse s;\n";
  assert_eq!(formatted(kept, &config), kept);
}

/// `fn_single_line` puts a function of one statement on its header's line, not a macro
/// statement nor one with a comment; without `empty_item_single_line` an empty body's
/// braces take a line each.
#[test]
fn test_single_line_items() {
  let config = FormatConfig {
    fn_single_line: true,
    empty_item_single_line: false,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(
      "fn a() -> i32 {\n    1\n}\nfn b() {\n    print!(\"x\");\n}\nfn c() {\n    // note\n    1\n}\nfn d() {}\nimpl A {}\n",
      &config
    ),
    "fn a() -> i32 { 1 }\nfn b() {\n    print!(\"x\");\n}\nfn c() {\n    // note\n    1\n}\nfn d() {\n}\nimpl A {\n}\n"
  );
  // A `{` on a line of its own (`brace_style`) leaves the function on lines.
  let next_line = FormatConfig {
    brace_next_line: true,
    ..config.clone()
  };
  assert_eq!(
    formatted("fn lorem() -> i32\n{\n    42\n}\n", &next_line),
    "fn lorem() -> i32\n{\n    42\n}\n"
  );
}

/// `struct_field_align_threshold`: fields' types lined up in each group between blank
/// lines whose names' widths differ by at most it; a literal's too, but for one whose
/// names differ more or that has a field written by its name alone.
#[test]
fn test_struct_fields_aligned() {
  let config = FormatConfig {
    struct_field_align_threshold: 20,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(
      "struct Foo {\n    a: u8,\n    bbbbbb: u16,\n    /// doc\n    pub c: u32,\n\n    dddddddddd: u8,\n    e: u8,\n}\n\nfn f() {\n    let x = Foo {\n        a: 1,\n        bbbbbb: 2,\n        cccccccccccccccccccccccccccccccc: 3,\n        dddddddddddddddddddddddddddddddddddddddddd: 4,\n    };\n    let y = Foo {\n        a: 1,\n        bbbb: 2,\n        c: cccccccccccccccccccccccccccccccc(),\n        dddddddddd: d,\n        e,\n    };\n    m!(Foo {\n        a: 1,\n        bbbb: 2\n    });\n}\n",
      &config
    ),
    "struct Foo {\n    a:      u8,\n    bbbbbb: u16,\n    /// doc\n    pub c:  u32,\n\n    dddddddddd: u8,\n    e:          u8,\n}\n\nfn f() {\n    let x = Foo {\n        a: 1,\n        bbbbbb: 2,\n        cccccccccccccccccccccccccccccccc: 3,\n        dddddddddddddddddddddddddddddddddddddddddd: 4,\n    };\n    let y = Foo {\n        a: 1,\n        bbbb: 2,\n        c: cccccccccccccccccccccccccccccccc(),\n        dddddddddd: d,\n        e,\n    };\n    m!(Foo { a: 1, bbbb: 2 });\n}\n"
  );
}

/// `try!(e)` written `e?`, one inside another too; arguments `?` would not bind to whole,
/// or that are not one expression, are left as written.
#[test]
fn test_try_shorthand() {
  let config = FormatConfig {
    try_shorthand: true,
    ..FormatConfig::default()
  };
  let source = "fn f() {\n    let a = try!(try!(d).e);\n    try!(g(|| { 1 }));\n    let b = try!(a + b);\n    let c = try!(i(), j);\n}\n";
  let once = format_file(source, Edition::E2015, &config);
  assert!(verify_format(source, &once, Edition::E2015).is_empty());
  assert_eq!(
    once,
    "fn f() {\n    let a = d?.e?;\n    g(|| 1)?;\n    let b = try!(a + b);\n    let c = try!(i(), j);\n}\n"
  );
  assert_eq!(format_file(&once, Edition::E2015, &config), once);
}

/// `where_single_line`: a `where` of one predicate on its line, a function's `{` after it
/// and a struct's on the next line; one of two predicates stays on lines.
#[test]
fn test_where_single_line() {
  let config = FormatConfig {
    where_single_line: true,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(
      "fn f<T>() -> T where T: Eq { x }\nstruct GroupInner<K, I, F> where I: Iterator { key: F, iter: I, current_key: Option<K> }\nfn g<T, U>() where T: Eq, U: Eq {}\n",
      &config
    ),
    "fn f<T>() -> T\nwhere T: Eq {\n    x\n}\nstruct GroupInner<K, I, F>\nwhere I: Iterator\n{\n    key: F,\n    iter: I,\n    current_key: Option<K>,\n}\nfn g<T, U>()\nwhere\n    T: Eq,\n    U: Eq,\n{\n}\n"
  );
}

/// The last two or more wildcards of a tuple pattern written `..`, a wildcard with a comment
/// the first of them; not one with a `..` already, which would then have two.
#[test]
fn test_wildcard_suffixes_condensed() {
  let config = FormatConfig {
    condense_wildcards: true,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted(
      "fn f() {\n    let (a, _, _) = t;\n    let S(_, _) = s;\n    let (a, _) = t;\n    let (a, .., _, _) = t;\n    let (a, _, /* c */ _, _) = t;\n}\n",
      &config
    ),
    "fn f() {\n    let (a, ..) = t;\n    let S(..) = s;\n    let (a, _) = t;\n    let (a, .., _, _) = t;\n    let (a, _, /* c */ ..) = t;\n}\n"
  );
}

#[test]
fn test_without_blank_lines_a_run_is_everything_adjacent() {
  let config = FormatConfig {
    blank_lines_upper_bound: 0,
    ..FormatConfig::default()
  };
  assert_eq!(
    formatted("use syn::a;\n\nuse crate::b;\n", &config),
    "use crate::b;\nuse syn::a;\n"
  );
}
