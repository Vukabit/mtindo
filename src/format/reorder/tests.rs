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

#[test]
fn test_derives_merged_in_runs() {
  let source = "#[derive(A)] #[derive(B, C,)]\n#[cfg(x)]\n#[derive(D)]\nstruct S;\n#[derive(A)]\n\n#[derive(B)]\nstruct T;\nfn f() {\n    #[derive(A)]\n    #[derive(B)]\n    enum E {}\n}\n";
  assert_eq!(
    formatted(source, &FormatConfig::default()),
    "#[derive(A, B, C)]\n#[cfg(x)]\n#[derive(D)]\nstruct S;\n#[derive(A, B)]\nstruct T;\nfn f() {\n    #[derive(A, B)]\n    enum E {}\n}\n"
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

#[test]
fn test_module_declarations_are_sorted() {
  assert_eq!(
    formatted("mod b;\nmod a;\nmod c {}\n", &FormatConfig::default()),
    "mod a;\nmod b;\nmod c {}\n"
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
