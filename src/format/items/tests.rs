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
fn test_brace_after_a_broken_parameter_list() {
  // The `{` stays while the closing line is within the line width less its indentation.
  let close = |n: usize| {
    let head = "        ) -> Poll<";
    format!("{}{}> {{", head, "x".repeat(n - head.len() - 3))
  };
  let source = |n: usize| {
    format!(
      "mod a {{\n    impl X {{\n        fn poll(\n            self: std::pin::Pin<&mut Self>,\n            cx: &mut std::task::Context<i8>,\n{}\n            x\n        }}\n    }}\n}}\n",
      close(n)
    )
  };
  assert!(formatted(&source(92)).contains("> {\n"));
  assert!(formatted(&source(93)).contains(">\n        {\n"));
}

#[test]
fn test_brace_style_always_next_line() {
  let config = FormatConfig {
    brace_next_line: true,
    ..FormatConfig::default()
  };
  let once = format_file("fn f(a: u8) {\n    a;\n}\n", Edition::E2021, &config);
  assert_eq!(once, "fn f(a: u8)\n{\n    a;\n}\n");
}

#[test]
fn test_declarations_keep_a_column_free() {
  // One line at 99 columns; at 100 the return type moves down; past it, parameters do.
  let decl = |n: usize| {
    let head = "    pub fn get_constants(this: &GpuProgrammableStage) -> Option<";
    format!("{}{}>;", head, "x".repeat(n - head.len() - 2))
  };
  let one = format!("extern \"C\" {{\n{}\n}}\n", decl(99));
  assert_eq!(formatted(&one), one);
  assert_eq!(
    formatted(&format!("extern \"C\" {{\n{}\n}}\n", decl(100))),
    format!(
      "extern \"C\" {{\n    pub fn get_constants(this: &GpuProgrammableStage)\n        -> Option<{}>;\n}}\n",
      "x".repeat(34)
    )
  );
  assert_eq!(
    formatted(&format!("extern \"C\" {{\n{}\n}}\n", decl(101))),
    format!(
      "extern \"C\" {{\n    pub fn get_constants(\n        this: &GpuProgrammableStage,\n    ) -> Option<{}>;\n}}\n",
      "x".repeat(35)
    )
  );
}

#[test]
fn test_headers_join_onto_one_line() {
  assert_eq!(
    formatted("pub\nfn\n  main<T>(\n  a: T,\n) -> u8\n{\n    1\n}\n"),
    "pub fn main<T>(a: T) -> u8 {\n    1\n}\n"
  );
  assert_eq!(
    formatted("extern\n    \"C\"\n{}\npub(crate)\n  struct\nS(u8)\n;\nimpl<T>\n  A for B<T> {}\n"),
    "extern \"C\" {}\npub(crate) struct S(u8);\nimpl<T> A for B<T> {}\n"
  );
}

#[test]
fn test_headers_left_as_written() {
  for kept in [
    "fn f<T>()\nwhere\n    T: Clone,\n{\n}\n",
    "fn f(\n    a: u8, // why\n) {\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_long_parameter_lists_take_a_line_each() {
  assert_eq!(
    formatted(
      "pub fn get_exclude_for_backend<'a>(backend: &str, metadata: &'a Metadata, other: u8) -> Vec<&'a str> {\n    x\n}\n"
    ),
    "pub fn get_exclude_for_backend<'a>(\n    backend: &str,\n    metadata: &'a Metadata,\n    other: u8,\n) -> Vec<&'a str> {\n    x\n}\n"
  );
}

#[test]
fn test_what_a_long_header_keeps() {
  // A C variadic takes no comma; a trait with bounds keeps its header as written.
  let long = format!(
    "extern \"C\" {{\n    pub fn sched_setscheduler(pid: crate::pid_t, policy: c_int, param: *const sched_param, {}: u8, ...) -> c_int;\n}}\n",
    "x".repeat(10)
  );
  assert!(formatted(&long).contains("        ...\n    ) -> c_int;"));
  let kept = "pub trait Bound:\n    Copy + Clone\n{\n}\n";
  assert_eq!(formatted(kept), kept);
}
