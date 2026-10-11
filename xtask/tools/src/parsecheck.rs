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

const DEFAULT_PATHS: &[&str] = &["src", "tests", "benches", "xtask/src", "fuzz/fuzz_targets"];

const SHOWN_MESSAGES: usize = 25;

const SHOWN_PER_FILE: usize = 2;

pub fn run(root: &Path, args: &[String]) -> ExitCode {
  let mut paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
  // The repository's own fixtures include deliberately broken files; skip them by default.
  let skip_fixtures = paths.is_empty();
  if paths.is_empty() {
    paths = DEFAULT_PATHS.iter().map(|p| root.join(p)).collect();
  }
  let mut files = Vec::new();
  for path in paths.iter() {
    collect(path, skip_fixtures, &mut files);
  }
  files.sort();

  let mut parsed = 0;
  let mut failing = 0;
  let mut messages: BTreeMap<String, usize> = BTreeMap::new();
  for file in files.iter() {
    let text = match fs::read_to_string(file) {
      Ok(text) => text,
      Err(_) => continue,
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
  if failing == 0 {
    ExitCode::SUCCESS
  } else {
    ExitCode::from(1)
  }
}

fn collect(path: &Path, skip_fixtures: bool, files: &mut Vec<PathBuf>) {
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
    if is_build_dir || name == ".git" || (skip_fixtures && name == "fixtures") {
      continue;
    }
    if child.is_dir() {
      collect(&child, skip_fixtures, files);
    } else if child.extension().is_some_and(|ext| ext == "rs") {
      files.push(child);
    }
  }
}
