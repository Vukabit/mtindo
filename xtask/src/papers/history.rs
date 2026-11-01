//! records/results/history.txt: each tracked record as it stood every time it changed,
//! oldest first, so that the history tables in docs/generated/ come from the tree alone
//! rather than from git (a shallow clone or a rewritten history gives the same pages).
//!
//! An entry is a line `== DATE COMMIT PATH`, then the record's lines without its comments
//! and blank lines; a folder's entry is `files N`. `cargo xtask docs` adds an entry for
//! each record that differs from its last one; the metrics-docs gate reports the ones
//! that do.

use std::fs;
use std::path::Path;

/// The history file, from the repository root.
pub const FILE: &str = "records/results/history.txt";

/// The records whose history is kept: files, and one folder (counted).
pub const TRACKED: &[&str] = &[
  "records/results/compatibility.txt",
  "records/policy/exceptions.txt",
  "records/results/ratchet.txt",
  "tests/fuzz_regressions",
];

/// One entry: when, the commit it was recorded on top of, which record, and its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
  pub date: String,
  pub commit: String,
  pub path: String,
  pub text: String,
}

/// Adds an entry for each stale record, dated `date` on top of `commit`; how many.
pub fn append(root: &Path, date: &str, commit: &str) -> Result<usize, String> {
  let stale = stale(root);
  if stale.is_empty() {
    return Ok(0);
  }
  let mut text = fs::read_to_string(root.join(FILE)).unwrap_or_default();
  for path in &stale {
    let now = current(root, path).unwrap_or_default();
    text.push_str(&format!("== {} {} {}\n{}\n", date, commit, path, now));
  }
  fs::write(root.join(FILE), text).map_err(|e| format!("{}: {}", FILE, e))?;
  Ok(stale.len())
}

/// A record's text as an entry holds it: its lines without comments or blank lines, or
/// for a folder the number of files in it.
pub fn current(root: &Path, path: &str) -> Option<String> {
  let full = root.join(path);
  if full.is_dir() {
    let mut count = 0;
    let mut stack = vec![full];
    while let Some(dir) = stack.pop() {
      for entry in fs::read_dir(&dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
          stack.push(path);
        } else {
          count += 1;
        }
      }
    }
    return Some(format!("files {}", count));
  }
  let text = fs::read_to_string(full).ok()?;
  Some(
    text
      .lines()
      .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
      .collect::<Vec<_>>()
      .join("\n"),
  )
}

/// The entries for `path`, newest first.
pub fn of(root: &Path, path: &str) -> Vec<Entry> {
  let mut entries: Vec<Entry> = read(root).into_iter().filter(|e| e.path == path).collect();
  entries.reverse();
  entries
}

pub fn parse(text: &str) -> Vec<Entry> {
  let mut entries: Vec<Entry> = Vec::new();
  for line in text.lines() {
    if let Some(head) = line.strip_prefix("== ") {
      let mut parts = head.splitn(3, ' ');
      entries.push(Entry {
        date: parts.next().unwrap_or("").to_string(),
        commit: parts.next().unwrap_or("").to_string(),
        path: parts.next().unwrap_or("").to_string(),
        text: String::new(),
      });
    } else if let Some(entry) = entries.last_mut()
      && !line.starts_with('#')
      && !line.trim().is_empty()
    {
      if !entry.text.is_empty() {
        entry.text.push('\n');
      }
      entry.text.push_str(line);
    }
  }
  entries
}

/// Every entry in the file, oldest first.
pub fn read(root: &Path) -> Vec<Entry> {
  parse(&fs::read_to_string(root.join(FILE)).unwrap_or_default())
}

/// The tracked records that differ from their last entry.
pub fn stale(root: &Path) -> Vec<&'static str> {
  let entries = read(root);
  TRACKED
    .iter()
    .copied()
    .filter(|path| {
      let last = entries.iter().rev().find(|e| e.path == *path);
      current(root, path).is_some_and(|now| last.is_none_or(|e| e.text != now))
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_changed_record_gains_an_entry_once() {
    let root = std::env::temp_dir().join(format!("mtindo-history-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("records/results")).unwrap();
    fs::create_dir_all(root.join("records/policy")).unwrap();
    fs::create_dir_all(root.join("tests/fuzz_regressions/lexer")).unwrap();
    fs::write(root.join("tests/fuzz_regressions/lexer/a"), "").unwrap();
    fs::write(
      root.join("records/results/compatibility.txt"),
      "# c\nset tests files 1 2\n",
    )
    .unwrap();
    assert_eq!(append(&root, "2026-11-01", "aaaaaaa").unwrap(), 2);
    assert!(stale(&root).is_empty());
    assert_eq!(append(&root, "2026-11-02", "bbbbbbb").unwrap(), 0);
    fs::write(
      root.join("records/results/compatibility.txt"),
      "set tests files 2 2\n",
    )
    .unwrap();
    assert_eq!(stale(&root), vec!["records/results/compatibility.txt"]);
    append(&root, "2026-11-03", "ccccccc").unwrap();
    let mine = of(&root, "records/results/compatibility.txt");
    assert_eq!(mine[0].text, "set tests files 2 2");
    assert_eq!(mine[1].date, "2026-11-01");
    assert_eq!(of(&root, "tests/fuzz_regressions")[0].text, "files 1");
    let _ = fs::remove_dir_all(&root);
  }

  #[test]
  fn test_entries_are_read_in_order_without_comments() {
    let entries = parse(
      "# about\n\n== 2026-10-10 abc1234 records/x.txt\nset a 1\n\n== 2026-10-11 def5678 records/x.txt\nset a 2\n",
    );
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].text, "set a 1");
    assert_eq!(entries[1].commit, "def5678");
  }
}
