//! The `config-docs` gate: docs/config/, the settings reference, is what `cargo xtask docs
//! config` writes from the settings table now. Like the `man` gate it runs code that links
//! mtindo, here `xtask-tools docs-config --check`, which renders every file of the
//! reference and compares it with the one on disk; a file that differs or is missing is a
//! problem naming the command that regenerates it.

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
        "docs-config",
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

/// The problems in what `docs-config --check` said: each stale file, or why it could not
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
    "cannot check docs/config/ (exit code {}){}{}",
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
      ["cannot check docs/config/ (exit code 101): error: could not compile `mtindo`"]
    );
  }

  #[test]
  fn test_stale_files_are_the_problems() {
    assert!(problems(0, "docs/config matches the settings table\n", "").is_empty());
    let stdout = "docs/config/README.md is stale; run `cargo xtask docs config`\n\
                  docs/config/mto.schema.json is stale; run `cargo xtask docs config`\n";
    assert_eq!(
      problems(1, stdout, ""),
      [
        "docs/config/README.md is stale; run `cargo xtask docs config`",
        "docs/config/mto.schema.json is stale; run `cargo xtask docs config`",
      ]
    );
  }
}
