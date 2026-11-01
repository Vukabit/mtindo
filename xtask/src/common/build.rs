//! The release builds the measuring commands run: mto, and xtask-tools' commands, from
//! where cargo puts them (`CARGO_TARGET_DIR` when set, as cargo reads it).

use std::path::{Path, PathBuf};

use super::process;

/// The release mto, built: its path, or cargo's output when it would not build.
pub fn mto(root: &Path) -> Result<PathBuf, String> {
  let args = ["build", "--quiet", "--release", "--bin", "mto"].map(String::from);
  let (code, output) = process::capture(root, "cargo", &args, &[], process::LONG);
  if code != 0 {
    return Err(format!("cannot build mto:\n{}", output));
  }
  Ok(mto_path(root))
}

/// Where the release mto is, built or not.
pub fn mto_path(root: &Path) -> PathBuf {
  release_dir(
    root,
    std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from),
  )
  .join(format!("mto{}", std::env::consts::EXE_SUFFIX))
}

/// cargo's arguments for running xtask-tools' `args` in release.
pub fn tools(args: &[String]) -> Vec<String> {
  [
    "run",
    "--quiet",
    "--release",
    "--package",
    "xtask-tools",
    "--",
  ]
  .iter()
  .map(|s| s.to_string())
  .chain(args.iter().cloned())
  .collect()
}

/// The release folder under `target` (cargo's, from the root, when relative), or under
/// the root's `target/`.
fn release_dir(root: &Path, target: Option<PathBuf>) -> PathBuf {
  root
    .join(target.unwrap_or_else(|| PathBuf::from("target")))
    .join("release")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_the_release_folder_follows_the_target_dir() {
    let root = Path::new("/repo");
    assert_eq!(release_dir(root, None), Path::new("/repo/target/release"));
    assert_eq!(
      release_dir(root, Some(PathBuf::from("out"))),
      Path::new("/repo/out/release")
    );
    assert_eq!(
      release_dir(root, Some(PathBuf::from("/elsewhere"))),
      Path::new("/elsewhere/release")
    );
    assert_eq!(
      tools(&["pairs".to_string()]).join(" "),
      "run --quiet --release --package xtask-tools -- pairs"
    );
  }
}
