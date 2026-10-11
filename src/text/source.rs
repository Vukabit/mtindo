//! A source file's byte-order mark and line endings: detected on the way in, restored on the way out.

const BOM: char = '\u{feff}';

/// The line ending a file uses throughout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
  Lf,
  CrLf,
}

/// Source text with its byte-order mark removed and line endings normalized to `\n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalized {
  pub bom: bool,
  pub line_ending: LineEnding,
  pub text: String,
}

impl Normalized {
  /// Puts back the original byte-order mark and line endings on text derived from `self.text`.
  pub fn restore(&self, text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 1);
    if self.bom {
      out.push(BOM);
    }
    match self.line_ending {
      LineEnding::Lf => out.push_str(text),
      LineEnding::CrLf => out.push_str(&text.replace('\n', "\r\n")),
    }
    out
  }
}

/// Splits off a byte-order mark and normalizes `\r\n` to `\n`.
///
/// Fails on mixed line endings, because restoring a single style would change the lines
/// that used the other one, including inside string literals.
pub fn normalize(source: &str) -> Result<Normalized, String> {
  let (bom, text) = match source.strip_prefix(BOM) {
    Some(rest) => (true, rest),
    None => (false, source),
  };
  let crlf_count = text.matches("\r\n").count();
  let lf_count = text.matches('\n').count();
  if crlf_count > 0 && crlf_count != lf_count {
    return Err("mixed line endings".to_string());
  }
  if crlf_count > 0 {
    Ok(Normalized {
      bom,
      line_ending: LineEnding::CrLf,
      text: text.replace("\r\n", "\n"),
    })
  } else {
    Ok(Normalized {
      bom,
      line_ending: LineEnding::Lf,
      text: text.to_string(),
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_crlf_and_bom_round_trip() {
    let source = "\u{feff}fn a() {}\r\n\r\nfn b() {}\r\n";
    let normalized = normalize(source).unwrap();
    assert_eq!(normalized.line_ending, LineEnding::CrLf);
    assert!(normalized.bom);
    assert_eq!(normalized.text, "fn a() {}\n\nfn b() {}\n");
    assert_eq!(normalized.restore(&normalized.text), source);
  }

  #[test]
  fn test_mixed_endings_are_refused() {
    assert!(normalize("fn a() {}\r\nfn b() {}\n").is_err());
  }

  #[test]
  fn test_no_newline_is_lf() {
    assert_eq!(normalize("fn a() {}").unwrap().line_ending, LineEnding::Lf);
    assert_eq!(normalize("").unwrap().text, "");
  }

  #[test]
  fn test_plain_lf_round_trips() {
    let source = "fn a() {}\n";
    let normalized = normalize(source).unwrap();
    assert_eq!(normalized.line_ending, LineEnding::Lf);
    assert!(!normalized.bom);
    assert_eq!(normalized.restore(&normalized.text), source);
  }
}
