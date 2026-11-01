//! The repository's files as git sees them.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::process;

/// The files matching `pathspecs`, tracked or new and not ignored by git, relative to
/// `root` with `/` separators, sorted. A file deleted but not yet committed is listed.
pub fn repo(root: &Path, pathspecs: &[&str]) -> Result<Vec<String>, String> {
  let listing = process::output(
    Command::new("git")
      .args([
        "ls-files",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
      ])
      .args(pathspecs)
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  )
  .map_err(|e| format!("cannot run git: {}", e))?;
  if listing.code != 0 {
    return Err(format!(
      "cannot list the repository's files with git: {}",
      String::from_utf8_lossy(&listing.stderr).trim()
    ));
  }
  let mut files: Vec<String> = String::from_utf8_lossy(&listing.stdout)
    .lines()
    .filter(|line| !line.is_empty())
    .map(str::to_string)
    .collect();
  files.sort();
  files.dedup();
  Ok(files)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_repo_lists_what_the_pathspecs_match() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = repo(&root, &["xtask/core"]).unwrap();
    assert!(files.contains(&"xtask/core/src/files.rs".to_string()));
    assert!(files.iter().all(|f| f.starts_with("xtask/core/")));
    assert!(files.windows(2).all(|w| w[0] < w[1]));
  }
}
