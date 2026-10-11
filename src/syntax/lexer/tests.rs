use proptest::prelude::*;

use super::*;

const ALL_EDITIONS: [Edition; 4] = [
  Edition::E2015,
  Edition::E2018,
  Edition::E2021,
  Edition::E2024,
];

/// Every token as (kind name, text, error).
fn lex(text: &str, edition: Edition) -> Vec<(&'static str, String, Option<LexError>)> {
  let mut offset = 0;
  let mut out = Vec::new();
  for token in tokenize(text, edition) {
    let end = offset + token.len as usize;
    out.push((
      token.kind.name(),
      text[offset..end].to_string(),
      token.error,
    ));
    offset = end;
  }
  out
}

fn one(text: &str, edition: Edition) -> (&'static str, Option<LexError>) {
  let tokens = lex(text, edition);
  assert_eq!(tokens.len(), 1, "{:?} lexed as {:?}", text, tokens);
  (tokens[0].0, tokens[0].2)
}

fn pairs(list: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
  list
    .iter()
    .map(|(kind, text)| (*kind, text.to_string()))
    .collect()
}

/// Tokens other than whitespace, as (kind name, text).
fn significant(text: &str, edition: Edition) -> Vec<(&'static str, String)> {
  lex(text, edition)
    .into_iter()
    .filter(|(kind, _, _)| *kind != "WHITESPACE")
    .map(|(kind, text, _)| (kind, text))
    .collect()
}

#[test]
fn test_c_strings_need_2021() {
  assert_eq!(
    significant(r#"c"x""#, Edition::E2018),
    pairs(&[("IDENT", "c"), ("STRING", "\"x\"")])
  );
  assert_eq!(
    significant(r#"c"x""#, Edition::E2021),
    pairs(&[("C_STRING", "c\"x\"")])
  );
}

#[test]
fn test_chars_and_bytes() {
  let e = Edition::E2021;
  for text in ["'a'", "'\\n'", "'\\''", "'\\u{1F600}'", "'中'", "'\"'"] {
    assert_eq!(one(text, e), ("CHAR", None), "{}", text);
  }
  assert_eq!(one("b'x'", e), ("BYTE", None));
  assert_eq!(one("b'\\x7f'", e), ("BYTE", None));
  assert_eq!(one("''", e), ("CHAR", Some(LexError::EmptyChar)));
  assert_eq!(
    lex("'\\n", e)[0],
    ("CHAR", "'\\n".to_string(), Some(LexError::UnterminatedChar))
  );
}

#[test]
fn test_comments() {
  let e = Edition::E2021;
  assert_eq!(one("// plain", e), ("LINE_COMMENT", None));
  assert_eq!(one("/// outer doc", e), ("OUTER_LINE_DOC", None));
  assert_eq!(one("//! inner doc", e), ("INNER_LINE_DOC", None));
  assert_eq!(one("//// not a doc", e), ("LINE_COMMENT", None));
  assert_eq!(one("/* plain */", e), ("BLOCK_COMMENT", None));
  assert_eq!(one("/** outer */", e), ("OUTER_BLOCK_DOC", None));
  assert_eq!(one("/*! inner */", e), ("INNER_BLOCK_DOC", None));
  assert_eq!(one("/*** not doc */", e), ("BLOCK_COMMENT", None));
  assert_eq!(one("/**/", e), ("BLOCK_COMMENT", None));
  assert_eq!(
    one("/* outer /* nested */ still */", e),
    ("BLOCK_COMMENT", None)
  );
  assert_eq!(
    one("/* open /* nested */", e),
    ("BLOCK_COMMENT", Some(LexError::UnterminatedBlockComment))
  );
  assert_eq!(
    significant("// to end\nx", e),
    pairs(&[("LINE_COMMENT", "// to end"), ("IDENT", "x")])
  );
}

#[test]
fn test_file_start() {
  let e = Edition::E2021;
  assert_eq!(
    significant("\u{feff}fn", e),
    pairs(&[("BOM", "\u{feff}"), ("IDENT", "fn")])
  );
  assert_eq!(
    significant("#!/usr/bin/env run\nfn", e),
    pairs(&[("SHEBANG", "#!/usr/bin/env run"), ("IDENT", "fn")])
  );
  assert_eq!(
    significant("#![allow(x)]", e)[0],
    ("POUND", "#".to_string())
  );
  assert_eq!(
    significant("#! /* c */ [attr]", e)[0],
    ("POUND", "#".to_string())
  );
  assert_eq!(
    significant("\u{feff}#!/bin/x\n", e),
    pairs(&[("BOM", "\u{feff}"), ("SHEBANG", "#!/bin/x")])
  );
}

#[test]
fn test_frontmatter() {
  let e = Edition::E2024;
  let text = "---cargo\n[dependencies]\n---\nfn main() {}\n";
  assert_eq!(
    significant(text, e),
    pairs(&[
      ("FRONTMATTER", "---cargo\n[dependencies]\n---"),
      ("IDENT", "fn"),
      ("IDENT", "main"),
      ("L_PAREN", "("),
      ("R_PAREN", ")"),
      ("L_BRACE", "{"),
      ("R_BRACE", "}"),
    ])
  );
  let after_shebang = "#!/usr/bin/env cargo\n\n----\nx\n----\n";
  assert_eq!(
    significant(after_shebang, e)[1],
    ("FRONTMATTER", "----\nx\n----".to_string())
  );
  assert_eq!(
    lex("  ---\n---\n", e)[0].2,
    Some(LexError::IndentedFrontmatter)
  );
  assert_eq!(
    lex("---\nnever closed\n", e)[0].2,
    Some(LexError::UnterminatedFrontmatter)
  );
  // Only before the first token: later dashes are minus signs.
  assert_eq!(significant("x\n---\n", e)[1].0, "MINUS");
}

#[test]
fn test_guarded_strings_reserved_in_2024() {
  assert_eq!(lex("#\"x\"#", Edition::E2021)[0].2, None);
  assert_eq!(
    lex("#\"x\"#", Edition::E2024)[0],
    (
      "POUND",
      "#".to_string(),
      Some(LexError::ReservedGuardedString)
    )
  );
  assert_eq!(
    lex("##", Edition::E2024)[0].2,
    Some(LexError::ReservedGuardedString)
  );
  assert_eq!(lex("#[attr]", Edition::E2024)[0].2, None);
  assert_eq!(lex("#![attr]", Edition::E2024)[0].2, None);
}

#[test]
fn test_identifiers() {
  let e = Edition::E2021;
  assert_eq!(one("snake_case", e), ("IDENT", None));
  assert_eq!(one("_", e), ("IDENT", None));
  assert_eq!(one("_private", e), ("IDENT", None));
  assert_eq!(one("café", e), ("IDENT", None));
  assert_eq!(one("中文", e), ("IDENT", None));
  assert_eq!(
    significant("a🦀b", e),
    pairs(&[("IDENT", "a"), ("UNKNOWN", "🦀"), ("IDENT", "b")])
  );
}

#[test]
fn test_lifetimes_and_labels() {
  let e = Edition::E2021;
  assert_eq!(one("'a", e), ("LIFETIME", None));
  assert_eq!(one("'static", e), ("LIFETIME", None));
  assert_eq!(
    significant("'outer: loop", e),
    pairs(&[("LIFETIME", "'outer"), ("COLON", ":"), ("IDENT", "loop")])
  );
  assert_eq!(
    significant("impl<'a, 'b>", e),
    pairs(&[
      ("IDENT", "impl"),
      ("L_ANGLE", "<"),
      ("LIFETIME", "'a"),
      ("COMMA", ","),
      ("LIFETIME", "'b"),
      ("R_ANGLE", ">"),
    ])
  );
  assert_eq!(
    one("'1", e),
    ("LIFETIME", Some(LexError::LifetimeStartsWithNumber))
  );
}

#[test]
fn test_numbers() {
  let e = Edition::E2021;
  for text in [
    "0",
    "1_000",
    "1u8",
    "0x_FF",
    "0xdead_beef",
    "0o7",
    "0b1010_u8",
    "123usize",
  ] {
    assert_eq!(one(text, e), ("INT_NUMBER", None), "{}", text);
  }
  for text in [
    "1.",
    "1.5",
    "1e10",
    "1e-3",
    "2.5E+7f64",
    "1_000.0",
    "1f32_is_int_with_suffix",
  ] {
    let expected = if text.starts_with("1f32") {
      "INT_NUMBER"
    } else {
      "FLOAT_NUMBER"
    };
    assert_eq!(one(text, e).0, expected, "{}", text);
  }
  assert_eq!(
    significant("1..2", e),
    pairs(&[
      ("INT_NUMBER", "1"),
      ("DOT", "."),
      ("DOT", "."),
      ("INT_NUMBER", "2")
    ])
  );
  assert_eq!(
    significant("1.foo()", e),
    pairs(&[
      ("INT_NUMBER", "1"),
      ("DOT", "."),
      ("IDENT", "foo"),
      ("L_PAREN", "("),
      ("R_PAREN", ")"),
    ])
  );
  assert_eq!(
    significant("x.0.1", e),
    pairs(&[("IDENT", "x"), ("DOT", "."), ("FLOAT_NUMBER", "0.1")])
  );
  assert_eq!(one("0x", e), ("INT_NUMBER", Some(LexError::EmptyIntDigits)));
  assert_eq!(
    one("1e", e),
    ("FLOAT_NUMBER", Some(LexError::EmptyExponent))
  );
}

#[test]
fn test_punctuation_is_single_characters() {
  let e = Edition::E2021;
  assert_eq!(
    significant("::->=>", e),
    pairs(&[
      ("COLON", ":"),
      ("COLON", ":"),
      ("MINUS", "-"),
      ("R_ANGLE", ">"),
      ("EQ", "="),
      ("R_ANGLE", ">"),
    ])
  );
  assert_eq!(one("\\", e), ("UNKNOWN", Some(LexError::UnknownChar)));
}

#[test]
fn test_raw_lifetimes_from_2021() {
  assert_eq!(one("'r#async", Edition::E2021), ("LIFETIME", None));
  assert_eq!(
    significant("'r#async", Edition::E2018),
    pairs(&[("LIFETIME", "'r"), ("POUND", "#"), ("IDENT", "async")])
  );
}

#[test]
fn test_raw_strings() {
  let e = Edition::E2021;
  assert_eq!(one(r#"r"raw \ ""#, e), ("RAW_STRING", None));
  assert_eq!(one(r###"r##"has "# inside"##"###, e), ("RAW_STRING", None));
  assert_eq!(one(r##"br#"bytes"#"##, e), ("RAW_BYTE_STRING", None));
  assert_eq!(one(r##"cr#"c"#"##, e), ("RAW_C_STRING", None));
  assert_eq!(
    one(r##"r#"open"##, e),
    ("RAW_STRING", Some(LexError::UnterminatedRawString))
  );
  assert_eq!(significant("r##x", e), pairs(&[("RAW_STRING", "r##x")]));
  assert_eq!(
    lex("r##x", e)[0].2,
    Some(LexError::InvalidRawStringDelimiter)
  );
  let many = format!("r{}\"x\"{}", "#".repeat(256), "#".repeat(256));
  assert_eq!(
    one(&many, e),
    ("RAW_STRING", Some(LexError::TooManyRawStringHashes))
  );
}

#[test]
fn test_reserved_prefixes_from_2021() {
  assert_eq!(lex("k#x", Edition::E2018)[0].2, None);
  assert_eq!(
    lex("k#x", Edition::E2021)[0],
    ("IDENT", "k".to_string(), Some(LexError::ReservedPrefix))
  );
  assert_eq!(
    lex("f\"x\"", Edition::E2021)[0].2,
    Some(LexError::ReservedPrefix)
  );
  assert_eq!(
    lex("b#x", Edition::E2021)[0].2,
    Some(LexError::ReservedPrefix)
  );
  assert_eq!(one("r#match", Edition::E2021), ("RAW_IDENT", None));
}

#[test]
fn test_split_tuple_field_floats() {
  let split = |text: &str| {
    let tokens = split_tuple_field_floats(text, tokenize(text, Edition::E2021));
    let mut offset = 0;
    let mut out = Vec::new();
    for t in tokens {
      let end = offset + t.len as usize;
      if t.kind != SyntaxKind::Whitespace {
        out.push((t.kind.name(), text[offset..end].to_string()));
      }
      offset = end;
    }
    assert_eq!(offset, text.len());
    out
  };
  assert_eq!(
    split("x.0.1"),
    pairs(&[
      ("IDENT", "x"),
      ("DOT", "."),
      ("INT_NUMBER", "0"),
      ("DOT", "."),
      ("INT_NUMBER", "1")
    ])
  );
  assert_eq!(
    split("x.0. await"),
    pairs(&[
      ("IDENT", "x"),
      ("DOT", "."),
      ("INT_NUMBER", "0"),
      ("DOT", "."),
      ("IDENT", "await")
    ])
  );
  // Not after a single dot: ranges and plain floats stay whole.
  assert_eq!(
    split("0..1.5"),
    pairs(&[
      ("INT_NUMBER", "0"),
      ("DOT", "."),
      ("DOT", "."),
      ("FLOAT_NUMBER", "1.5")
    ])
  );
  assert_eq!(split("1.5"), pairs(&[("FLOAT_NUMBER", "1.5")]));
  assert_eq!(split("x.1e3")[2].0, "FLOAT_NUMBER");
}

#[test]
fn test_strings() {
  let e = Edition::E2021;
  assert_eq!(one(r#""plain""#, e), ("STRING", None));
  assert_eq!(one(r#""esc \" \\ \n""#, e), ("STRING", None));
  assert_eq!(one("\"spans\nlines\"", e), ("STRING", None));
  assert_eq!(one(r#""suffix"sfx"#, e), ("STRING", None));
  assert_eq!(one(r#"b"bytes""#, e), ("BYTE_STRING", None));
  assert_eq!(one(r#"c"cstr""#, e), ("C_STRING", None));
  assert_eq!(
    one("\"never closed", e),
    ("STRING", Some(LexError::UnterminatedString))
  );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn prop_tokens_cover_any_input_exactly(text in any::<String>()) {
        for edition in ALL_EDITIONS {
            let tokens = tokenize(&text, edition);
            let total: usize = tokens.iter().map(|t| t.len as usize).sum();
            prop_assert_eq!(total, text.len());
            prop_assert!(tokens.iter().all(|t| t.len > 0));
            let mut offset = 0;
            for token in tokens.iter() {
                offset += token.len as usize;
                prop_assert!(text.is_char_boundary(offset));
            }
        }
    }

    #[test]
    fn prop_tokens_cover_rust_like_input_exactly(
        text in "[ a-z0-9_'\"#!/*\\\\.\\-<>{}()\\[\\];:,\n\rrbc]{0,80}"
    ) {
        for edition in ALL_EDITIONS {
            let tokens = tokenize(&text, edition);
            let total: usize = tokens.iter().map(|t| t.len as usize).sum();
            prop_assert_eq!(total, text.len());
            prop_assert!(tokens.iter().all(|t| t.len > 0));
        }
    }
}

#[test]
fn test_unterminated_string_mid_file_keeps_going() {
  let tokens = lex("fn a() { \"open\n}\nfn b() {}", Edition::E2021);
  let last = tokens.last().unwrap();
  assert_eq!(last.0, "STRING");
  assert_eq!(last.2, Some(LexError::UnterminatedString));
}

#[test]
fn test_whitespace_set() {
  for c in [
    '\t', '\n', '\u{000B}', '\u{000C}', '\r', ' ', '\u{0085}', '\u{200E}', '\u{200F}', '\u{2028}',
    '\u{2029}',
  ] {
    assert!(is_whitespace(c), "U+{:04X}", c as u32);
  }
  assert!(!is_whitespace('\u{00A0}'));
  assert_eq!(one(" \t\r\n\u{2028}", Edition::E2021), ("WHITESPACE", None));
}
