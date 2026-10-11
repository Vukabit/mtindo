//! Parses every `.rs` file under the given paths and reports syntax errors, to find grammar
//! gaps on real code. Each file is parsed with its crate's edition (from the nearest
//! Cargo.toml), or the edition named by a `// rustfmt-edition: YEAR` comment in rustfmt's tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mtindo::syntax::parse;
use mtindo::text::line_index::LineIndex;

use crate::common;

const SHOWN_MESSAGES: usize = 25;

const SHOWN_PER_FILE: usize = 2;

pub fn run(root: &Path, args: &[String]) -> ExitCode {
  let paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
  let mut files = Vec::new();
  if paths.is_empty() {
    // Every file of the repository, but its fixtures, which include deliberately broken
    // files.
    match common::repo_rust_files(root) {
      Ok(all) => files.extend(
        all
          .into_iter()
          .filter(|f| !f.components().any(|c| c.as_os_str() == "fixtures")),
      ),
      Err(e) => {
        eprintln!("parsecheck: {}", e);
        return ExitCode::from(1);
      }
    }
  }
  for path in paths.iter() {
    collect(path, &mut files);
  }
  files.sort();

  let mut parsed = 0;
  let mut failing = 0;
  let mut messages: BTreeMap<String, usize> = BTreeMap::new();
  for file in files.iter() {
    let text = match fs::read_to_string(file) {
      Ok(text) => text,
      Err(e) => {
        failing += 1;
        println!("{}: cannot read: {}", file.display(), e);
        continue;
      }
    };
    parsed += 1;
    let edition = common::edition_for(file, &text);
    let result = parse(&text, edition);
    assert_eq!(
      result.syntax_node().text(),
      text,
      "{} did not round-trip",
      file.display()
    );
    if result.errors().is_empty() {
      continue;
    }
    failing += 1;
    let index = LineIndex::new(&text);
    println!("{} (edition {}):", file.display(), edition.year());
    for error in result.errors().iter().take(SHOWN_PER_FILE) {
      let position = index.line_col(error.range.start as usize);
      println!(
        "  {}:{}: {}",
        position.line + 1,
        position.byte_column + 1,
        error.message
      );
    }
    for error in result.errors() {
      *messages.entry(error.message.clone()).or_insert(0) += 1;
    }
  }
  if !messages.is_empty() {
    let mut ranked: Vec<(&String, &usize)> = messages.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));
    println!("most common errors:");
    for (message, count) in ranked.into_iter().take(SHOWN_MESSAGES) {
      println!("  {:>6}  {}", count, message);
    }
  }
  println!("parsecheck: {} files, {} with errors", parsed, failing);
  if parsed == 0 {
    println!("parsecheck: no files parsed, so nothing was checked");
    return ExitCode::from(1);
  }
  if failing == 0 {
    ExitCode::SUCCESS
  } else {
    ExitCode::from(1)
  }
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
  if path.is_file() {
    if path.extension().is_some_and(|ext| ext == "rs") {
      files.push(path.to_path_buf());
    }
    return;
  }
  let entries = match fs::read_dir(path) {
    Ok(entries) => entries,
    Err(_) => return,
  };
  for entry in entries.flatten() {
    let child = entry.path();
    let name = entry.file_name().to_string_lossy().to_string();
    let is_build_dir = name == "target" && path.join("Cargo.toml").exists();
    if is_build_dir || name == ".git" {
      continue;
    }
    if child.is_dir() {
      collect(&child, files);
    } else if child.extension().is_some_and(|ext| ext == "rs") {
      files.push(child);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A scratch folder holding `files`; removed when dropped.
  struct Scratch(PathBuf);

  impl Scratch {
    fn new(name: &str, files: &[(&str, &str)]) -> Scratch {
      let dir = std::env::temp_dir().join(format!("{}-{}", name, std::process::id()));
      let _ = fs::remove_dir_all(&dir);
      fs::create_dir_all(&dir).unwrap();
      for (path, text) in files {
        fs::write(dir.join(path), text).unwrap();
      }
      Scratch(dir)
    }

    fn arg(&self) -> Vec<String> {
      vec![self.0.display().to_string()]
    }
  }

  impl Drop for Scratch {
    fn drop(&mut self) {
      let _ = fs::remove_dir_all(&self.0);
    }
  }

  #[test]
  fn test_errors_fail_and_clean_files_pass() {
    let clean = Scratch::new("parsecheck-clean", &[("a.rs", "fn f() {}\n")]);
    assert_eq!(run(&clean.0, &clean.arg()), ExitCode::SUCCESS);
    let broken = Scratch::new("parsecheck-broken", &[("a.rs", "fn f( {\n")]);
    assert_eq!(run(&broken.0, &broken.arg()), ExitCode::from(1));
  }

  #[test]
  fn test_no_files_is_a_failure() {
    let empty = Scratch::new("parsecheck-empty", &[("notes.txt", "no rust here")]);
    assert_eq!(run(&empty.0, &empty.arg()), ExitCode::from(1));
  }
}
