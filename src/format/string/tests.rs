use super::*;

fn literal(text: &str, width: usize, indent: &str) -> Option<String> {
  let fmt = StringFormat {
    opener: "\"",
    closer: "\"",
    line_start: " ",
    line_end: "\\",
    indent,
    width,
    offset: 0,
    trim_end: false,
    max_width: 100,
    comment: false,
  };
  rewrite_string(text, &fmt, width.saturating_sub(2))
}

/// A string literal breaks at the last space before the width, keeping the space before
/// its `\`; one that fits, and one written with continuations that fits once joined, is
/// written on one line.
#[test]
fn test_string_literals_break_between_words() {
  assert_eq!(
    literal(
      "ipsum dolor sit amet consectetur adipiscing elit",
      30,
      "    "
    )
    .as_deref(),
    Some("\"ipsum dolor sit amet \\\n     consectetur adipiscing elit\"")
  );
  assert_eq!(literal("short", 30, "").as_deref(), Some("\"short\""));
  assert_eq!(
    literal("H\\\n    e\\\n    llo", 30, "").as_deref(),
    Some("\"Hello\"")
  );
}

/// A URL is not broken inside, and a line break of the string's own stays.
#[test]
fn test_string_literals_keep_urls_and_line_breaks() {
  let url = "see https://example.com/a/very/long/path/that/goes/on for more";
  let broken = literal(url, 60, "").unwrap_or_default();
  assert!(
    broken.contains("https://example.com/a/very/long/path/that/goes/on"),
    "{}",
    broken
  );
  // One that cannot fit at all is given up on.
  assert_eq!(literal(url, 30, ""), None);
  assert_eq!(literal("one\ntwo", 30, "").as_deref(), Some("\"one\ntwo\""));
}
