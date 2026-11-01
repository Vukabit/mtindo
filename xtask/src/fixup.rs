//! `cargo xtask fix-up`: what a commit wants done first, in order, so that the hooks pass:
//!
//! 1. mto's layout and formatting on the Rust files changed since HEAD, each alone
//!    (`--skip-children`; untracked ones too, test fixtures and fuzz inputs aside, which
//!    hold their bytes on purpose);
//! 2. `cargo fmt --all`;
//! 3. every generated file, from the tracked records only (`cargo xtask docs --tracked`),
//!    so a page that only this machine's data would move is left as committed.

use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::common::process;

pub fn run(root: &Path, _m: &clap::ArgMatches) -> ExitCode {
  let files = changed_rust_files(root);
  if files.is_empty() {
    println!("fix-up: no Rust file changed since HEAD");
  } else {
    println!("fix-up: mto on {} changed file(s)", files.len());
    // The files alone, not the modules they declare.
    let mut args = vec![
      "run".to_string(),
      "--quiet".into(),
      "--bin".into(),
      "mto".into(),
      "--".into(),
      "--skip-children".into(),
    ];
    args.extend(files);
    if process::forward(root, &[], &args) != ExitCode::SUCCESS {
      println!("fix-up: mto failed; nothing after it was run");
      return ExitCode::from(1);
    }
  }
  println!("fix-up: cargo fmt --all");
  if process::forward(root, &["fmt", "--all"], &[]) != ExitCode::SUCCESS {
    return ExitCode::from(1);
  }
  println!("fix-up: cargo xtask docs --tracked");
  if process::forward(root, &["xtask", "docs", "--tracked"], &[]) != ExitCode::SUCCESS {
    return ExitCode::from(1);
  }
  println!("fix-up: done; `cargo xtask check` before committing");
  ExitCode::SUCCESS
}

/// The `.rs` files changed since HEAD, untracked ones included, that mto may format:
/// not test fixtures or fuzz inputs, and only those that still exist.
fn changed_rust_files(root: &Path) -> Vec<String> {
  let list = |args: &[&str]| -> Vec<String> {
    process::output(
      Command::new("git")
        .args(args)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()),
      process::QUICK,
    )
    .map(|done| {
      String::from_utf8_lossy(&done.stdout)
        .lines()
        .map(str::to_string)
        .collect()
    })
    .unwrap_or_default()
  };
  let mut files = list(&["diff", "--name-only", "HEAD", "--", "*.rs"]);
  files.extend(list(&[
    "ls-files",
    "--others",
    "--exclude-standard",
    "--",
    "*.rs",
  ]));
  files.sort();
  files.dedup();
  files.retain(|f| formattable(f) && root.join(f).is_file());
  files
}

/// Whether mto may format the file: not a test fixture or fuzz input, whose bytes are
/// the test.
fn formattable(file: &str) -> bool {
  !file.starts_with("tests/fixtures/") && !file.starts_with("tests/fuzz_regressions/")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fixtures_and_fuzz_inputs_are_left_alone() {
    assert!(formattable("src/lib.rs"));
    assert!(formattable("xtask/src/fixup.rs"));
    assert!(!formattable("tests/fixtures/syntax/err/a.rs"));
    assert!(!formattable("tests/fuzz_regressions/lexer/x.rs"));
  }
}
