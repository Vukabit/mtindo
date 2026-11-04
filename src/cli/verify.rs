//! `mto verify REV`: whether a commit (a layout commit above all, the kind a project lists
//! in `.git-blame-ignore-revs`) is mto's work and nothing else. Each Rust file it changed is
//! taken from the commit's parent and run through mto with the project's settings: the
//! commit's text is reproduced, the checks passing on the way, or at least the change is
//! one layout or formatting may make. The settings are the ones the commit holds (its
//! mto.toml, rustfmt.toml and Cargo.toml files), not the working tree's, which may have
//! changed since. git is asked for the commit and its files; without git, nothing can be
//! verified.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Committed, Resolver, is_format_settings_file, is_settings_file};
use crate::settle::{Verdict, verdict};

/// What was found for one file.
pub enum Found {
  Verdict(Verdict),
  /// An mto.toml or the standard formatter's file: a layout commit may change them.
  Settings,
  /// Not a Rust file: what changed in it cannot be proved either way.
  NotRust,
  /// A Rust file deleted: no layout or formatting deletes a file.
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
  // The settings as the commit holds them.
  let tree = git(&root, &["ls-tree", "-r", "-z", "--name-only", &commit])?;
  let mut settings_files = Vec::new();
  for path in tree.split('\0').filter(|p| !p.is_empty()) {
    let name = path.rsplit('/').next().unwrap_or(path);
    if is_settings_file(name) {
      let text = git(&root, &["show", &format!("{}:{}", commit, path)])?;
      settings_files.push((path.to_string(), text));
    }
  }
  resolver.read_committed(Committed::new(&root, settings_files));
  let fields: Vec<&str> = listed.split('\0').filter(|f| !f.is_empty()).collect();
  let mut checked = Vec::new();
  for pair in fields.chunks(2) {
    let [status, path] = pair else {
      continue;
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    let found = if is_format_settings_file(name) {
      Found::Settings
    } else if !path.ends_with(".rs") {
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
      let config = resolver
        .config_for(&folder)
        .map_err(|e| format!("{}: the settings {} holds: {}", path, &commit[..10], e))?;
      let settings = crate::Settings::of(&config);
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
