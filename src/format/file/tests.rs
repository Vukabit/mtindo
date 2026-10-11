use super::*;
use crate::verify::format::verify_format;

fn formatted(source: &str) -> String {
    let config = FormatConfig::default();
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
fn test_blank_lines_and_trailing_whitespace() {
    assert_eq!(
        formatted("\n\nfn a() {   \n\n\n    x();  \n\n}\n\n\n\nfn b() {} // note   \n\n\n"),
        "fn a() {\n    x();\n}\n\nfn b() {} // note\n"
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
        formatted("fn f() {\n    let g = |a: u8,\n             b: u8| {\n  a\n    };\n}\n"),
        "fn f() {\n    let g = |a: u8,\n             b: u8| {\n        a\n    };\n}\n"
    );
    // A method call's arguments follow the line of its `.`.
    assert_eq!(
        formatted("fn f() {\n    x\n        .g(\n  1,\n  );\n}\n"),
        "fn f() {\n    x\n        .g(\n            1,\n        );\n}\n"
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
        formatted("fn f() {\n  let x = foo\n      .bar()\n      .baz();\n}\n"),
        "fn f() {\n    let x = foo\n        .bar()\n        .baz();\n}\n"
    );
    // Arguments on their own lines are one level past the call's line.
    assert_eq!(
        formatted("fn f() {\n    g(\n  a,\n          b,\n  );\n}\n"),
        "fn f() {\n    g(\n        a,\n        b,\n    );\n}\n"
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
fn test_what_is_left_as_written() {
    let source = "macro_rules! m {\n  () => {\n        1\n  };\n}\n#[rustfmt::skip]\nfn  f() {\n  x( );\n}\n// mto: off\nfn   g() {\n x();\n}\n// mto: on\n";
    assert_eq!(formatted(source), source);
    let skipped = "#![rustfmt::skip]\nfn  f() {\n  x();\n}\n";
    assert_eq!(formatted(skipped), skipped);
    let broken = "fn f( {\n  x();\n";
    assert_eq!(
        format_file(broken, Edition::E2021, &FormatConfig::default()),
        broken
    );
}
