use super::*;

fn lone(text: &str, normalize: bool) -> String {
  let root = crate::syntax::parse(text, crate::syntax::Edition::E2021).syntax_node();
  lone_comments(&root, text, &FormatConfig::default(), normalize)
    .unwrap_or_else(|| text.to_string())
}

fn normalize_text(text: &str) -> String {
  let root = crate::syntax::parse(text, crate::syntax::Edition::E2021).syntax_node();
  normalize(&root, text, &FormatConfig::default()).unwrap_or_else(|| text.to_string())
}

#[test]
fn test_a_block_comment_ending_its_line_becomes_a_line_comment() {
  assert_eq!(normalized("/* sit amet: */", false, ""), "// sit amet:");
  assert_eq!(
    normalized("/*yes, it's possible!  */", false, ""),
    "// yes, it's possible!"
  );
  assert_eq!(normalized("/** doc */", false, ""), "/// doc");
  assert_eq!(normalized("/*! inner */", false, ""), "//! inner");
}

#[test]
fn test_a_block_comment_over_lines_becomes_one_line_comment_per_line() {
  assert_eq!(
    normalized("/*\n     * a\n     * b\n     */", false, "    "),
    "// a\n    // b"
  );
}

#[test]
fn test_a_body_holding_one_block_comment_is_laid_out_by_its_kind() {
  assert_eq!(
    lone("impl E { /* e */ }\n", false),
    "impl E {\n    /* e */\n}\n"
  );
  assert_eq!(lone("mod m {/* f */}\n", false), "mod m { /* f */\n}\n");
  assert_eq!(lone("fn i() {/* i */\n}\n", false), "fn i() { /* i */\n}\n");
  assert_eq!(lone("enum C { /* c */ }\n", false), "enum C {/* c */}\n");
  assert_eq!(
    lone("fn f() {\n    let x = { /* d */ };\n}\n", false),
    "fn f() {\n    let x = { /* d */ };\n}\n"
  );
}

#[test]
fn test_a_comment_with_code_after_it_stays_as_written() {
  let text = "fn f(/* a */ x: u8) {\n    let y = 1; /* b */\n    /*c*/\n}\n";
  assert_eq!(
    normalize_text(text),
    "fn f(/* a */ x: u8) {\n    let y = 1; // b\n    // c\n}\n"
  );
}

#[test]
fn test_a_normalized_lone_comment_goes_on_a_line_of_its_own() {
  assert_eq!(
    lone("struct B {/* b */}\n", true),
    "struct B {\n    // b\n}\n"
  );
  assert_eq!(
    lone("struct H(/* h */);\n", true),
    "struct H(\n    // h\n);\n"
  );
  assert_eq!(lone("mod m { /* f */ }\n", true), "mod m { // f\n}\n");
}

#[test]
fn test_comments_in_macro_arguments_stay_as_written() {
  let text = "m! {\n    /* a */\n}\n";
  assert_eq!(normalize_text(text), text);
}

#[test]
fn test_line_comments_get_a_space_and_keep_a_trailing_empty_line() {
  assert_eq!(normalized("//a\n//b", false, ""), "// a\n// b");
  assert_eq!(normalized("// a\n//", false, ""), "// a\n//");
  assert_eq!(normalized("///a", false, ""), "/// a");
  assert_eq!(normalized("//", false, ""), "//");
}
