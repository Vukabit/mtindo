//! `mto verify REV`: whether a commit (a layout commit above all, the kind a project lists
//! in `.git-blame-ignore-revs`) is mto's work and nothing else. Each Rust file it changed is
//! taken from the commit's parent and run through mto with the project's settings: the
//! commit's text is reproduced, the checks passing on the way, or at least the change is
//! one layout or formatting may make. git is asked for the commit and its files; without
//! git, nothing can be verified.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Resolver;
use crate::settle::{Verdict, verdict};

/// What was found for one file.
pub enum Found {
  Verdict(Verdict),
  /// Not a Rust file: not mto's to check.
  NotRust,
  /// Deleted: no layout or formatting deletes a file.
  Deleted,
}

/// One file a commit changed, and what was found.
pub struct Checked {
  pub path: String,
  pub found: Found,
}

/// The commit `rev` names, checked file by file; an error saying why it cannot be.
pub fn verify(
  rev: &str,
  resolver: &mut Resolver,
  here: &Path,
) -> Result<(String, Vec<Checked>), String> {
  let root = PathBuf::from(git(here, &["rev-parse", "--show-toplevel"])?.trim());
  let commit = git(
    &root,
    &["rev-parse", "--verify", &format!("{}^{{commit}}", rev)],
  )?;
  let commit = commit.trim().to_string();
  let parent = git(&root, &["rev-parse", "--verify", &format!("{}^", commit)])
    .map_err(|_| format!("{} has no parent to compare it with", rev))?;
  let parent = parent.trim().to_string();
  let listed = git(
    &root,
    &[
      "diff",
      "--name-status",
      "--no-renames",
      "-z",
      &parent,
      &commit,
    ],
  )?;
  let fields: Vec<&str> = listed.split('\0').filter(|f| !f.is_empty()).collect();
  let mut checked = Vec::new();
  for pair in fields.chunks(2) {
    let [status, path] = pair else {
      continue;
    };
    let found = if !path.ends_with(".rs") {
      Found::NotRust
    } else if status.starts_with('D') {
      Found::Deleted
    } else {
      let after = git(&root, &["show", &format!("{}:{}", commit, path)])?;
      // A file the commit adds is checked as mto would leave it.
      let before = if status.starts_with('A') {
        after.clone()
      } else {
        git(&root, &["show", &format!("{}:{}", parent, path)])?
      };
      let folder = root
        .join(path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(root.clone());
      let settings = resolver
        .config_for(&folder)
        .ok()
        .and_then(|config| crate::Settings::of(&config));
      Found::Verdict(match settings {
        Some(settings) => verdict(&before, &after, &settings),
        None => Verdict::LeftAlone("layout and formatting are both off here".into()),
      })
    };
    checked.push(Checked {
      path: path.to_string(),
      found,
    });
  }
  Ok((commit, checked))
}

/// What `git ARGS` prints in `dir`, or why it failed.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
  let output = Command::new("git")
    .args(args)
    .current_dir(dir)
    .output()
    .map_err(|e| format!("git cannot be run ({}); verifying a commit needs it", e))?;
  if !output.status.success() {
    return Err(format!(
      "git {}: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr).trim()
    ));
  }
  String::from_utf8(output.stdout).map_err(|_| format!("git {}: not UTF-8", args.join(" ")))
}
