//! Gathers this build's identity at compile time, for `mto --version`, bug reports and the
//! doctor's holds (which lapse when the build changes): the commit, and when the source
//! differs from it a hash of the source; the target, the profile and the compiler.
//!
//! git runs here, while compiling, if this is a checkout; the mto it builds never runs it.
//! A build from a package (crates.io) has no git: a hash of its source identifies it.

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
  let git_dir = git(&root, &["rev-parse", "--git-dir"]).map(|d| root.join(d));
  if let Some(git_dir) = &git_dir {
    // The commit moves with HEAD, and a commit or a staged change touches the index.
    for watched in ["HEAD", "index"] {
      println!("cargo:rerun-if-changed={}", git_dir.join(watched).display());
    }
  }
  let source = source_hash(&root);
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
  let id = match &commit {
    Some(commit) if clean => commit.clone(),
    Some(commit) => format!("{}-dirty.{}", commit, &source[..8]),
    None => format!("src.{}", &source[..10]),
  };
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
