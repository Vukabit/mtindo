//! What the flow asks git: the files and lines a commit changes, the paths in the tree,
//! the commits in a range.

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::common::process;

/// What commit `sha` changes, against its first parent (or nothing, for a root commit).
pub fn changed_by(root: &Path, sha: &str) -> Result<(Vec<String>, usize), String> {
  Ok(numstat(&git(
    root,
    &[
      "diff-tree",
      "--root",
      "--no-commit-id",
      "-r",
      "--numstat",
      "--no-renames",
      sha,
    ],
  )?))
}

/// The commits in `range` (`rev-list` arguments: `A..B`, or `B --not --remotes`), oldest
/// first, each with whether it is a merge.
pub fn commits(root: &Path, range: &[String]) -> Result<Vec<(String, bool)>, String> {
  let mut args = vec!["rev-list", "--reverse", "--parents"];
  args.extend(range.iter().map(String::as_str));
  let text = git(root, &args)?;
  Ok(
    text
      .lines()
      .filter_map(|line| {
        let mut words = line.split_whitespace();
        let sha = words.next()?.to_string();
        Some((sha, words.count() > 1))
      })
      .collect(),
  )
}

/// `git args...` at `root`: its output, or what went wrong.
pub fn git(root: &Path, args: &[&str]) -> Result<String, String> {
  let done = process::output(
    Command::new("git")
      .args(args)
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  )
  .map_err(|e| format!("git could not be run: {}", e))?;
  if done.code != 0 {
    return Err(format!(
      "git {} failed: {}",
      args.join(" "),
      String::from_utf8_lossy(&done.stderr).trim()
    ));
  }
  Ok(String::from_utf8_lossy(&done.stdout).into_owned())
}

/// A path under the repository's git folder (`.git/MERGE_HEAD`), as git resolves it.
pub fn git_path(root: &Path, name: &str) -> Result<std::path::PathBuf, String> {
  let path = git(root, &["rev-parse", "--git-path", name])?;
  Ok(root.join(path.trim()))
}

/// Commit `sha`'s message.
pub fn message(root: &Path, sha: &str) -> Result<String, String> {
  git(root, &["log", "-1", "--format=%B", sha])
}

/// Whether `path` names a file in `files` or a folder holding one.
pub fn names(files: &HashSet<String>, extra: &[String], path: &str) -> bool {
  let path = path.trim_end_matches('/');
  let prefix = format!("{}/", path);
  files
    .iter()
    .chain(extra)
    .any(|f| f == path || f.starts_with(&prefix))
}

/// A commit's files and changed lines, from `git diff --numstat` output: added and
/// removed lines (none counted for a binary file) and each path.
pub fn numstat(text: &str) -> (Vec<String>, usize) {
  let mut files = Vec::new();
  let mut lines = 0;
  for line in text.lines() {
    let mut parts = line.splitn(3, '\t');
    let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
    else {
      continue;
    };
    lines += added.parse::<usize>().unwrap_or(0) + removed.parse::<usize>().unwrap_or(0);
    files.push(path.to_string());
  }
  (files, lines)
}

/// What is staged: its files and changed lines.
pub fn staged(root: &Path) -> Result<(Vec<String>, usize), String> {
  Ok(numstat(&git(
    root,
    &["diff", "--cached", "--numstat", "--no-renames"],
  )?))
}

/// Every file in the index, or in commit `at`'s tree.
pub fn tree_files(root: &Path, at: Option<&str>) -> Result<HashSet<String>, String> {
  let text = match at {
    None => git(root, &["ls-files", "--cached"])?,
    Some(sha) => git(root, &["ls-tree", "-r", "--name-only", sha])?,
  };
  Ok(text.lines().map(String::from).collect())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_numstat_counts_lines_and_lists_files() {
    let (files, lines) = numstat("3\t1\tsrc/a.rs\n-\t-\tlogo.png\n10\t0\tREADME.md\n");
    assert_eq!(files, vec!["src/a.rs", "logo.png", "README.md"]);
    assert_eq!(lines, 14);
  }
}
