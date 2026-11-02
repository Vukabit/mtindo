//! Terminal colors for help screens and messages.
//!
//! This file is also compiled into the xtask runner with `#[path]`, so that the runner
//! builds even when the rest of mtindo does not. Keep it self-contained: std only, no
//! `crate::` paths.

/// The colors of mto's lists and statuses (`mto history`, `mto doctor`), one per meaning,
/// in the help screens' hues.
pub mod tone {
  /// All is well: on, current, as written.
  pub const GOOD: &str = "32";

  /// A column's or a section's name.
  pub const HEADING: &str = "1;34";

  /// What names a thing on the list: a run's handle, a report's id.
  pub const ID: &str = "33";

  /// A setting's or a field's name.
  pub const KEY: &str = "36";

  /// What says little: times, sources, "none".
  pub const QUIET: &str = "90";

  /// A setting's value.
  pub const VALUE: &str = "32";

  /// Something wants a look: stale, changed since.
  pub const WARN: &str = "33";
}

use std::env;
use std::io::IsTerminal;

/// `--color`: 0 auto, 1 always, 2 never.
static CHOICE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The user config's `output.color`, in the same code.
static CONFIGURED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The terminal prints escape codes as text (an old Windows console): no color unless
/// asked for.
static NO_ESCAPES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Colors in the style of Python's argparse help, shared by every help screen in the project
/// so they look alike: bold blue headings, bold magenta program name, bold green short flags,
/// bold cyan long flags, bold yellow values; usage lines use the same hues without bold.
pub struct HelpPalette {
  on: bool,
  program: &'static str,
  /// The column descriptions wrap before.
  width: usize,
}

impl HelpPalette {
  pub fn new(on: bool, program: &'static str) -> Self {
    HelpPalette {
      on,
      program,
      width: help_width(),
    }
  }

  pub fn heading(&self, text: &str) -> String {
    self.paint("1;34", text)
  }

  pub fn long(&self, text: &str) -> String {
    self.paint("1;36", text)
  }

  pub fn paint(&self, code: &str, text: &str) -> String {
    paint(self.on, code, text)
  }

  pub fn program(&self, text: &str) -> String {
    self.paint("1;35", text)
  }

  /// A help row: `label` (whose visible width is `width`) padded to `column`, then `help`,
  /// wrapped at the help width with its lines starting at `column`. A label too wide for
  /// the column puts the help on the next line.
  pub fn row(&self, label: &str, width: usize, column: usize, help: &str) -> String {
    let lines = wrap(help, self.width.saturating_sub(column).max(20));
    let indent = " ".repeat(column);
    let mut out = if width + 2 <= column {
      format!("{}{}", label, " ".repeat(column - width))
    } else {
      format!("{}\n{}", label, indent)
    };
    out.push_str(&lines.join(&format!("\n{}", indent)));
    out.push('\n');
    out
  }

  pub fn short(&self, text: &str) -> String {
    self.paint("1;32", text)
  }

  /// A usage line: the program name, then flags and placeholders in their colors.
  pub fn usage_line(&self, line: &str) -> String {
    line
      .split(' ')
      .map(|word| {
        if self.program.split(' ').any(|part| part == word) {
          self.program(word)
        } else if word.starts_with("--") {
          self.paint("36", word)
        } else if word.starts_with('-') {
          self.paint("32", word)
        } else if word.starts_with('<')
          || word.starts_with('[')
          || (word.contains('|') && word.len() > 1)
          || word.ends_with(']')
        {
          self.paint("33", word)
        } else {
          word.to_string()
        }
      })
      .collect::<Vec<_>>()
      .join(" ")
  }

  pub fn value(&self, text: &str) -> String {
    self.paint("1;33", text)
  }
}

/// Whether to color stdout (or stderr, with `is_err`). `--color always|never` decides when
/// given; otherwise `NO_COLOR` turns color off and wins, then `FORCE_COLOR` turns it on,
/// then the user config's `output.color`, then color goes only to a terminal.
pub fn color_enabled(is_err: bool) -> bool {
  match CHOICE.load(std::sync::atomic::Ordering::Relaxed) {
    1 => return true,
    2 => return false,
    _ => {}
  }
  // Asked for every line of a diff, so worked out once per stream: the answer cannot
  // change during a run.
  static DETECTED: std::sync::OnceLock<(Option<bool>, bool, bool)> = std::sync::OnceLock::new();
  // Asked here rather than of `terminal`, as the xtask runner compiles this file alone.
  #[allow(clippy::disallowed_methods)]
  let (forced, stdout, stderr) = *DETECTED.get_or_init(|| {
    let set = |name: &str| env::var_os(name).is_some_and(|value| !value.is_empty());
    let forced = if set("NO_COLOR") {
      Some(false)
    } else if set("FORCE_COLOR") {
      Some(true)
    } else {
      None
    };
    (
      forced,
      std::io::stdout().is_terminal(),
      std::io::stderr().is_terminal(),
    )
  });
  if let Some(forced) = forced {
    return forced;
  }
  match CONFIGURED.load(std::sync::atomic::Ordering::Relaxed) {
    1 => return true,
    2 => return false,
    _ => {}
  }
  !NO_ESCAPES.load(std::sync::atomic::Ordering::Relaxed) && if is_err { stderr } else { stdout }
}

/// The width help screens wrap to: `$COLUMNS` when set, kept between 60 and 100 so the text
/// stays readable, else 80.
pub fn help_width() -> usize {
  env::var("COLUMNS")
    .ok()
    .and_then(|c| c.trim().parse::<usize>().ok())
    .map(|c| c.clamp(60, 100))
    .unwrap_or(80)
}

/// `text` in `code` when `on`; empty text stays empty.
pub fn paint(on: bool, code: &str, text: &str) -> String {
  if on && !text.is_empty() {
    format!("\x1b[{}m{}\x1b[0m", code, text)
  } else {
    text.to_string()
  }
}

/// Sets `--color`: `Some(true)` always, `Some(false)` never, `None` automatic.
pub fn set_color(choice: Option<bool>) {
  CHOICE.store(code(choice), std::sync::atomic::Ordering::Relaxed);
}

/// Sets the user config's `output.color`, which the environment and `--color` override.
pub fn set_configured_color(choice: Option<bool>) {
  CONFIGURED.store(code(choice), std::sync::atomic::Ordering::Relaxed);
}

/// A shell script (`mto completions`) with its comments, strings, variables and keywords
/// colored as `mto` colors Rust; every line closes its colors. The rules are the shells'
/// common ground (bash, zsh, fish, PowerShell), enough for the scripts mto writes.
pub fn shell(text: &str, on: bool) -> String {
  if !on {
    return text.to_string();
  }
  const COMMENT: &str = "90";
  const KEYWORD: &str = "35";
  const STRING: &str = "32";
  const VARIABLE: &str = "36";
  const KEYWORDS: &[&str] = &[
    "break", "case", "continue", "do", "done", "elif", "else", "end", "esac", "fi", "for",
    "foreach", "function", "if", "local", "param", "return", "switch", "then", "until", "while",
  ];
  let mut out = String::with_capacity(text.len() * 2);
  // Paints `piece` line by line, so a string running over a line end closes its color.
  let mut push = |color: Option<&str>, piece: &str| match color {
    Some(color) => {
      for (at, line) in piece.split('\n').enumerate() {
        if at > 0 {
          out.push('\n');
        }
        out.push_str(&paint(true, color, line));
      }
    }
    None => out.push_str(piece),
  };
  let bytes = text.as_bytes();
  let name_end = |from: usize| {
    let mut end = from;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
      end += 1;
    }
    end
  };
  // Past the character at `from`, or the end.
  let after = |from: usize| {
    from
      + text
        .get(from..)
        .and_then(|rest| rest.chars().next())
        .map_or(0, char::len_utf8)
  };
  // Where the variable at `from` (a `$`) ends, or `from` itself when it is not one.
  let variable_end = |from: usize| match bytes.get(from + 1) {
    Some(b'{') => text[from..]
      .find('}')
      .map_or(text.len(), |close| from + close + 1),
    Some(b'#' | b'@' | b'*' | b'?') => from + 2,
    Some(_) => name_end(from + 1),
    None => from,
  };
  // A word is a command when it opens a line or follows `;`, `|`, `&`, `(` or `{`.
  let mut command = true;
  // `in` is a keyword on a line that opened with `for`, `foreach`, `case` or `select`.
  let mut looping = false;
  // Inside `name=( ... )`, an array's words, which are never commands.
  let mut array = false;
  let mut plain = 0;
  let mut at = 0;
  while at < bytes.len() {
    let byte = bytes[at];
    let word_start =
      at == 0 || bytes[at - 1].is_ascii_whitespace() || b";|&({".contains(&bytes[at - 1]);
    let end = match byte {
      b'#' if word_start => Some((
        COMMENT,
        text[at..].find('\n').map_or(text.len(), |n| at + n),
      )),
      b'\'' => Some((
        STRING,
        text[at + 1..].find('\'').map_or(text.len(), |n| at + n + 2),
      )),
      b'"' => {
        push(None, &text[plain..at]);
        let mut end = at + 1;
        let mut from = at;
        while end < bytes.len() && bytes[end] != b'"' {
          match bytes[end] {
            b'\\' | b'`' => end = after(end + 1),
            b'$' if variable_end(end) > end + 1 => {
              push(Some(STRING), &text[from..end]);
              let stop = variable_end(end).min(text.len());
              push(Some(VARIABLE), &text[end..stop]);
              from = stop;
              end = stop;
            }
            _ => end += 1,
          }
        }
        let end = (end + 1).min(text.len());
        push(Some(STRING), &text[from..end]);
        plain = end;
        at = end;
        command = false;
        continue;
      }
      b'$' if variable_end(at) > at + 1 => Some((VARIABLE, variable_end(at).min(text.len()))),
      b'\\' => {
        at = after(at + 1);
        continue;
      }
      _ if word_start && (byte.is_ascii_alphabetic() || byte == b'_') => {
        let end = name_end(at);
        let word = &text[at..end];
        let opens = command && !array;
        let keyword = (opens && KEYWORDS.contains(&word)) || (looping && word == "in");
        if opens && word != "local" {
          looping = matches!(word, "for" | "foreach" | "case" | "select");
        }
        // After these the next word is a command again: `then echo`, `do set`.
        command = matches!(
          word,
          "then" | "do" | "else" | "elif" | "if" | "while" | "until"
        );
        if keyword {
          Some((KEYWORD, end))
        } else {
          at = end;
          continue;
        }
      }
      _ => None,
    };
    match end {
      Some((color, end)) => {
        push(None, &text[plain..at]);
        push(Some(color), &text[at..end]);
        plain = end;
        at = end;
        // A keyword has set whether a command follows it.
        command &= color == KEYWORD;
      }
      None => {
        match byte {
          b'\n' => {
            command = true;
            looping = false;
          }
          b'(' if at > 0 && bytes[at - 1] == b'=' => array = true,
          b')' => array = false,
          b';' | b'|' | b'&' | b'(' | b'{' => command = true,
          _ if !byte.is_ascii_whitespace() => command = false,
          _ => {}
        }
        at += 1;
      }
    }
  }
  push(None, &text[plain..]);
  out
}

/// The terminal prints escape codes as text: color goes to it only when asked for.
pub fn terminal_reads_no_escapes() {
  NO_ESCAPES.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// TOML as `mto config print` writes it, in color when `on`: comments and the sources
/// after values quiet, a `# section` or `[table]` line a heading, then keys and values.
pub fn toml(text: &str, on: bool) -> String {
  if !on {
    return text.to_string();
  }
  let mut out = String::with_capacity(text.len() * 2);
  for line in text.split_inclusive('\n') {
    let (body, end) = match line.strip_suffix('\n') {
      Some(body) => (body, "\n"),
      None => (line, ""),
    };
    let painted = if body.starts_with("# ") && !body.contains(" = ") && body.ends_with(')') {
      paint(true, tone::HEADING, body)
    } else if body.starts_with('#') {
      paint(true, tone::QUIET, body)
    } else if body.starts_with('[') {
      paint(true, tone::HEADING, body)
    } else if let Some((key, rest)) = body.split_once(" = ") {
      let (value, comment) = match rest.find("  # ") {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
      };
      format!(
        "{} = {}{}",
        paint(true, tone::KEY, key),
        paint(true, tone::VALUE, value),
        paint(true, tone::QUIET, comment)
      )
    } else {
      body.to_string()
    };
    out.push_str(&painted);
    out.push_str(end);
  }
  out
}

/// `text` in lines of at most `width` characters, broken at spaces; a word longer than the
/// width gets a line of its own.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
  let mut lines = Vec::new();
  let mut line = String::new();
  for word in text.split_whitespace() {
    if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
      lines.push(std::mem::take(&mut line));
    }
    if !line.is_empty() {
      line.push(' ');
    }
    line.push_str(word);
  }
  if !line.is_empty() || lines.is_empty() {
    lines.push(line);
  }
  lines
}

fn code(choice: Option<bool>) -> u8 {
  match choice {
    None => 0,
    Some(true) => 1,
    Some(false) => 2,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// `text` without its color codes.
  fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("\x1b[") {
      out.push_str(&rest[..at]);
      let end = rest[at..].find('m').expect("a code ends in m");
      rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    out
  }

  #[test]
  fn test_shell_closes_a_string_at_each_line_end() {
    assert_eq!(
      shell("'one\ntwo'\n", true),
      "\x1b[32m'one\x1b[0m\n\x1b[32mtwo'\x1b[0m\n"
    );
  }

  #[test]
  fn test_shell_colors_a_keyword_after_then_and_do() {
    let colored = shell(
      "if a; then return 1; fi
",
      true,
    );
    assert!(
      colored.contains("\x1b[35mthen\x1b[0m \x1b[35mreturn\x1b[0m 1; \x1b[35mfi\x1b[0m"),
      "{:?}",
      colored
    );
  }

  #[test]
  fn test_shell_colors_each_kind_of_word() {
    let text = "# note\nfor w in $words; do\n  echo \"a ${x} é\\é\" 'it'\\''s' # end\ndone\n";
    let colored = shell(text, true);
    let expect = [
      "\x1b[90m# note\x1b[0m\n",
      "\x1b[35mfor\x1b[0m w \x1b[35min\x1b[0m \x1b[36m$words\x1b[0m; \x1b[35mdo\x1b[0m\n",
      "echo \x1b[32m\"a \x1b[0m\x1b[36m${x}\x1b[0m\x1b[32m é\\é\"\x1b[0m",
      "\x1b[32m'it'\x1b[0m\\'\x1b[32m's'\x1b[0m \x1b[90m# end\x1b[0m\n",
      "\x1b[35mdone\x1b[0m\n",
    ];
    for piece in expect {
      assert!(colored.contains(piece), "{:?} not in {:?}", piece, colored);
    }
    assert_eq!(plain(&colored), text);
  }

  #[test]
  fn test_shell_leaves_words_and_unended_text_as_written() {
    for text in [
      "keys=(for done in)\n",
      "a#b $ \"open $",
      "x\\",
      "'open",
      "${open",
    ] {
      assert_eq!(plain(&shell(text, true)), text);
    }
    assert!(!shell("keys=(for done)\n", true).contains("\x1b[35m"));
    assert_eq!(shell("# x\n", false), "# x\n");
  }
}
