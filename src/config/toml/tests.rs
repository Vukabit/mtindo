use super::*;

fn err(text: &str) -> Error {
    parse(text).expect_err(text)
}

fn ok(text: &str) -> Table {
    parse(text).unwrap_or_else(|e| panic!("{}\n{}", text, e))
}

#[test]
fn test_crlf_and_lone_carriage_returns() {
    let table = ok("a = 1\r\nb = '''x\r\ny'''\r\n");
    assert_eq!(table.get("b").and_then(Value::as_str), Some("x\ny"));
    assert!(err("a = 1\rb = 2\n").message.contains("carriage return"));
}

#[test]
fn test_datetimes() {
    let text = |t: &str| match ok(t).get("a") {
        Some(Value::Datetime(d)) => d.to_string(),
        other => panic!("{:?}", other),
    };
    assert_eq!(text("a = 1979-05-27"), "1979-05-27");
    assert_eq!(text("a = 07:32:00.999"), "07:32:00.999");
    assert_eq!(
        text("a = 1979-05-27t07:32:00-07:30"),
        "1979-05-27T07:32:00-07:30"
    );
    assert_eq!(text("a = 2000-02-29T23:59:60"), "2000-02-29T23:59:60");
    assert!(parse("a = 1999-02-29").is_err(), "not a leap year");
    assert!(parse("a = 07:32").is_err(), "TOML 1.0 requires seconds");
    assert!(
        parse("a = 07:32:00Z").is_err(),
        "a local time has no offset"
    );
}

#[test]
fn test_deep_nesting_is_refused_not_overflowed() {
    let deep = format!("a = {}{}\n", "[".repeat(10_000), "]".repeat(10_000));
    assert!(err(&deep).message.contains("nested more than"));
    let tables = format!("a = {}{}\n", "{ b = ".repeat(5_000), "}".repeat(5_000));
    assert!(parse(&tables).is_err());
}

#[test]
fn test_errors_point_at_the_problem() {
    let e = err("a = 1\nb = @\n");
    assert_eq!((e.line, e.column), (2, 5));
    assert!(e.message.contains("expected a value"), "{}", e.message);
    let e = err("a = 1\na = 2\n");
    assert_eq!((e.line, e.column), (2, 1));
    assert_eq!(e.message, "key `a` is defined twice");
    // Columns count characters, not bytes.
    let e = err("\"é\" = \"x\" 1\n");
    assert_eq!((e.line, e.column), (1, 11));
    assert!(e.message.contains("end of the line"), "{}", e);
}

#[test]
fn test_keys_remember_where_they_were_written() {
    let table = ok("# config\nmax_width = 80\n\n[section]\n  hard_tabs = true\n");
    let entry = table.entry("max_width").unwrap();
    assert_eq!((entry.line, entry.column), (2, 1));
    let section = table.get("section").and_then(Value::as_table).unwrap();
    let inner = section.entry("hard_tabs").unwrap();
    assert_eq!((inner.line, inner.column), (5, 3));
}

#[test]
fn test_numbers() {
    assert_eq!(ok("a = +1_000").get("a"), Some(&Value::Integer(1000)));
    assert_eq!(ok("a = -0").get("a"), Some(&Value::Integer(0)));
    assert!(parse("a = 01").is_err(), "leading zeros");
    assert!(parse("a = 9223372036854775808").is_err(), "out of range");
    assert_eq!(
        ok("a = -9223372036854775808").get("a"),
        Some(&Value::Integer(i64::MIN))
    );
    assert!(matches!(ok("a = -inf").get("a"), Some(Value::Float(f)) if *f == f64::NEG_INFINITY));
    assert!(matches!(ok("a = nan").get("a"), Some(Value::Float(f)) if f.is_nan()));
    assert!(parse("a = 1.").is_err());
    assert!(parse("a = .5").is_err());
    assert!(parse("a = 1e_5").is_err());
}

#[test]
fn test_strings() {
    let table = ok(
        "a = \"tab\\there \\u00e9 \\U0001F600\"\nb = \"\"\"\nline one \\\n    continued\"\"\"\nc = '''\n'quoted' \"\" '''''\nd = \"\"\"two quotes at the end\"\"\"\"\"\n",
    );
    assert_eq!(
        table.get("a").and_then(Value::as_str),
        Some("tab\there é 😀")
    );
    assert_eq!(
        table.get("b").and_then(Value::as_str),
        Some("line one continued")
    );
    assert_eq!(
        table.get("c").and_then(Value::as_str),
        Some("'quoted' \"\" ''")
    );
    assert_eq!(
        table.get("d").and_then(Value::as_str),
        Some("two quotes at the end\"\"")
    );
    assert!(
        parse("a = \"\\ud800\"").is_err(),
        "surrogates are not characters"
    );
    assert!(
        parse("a = \"bell \u{7}\"").is_err(),
        "control characters are refused"
    );
}

#[test]
fn test_table_rules() {
    // A table may be defined once; parents made by deeper headers may follow once.
    ok("[a.b]\nx = 1\n[a]\ny = 2\n");
    assert!(err("[a]\n[a]\n").message.contains("defined twice"));
    // Dotted keys make tables that headers may not reopen, but may add sub-tables to.
    ok("[fruit]\napple.color = 'red'\n[fruit.apple.texture]\nsmooth = true\n");
    assert!(
        err("[fruit]\napple.color = 'red'\n[fruit.apple]\n")
            .message
            .contains("dotted keys")
    );
    // Inline tables are complete as written.
    assert!(
        err("a = { b = 1 }\na.c = 2\n")
            .message
            .contains("inline table")
    );
    assert!(
        err("a = { b = 1 }\n[a.c]\n")
            .message
            .contains("inline table")
    );
    // Arrays written as values cannot be extended by [[headers]].
    assert!(parse("a = []\n[[a]]\n").is_err());
    let aot = ok("[[a]]\nx = 1\n[[a]]\nx = 2\n[a.b]\ny = 3\n");
    let items = aot.get("a").and_then(Value::as_array).unwrap();
    assert_eq!(items.len(), 2);
    assert!(
        items[1].as_table().unwrap().get("b").is_some(),
        "headers go into the last table"
    );
}

#[test]
fn test_value_types_and_accessors() {
    let table = ok(
        "s = 'lit'\ni = 0xff\nf = 6.5e-1\nb = false\nd = 1979-05-27 07:32:00Z\na = [1, 'two']\nt = { x = 1 }\n",
    );
    assert_eq!(table.get("s").and_then(Value::as_str), Some("lit"));
    assert!(
        parse("i = 0x_ff").is_err(),
        "an underscore must sit between digits"
    );
    let table2 = ok("i = 0xff\n");
    assert_eq!(table2.get("i").and_then(Value::as_integer), Some(255));
    assert_eq!(table.get("f"), Some(&Value::Float(0.65)));
    assert_eq!(table.get("b").and_then(Value::as_bool), Some(false));
    match table.get("d") {
        Some(Value::Datetime(d)) => assert_eq!(d.to_string(), "1979-05-27T07:32:00Z"),
        other => panic!("{:?}", other),
    }
    assert_eq!(
        table.get("a").and_then(Value::as_array).map(|a| a.len()),
        Some(2)
    );
    assert_eq!(table.get("t").map(Value::type_name), Some("table"));
    assert_eq!(table.len(), 7);
}
