//! Finds the latest stable and nightly rustfmt from Rust's channel manifests (beta is never
//! read) and checks out the rustfmt part of rust-lang/rust at a commit.
//!
//! Checkouts are sparse, shallow and blob-filtered (a few MB), and are cached in
//! `compat/cache/rustfmt/<commit>/`; a `.complete` marker distinguishes a finished checkout
//! from an interrupted one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::common::log::Logger;

const DIST_URL: &str = "https://static.rust-lang.org/dist";

const RUST_REPO: &str = "https://github.com/rust-lang/rust.git";

/// The parts of rustfmt the readers and the case runner use.
const SPARSE_PATHS: &[&str] = &[
  "/src/tools/rustfmt/Configurations.md",
  "/src/tools/rustfmt/src/config/",
  "/src/tools/rustfmt/src/bin/",
  "/src/tools/rustfmt/src/cargo-fmt/main.rs",
  "/src/tools/rustfmt/src/imports.rs",
  "/src/tools/rustfmt/src/items.rs",
  "/src/tools/rustfmt/src/reorder.rs",
  "/src/tools/rustfmt/src/rustfmt_diff.rs",
  "/src/tools/rustfmt/src/emitter/",
  "/src/tools/rustfmt/src/modules.rs",
  "/src/tools/rustfmt/src/modules/",
  "/src/tools/rustfmt/src/sort.rs",
  "/src/tools/rustfmt/tests/source/",
  "/src/tools/rustfmt/tests/target/",
  "/src/tools/rustfmt/tests/config/",
];

#[derive(Debug, PartialEq)]
pub struct Release {
  pub version: String,
  pub commit: String,
  pub date: String,
}

pub fn cache_root(root: &Path) -> PathBuf {
  root.join("compat").join("cache").join("rustfmt")
}

/// The rustfmt directory of a complete cached checkout, after undoing any edits made to it
/// (for example by a formatter run over the whole repository). Never uses the network.
pub fn cached_checkout(root: &Path, commit: &str, log: &Logger) -> Result<Option<PathBuf>, String> {
  let repo = cache_root(root).join(commit);
  if !repo.join(".complete").exists() {
    return Ok(None);
  }
  let changed = git(&repo, &["status", "--porcelain", "--untracked-files=no"])?;
  if !changed.trim().is_empty() {
    log.warning(&format!(
      "cached checkout of {} was modified; restoring:\n{}",
      commit,
      changed.trim_end()
    ));
    println!(
      "restoring modified files in the cached checkout of {}",
      &commit[..12]
    );
    git(&repo, &["checkout", "--quiet", "--", "."])?;
  }
  Ok(Some(repo.join("src").join("tools").join("rustfmt")))
}

/// Checks out rustfmt at `commit`, or reuses the cached checkout.
pub fn checkout(root: &Path, commit: &str, log: &Logger) -> Result<PathBuf, String> {
  if let Some(dir) = cached_checkout(root, commit, log)? {
    // A checkout made before a path was added to SPARSE_PATHS gets it now; this may
    // fetch the missing files.
    let repo = cache_root(root).join(commit);
    let listed = git(&repo, &["sparse-checkout", "list"])?;
    let current: Vec<&str> = listed.lines().map(str::trim).collect();
    if SPARSE_PATHS.iter().any(|p| !current.contains(p)) {
      log.info(&format!("widening the sparse checkout of {}", commit));
      let mut set = vec!["sparse-checkout", "set", "--no-cone"];
      set.extend_from_slice(SPARSE_PATHS);
      git(&repo, &set)?;
    }
    log.info(&format!("reusing cached checkout of {}", commit));
    return Ok(dir);
  }
  let dir = cache_root(root).join(commit);
  if dir.exists() {
    fs::remove_dir_all(&dir).map_err(|e| format!("cannot clear {}: {}", dir.display(), e))?;
  }
  fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
  let mut sparse = vec!["sparse-checkout", "set", "--no-cone"];
  sparse.extend_from_slice(SPARSE_PATHS);
  let steps: Vec<Vec<&str>> = vec![
    vec!["init", "--quiet"],
    vec!["remote", "add", "origin", RUST_REPO],
    sparse,
    vec![
      "fetch",
      "--quiet",
      "--depth",
      "1",
      "--filter=blob:none",
      "origin",
      commit,
    ],
    vec![
      "-c",
      "advice.detachedHead=false",
      "checkout",
      "--quiet",
      "FETCH_HEAD",
    ],
  ];
  for step in steps {
    log.info(&format!("git {}", step.join(" ")));
    git(&dir, &step)?;
  }
  fs::write(dir.join(".complete"), "").map_err(|e| e.to_string())?;
  Ok(dir.join("src").join("tools").join("rustfmt"))
}

/// Downloads `channel`'s manifest (`stable` or `nightly`) and reads rustfmt's release.
pub fn latest(root: &Path, channel: &str, log: &Logger) -> Result<Release, String> {
  let url = format!("{}/channel-rust-{}.toml", DIST_URL, channel);
  let dir = cache_root(root);
  fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
  let path = dir.join(format!("channel-rust-{}.toml", channel));
  log.info(&format!("curl {}", url));
  let status = Command::new("curl")
    .args([
      "--fail",
      "--silent",
      "--show-error",
      "--location",
      "--max-time",
      "60",
      "--output",
    ])
    .arg(&path)
    .arg(&url)
    .status()
    .map_err(|e| format!("cannot run curl: {}", e))?;
  if !status.success() {
    return Err(format!("cannot download {}", url));
  }
  let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
  parse_channel_manifest(&text)
}

/// Reads `date` and rustfmt's `version` and `git_commit_hash` from a channel manifest.
pub fn parse_channel_manifest(text: &str) -> Result<Release, String> {
  let value = |line: &str, key: &str| {
    line
      .strip_prefix(key)
      .and_then(|rest| rest.trim().strip_prefix('='))
      .map(|rest| rest.trim().trim_matches('"').to_string())
  };
  let date = text
    .lines()
    .find_map(|line| value(line, "date"))
    .ok_or("channel manifest has no date")?;
  let mut in_rustfmt = false;
  let mut version = None;
  let mut commit = None;
  for line in text.lines() {
    if line.starts_with('[') {
      in_rustfmt = line == "[pkg.rustfmt-preview]";
      continue;
    }
    if in_rustfmt {
      version = version.or_else(|| value(line, "version"));
      commit = commit.or_else(|| value(line, "git_commit_hash"));
    }
  }
  match (version, commit) {
    (Some(version), Some(commit)) if commit.len() == 40 => Ok(Release {
      version,
      commit,
      date,
    }),
    _ => Err("channel manifest has no rustfmt version and commit".to_string()),
  }
}

/// Runs git in `dir` and returns its standard output.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
  let output = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .output()
    .map_err(|e| format!("cannot run git: {}", e))?;
  if !output.status.success() {
    return Err(format!(
      "`git {}` failed: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr).trim()
    ));
  }
  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_parse_channel_manifest() {
    let text = "manifest-version = \"2\"\ndate = \"2026-10-01\"\n[pkg.rust]\nversion = \"1.99.0 (b940084d7 2026-09-28)\"\ngit_commit_hash = \"0000000000000000000000000000000000000000\"\n\n[pkg.rustfmt-preview]\nversion = \"1.10.0\"\ngit_commit_hash = \"b940084d7eb6a299eb4bfeb8e34901bc051e7ac4\"\n\n[pkg.rustfmt-preview.target.aarch64-apple-darwin]\navailable = true\n";
    assert_eq!(
      parse_channel_manifest(text).unwrap(),
      Release {
        version: "1.10.0".to_string(),
        commit: "b940084d7eb6a299eb4bfeb8e34901bc051e7ac4".to_string(),
        date: "2026-10-01".to_string(),
      }
    );
    assert!(parse_channel_manifest("date = \"x\"\n[pkg.rust]\n").is_err());
  }
}
