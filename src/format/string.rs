//! Text broken over lines to fit a width, as the standard formatter's `rewrite_string`
//! breaks it: a string literal for `format_strings`, with `\` ending each line, and a
//! comment's line for `wrap_comments`. A line breaks at the last space before the width,
//! else at punctuation, else at the first place past it; never inside a URL.

use crate::text::unicode::{char_width, is_other_punctuation, str_width};

/// The fewest characters a broken line keeps before its break.
const MIN_STRING: usize = 10;

enum Snippet {
  /// The input could not be broken: the rest as it is.
  EndOfInput(String),
  /// A line to end with the line end, and how much of the input it took.
  LineEnd(String, usize),
  /// A line ending in a line break of the input's own, and how much it took.
  EndWithLineFeed(String, usize),
}

/// How the text is written (`StringFormat`).
pub struct StringFormat<'a> {
  /// What opens and closes it (`"`, or nothing for a comment's text).
  pub opener: &'a str,
  pub closer: &'a str,
  /// What starts each line after the first, after `indent`, and ends each line broken.
  pub line_start: &'a str,
  pub line_end: &'a str,
  /// The indentation of the lines after the first.
  pub indent: &'a str,
  /// The width the first line may take, and where it starts past `indent`.
  pub width: usize,
  pub offset: usize,
  /// Whether whitespace ending a line is dropped (a comment's) or kept (a string's).
  pub trim_end: bool,
  pub max_width: usize,
  /// Whether the text is a comment's: then only what is not a comment counts toward the
  /// widths checked last (`filter_normal_code`), so a later line's marker and the rest
  /// after it do not.
  pub comment: bool,
}

impl StringFormat<'_> {
  fn max_width_with_indent(&self) -> Option<usize> {
    Some(
      self
        .width
        .checked_sub(self.opener.len() + self.line_end.len() + 1)?
        + 1,
    )
  }
}

/// `orig` broken to fit, or `None` when what it makes does not fit (`wrap_str`). Lines
/// after the first may take `newline_max_chars`.
pub fn rewrite_string(orig: &str, fmt: &StringFormat, newline_max_chars: usize) -> Option<String> {
  let max_width_with_indent = fmt.max_width_with_indent()?;
  let max_width_without_indent = fmt.max_width.checked_sub(fmt.line_end.len())?;
  let stripped = strip_line_breaks(orig);
  let graphemes = graphemes(&stripped);
  let mut cur_start = 0;
  let mut result = String::with_capacity(stripped.len() + 16);
  result.push_str(fmt.opener);
  let mut cur_max_width = max_width_with_indent;
  let bare_line_ok = fmt.line_start.is_empty() || is_whitespace(fmt.line_start);
  loop {
    if graphemes_width(&graphemes[cur_start..]) <= cur_max_width {
      for (i, grapheme) in graphemes[cur_start..].iter().enumerate() {
        if is_new_line(grapheme) {
          // A blank line, ended as written (`\r\n` kept).
          trim_end_but_line_feed(fmt.trim_end, &mut result);
          result.push_str(grapheme);
          if !bare_line_ok && cur_start + i + 1 < graphemes.len() {
            result.push_str(fmt.indent);
            result.push_str(fmt.line_start);
          }
        } else {
          result.push_str(grapheme);
        }
      }
      trim_end_but_line_feed(fmt.trim_end, &mut result);
      break;
    }
    match break_string(
      cur_max_width,
      fmt.trim_end,
      fmt.line_end,
      &graphemes[cur_start..],
    ) {
      Snippet::LineEnd(line, len) => {
        result.push_str(&line);
        result.push_str(fmt.line_end);
        result.push('\n');
        result.push_str(fmt.indent);
        result.push_str(fmt.line_start);
        cur_max_width = newline_max_chars;
        cur_start += len;
      }
      Snippet::EndWithLineFeed(line, len) => {
        if line == "\n" && fmt.trim_end {
          let kept = result.trim_end().len();
          result.truncate(kept);
        }
        result.push_str(&line);
        if bare_line_ok {
          // The next line may take the whole width.
          cur_max_width = max_width_without_indent;
        } else {
          result.push_str(fmt.indent);
          result.push_str(fmt.line_start);
          cur_max_width = max_width_with_indent;
        }
        cur_start += len;
      }
      Snippet::EndOfInput(line) => {
        result.push_str(&line);
        break;
      }
    }
  }
  result.push_str(fmt.closer);
  fits(&result, fmt).then_some(result)
}

/// Breaks `input` at a space or punctuation around `max_width`.
fn break_string(max_width: usize, trim_end: bool, line_end: &str, input: &[&str]) -> Snippet {
  let break_at = |index: usize| {
    let index_minus_ws = input[..=index]
      .iter()
      .rposition(|g| !is_whitespace_but_line_feed(g))
      .unwrap_or(index);
    // A line break of the input's own up to here ends the line there.
    for (i, grapheme) in input[..=index].iter().enumerate() {
      if is_new_line(grapheme) {
        if i <= index_minus_ws {
          let line = input[..i].concat();
          let line = if trim_end { line.trim_end() } else { &line };
          return Snippet::EndWithLineFeed(format!("{}{}", line, grapheme), i + 1);
        }
        break;
      }
    }
    let mut index_plus_ws = index;
    for (i, grapheme) in input[index + 1..].iter().enumerate() {
      if !trim_end && is_new_line(grapheme) {
        return Snippet::EndWithLineFeed(input[..=index + 1 + i].concat(), index + 2 + i);
      } else if !is_whitespace_but_line_feed(grapheme) {
        index_plus_ws = index + i;
        break;
      }
    }
    if trim_end {
      Snippet::LineEnd(input[..=index_minus_ws].concat(), index_plus_ws + 1)
    } else {
      Snippet::LineEnd(input[..=index_plus_ws].concat(), index_plus_ws + 1)
    }
  };
  // The first index where the width so far passes `max_width`.
  let max_width_index = {
    let (mut width, mut at) = (0, 0);
    for (i, grapheme) in input.iter().enumerate() {
      width += str_width(grapheme);
      at = i;
      if width > max_width {
        break;
      }
    }
    at
  };
  if max_width_index == 0 {
    return Snippet::EndOfInput(input.concat());
  }
  // At a break already: no room needed for a line end, and spaces after it are trimmed.
  if line_end.is_empty()
    && trim_end
    && !is_whitespace(input[max_width_index - 1])
    && is_whitespace(input[max_width_index])
  {
    return break_at(max_width_index - 1);
  }
  if let Some(url_end) = detect_url(input, max_width_index) {
    let index_plus_ws = url_end
      + input[url_end..]
        .iter()
        .skip(1)
        .position(|g| !is_whitespace_but_line_feed(g))
        .unwrap_or(0);
    return if trim_end {
      Snippet::LineEnd(input[..=url_end].concat(), index_plus_ws + 1)
    } else {
      Snippet::LineEnd(input[..=index_plus_ws].concat(), index_plus_ws + 1)
    };
  }
  match input[..max_width_index]
    .iter()
    .rposition(|g| is_whitespace(g))
  {
    Some(index) if index >= MIN_STRING => break_at(index),
    _ => match (0..max_width_index)
      .rev()
      .find(|&pos| is_valid_linebreak(input, pos))
    {
      Some(index) if index >= MIN_STRING => break_at(index),
      _ => match (max_width_index..input.len()).find(|&pos| is_valid_linebreak(input, pos)) {
        Some(index) => break_at(index),
        None => Snippet::EndOfInput(input.concat()),
      },
    },
  }
}

/// Where a URL ends when the break at `index` would fall inside one.
fn detect_url(s: &[&str], index: usize) -> Option<usize> {
  let start = s[..=index]
    .iter()
    .rposition(|g| is_whitespace(g))
    .map_or(0, |pos| pos + 1);
  // 8 = the shortest a URL can be.
  if s.len() < start + 8 {
    return None;
  }
  let split = s[start..].concat();
  if ["https://", "http://", "ftp://", "file://"]
    .iter()
    .any(|scheme| split.contains(scheme))
  {
    match s[index..].iter().position(|g| is_whitespace(g)) {
      Some(pos) => Some(index + pos - 1),
      None => Some(s.len() - 1),
    }
  } else {
    None
  }
}

/// Whether what was made fits (`wrap_str`): its first line within the width, the rest
/// within the line, the last no further than the first may go.
fn fits(text: &str, fmt: &StringFormat) -> bool {
  let measured = |line: &str| match line.find("//").filter(|_| fmt.comment) {
    Some(at) => str_width(&line[..at]),
    None => str_width(line),
  };
  let mut lines = text.lines();
  let Some(first) = lines.next() else {
    return true;
  };
  if measured(first) > fmt.width {
    return false;
  }
  if !text.contains('\n') {
    return true;
  }
  let mut last = 0;
  for line in text.lines().skip(1) {
    last = measured(line);
    if last > fmt.max_width {
      return false;
    }
  }
  last <= str_width(fmt.indent) + fmt.offset + fmt.width
}

/// The text's characters, each with the zero-width ones after it, a `\r\n` as one.
fn graphemes(text: &str) -> Vec<&str> {
  let mut out: Vec<&str> = Vec::with_capacity(text.len());
  let mut start: Option<usize> = None;
  let mut prev = '\0';
  for (at, c) in text.char_indices() {
    let joins = (char_width(c) == 0 && !c.is_control()) || (prev == '\r' && c == '\n');
    prev = c;
    if start.is_some() && joins {
      continue;
    }
    if let Some(from) = start {
      out.push(&text[from..at]);
    }
    start = Some(at);
  }
  if let Some(from) = start {
    out.push(&text[from..]);
  }
  out
}

fn graphemes_width(graphemes: &[&str]) -> usize {
  graphemes.iter().map(|g| str_width(g)).sum()
}

fn is_new_line(grapheme: &str) -> bool {
  grapheme.starts_with('\n') || grapheme.starts_with("\r\n")
}

fn is_valid_linebreak(input: &[&str], pos: usize) -> bool {
  if is_whitespace(input[pos]) {
    return true;
  }
  let punctuation = input[pos].chars().all(is_other_punctuation);
  // Not inside a path (`a::b`).
  let path = input.get(pos..=pos + 1) == Some(&[":", ":"])
    || input.get(pos.saturating_sub(1)..=pos) == Some(&[":", ":"]);
  punctuation && !path
}

/// The whitespace of the language (a space, tab, line ending, vertical tab or form feed).
fn is_whitespace(grapheme: &str) -> bool {
  grapheme
    .chars()
    .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0B' | '\x0C'))
}

fn is_whitespace_but_line_feed(grapheme: &str) -> bool {
  !is_new_line(grapheme) && is_whitespace(grapheme)
}

/// The text without its line continuations: a `\` ending a line (not one escaped), the
/// line break and the whitespace after it.
fn strip_line_breaks(text: &str) -> String {
  let bytes = text.as_bytes();
  let mut out = String::with_capacity(text.len());
  let mut i = 0;
  while i < text.len() {
    if bytes[i] == b'\\' && matches!(bytes.get(i + 1), Some(b'\n' | b'\r')) {
      // The backslashes ending here, this one among them.
      let run = text[..=i].bytes().rev().take_while(|&b| b == b'\\').count();
      if run % 2 == 1 && i + 1 > run {
        i += 1;
        while let Some(c) = text[i..].chars().next().filter(|c| c.is_whitespace()) {
          i += c.len_utf8();
        }
        continue;
      }
    }
    let c = text[i..].chars().next().unwrap_or(' ');
    out.push(c);
    i += c.len_utf8();
  }
  out
}

fn trim_end_but_line_feed(trim_end: bool, text: &mut String) {
  if trim_end {
    let kept = text
      .trim_end_matches(|c: char| c.is_whitespace() && c != '\n')
      .len();
    text.truncate(kept);
  }
}

#[cfg(test)]
mod tests;
