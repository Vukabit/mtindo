use lofmt::{extract_blocks, reflow_delimited_blocks, wrap_line, ItemKind};

#[test]
fn test_reflow_delimited_blocks_ignores_strings() {
    let input = "pub fn c_cyan() -> &'static str { if true { \"\\x1b[1;36m\" } else { \"\" } }";
    let result = reflow_delimited_blocks(input, 10, 4);
    assert!(result.contains("\"\\x1b[1;36m\""));
    assert!(!result.contains("[\n"));
}

#[test]
fn test_reflow_delimited_blocks_unpacks_args() {
    let input = "fn complex_call(a: usize, b: usize, c: usize, d: usize)";
    let result = reflow_delimited_blocks(input, 20, 4);
    assert!(result.contains("(\n"));
    assert!(result.contains("    a: usize,\n"));
}

#[test]
fn test_wrap_line_isolates_comments() {
    let input = "let x = 5; // This is a very long comment that must be safely isolated and wrapped independently.";
    let result = wrap_line(input, 40, 4);
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines[0], "let x = 5; // This is a very long");
    assert_eq!(lines[1], "// comment that must be safely isolated");
}

#[test]
fn test_extract_blocks_handles_raw_strings() {
    let input = "fn get_raw() {\n    let s = r#\" { struct Fake; } \"#;\n}\nstruct Real { }";
    let blocks = extract_blocks(input);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].kind, ItemKind::FnDecl);
    assert_eq!(blocks[1].kind, ItemKind::StructDecl);
}

#[test]
fn test_wrap_line_avoids_truncating_long_tokens() {
    let input = "let very_long_variable_name_that_should_not_be_cut = 1;";
    let result = wrap_line(input, 10, 4);
    assert!(result.contains("let"));
    assert!(result.contains("very_long_variable_name_that_should_not_be_cut"));
}
