//! Strings in the TOML files xtask writes by hand (incidents, run records): quoted as a
//! basic string, and read back with mtindo's own TOML parser.

/// `s` as a TOML basic string.
pub fn quote(s: &str) -> String {
  let mut out = String::from("\"");
  for c in s.chars() {
    match c {
      '"' => out.push_str("\\\""),
      '\\' => out.push_str("\\\\"),
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{:04X}", c as u32)),
      c => out.push(c),
    }
  }
  out.push('"');
  out
}

/// The content of a quoted TOML string, `value` as it follows a key's `=`.
pub fn unquote(value: &str) -> Option<String> {
  let table = crate::toml::parse(&format!("v = {}\n", value.trim())).ok()?;
  table.get("v")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_strings_read_back_as_written() {
    for s in [
      "",
      "plain",
      "a \"b\" \\c",
      "two\nlines\r\tand \u{1} \u{7f} é",
    ] {
      assert_eq!(unquote(&quote(s)).as_deref(), Some(s), "{}", quote(s));
    }
    assert_eq!(unquote("x"), None);
  }
}
