//! `toml-test DIR`: the TOML conformance suite against `mtindo::config::toml`.
//!
//! Every case `tests/files-toml-1.0.0` lists is run. A `valid/*.toml` must parse to the
//! values its `.json` file gives in toml-test's tagged form (`{"type": "integer", "value":
//! "42"}`); an `invalid/*.toml` must be rejected. Values are compared by meaning: numbers by
//! value (`nan` and the infinities included), date-times after reading both as RFC 3339.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use mtindo::config::toml::{self, Table, Value};

/// A JSON value, as the expected-output files use them.
#[derive(Debug, PartialEq)]
enum Json {
    Object(Vec<(String, Json)>),
    Array(Vec<Json>),
    String(String),
    Other(String),
}

pub fn run(args: &[String]) -> ExitCode {
    let Some(dir) = args.first() else {
        eprintln!("toml-test: needs the suite's directory");
        return ExitCode::from(2);
    };
    let tests = Path::new(dir).join("tests");
    let list = match fs::read_to_string(tests.join("files-toml-1.0.0")) {
        Ok(list) => list,
        Err(e) => {
            println!("cannot read the suite's file list: {}", e);
            return ExitCode::from(1);
        }
    };
    let mut passed = 0;
    let mut failures = Vec::new();
    for case in list.lines().map(str::trim).filter(|l| l.ends_with(".toml")) {
        let path = tests.join(case);
        let Ok(bytes) = fs::read(&path) else {
            failures.push(format!("{}: cannot read", case));
            continue;
        };
        let result = match std::str::from_utf8(&bytes) {
            Ok(text) => check(case, text, &tests),
            // Invalid UTF-8 is an invalid document, which a caller sees as a read error.
            Err(_) if case.starts_with("invalid/") => Ok(()),
            Err(_) => Err("not valid UTF-8".to_string()),
        };
        match result {
            Ok(()) => passed += 1,
            Err(reason) => failures.push(format!("{}: {}", case, reason)),
        }
    }
    for failure in failures.iter() {
        println!("  {}", failure);
    }
    println!(
        "toml-test (TOML 1.0.0): {} passed, {} failed",
        passed,
        failures.len()
    );
    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn check(case: &str, text: &str, tests: &Path) -> Result<(), String> {
    let parsed = std::panic::catch_unwind(|| toml::parse(text))
        .map_err(|_| "the parser panicked".to_string())?;
    if case.starts_with("invalid/") {
        return match parsed {
            Ok(_) => Err("accepted an invalid document".to_string()),
            Err(_) => Ok(()),
        };
    }
    let table = parsed.map_err(|e| format!("rejected a valid document: {}", e))?;
    let expected_path = tests.join(case.replace(".toml", ".json"));
    let expected_text =
        fs::read_to_string(&expected_path).map_err(|e| format!("cannot read the .json: {}", e))?;
    let expected = parse_json(&expected_text).map_err(|e| format!("bad .json: {}", e))?;
    compare_table(&table, &expected, "")
}

fn compare(actual: &Value, expected: &Json, path: &str) -> Result<(), String> {
    match (actual, expected) {
        (Value::Table(t), Json::Object(fields)) if !is_tagged(fields) => {
            compare_table(t, expected, path)
        }
        (Value::Array(a), Json::Array(items)) => {
            if a.items.len() != items.len() {
                return Err(format!(
                    "{}: {} items, expected {}",
                    path,
                    a.items.len(),
                    items.len()
                ));
            }
            for (idx, (x, y)) in a.items.iter().zip(items).enumerate() {
                compare(x, y, &format!("{}[{}]", path, idx))?;
            }
            Ok(())
        }
        (_, Json::Object(fields)) if is_tagged(fields) => {
            let tag = field(fields, "type");
            let want = field(fields, "value");
            let ok = match (actual, tag.as_str()) {
                (Value::String(s), "string") => *s == want,
                (Value::Integer(i), "integer") => want.parse::<i64>().ok() == Some(*i),
                (Value::Float(f), "float") => same_float(*f, &want),
                (Value::Boolean(b), "bool") => want == b.to_string(),
                (
                    Value::Datetime(d),
                    "datetime" | "datetime-local" | "date-local" | "time-local",
                ) => {
                    let kind = match (d.date.is_some(), d.time.is_some(), d.offset.is_some()) {
                        (true, true, true) => "datetime",
                        (true, true, false) => "datetime-local",
                        (true, false, false) => "date-local",
                        _ => "time-local",
                    };
                    kind == tag && normalize_datetime(&d.to_string()) == normalize_datetime(&want)
                }
                _ => false,
            };
            if ok {
                Ok(())
            } else {
                Err(format!(
                    "{}: got {:?}, expected {} {:?}",
                    path, actual, tag, want
                ))
            }
        }
        _ => Err(format!(
            "{}: got {:?}, expected {:?}",
            path, actual, expected
        )),
    }
}

fn compare_table(table: &Table, expected: &Json, path: &str) -> Result<(), String> {
    let Json::Object(fields) = expected else {
        return Err(format!("{}: expected {:?}, got a table", path, expected));
    };
    if fields.len() != table.len() {
        return Err(format!(
            "{}: {} keys, expected {}",
            path,
            table.len(),
            fields.len()
        ));
    }
    for (key, value) in fields {
        let here = format!("{}.{}", path, key);
        let Some(actual) = table.get(key) else {
            return Err(format!("{}: missing", here));
        };
        compare(actual, value, &here)?;
    }
    Ok(())
}

fn field(fields: &[(String, Json)], name: &str) -> String {
    match fields.iter().find(|(k, _)| k == name) {
        Some((_, Json::String(s))) => s.clone(),
        _ => String::new(),
    }
}

/// `{"type": ..., "value": ...}`: a scalar, rather than a table that happens to have keys
/// named `type` and `value`.
fn is_tagged(fields: &[(String, Json)]) -> bool {
    fields.len() == 2
        && fields
            .iter()
            .all(|(k, v)| (k == "type" || k == "value") && matches!(v, Json::String(_)))
}

fn json_value(chars: &[char], pos: &mut usize) -> Result<Json, String> {
    skip(chars, pos);
    match chars.get(*pos) {
        Some('{') => {
            *pos += 1;
            let mut fields = Vec::new();
            loop {
                skip(chars, pos);
                if chars.get(*pos) == Some(&'}') {
                    *pos += 1;
                    return Ok(Json::Object(fields));
                }
                let Json::String(key) = json_value(chars, pos)? else {
                    return Err("object key is not a string".to_string());
                };
                skip(chars, pos);
                if chars.get(*pos) != Some(&':') {
                    return Err(format!("expected `:` at {}", pos));
                }
                *pos += 1;
                fields.push((key, json_value(chars, pos)?));
                skip(chars, pos);
                match chars.get(*pos) {
                    Some(',') => *pos += 1,
                    Some('}') => {}
                    _ => return Err(format!("expected `,` or `}}` at {}", pos)),
                }
            }
        }
        Some('[') => {
            *pos += 1;
            let mut items = Vec::new();
            loop {
                skip(chars, pos);
                if chars.get(*pos) == Some(&']') {
                    *pos += 1;
                    return Ok(Json::Array(items));
                }
                items.push(json_value(chars, pos)?);
                skip(chars, pos);
                match chars.get(*pos) {
                    Some(',') => *pos += 1,
                    Some(']') => {}
                    _ => return Err(format!("expected `,` or `]` at {}", pos)),
                }
            }
        }
        Some('"') => {
            *pos += 1;
            let mut out = String::new();
            loop {
                match chars.get(*pos) {
                    None => return Err("unterminated string".to_string()),
                    Some('"') => {
                        *pos += 1;
                        return Ok(Json::String(out));
                    }
                    Some('\\') => {
                        *pos += 1;
                        let c = match chars.get(*pos) {
                            Some('n') => '\n',
                            Some('t') => '\t',
                            Some('r') => '\r',
                            Some('b') => '\u{8}',
                            Some('f') => '\u{c}',
                            Some('u') => {
                                let hex: String = chars
                                    .get(*pos + 1..*pos + 5)
                                    .unwrap_or(&[])
                                    .iter()
                                    .collect();
                                let mut code =
                                    u32::from_str_radix(&hex, 16).map_err(|_| "bad \\u")?;
                                *pos += 4;
                                // A surrogate pair.
                                if (0xD800..0xDC00).contains(&code)
                                    && chars.get(*pos + 1) == Some(&'\\')
                                {
                                    let low: String = chars
                                        .get(*pos + 3..*pos + 7)
                                        .unwrap_or(&[])
                                        .iter()
                                        .collect();
                                    let low =
                                        u32::from_str_radix(&low, 16).map_err(|_| "bad \\u")?;
                                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                    *pos += 6;
                                }
                                char::from_u32(code).ok_or("bad code point")?
                            }
                            Some(other) => *other,
                            None => return Err("unterminated escape".to_string()),
                        };
                        out.push(c);
                        *pos += 1;
                    }
                    Some(c) => {
                        out.push(*c);
                        *pos += 1;
                    }
                }
            }
        }
        Some(_) => {
            let start = *pos;
            while *pos < chars.len()
                && !matches!(chars[*pos], ',' | '}' | ']')
                && !chars[*pos].is_whitespace()
            {
                *pos += 1;
            }
            Ok(Json::Other(chars[start..*pos].iter().collect()))
        }
        None => Err("unexpected end".to_string()),
    }
}

/// Date-times read the same however they were spelled: `T`/space and `z`/`Z`, a `+00:00`
/// offset written as `Z`, and fractional-second zeros at the end.
fn normalize_datetime(text: &str) -> String {
    let mut out: String = text
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i == 10 && (c == ' ' || c == 't') {
                'T'
            } else {
                c.to_ascii_uppercase()
            }
        })
        .collect();
    if out.ends_with("+00:00") || out.ends_with("-00:00") {
        out.truncate(out.len() - 6);
        out.push('Z');
    }
    if let Some(dot) = out.find('.') {
        let end = out[dot + 1..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| dot + 1 + i)
            .unwrap_or(out.len());
        let digits = out[dot + 1..end].trim_end_matches('0').to_string();
        let tail = out[end..].to_string();
        out.truncate(dot);
        if !digits.is_empty() {
            out.push('.');
            out.push_str(&digits);
        }
        out.push_str(&tail);
    }
    out
}

/// A small JSON reader: objects, arrays, strings with escapes, and other literals kept as
/// text.
fn parse_json(text: &str) -> Result<Json, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut pos = 0;
    let value = json_value(&chars, &mut pos)?;
    skip(&chars, &mut pos);
    if pos != chars.len() {
        return Err(format!("trailing text at {}", pos));
    }
    Ok(value)
}

fn same_float(value: f64, want: &str) -> bool {
    match want {
        "nan" | "+nan" | "-nan" => value.is_nan(),
        "inf" | "+inf" => value == f64::INFINITY,
        "-inf" => value == f64::NEG_INFINITY,
        other => other.parse::<f64>().is_ok_and(|w| w == value),
    }
}

fn skip(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos].is_whitespace() {
        *pos += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_reader() {
        let json =
            parse_json(r#"{"a": {"type": "string", "value": "xé\n"}, "b": [1, true]}"#).unwrap();
        let Json::Object(fields) = json else {
            panic!("not an object")
        };
        assert_eq!(fields[0].0, "a");
        assert_eq!(
            fields[1].1,
            Json::Array(vec![Json::Other("1".into()), Json::Other("true".into())])
        );
    }

    #[test]
    fn test_datetime_normalization() {
        assert_eq!(
            normalize_datetime("1979-05-27 07:32:00z"),
            "1979-05-27T07:32:00Z"
        );
        assert_eq!(
            normalize_datetime("1979-05-27T07:32:00.500+00:00"),
            "1979-05-27T07:32:00.5Z"
        );
        assert_eq!(normalize_datetime("07:32:00.000"), "07:32:00");
    }
}
