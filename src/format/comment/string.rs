//! A comment's line broken between words to fit a width, as the standard formatter's
//! `rewrite_string` breaks it for `wrap_comments`: at the last space before the width, or
//! else at punctuation, or else at the first place past it; never inside a URL.

use crate::text::unicode::{char_width, is_other_punctuation, str_width};

/// The fewest characters a broken line keeps before its break.
const MIN_STRING: usize = 10;

enum Snippet {
  /// The input could not be broken: the rest as it is.
  EndOfInput(String),
  /// A line to end, and how much of the input it took (with the whitespace after it).
  LineEnd(String, usize),
}

/// How the line is written: each line after the first starts on a new line at `indent`
/// with `line_start`; the first may take `width` columns, the later ones `newline_width`.
pub struct Layout<'a> {
  pub line_start: &'a str,
  pub indent: &'a str,
  pub width: usize,
  /// Where the first line starts past `indent` (`shape.offset`).
  pub offset: usize,
  pub max_width: usize,
}

/// `orig` broken to fit, or `None` when what it makes does not fit (`wrap_str`).
pub fn rewrite_string(orig: &str, layout: &Layout, newline_width: usize) -> Option<String> {
  let graphemes = graphemes(orig);
  let mut cur_start = 0;
  let mut result = String::with_capacity(orig.len() + 16);
  let mut cur_max_width = layout.width;
  loop {
    if graphemes_width(&graphemes[cur_start..]) <= cur_max_width {
      for grapheme in &graphemes[cur_start..] {
        result.push_str(grapheme);
      }
      trim_end_but_line_feed(&mut result);
      break;
    }
    match break_string(cur_max_width, &graphemes[cur_start..]) {
      Snippet::LineEnd(line, len) => {
        result.push_str(&line);
        result.push('\n');
        result.push_str(layout.indent);
        result.push_str(layout.line_start);
        cur_max_width = newline_width;
        cur_start += len;
      }
      Snippet::EndOfInput(line) => {
        result.push_str(&line);
        break;
      }
    }
  }
  fits(&result, layout).then_some(result)
}

/// Breaks `input` at a space or punctuation around `max_width`.
fn break_string(max_width: usize, input: &[&str]) -> Snippet {
  let break_at = |index: usize| {
    let index_minus_ws = input[..=index]
      .iter()
      .rposition(|g| !is_whitespace(g))
      .unwrap_or(index);
    let mut index_plus_ws = index;
    for (i, grapheme) in input[index + 1..].iter().enumerate() {
      if !is_whitespace(grapheme) {
        index_plus_ws = index + i;
        break;
      }
    }
    Snippet::LineEnd(input[..=index_minus_ws].concat(), index_plus_ws + 1)
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
  // At a break already: no space needed for a line end, and spaces after it are trimmed.
  if !is_whitespace(input[max_width_index - 1]) && is_whitespace(input[max_width_index]) {
    return break_at(max_width_index - 1);
  }
  if let Some(url_end) = detect_url(input, max_width_index) {
    let index_plus_ws = url_end
      + input[url_end..]
        .iter()
        .skip(1)
        .position(|g| !is_whitespace(g))
        .unwrap_or(0);
    return Snippet::LineEnd(input[..=url_end].concat(), index_plus_ws + 1);
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

/// Whether what the comment's line makes fits: its first line within the width, the rest
/// within the line, the last no further than the first may go. The standard formatter
/// measures only the text outside comments there, so a later line's marker and all after
/// it are not counted.
fn fits(text: &str, layout: &Layout) -> bool {
  let outside = |line: &str| match line.find("//") {
    Some(at) => str_width(&line[..at]),
    None => str_width(line),
  };
  let mut lines = text.lines();
  let Some(first) = lines.next() else {
    return true;
  };
  if outside(first) > layout.width {
    return false;
  }
  if !text.contains('\n') {
    return true;
  }
  let mut last = 0;
  for line in text.lines().skip(1) {
    last = outside(line);
    if last > layout.max_width {
      return false;
    }
  }
  last <= str_width(layout.indent) + layout.offset + layout.width
}

/// The text's characters, each with the zero-width ones after it.
fn graphemes(text: &str) -> Vec<&str> {
  let mut out: Vec<&str> = Vec::with_capacity(text.len());
  let mut start = None;
  for (at, c) in text.char_indices() {
    // A combining mark goes with the character before it.
    if start.is_some() && char_width(c) == 0 && !c.is_control() {
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

fn trim_end_but_line_feed(text: &mut String) {
  let kept = text
    .trim_end_matches(|c: char| c.is_whitespace() && c != '\n')
    .len();
  text.truncate(kept);
}
