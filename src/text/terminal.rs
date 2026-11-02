//! Everything mto prints goes through here: [`out!`](crate::out), [`outln!`](crate::outln),
//! [`err!`](crate::err) and [`errln!`](crate::errln) stand in for `print!` and the rest,
//! which clippy.toml refuses elsewhere. Much of what mto prints it did not write: a file's
//! name, its source in a `--check` diff, a report, a command's output. On a terminal, a
//! control byte in that text would be read as a command (move the cursor, clear the
//! screen, set the title, hide the lines after it), so it is shown instead: `^[` for an
//! escape, `<U+202E>` for a character that turns text around. The color codes mto writes
//! itself are kept while color is on. A pipe or a file gets the bytes unchanged.

use std::borrow::Cow;
use std::fmt;
use std::io::{IsTerminal, Write};

use crate::text::color::color_enabled;

/// The escape codes mto colors with (`ESC [ code m`); any other is shown, never obeyed.
const COLORS: &[&str] = &[
  "0", "1", "1;31", "1;32", "1;33", "1;34", "1;35", "1;36", "32", "33", "35", "36", "90",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stream {
  Out,
  Err,
}

impl Stream {
  /// Whether the stream is a terminal, worked out once: it cannot change during a run.
  #[allow(clippy::disallowed_methods, reason = "the one place that asks")]
  pub fn is_terminal(self) -> bool {
    static TERMINAL: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    let (out, err) = *TERMINAL.get_or_init(|| {
      (
        std::io::stdout().is_terminal(),
        std::io::stderr().is_terminal(),
      )
    });
    match self {
      Stream::Out => out,
      Stream::Err => err,
    }
  }
}

/// Writes `args` to `stream`, as `print!` does: shown safe on a terminal (see the module
/// comment). A failed write panics with `print!`'s words, so a closed pipe still ends the
/// run quietly.
#[allow(clippy::disallowed_methods, reason = "the one place that writes")]
pub fn write(stream: Stream, args: fmt::Arguments) {
  let text = match args.as_str() {
    Some(text) => Cow::Borrowed(text),
    None => Cow::Owned(args.to_string()),
  };
  let text = if stream.is_terminal() {
    shown(&text, color_enabled(stream == Stream::Err))
  } else {
    text
  };
  let result = match stream {
    Stream::Out => std::io::stdout().lock().write_all(text.as_bytes()),
    Stream::Err => std::io::stderr().lock().write_all(text.as_bytes()),
  };
  if let Err(e) = result {
    let name = match stream {
      Stream::Out => "stdout",
      Stream::Err => "stderr",
    };
    panic!("failed printing to {}: {}", name, e);
  }
}

/// `text` with every character that could control a terminal shown instead, but for the
/// color codes in `COLORS` when `color` is on. Lines and tabs are kept; a carriage return
/// is kept only before a line end, since alone it lets a line write over the one shown.
pub fn shown(text: &str, color: bool) -> Cow<'_, str> {
  if !text.chars().any(controls) {
    return Cow::Borrowed(text);
  }
  let mut out = String::with_capacity(text.len() + 8);
  let mut rest = text;
  while let Some(c) = rest.chars().next() {
    let len = c.len_utf8();
    if c == '\x1b'
      && color
      && let Some(code) = color_code(rest)
    {
      out.push_str(&rest[..code]);
      rest = &rest[code..];
      continue;
    }
    match c {
      '\r' if rest[1..].starts_with('\n') => out.push(c),
      '\n' | '\t' => out.push(c),
      '\0'..='\x1f' => {
        out.push('^');
        out.push((c as u8 + b'@') as char);
      }
      '\x7f' => out.push_str("^?"),
      _ if controls(c) => out.push_str(&format!("<U+{:04X}>", c as u32)),
      _ => out.push(c),
    }
    rest = &rest[len..];
  }
  Cow::Owned(out)
}

/// Whether `c` can do more on a terminal than show itself: the C0 and C1 controls, and the
/// characters that change the direction text runs in.
fn controls(c: char) -> bool {
  matches!(
    c,
    '\0'..='\x08'
      | '\x0b'..='\x1f'
      | '\x7f'..='\u{9f}'
      | '\u{61c}'
      | '\u{200e}'
      | '\u{200f}'
      | '\u{202a}'..='\u{202e}'
      | '\u{2066}'..='\u{2069}'
  )
}

/// The length of the color code `text` starts with, when it is one of `COLORS`.
fn color_code(text: &str) -> Option<usize> {
  let body = text.strip_prefix("\x1b[")?;
  let end = body.find('m')?;
  COLORS.contains(&&body[..end]).then_some(end + 3)
}

/// `print!` through [`write`].
#[macro_export]
macro_rules! out {
  ($($arg:tt)*) => {
    $crate::text::terminal::write($crate::text::terminal::Stream::Out, format_args!($($arg)*))
  };
}

/// `println!` through [`write`].
#[macro_export]
macro_rules! outln {
  () => {
    $crate::out!("\n")
  };
  ($($arg:tt)*) => {
    $crate::out!("{}\n", format_args!($($arg)*))
  };
}

/// `eprint!` through [`write`].
#[macro_export]
macro_rules! err {
  ($($arg:tt)*) => {
    $crate::text::terminal::write($crate::text::terminal::Stream::Err, format_args!($($arg)*))
  };
}

/// `eprintln!` through [`write`].
#[macro_export]
macro_rules! errln {
  () => {
    $crate::err!("\n")
  };
  ($($arg:tt)*) => {
    $crate::err!("{}\n", format_args!($($arg)*))
  };
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_controls_are_shown_and_mtos_colors_kept() {
    // A name that would set the window title, then clear the screen.
    let name = "src/\x1b]0;owned\x07a.rs\x1b[2J";
    assert_eq!(shown(name, true), "src/^[]0;owned^Ga.rs^[[2J");
    // mto's own colors stay while color is on, and are shown once it is off.
    let error = "\x1b[1;31mError: bad\x1b[0m";
    assert_eq!(shown(error, true), error);
    assert_eq!(shown(error, false), "^[[1;31mError: bad^[[0m");
    // Hiding text is not one of mto's colors.
    assert_eq!(shown("\x1b[8mhidden", true), "^[[8mhidden");
    // Lines, tabs and Windows line ends are text; a lone carriage return is not.
    assert_eq!(shown("a\tb\r\nc\rd\n", true), "a\tb\r\nc^Md\n");
    assert_eq!(
      shown("x\u{202e}y\u{9b}z\x7f", true),
      "x<U+202E>y<U+009B>z^?"
    );
    assert!(matches!(shown("plain ünïcode", true), Cow::Borrowed(_)));
  }

  #[test]
  fn test_every_color_mto_writes_is_kept() {
    // Every `\x1b[...m` written in src/ is one of COLORS, or it would be shown as text.
    fn walk(dir: &std::path::Path, found: &mut Vec<(String, String)>) {
      for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
          walk(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("terminal.rs") {
          let text = std::fs::read_to_string(&path).unwrap();
          for (at, _) in text.match_indices("\\x1b[") {
            let rest = &text[at + 5..];
            let end = rest
              .find(|c: char| !c.is_ascii_digit() && c != ';' && c != '{' && c != '}')
              .unwrap_or(0);
            if rest[end..].starts_with('m') {
              found.push((path.display().to_string(), rest[..end].to_string()));
            }
          }
        }
      }
    }
    let mut found = Vec::new();
    walk(
      &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
      &mut found,
    );
    assert!(!found.is_empty());
    for (file, code) in found {
      // `\x1b[{}m` is paint's: its codes are checked through HelpPalette's calls below.
      if code == "{}" {
        continue;
      }
      assert!(
        COLORS.contains(&code.as_str()),
        "{} writes \\x1b[{}m",
        file,
        code
      );
    }
    let p = crate::text::color::HelpPalette::new(true, "mto");
    let painted = [
      p.heading("h"),
      p.long("l"),
      p.program("p"),
      p.short("s"),
      p.value("v"),
      p.usage_line("mto [OPTIONS] <PATH>... --flag"),
    ];
    for text in painted {
      assert_eq!(shown(&text, true), text);
    }
  }
}
