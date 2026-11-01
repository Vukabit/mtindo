//! Gathers this build's identity at compile time, for `mto --version`, bug reports and the
//! doctor's holds (which lapse when the build changes): the commit, and when the source
//! differs from it a hash of the source; the target, the profile and the compiler.
//!
//! git runs here, while compiling, if this is a checkout; the mto it builds never runs it.
//! A build from a package (crates.io) has no git: the package's `.cargo_vcs_info.json`
//! names the commit it was made from, and without one a hash of the source identifies it.
//!
//! Each build is a release (`-rel`) or a development build (`-dev`): a release is built
//! from a published package, or from a clean checkout of the tag `v<version>` (the
//! release workflow's; in CI, the tag being built counts when the shallow clone lacks
//! it). Anything else, a dirty tree always, is a development build.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
  let Ok(entries) = fs::read_dir(dir) else {
    return;
  };
  for entry in entries.flatten() {
    let path = entry.path();
    if path.is_dir() {
      collect(&path, files);
    } else {
      files.push(path);
    }
  }
}

/// What `git ARGS` prints in `root`, trimmed; `None` when git is missing or fails.
fn git(root: &Path, args: &[&str]) -> Option<String> {
  let out = Command::new("git")
    .args(args)
    .current_dir(root)
    .output()
    .ok()?;
  out
    .status
    .success()
    .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
  let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
  for watched in ["src", "Cargo.toml", "Cargo.lock", "build.rs"] {
    println!("cargo:rerun-if-changed={}", watched);
  }
  // The commit moves with HEAD, the branch it names, and the packed refs. Not the index:
  // any `git status` rewrites it, and each rewrite would rebuild mto and all that uses it
  // (the source's own changes are watched above). Only files that exist are named, as
  // cargo reruns the script on every build for one that does not.
  let branch = git(&root, &["symbolic-ref", "-q", "HEAD"]);
  for name in ["HEAD", "packed-refs"].into_iter().chain(branch.as_deref()) {
    if let Some(path) = git(&root, &["rev-parse", "--git-path", name]) {
      let path = root.join(path);
      if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
      }
    }
  }
  // A new tag, or one moved, can make the same commit a release.
  if let Some(tags) = git(&root, &["rev-parse", "--git-path", "refs/tags"]) {
    let tags = root.join(tags);
    if tags.exists() {
      println!("cargo:rerun-if-changed={}", tags.display());
    }
  }
  let vcs_info = root.join(".cargo_vcs_info.json");
  if vcs_info.exists() {
    println!("cargo:rerun-if-changed={}", vcs_info.display());
  }
  println!("cargo:rerun-if-env-changed=GITHUB_REF_TYPE");
  println!("cargo:rerun-if-env-changed=GITHUB_REF_NAME");
  let source = source_hash(&root);
  let packaged = fs::read_to_string(&vcs_info)
    .ok()
    .map(|text| vcs_info_of(&text));
  let commit = git(&root, &["rev-parse", "--short=10", "HEAD"]);
  // The committed source, unchanged, is the commit alone; anything else adds the hash of
  // the source as it is.
  let clean = commit.is_some()
    && git(
      &root,
      &[
        "status",
        "--porcelain",
        "--",
        "src",
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
      ],
    )
    .is_some_and(|status| status.is_empty());
  let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
  let tag = format!("v{}", version);
  let at_tag = || {
    git(&root, &["tag", "--points-at", "HEAD"]).is_some_and(|tags| tags.lines().any(|t| t == tag))
      || env::var("GITHUB_REF_TYPE").is_ok_and(|t| t == "tag")
        && env::var("GITHUB_REF_NAME").is_ok_and(|name| name == tag)
  };
  // A package's record of its commit comes first: unpacked inside another repository
  // (a vendored copy, `cargo package`'s own build), git would name that one's.
  let (base, release) = match (&packaged, &commit) {
    (Some((sha, false)), _) => (sha.clone(), true),
    (Some((sha, true)), _) => (format!("{}-dirty.{}", sha, &source[..8]), false),
    (None, Some(commit)) if clean => (commit.clone(), at_tag()),
    (None, Some(commit)) => (format!("{}-dirty.{}", commit, &source[..8]), false),
    (None, None) => (format!("src.{}", &source[..10]), false),
  };
  let channel = if release { "rel" } else { "dev" };
  let id = format!("{}-{}", base, channel);
  let commit = packaged.map(|(sha, _)| sha).or(commit);
  let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
  let compiler = Command::new(rustc)
    .arg("-V")
    .output()
    .ok()
    .and_then(|out| String::from_utf8(out.stdout).ok())
    .map(|v| v.trim().to_string())
    .unwrap_or_default();
  let set = |name: &str, value: &str| println!("cargo:rustc-env={}={}", name, value);
  set("MTINDO_BUILD_ID", &id);
  set("MTINDO_BUILD_CHANNEL", channel);
  set("MTINDO_BUILD_COMMIT", commit.as_deref().unwrap_or(""));
  set(
    "MTINDO_BUILD_TARGET",
    &env::var("TARGET").unwrap_or_default(),
  );
  set(
    "MTINDO_BUILD_PROFILE",
    &env::var("PROFILE").unwrap_or_default(),
  );
  set("MTINDO_BUILD_RUSTC", &compiler);
}

/// A hash (FNV-1a, 64 bits, as hex) of every file the build reads: the sources and the
/// manifests, each with its path, in order.
fn source_hash(root: &Path) -> String {
  let mut files = Vec::new();
  collect(&root.join("src"), &mut files);
  for name in ["Cargo.toml", "Cargo.lock", "build.rs"] {
    files.push(root.join(name));
  }
  files.sort();
  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  let mut feed = |bytes: &[u8]| {
    for byte in bytes {
      hash ^= u64::from(*byte);
      hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
  };
  for file in files {
    let relative = file.strip_prefix(root).unwrap_or(&file);
    feed(relative.to_string_lossy().replace('\\', "/").as_bytes());
    feed(&[0]);
    feed(&fs::read(&file).unwrap_or_default());
    feed(&[0]);
  }
  format!("{:016x}", hash)
}

/// The commit a package was made from, in ten hex digits, and whether it was packaged
/// with changes not committed (`"dirty": true`), from its `.cargo_vcs_info.json`.
fn vcs_info_of(text: &str) -> (String, bool) {
  let sha = text
    .split("\"sha1\"")
    .nth(1)
    .and_then(|rest| rest.split('"').nth(1))
    .unwrap_or("")
    .chars()
    .take(10)
    .collect();
  let dirty = text
    .split("\"dirty\"")
    .nth(1)
    .is_some_and(|rest| rest.trim_start_matches([':', ' ']).starts_with("true"));
  (sha, dirty)
}
