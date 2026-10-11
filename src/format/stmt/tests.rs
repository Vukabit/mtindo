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
fn test_calls_are_joined_and_broken_to_fit() {
  assert_eq!(
    formatted("fn f() {\n    let x = foo(\n        a,\n        b,\n    );\n}\n"),
    "fn f() {\n    let x = foo(a, b);\n}\n"
  );
  let long = "fn f() {\n    call(first_argument_name, second_argument_name, third_argument_name, fourth);\n}\n";
  assert_eq!(
    formatted(long),
    "fn f() {\n    call(\n        first_argument_name,\n        second_argument_name,\n        third_argument_name,\n        fourth,\n    );\n}\n"
  );
}

#[test]
fn test_chains_as_the_standard_formatter_lays_them_out() {
  for kept in [
    // A tuple receiver spanning lines: its links one level in.
    "fn f() {\n    let res = (\n        StatusCode::SEE_OTHER,\n        [(\"location\", \"foo\")],\n        StatusCode::NO_CONTENT,\n    )\n        .into_response();\n}\n",
    // A call receiver spanning lines: its links at its indentation.
    "fn f() {\n    cmd.error(\n        ErrorKind::ArgumentConflict,\n        \"Can't do relative and absolute version change\",\n    )\n    .exit();\n}\n",
    // A chain overflowing an argument list keeps its links on one line.
    "fn f() {\n    let h = tokio::spawn(KEY.scope(\n        42,\n        MyFuture {\n            tx_poll: Some(tx_poll),\n            tx_drop: Some(tx_drop),\n        },\n    ));\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_layouts_keep_every_token() {
  // A float before a method keeps its space; a rest with no base keeps its `..`; generic
  // lists closing together stay as they are.
  for kept in [
    "fn f() {\n    0. .to_string();\n}\n",
    "fn f() {\n    let s = S { x: 5, .. };\n}\n",
    "fn f() {\n    let x = foo::<Vec<u8>>(a, b);\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_let_patterns_and_types() {
  // A struct pattern past `struct_lit_width` takes a field per line, the initializer after
  // its closing brace; a pattern spanning lines leaves the rest of its last line.
  assert_eq!(
    formatted("fn f() {\n    let Tokens { all_columns, all_variants } = generate(&a, kind);\n}\n"),
    "fn f() {\n    let Tokens {\n        all_columns,\n        all_variants,\n    } = generate(&a, kind);\n}\n"
  );
  for kept in [
    "fn f() {\n    let [a, .., b] = f(x);\n}\n",
    "fn f() {\n    let T(a, b) = f(x);\n}\n",
    "fn f() {\n    let (a,) = f(x);\n}\n",
    "fn f() {\n    let S { x, .. }: S = e(1);\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
  // A type is laid out with the rest of the line.
  assert_eq!(
    formatted("fn f() {\n    let x: Vec<u8> = f(\n        a,\n    );\n}\n"),
    "fn f() {\n    let x: Vec<u8> = f(a);\n}\n"
  );
}

#[test]
fn test_statements_left_to_the_whole_file_rules() {
  // A comment, a block, a macro: kept as written.
  for kept in [
    "fn f() {\n    let x = foo(\n        a, // why\n        b,\n    );\n}\n",
    "fn f() {\n    let x = foo(\n        a,\n        || {\n            b\n        },\n    );\n}\n",
    "fn f() {\n    let x = foo(\n        a,\n        m!(b),\n    );\n}\n",
  ] {
    assert_eq!(formatted(kept), kept);
  }
}

#[test]
fn test_width_settings_reach_the_layout() {
  // At the defaults a 66-column argument list breaks (fn_call_width is 60); with
  // use_small_heuristics = "Max" it may run to the line width.
  let source =
    "fn f() {\n    call(first_argument_name, second_argument_name, third_argument, fourth);\n}\n";
  let broken = format_file(source, Edition::E2021, &FormatConfig::default());
  assert!(broken.contains("call(\n"), "{}", broken);
  let max = FormatConfig {
    fn_call_width: 100,
    chain_width: 100,
    struct_lit_width: 100,
    array_width: 100,
    ..FormatConfig::default()
  };
  assert_eq!(format_file(source, Edition::E2021, &max), source);
}
