//! What a release tag's workflow (`.github/workflows/release.yml`) runs, so that its
//! steps are written once here: each target's archive, the checksums and notes the
//! GitHub release takes, and the Homebrew formula the tap gets.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::checks::{self, Scope};
use super::config::Flow;
use super::git;
use crate::common::process;

/// What each archive holds beside the binaries.
const BESIDE: &[&str] = &[
  "README.md",
  "CREDITS.md",
  "LICENSE-MIT",
  "LICENSE-APACHE",
  "LICENSE-UNICODE",
];

/// The release's archives, checksums and notes, under `dist/`.
pub fn dist(root: &Path) -> PathBuf {
  root.join("dist")
}

/// `formula TAG`: the tap's `Formula/mto.rb` for the release, from `dist/SHA256SUMS`.
pub fn formula(root: &Path, flow: &Flow, tag: &str) -> Result<(), String> {
  let version = tag.trim_start_matches('v');
  let sums = fs::read_to_string(dist(root).join("SHA256SUMS"))
    .map_err(|e| format!("dist/SHA256SUMS: {} (run `flow prepare` first)", e))?;
  let sum = |target: &str| -> Result<String, String> {
    let archive = format!("mto-{}-{}.tar.gz", version, target);
    sums
      .lines()
      .find_map(|l| {
        l.split_once("  ")
          .filter(|(_, n)| *n == archive)
          .map(|(s, _)| s.to_string())
      })
      .ok_or(format!("SHA256SUMS has no {}", archive))
  };
  let url = |target: &str| {
    format!(
      "https://github.com/{}/releases/download/{}/mto-{}-{}.tar.gz",
      flow.release.repository, tag, version, target
    )
  };
  let block = |target: &str| -> Result<String, String> {
    Ok(format!(
      "      url \"{}\"\n      sha256 \"{}\"",
      url(target),
      sum(target)?
    ))
  };
  print!(
    "# Written by `cargo xtask flow formula` for each mtindo release.
class Mto < Formula
  desc \"Formats Rust as the standard formatter does, and lays out its items\"
  homepage \"https://github.com/{repository}\"
  version \"{version}\"
  license any_of: [\"MIT\", \"Apache-2.0\"]

  on_macos do
    on_arm do
{mac_arm}
    end
    on_intel do
{mac_intel}
    end
  end

  on_linux do
    on_arm do
{linux_arm}
    end
    on_intel do
{linux_intel}
    end
  end

  def install
    bin.install \"mto\", \"cargo-mto\"
  end

  test do
    assert_match version.to_s, shell_output(\"#{{bin}}/mto --version\")
  end
end
",
    repository = flow.release.repository,
    version = version,
    mac_arm = block("aarch64-apple-darwin")?,
    mac_intel = block("x86_64-apple-darwin")?,
    linux_arm = block("aarch64-unknown-linux-musl")?,
    linux_intel = block("x86_64-unknown-linux-musl")?,
  );
  Ok(())
}

/// `package TARGET`: `dist/mto-VERSION-TARGET.tar.gz` (`.zip` for Windows) with `mto`,
/// `cargo-mto` and the files [`BESIDE`], from `cargo build --release --target TARGET`.
pub fn package(root: &Path, flow: &Flow, target: &str) -> Result<(), String> {
  if !flow.release.targets.iter().any(|t| t == target) {
    return Err(format!(
      "{} is not a release target (flow.toml release.targets)",
      target
    ));
  }
  let version = version(root)?;
  let name = format!("mto-{}-{}", version, target);
  let windows = target.contains("windows");
  let built = root.join("target").join(target).join("release");
  let folder = dist(root).join(&name);
  let _ = fs::remove_dir_all(&folder);
  fs::create_dir_all(&folder).map_err(|e| format!("{}: {}", folder.display(), e))?;
  for bin in ["mto", "cargo-mto"] {
    let file = if windows {
      format!("{}.exe", bin)
    } else {
      bin.to_string()
    };
    fs::copy(built.join(&file), folder.join(&file)).map_err(|e| {
      format!(
        "{}: {} (build it: cargo build --release --locked --target {})",
        file, e, target
      )
    })?;
  }
  for file in BESIDE {
    fs::copy(root.join(file), folder.join(file)).map_err(|e| format!("{}: {}", file, e))?;
  }
  let archive = if windows {
    format!("{}.zip", name)
  } else {
    format!("{}.tar.gz", name)
  };
  // bsdtar writes a zip from the name (`-a`); every runner has a tar.
  let args: Vec<&str> = if windows {
    vec!["-a", "-cf", &archive, &name]
  } else {
    vec!["-czf", &archive, &name]
  };
  let done = process::output(
    Command::new("tar").args(&args).current_dir(dist(root)),
    process::QUICK,
  )
  .map_err(|e| format!("tar could not be run: {}", e))?;
  let _ = fs::remove_dir_all(&folder);
  if done.code != 0 {
    return Err(format!("tar failed for {}", archive));
  }
  println!("dist/{}", archive);
  Ok(())
}

/// `prepare TAG`: the publish stage's checks, then `dist/SHA256SUMS` over the archives
/// there, `dist/notes.md` from the changelog's section, and whether the release is left
/// a draft (`draft=true`), on stdout and in `$GITHUB_OUTPUT` when set. A dry run (no
/// tag: a `dry-run/` branch) checks only that the crate packages, for Cargo.toml's
/// version.
pub fn prepare(root: &Path, flow: &Flow, tag: &str, dry_run: bool) -> Result<(), String> {
  let tag = if dry_run {
    format!("v{}", version(root)?)
  } else {
    tag.to_string()
  };
  let tag = tag.as_str();
  let only_package = ["package".to_string()];
  let names = if dry_run {
    &only_package[..]
  } else {
    flow.stage("publish").unwrap_or_default()
  };
  let scope = Scope {
    stage: "publish".to_string(),
    files: None,
    staged: false,
    ranges: Vec::new(),
    pushed: Vec::new(),
  };
  checks::run_stage(root, flow, names, &scope)?;
  let version = tag.trim_start_matches('v');
  let mut archives: Vec<PathBuf> = fs::read_dir(dist(root))
    .map_err(|e| format!("dist: {}", e))?
    .filter_map(|e| e.ok().map(|e| e.path()))
    .filter(|p| {
      p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("mto-") && (n.ends_with(".tar.gz") || n.ends_with(".zip")))
    })
    .collect();
  archives.sort();
  if archives.len() != flow.release.targets.len() {
    return Err(format!(
      "dist holds {} archive(s), flow.toml lists {} target(s)",
      archives.len(),
      flow.release.targets.len()
    ));
  }
  let mut sums = String::new();
  for path in &archives {
    let bytes = fs::read(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    sums.push_str(&format!(
      "{}  {}\n",
      crate::sha256::hex_digest(&bytes),
      name
    ));
  }
  fs::write(dist(root).join("SHA256SUMS"), &sums).map_err(|e| format!("SHA256SUMS: {}", e))?;
  let changelog = fs::read_to_string(root.join("CHANGELOG.md")).unwrap_or_default();
  let notes =
    section(&changelog, version).ok_or(format!("CHANGELOG.md has no ## [{}] section", version))?;
  fs::write(dist(root).join("notes.md"), notes).map_err(|e| format!("notes.md: {}", e))?;
  let earlier = git::git(root, &["tag", "--list", "v[0-9]*"])?
    .lines()
    .filter(|t| t.trim() != tag)
    .count();
  let draft = flow.release.first_as_draft && earlier == 0;
  let line = format!("draft={}", draft);
  println!("{}", line);
  if let Some(output) = std::env::var_os("GITHUB_OUTPUT") {
    let mut text = fs::read_to_string(&output).unwrap_or_default();
    text.push_str(&format!("{}\n", line));
    fs::write(&output, text).map_err(|e| format!("GITHUB_OUTPUT: {}", e))?;
  }
  Ok(())
}

/// `tap TAG`: a pull request in the Homebrew tap with the release's formula, made with
/// the token in `GH_TOKEN` (the release workflow's `homebrew` job, once approved).
pub fn tap(root: &Path, flow: &Flow, tag: &str) -> Result<(), String> {
  if std::env::var_os("GH_TOKEN").is_none() {
    return Err("tap: GH_TOKEN holds the tap's token (HOMEBREW_TAP_TOKEN)".to_string());
  }
  let version = tag.trim_start_matches('v');
  let mut text = String::new();
  // The formula as `formula` prints it.
  let out = process::output(
    Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
      .args(["flow", "formula", tag])
      .current_dir(root)
      .stdout(std::process::Stdio::piped()),
    process::QUICK,
  )
  .map_err(|e| format!("formula: {}", e))?;
  if out.code != 0 {
    return Err("formula failed".to_string());
  }
  text.push_str(&String::from_utf8_lossy(&out.stdout));
  let clone = dist(root).join("tap");
  let _ = fs::remove_dir_all(&clone);
  let branch = format!("mto-{}", version);
  let url = format!("https://github.com/{}.git", flow.release.tap);
  let clone_text = clone.display().to_string();
  let step = |program: &str, args: &[&str], dir: &Path| -> Result<(), String> {
    let done = process::output(
      Command::new(program).args(args).current_dir(dir),
      process::NETWORK,
    )
    .map_err(|e| format!("{} could not be run: {}", program, e))?;
    if done.code == 0 {
      Ok(())
    } else {
      Err(format!("`{} {}` failed", program, args.join(" ")))
    }
  };
  step("gh", &["auth", "setup-git"], root)?;
  step("git", &["clone", "--depth", "1", &url, &clone_text], root)?;
  fs::create_dir_all(clone.join("Formula")).map_err(|e| e.to_string())?;
  fs::write(clone.join("Formula/mto.rb"), text).map_err(|e| format!("mto.rb: {}", e))?;
  step("git", &["switch", "-c", &branch], &clone)?;
  step("git", &["add", "Formula/mto.rb"], &clone)?;
  step(
    "git",
    &[
      "-c",
      "user.name=mtindo release",
      "-c",
      "user.email=mtindo-release@users.noreply.github.com",
      "commit",
      "-m",
      &format!("mto {}", version),
    ],
    &clone,
  )?;
  step(
    "git",
    &["push", "--set-upstream", "origin", &branch],
    &clone,
  )?;
  step(
    "gh",
    &[
      "pr",
      "create",
      "--repo",
      &flow.release.tap,
      "--head",
      &branch,
      "--title",
      &format!("mto {}", version),
      "--body",
      &format!(
        "The formula for https://github.com/{}/releases/tag/{}, written by `cargo xtask flow formula`.",
        flow.release.repository, tag
      ),
    ],
    &clone,
  )
}

/// The body of `changelog`'s `## [version]` section, up to the next release's heading.
fn section(changelog: &str, version: &str) -> Option<String> {
  let heading = format!("## [{}]", version);
  let start = changelog.find(&heading)?;
  let body = &changelog[start..];
  let body = body.split_once('\n').map_or("", |(_, rest)| rest);
  let end = body.find("\n## [").map_or(body.len(), |i| i + 1);
  Some(format!("{}\n", body[..end].trim()))
}

/// Cargo.toml's `[package]` version.
fn version(root: &Path) -> Result<String, String> {
  let text =
    fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {}", e))?;
  crate::toml::parse(&text)
    .ok()
    .and_then(|t| {
      t.get("package")?
        .as_table()?
        .get("version")?
        .as_str()
        .map(String::from)
    })
    .ok_or("Cargo.toml has no [package] version".to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_release_section_is_its_body_alone() {
    let changelog = "# Changelog\n\n## [0.2.0] - 2027-01-14\n\n### Fixed\n\n- A\n\n## [0.1.0] - 2026-11-01\n\nFirst.\n";
    assert_eq!(
      section(changelog, "0.2.0").as_deref(),
      Some("### Fixed\n\n- A\n")
    );
    assert_eq!(section(changelog, "0.1.0").as_deref(), Some("First.\n"));
    assert_eq!(section(changelog, "0.3.0"), None);
  }
}
