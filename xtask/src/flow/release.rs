//! `flow release VERSION`: a release cut, after `[stages] release`'s checks: the
//! version set, the changelog's section named for it, the release commit and its tag,
//! both marked for `hold` to let through. It never pushes: it prints the push.
//! `flow yank VERSION` prints how to take a release back.

use std::fs;
use std::path::Path;

use super::checks::{self, Scope};
use super::config::Flow;
use super::git;

pub fn release(root: &Path, flow: &Flow, version: &str, go: bool) -> Result<(), String> {
  let version = version.trim_start_matches('v');
  let numbers = semver(version).ok_or(format!(
    "`{}` is not a version (MAJOR.MINOR.PATCH)",
    version
  ))?;
  let tag = format!("v{}", version);
  let branch = git::git(root, &["symbolic-ref", "--short", "HEAD"])
    .map(|b| b.trim().to_string())
    .map_err(|_| "release from a branch, not a detached HEAD".to_string())?;
  let lands = target(&branch, flow);
  // A fix release continues its own line; any other follows the newest tag.
  if let Some(line) = branch.strip_prefix("release/")
    && !version.starts_with(&format!("{}.", line))
  {
    return Err(format!(
      "{} is on the {} line, not {}",
      branch, line, version
    ));
  }
  if let Some(last) = last_tag(root)
    && let Some(last_numbers) = semver(last.trim_start_matches('v'))
    && !branch.starts_with("release/")
    && numbers <= last_numbers
  {
    return Err(format!("{} does not follow {}", tag, last));
  }
  if git::git(
    root,
    &["rev-parse", "-q", "--verify", &format!("refs/tags/{}", tag)],
  )
  .is_ok()
  {
    return Err(format!("{} is already tagged", tag));
  }
  super::tools::require(flow, "release")?;
  let scope = Scope {
    stage: "release".to_string(),
    files: None,
    staged: false,
    ranges: Vec::new(),
    pushed: Vec::new(),
  };
  checks::run_stage(
    root,
    flow,
    flow.stage("release").unwrap_or_default(),
    &scope,
  )?;
  let first = last_tag(root).is_none();
  println!("would:");
  println!("  1. set version {} in Cargo.toml and Cargo.lock", version);
  if first {
    println!(
      "  2. keep CHANGELOG.md's [{}] section, written by hand",
      version
    );
  } else {
    println!(
      "  2. name CHANGELOG.md's newest section [{}], dated today",
      version
    );
  }
  println!("  3. commit  chore(release): {}", tag);
  println!("  4. tag     {} (annotated), both marked for the hold", tag);
  let push = format!(
    "git push origin HEAD:refs/heads/{} refs/tags/{}",
    lands, tag
  );
  println!("then run:  {}", push);
  if !go {
    println!("dry run: add --go to do 1-4");
    return Ok(());
  }
  let made = make(root, version, &tag, first);
  if let Err(e) = made {
    // Back to the tree as it was: nothing half made.
    let _ = git::git(
      root,
      &[
        "checkout",
        "HEAD",
        "--",
        "Cargo.toml",
        "Cargo.lock",
        "CHANGELOG.md",
      ],
    );
    return Err(format!(
      "{}; Cargo.toml, Cargo.lock and CHANGELOG.md are as they were",
      e
    ));
  }
  println!("released {}: now run  {}", tag, push);
  Ok(())
}

/// How to take a release back, in order: nothing is done.
pub fn yank(version: &str) -> Result<(), String> {
  let version = version.trim_start_matches('v');
  semver(version).ok_or(format!("`{}` is not a version", version))?;
  println!("to take back {} (nothing here is run for you):", version);
  println!("  1. cargo yank --version {} mtindo", version);
  println!("     (no new project can depend on it; ones that do keep building)");
  println!(
    "  2. gh release edit v{} --prerelease --title \"v{} (yanked)\"",
    version, version
  );
  println!("     (keeps the binaries reachable for anyone who pinned them)");
  println!("  3. cut the fix as the next patch release: cargo xtask flow release <next>");
  println!("the tag stays: a tag others have fetched is not taken back.");
  Ok(())
}

/// The newest release tag, if any.
fn last_tag(root: &Path) -> Option<String> {
  git::git(
    root,
    &["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*"],
  )
  .ok()
  .map(|t| t.trim().to_string())
  .filter(|t| !t.is_empty())
}

/// Steps 1-4.
fn make(root: &Path, version: &str, tag: &str, first: bool) -> Result<(), String> {
  set_version(root, version)?;
  run(
    root,
    "cargo",
    &["update", "--workspace", "--offline", "--quiet"],
  )?;
  if first {
    let path = root.join("CHANGELOG.md");
    let changelog = fs::read_to_string(&path).unwrap_or_default();
    if !changelog.contains(&format!("## [{}]", version)) {
      return Err(format!(
        "the first release's section, ## [{}], is written by hand in CHANGELOG.md",
        version
      ));
    }
    // Its date, once it is known.
    let today = crate::common::log::timestamp(std::time::SystemTime::now());
    let dated = changelog.replacen(
      &format!("## [{}] - unreleased", version),
      &format!("## [{}] - {}", version, today.get(..10).unwrap_or_default()),
      1,
    );
    fs::write(&path, dated).map_err(|e| format!("CHANGELOG.md: {}", e))?;
  } else {
    crate::changelog::write(root, Some(tag))?;
  }
  git::git(root, &["add", "Cargo.toml", "Cargo.lock", "CHANGELOG.md"])?;
  run(
    root,
    "git",
    &[
      "commit",
      "--quiet",
      "-m",
      &format!("chore(release): {}", tag),
    ],
  )?;
  git::git(
    root,
    &["tag", "-a", tag, "-m", &format!("mtindo {}", version)],
  )?;
  // The hold reads the commit pushed to the branch and the tag's own object.
  let commit = git::git(root, &["rev-parse", "HEAD"])?;
  let object = git::git(root, &["rev-parse", tag])?;
  fs::write(
    git::git_path(root, "flow-release")?,
    format!("{}\n{}\n", commit.trim(), object.trim()),
  )
  .map_err(|e| format!("the release mark: {}", e))
}

/// A command that must succeed, its output passed through.
fn run(root: &Path, program: &str, args: &[&str]) -> Result<(), String> {
  let done = crate::common::process::output(
    std::process::Command::new(program)
      .args(args)
      .current_dir(root),
    crate::common::process::LONG,
  )
  .map_err(|e| format!("{} could not be run: {}", program, e))?;
  if done.code == 0 {
    Ok(())
  } else {
    Err(format!("`{} {}` failed", program, args.join(" ")))
  }
}

fn semver(text: &str) -> Option<(u64, u64, u64)> {
  let mut parts = text.split('.');
  let numbers = (
    parts.next()?.parse().ok()?,
    parts.next()?.parse().ok()?,
    parts.next()?.parse().ok()?,
  );
  parts.next().is_none().then_some(numbers)
}

/// Cargo.toml's `[package]` version set to `version`.
fn set_version(root: &Path, version: &str) -> Result<(), String> {
  let path = root.join("Cargo.toml");
  let text = fs::read_to_string(&path).map_err(|e| format!("Cargo.toml: {}", e))?;
  let mut in_package = false;
  let mut done = false;
  let lines: Vec<String> = text
    .lines()
    .map(|line| {
      if line.starts_with('[') {
        in_package = line.trim() == "[package]";
      }
      if in_package && !done && line.starts_with("version = ") {
        done = true;
        return format!("version = \"{}\"", version);
      }
      line.to_string()
    })
    .collect();
  if !done {
    return Err("Cargo.toml has no [package] version".to_string());
  }
  fs::write(&path, format!("{}\n", lines.join("\n"))).map_err(|e| format!("Cargo.toml: {}", e))
}

/// Where a release lands: `main` from any branch it fast-forwards to, or a
/// `release/X.Y` branch for a fix to an earlier release.
fn target(branch: &str, flow: &Flow) -> String {
  if branch.starts_with("release/") {
    branch.to_string()
  } else {
    flow.release.branch.clone()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_versions_are_three_numbers() {
    assert_eq!(semver("0.2.0"), Some((0, 2, 0)));
    assert_eq!(semver("0.10.1"), Some((0, 10, 1)));
    assert!(semver("0.2.0") > semver("0.1.9"));
    assert_eq!(semver("0.2"), None);
    assert_eq!(semver("0.2.0-rc1"), None);
  }
}
