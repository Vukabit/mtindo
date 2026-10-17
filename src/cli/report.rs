//! `mto report`: the bug reports mto keeps when it finds a bug in itself (see
//! `crate::report`), in a folder beside history.
//!
//! - `mto report`: every report, the most recently seen first.
//! - `mto report show ID`: one report, as it would be filed.
//! - `mto report save ID FILE [--with-file]`: the report into FILE; `--with-file` adds the
//!   whole file it was found in, which a report otherwise never holds.
//! - `mto report send ID`: opens a new issue on mto's repository in the browser, filled in
//!   with the report, to read over and submit. mto itself sends nothing.
//! - `mto report prune`: removes every report.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::{Resolver, user};
use crate::history::sha256;
use crate::report::{Entry, Store};

/// What ends a body cut to fit a link. No path: the issue is public, and the path names the
/// user's folders.
const CUT_NOTE: &str = "\n\n…\n\n(Cut to fit a link: paste the rest of the report here.)\n";

/// The longest link opened, as encoded: the site refuses links a little past 8,000
/// characters (`414 URI Too Long`), and some browsers cut them.
const MAX_LINK: usize = 8000;

/// The longest title put in a link, as encoded.
const MAX_TITLE: usize = 400;

/// Where reports are kept: beside history, in mto's state folder.
pub fn dir(resolver: &Resolver) -> Option<PathBuf> {
  user::history_dir(resolver.env())
    .and_then(|history| history.parent().map(|state| state.join("reports")))
}

/// `mto report`: every report, the most recently seen first.
pub fn list(store: &Store) -> String {
  let entries = store.list();
  if entries.is_empty() {
    return "no bug reports: mto has found no bug in itself here\n".to_string();
  }
  let mut out = format!(
    "{:<8} {:<20} {:>5}  {}\n",
    "REPORT", "LAST SEEN", "SEEN", "WHAT"
  );
  for entry in &entries {
    let file = entry
      .files
      .first()
      .map_or(String::new(), |seen| format!(" ({})", seen.shown));
    out.push_str(&format!(
      "{:<8} {:<20} {:>5}  {}{}\n",
      &entry.id[..6],
      super::history::utc(entry.last),
      format!("{}x", entry.count),
      entry.title,
      file
    ));
  }
  out.push_str("`mto report show ID` shows one; `mto report send ID` files it\n");
  out
}

/// `mto report prune`: removes every report.
pub fn prune(store: &Store) -> Result<String, String> {
  let removed = store
    .clear()
    .map_err(|e| format!("cannot remove reports: {}", e))?;
  Ok(format!("removed {} report(s)\n", removed))
}

/// `mto report save ID FILE [--with-file]`: the report into `file`, with the whole file it
/// was first found in when `with_file`.
pub fn save(store: &Store, id: &str, file: &str, with_file: bool) -> Result<String, String> {
  let entry = store.find(id)?;
  let mut text = entry
    .read()
    .map_err(|e| format!("cannot read report {}: {}", entry.id, e))?;
  let mut note = String::new();
  if with_file {
    let first = entry.files.first();
    let Some((seen, path)) = first.and_then(|s| s.path.as_ref().map(|p| (s, p))) else {
      return Err(
        "the file came from stdin; a report keeps no more of it than the smallest part".to_string(),
      );
    };
    let whole =
      fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    if sha256::hex(whole.as_bytes()) != seen.hash {
      note = format!(
        "note: {} has changed since the bug was found; its present contents are added\n",
        seen.shown
      );
    }
    text.push_str(&format!(
      "\nThe whole file, `{}`:\n\n```rust\n{}\n```\n",
      seen.shown,
      whole.trim_end()
    ));
  }
  fs::write(file, text).map_err(|e| format!("cannot write {}: {}", file, e))?;
  Ok(format!(
    "{}saved report {} to {}\n",
    note,
    &entry.id[..6],
    file
  ))
}

/// `mto report send ID`: opens a new issue on mto's repository in the browser, filled in
/// with the report. A report too long for a link is cut, and the terminal says where the
/// whole of it is, to paste.
pub fn send(store: &Store, id: &str) -> Result<String, String> {
  let entry = store.find(id)?;
  let text = entry
    .read()
    .map_err(|e| format!("cannot read report {}: {}", entry.id, e))?;
  let whole = entry.dir.join("report.md");
  let (link, cut) = issue_link(&entry.title, &text);
  let mut out = String::new();
  if cut {
    out.push_str(&format!(
      "the report is too long for a link: the issue holds its start; paste the rest from {}\n",
      whole.display()
    ));
  }
  if open(&link) {
    out.push_str(
      "opened a new issue in the browser, filled in with the report: read it over, then \
       submit it\n",
    );
  } else {
    out.push_str(&format!(
      "cannot open a browser; open this link to file the report:\n{}\n",
      link
    ));
  }
  Ok(out)
}

/// `mto report show ID`: the report, then when and where it was seen.
pub fn show(store: &Store, id: &str) -> Result<String, String> {
  let entry = store.find(id)?;
  let text = entry
    .read()
    .map_err(|e| format!("cannot read report {}: {}", entry.id, e))?;
  Ok(format!("{}\n{}", text, seen(&entry)))
}

/// The store a run keeps reports in; `None` with `report.save = false` or no state folder,
/// when a bug's whole report is printed instead.
pub fn store(resolver: &Resolver) -> Option<Store> {
  let save = resolver
    .user_value("report.save")
    .and_then(|v| v.as_bool())
    .unwrap_or(true);
  dir(resolver).filter(|_| save).map(Store::new)
}

/// `text` with everything but unreserved characters percent-encoded, for a link.
fn encode(text: &str) -> String {
  let mut out = String::with_capacity(text.len() * 3);
  for byte in text.bytes() {
    match byte {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
      _ => out.push_str(&format!("%{:02X}", byte)),
    }
  }
  out
}

/// How long `c` is once [`encode`]d.
fn encoded_len(c: char) -> usize {
  if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
    1
  } else {
    3 * c.len_utf8()
  }
}

/// The longest start of `text` that is at most `budget` characters once encoded, ending
/// after a line when one fits, else at a character.
fn fitting(text: &str, budget: usize) -> &str {
  let mut used = 0;
  let mut line_end = None;
  for (at, c) in text.char_indices() {
    used += encoded_len(c);
    if used > budget {
      return &text[..line_end.unwrap_or(at)];
    }
    if c == '\n' {
      line_end = Some(at + 1);
    }
  }
  text
}

/// A link to a new issue on mto's repository with `title` and `body` filled in, at most
/// [`MAX_LINK`] characters long; and whether the body was cut to fit.
fn issue_link(title: &str, body: &str) -> (String, bool) {
  let head = format!(
    "{}/issues/new?title={}&body=",
    env!("CARGO_PKG_REPOSITORY"),
    encode(fitting(title, MAX_TITLE))
  );
  let (body, cut) = link_body(body, MAX_LINK.saturating_sub(head.len()));
  (format!("{}{}", head, encode(&body)), cut)
}

/// The body put in a link `budget` characters are left for once encoded, and whether it
/// was cut: all of `text` when it fits; else its start, a code block left open closed, and
/// [`CUT_NOTE`].
fn link_body(text: &str, budget: usize) -> (String, bool) {
  if text.chars().map(encoded_len).sum::<usize>() <= budget {
    return (text.to_string(), false);
  }
  let close = "\n```";
  let reserved = CUT_NOTE
    .chars()
    .chain(close.chars())
    .map(encoded_len)
    .sum::<usize>();
  let mut body = fitting(text, budget.saturating_sub(reserved)).to_string();
  let fences = body
    .lines()
    .filter(|line| line.trim_start().starts_with("```"))
    .count();
  if fences % 2 == 1 {
    body.push_str(close);
  }
  body.push_str(CUT_NOTE);
  (body, true)
}

/// Opens `link` in the browser, as the system opens links; whether it could.
fn open(link: &str) -> bool {
  let mut command = if cfg!(target_os = "macos") {
    let mut c = Command::new("open");
    c.arg(link);
    c
  } else if cfg!(windows) {
    // Not `cmd /c start`, which reads the link's `&` as its own.
    let mut c = Command::new("rundll32");
    c.args(["url.dll,FileProtocolHandler", link]);
    c
  } else {
    let mut c = Command::new("xdg-open");
    c.arg(link);
    c
  };
  command
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|status| status.success())
}

/// When and where a report's bug was seen.
fn seen(entry: &Entry) -> String {
  let files: Vec<&str> = entry.files.iter().map(|s| s.shown.as_str()).collect();
  format!(
    "seen {} time(s), first {}, last {}, in {}\n",
    entry.count,
    super::history::utc(entry.first),
    super::history::utc(entry.last),
    files.join(", ")
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_body_is_cut_at_a_line_and_a_character() {
    assert_eq!(fitting("ab\ncd\nef", 7), "ab\n");
    assert_eq!(fitting("abcdef", 3), "abc");
    assert_eq!(fitting("é", 5), "");
    let (body, cut) = link_body("short", 100);
    assert_eq!((body.as_str(), cut), ("short", false));
  }

  #[test]
  fn test_a_long_report_link_fits_once_encoded() {
    // Punctuation and spaces triple when encoded; non-ASCII grows ninefold.
    let line = "    let x = (a, b) => { c[d] }; // é ü → ✓\n";
    let text = format!("## Input\n\n```rust\n{}```\n", line.repeat(400));
    let title = "panic: ".to_string() + &"ü".repeat(1000);
    let (link, cut) = issue_link(&title, &text);
    assert!(cut);
    assert!(link.len() <= MAX_LINK, "{}", link.len());
    let body = link.split("&body=").nth(1).unwrap();
    // The open code block is closed before the note.
    assert!(body.contains(&encode("\n```\n\n…")), "{}", body);
    assert!(!body.contains(&encode("/Users/")));
  }

  #[test]
  fn test_an_issue_link_encodes_its_text() {
    assert_eq!(encode("a b&c=d\n`x`"), "a%20b%26c%3Dd%0A%60x%60");
    let (link, cut) = issue_link("panic: boom", "body");
    assert!(!cut);
    assert!(
      link.ends_with("/issues/new?title=panic%3A%20boom&body=body"),
      "{}",
      link
    );
  }
}
