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
fn test_module_declarations_are_sorted() {
  assert_eq!(
    formatted("mod b;\nmod a;\nmod c {}\n", &FormatConfig::default()),
    "mod a;\nmod b;\nmod c {}\n"
  );
}

#[test]
fn test_runs_left_in_their_order() {
  let config = FormatConfig::default();
  // A comment in the run; the settings off; an `// mto: off` region.
  for kept in [
    "use c; // on c\nuse b;\n",
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
