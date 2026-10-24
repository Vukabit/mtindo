//! The `generated-docs` gate: the settings reference in docs/generated/config/ and the
//! manual, docs/generated/manual.md, are what `cargo xtask docs` writes from mtindo's tables
//! now. Like the `man` gate it runs code that links mtindo, here `xtask-tools docs --check`,
//! which renders every file and compares it with the one on disk; a file that differs or is
//! missing is a problem naming the command that regenerates it.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::common::process;

pub fn check_reference(root: &Path) -> Vec<String> {
  let output = process::output(
    Command::new("cargo")
      .args([
        "run",
        "--quiet",
        "--package",
        "xtask-tools",
        "--",
        "docs",
        "--check",
      ])
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::LONG,
  );
  match output {
    Ok(output) => problems(
      output.code,
      &String::from_utf8_lossy(&output.stdout),
      &String::from_utf8_lossy(&output.stderr),
    ),
    Err(e) => vec![format!("cannot run xtask-tools: {}", e)],
  }
}

/// The problems in what `docs --check` said: each stale file, or why it could not
/// tell.
fn problems(code: i32, stdout: &str, stderr: &str) -> Vec<String> {
  if code == 0 {
    return Vec::new();
  }
  let stale: Vec<String> = stdout
    .lines()
    .filter(|line| line.contains(" is stale; "))
    .map(str::to_string)
    .collect();
  if !stale.is_empty() {
    return stale;
  }
  let said = stderr.trim();
  vec![format!(
    "cannot check docs/generated/ (exit code {}){}{}",
    code,
    if said.is_empty() { "" } else { ": " },
    said
  )]
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_failure_to_check_says_why() {
    assert_eq!(
      problems(101, "", "error: could not compile `mtindo`\n"),
      ["cannot check docs/generated/ (exit code 101): error: could not compile `mtindo`"]
    );
  }

  #[test]
  fn test_stale_files_are_the_problems() {
    assert!(problems(0, "docs/generated/config matches\n", "").is_empty());
    let stdout = "docs/generated/config/README.md is stale; run `cargo xtask docs`\n\
                  docs/generated/manual.md is stale; run `cargo xtask docs`\n";
    assert_eq!(
      problems(1, stdout, ""),
      [
        "docs/generated/config/README.md is stale; run `cargo xtask docs`",
        "docs/generated/manual.md is stale; run `cargo xtask docs`",
      ]
    );
  }
}
