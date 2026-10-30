use crate::format::file::format_file;
use crate::options::format::FormatConfig;
use crate::syntax::Edition;
use crate::verify::format::verify_format;

/// `source` formatted, checked for meaning and stability.
fn formatted(source: &str) -> String {
  let config = FormatConfig::default();
  let once = format_file(source, Edition::E2021, &config);
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
fn test_a_colon_is_spaced_before_any_type() {
  // A struct-like variant's fields joined on one line, with types opening a delimiter.
  assert_eq!(
    formatted("enum E {\n    A { x:[u8; 4], y:(u8,u8) },\n    B { z:<T as A>::B },\n}\n"),
    "enum E {\n    A { x: [u8; 4], y: (u8, u8) },\n    B { z: <T as A>::B },\n}\n"
  );
}

#[test]
fn test_a_statement_too_wide_on_its_first_line_is_left_as_written() {
  let long = format!("fn f() {{\n    let x= g(\"{}\") ;\n}}\n", "x".repeat(90));
  assert_eq!(formatted(&long), long);
}

#[test]
fn test_cases_the_standard_formatter_writes_its_own_way() {
  // Each of these was once spaced wrongly; each is left as the standard formatter writes it.
  for kept in [
    "#[unsafe(no_mangle)]\nfn f() {}\n",
    "pub struct E(pub(super) ());\n",
    "struct S<const N: usize>(pub [u8; N]);\n",
    "struct M(#[allow(dead_code)] [usize; 4]);\n",
    "impl<'de> ::serde::Deserialize<'de> for F {}\n",
    "type R = Option<<T as A>::B>;\n",
    "type R = Option< <T as A>::B>;\n",
    "fn f() {\n    x.0 .0;\n    y.1 .0;\n    1.into();\n}\n",
    "fn f() {\n    if a > (b) {}\n}\n",
    "macro_rules! m (\n    () => {}\n);\n",
    "fn f() {\n    let s = quote! (span => #s);\n}\n",
    "fn f() {\n    if a == b\n        // why\n        ||!c\n    {}\n}\n",
    "#[attr(is = | _ | false , path = :: a :: B)]\nfn f() {}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_generics_closures_and_braces() {
  assert_eq!(
    formatted("fn f < T : Clone > ( v : Vec < Vec < T > > ) { }\n"),
    "fn f<T: Clone>(v: Vec<Vec<T>>) {}\n"
  );
  assert_eq!(
    formatted("fn f() { let g = | a , b | a + b ; S {a , b} ; }\n"),
    "fn f() {\n    let g = |a, b| a + b;\n    S { a, b };\n}\n"
  );
}

#[test]
fn test_operators_are_spaced_and_prefixes_bind() {
  assert_eq!(
    formatted("fn f() {\n    let x=a+b*- c ;\n}\n"),
    "fn f() {\n    let x = a + b * -c;\n}\n"
  );
  assert_eq!(
    formatted("fn f(x :& mut u8) -> bool { * x<1 && ! g ( & x ) }\n"),
    "fn f(x: &mut u8) -> bool {\n    *x < 1 && !g(&x)\n}\n"
  );
  assert_eq!(
    formatted("fn f() { for i in 0 .. n { a [ ..= 2 ] ; } }\n"),
    "fn f() {\n    for i in 0..n {\n        a[..=2];\n    }\n}\n"
  );
}

#[test]
fn test_paths_calls_and_fields_are_tight() {
  assert_eq!(formatted("use  std :: io ;\n"), "use std::io;\n");
  assert_eq!(
    formatted("fn f() {\n    a . b ( 1 ,2 ) [ 0 ] . c ;\n}\n"),
    "fn f() {\n    a.b(1, 2)[0].c;\n}\n"
  );
  assert_eq!(formatted("use a :: { b ,c };\n"), "use a::{b, c};\n");
  assert_eq!(
    formatted("#[cfg(x)]\npub(crate)  use a::b;\n"),
    "#[cfg(x)]\npub(crate) use a::b;\n"
  );
  // The macro's name and `!` are tightened, and its arguments laid out as a call's.
  assert_eq!(
    formatted("fn f() { foo ! ( x ) ; }\n"),
    "fn f() {\n    foo!(x);\n}\n"
  );
}

#[test]
fn test_spacing_settings() {
  let config = FormatConfig {
    spaces_around_ranges: true,
    space_before_colon: true,
    space_after_colon: false,
    compressed_type_punctuation: true,
    ..FormatConfig::default()
  };
  let source = "fn f<T: A + B>(x: u8) { let r = 0..x; g::<Item = u8>(); }\n";
  let once = format_file(source, Edition::E2021, &config);
  assert_eq!(
    once,
    "fn f<T :A+B>(x :u8) {\n    let r = 0 .. x;\n    g::<Item=u8>();\n}\n"
  );
  assert!(verify_format(source, &once, Edition::E2021).is_empty());
  assert_eq!(format_file(&once, Edition::E2021, &config), once);
}

#[test]
fn test_what_spacing_leaves_alone() {
  // Macro arguments that do not read as expressions, and tokens that would merge; a trailing
  // comment gets one space.
  assert_eq!(
    formatted("fn f() {\n    m!( a  => b );\n    x;\t  // note\n}\n"),
    "fn f() {\n    m!( a  => b );\n    x; // note\n}\n"
  );
  assert_eq!(
    formatted("fn f() { a - - b; }\n"),
    "fn f() {\n    a - -b;\n}\n"
  );
  assert_eq!(formatted("fn f() { & & x; }\n"), "fn f() {\n    &&x;\n}\n");
}
