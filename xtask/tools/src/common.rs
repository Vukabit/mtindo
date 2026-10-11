//! Helpers shared by the tools.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use mtindo::syntax::Edition;

use crate::process;

/// The edition to parse `file` with.
pub fn edition_for(file: &Path, text: &str) -> Edition {
  for line in text.lines().take(20) {
    if let Some(year) = line.trim().strip_prefix("// rustfmt-edition:")
      && let Some(edition) = Edition::from_year(year)
    {
      return edition;
    }
  }
  let mut dir = file.parent();
  while let Some(current) = dir {
    if let Ok(manifest) = fs::read_to_string(current.join("Cargo.toml")) {
      for line in manifest.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("edition") {
          let year = value
            .trim()
            .trim_start_matches('=')
            .trim()
            .trim_matches('"');
          if let Some(edition) = Edition::from_year(year) {
            return edition;
          }
        }
      }
      if manifest.contains("[package]") && !manifest.contains("edition.workspace") {
        return Edition::E2015;
      }
    }
    dir = current.parent();
  }
  Edition::E2015
}

/// Every `.rs` file of the repository, tracked or new and not ignored by git, sorted: what
/// the checks over the repository's own code read when given no paths, so that a new
/// folder is covered without being listed anywhere.
pub fn repo_rust_files(root: &Path) -> Result<Vec<PathBuf>, String> {
  let listing = process::output(
    Command::new("git")
      .args([
        "ls-files",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
        "*.rs",
      ])
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
  let mut files: Vec<PathBuf> = String::from_utf8_lossy(&listing.stdout)
    .lines()
    .filter(|line| !line.is_empty())
    .map(|line| root.join(line))
    // Listed but deleted in the working tree.
    .filter(|path| path.is_file())
    .collect();
  files.sort();
  files.dedup();
  Ok(files)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_edition_from_a_header_comment() {
    let text = "// rustfmt-edition: 2018\nfn f() {}\n";
    assert_eq!(
      edition_for(Path::new("/nowhere/x.rs"), text),
      Edition::E2018
    );
    assert_eq!(
      edition_for(Path::new("/nowhere/x.rs"), "fn f() {}\n"),
      Edition::E2015
    );
  }

  #[test]
  fn test_repo_rust_files_cover_every_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap();
    let files = repo_rust_files(&root).unwrap();
    for expected in [
      "src/lib.rs",
      "xtask/src/main.rs",
      "xtask/tools/src/main.rs",
      "fuzz/fuzz_targets/format.rs",
    ] {
      assert!(files.contains(&root.join(expected)), "{} missing", expected);
    }
    assert!(
      files
        .iter()
        .all(|f| f.extension().is_some_and(|e| e == "rs"))
    );
    assert!(!files.iter().any(|f| f.starts_with(root.join("target"))));
  }
}
