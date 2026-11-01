//! Proofs: that a check passed, kept by a hash of everything it read, so that a change it
//! did not read (a doc, the changelog) leaves the proof standing and nothing is run again
//! to show the same thing.
//!
//! ```text
//! .dev/proofs/<check>.txt   one line per pass: INPUTS-HASH DATE RUN
//! ```
//!
//! The hash covers the content of the files the check reads, tracked or not (so it holds
//! for uncommitted work too), and the versions of the tools it runs.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::common::log::timestamp;
use crate::common::process;

/// What `cargo xtask diff` reads: the formatter, the harness that scores it, and the
/// records it compares with (not the generated READMEs or the history).
pub const COMPAT: Check = Check {
  name: "compat",
  paths: &[
    "src",
    "build.rs",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "xtask/core",
    "xtask/tools",
    "xtask/src/diff.rs",
    "xtask/src/corpus.rs",
    "records/pins",
    "records/policy",
    "records/upstream",
    "records/results/compatibility.txt",
  ],
  tools: &[&["rustc", "--version"], &["rustfmt", "--version"]],
};

/// A check a proof can stand for: the files it reads and the tools whose versions matter.
pub struct Check {
  pub name: &'static str,
  pub paths: &'static [&'static str],
  pub tools: &'static [&'static [&'static str]],
}

/// The run that proved `check` for `hash`, when one did.
pub fn holds(root: &Path, check: &Check, hash: &str) -> Option<String> {
  let text = fs::read_to_string(file(root, check)).ok()?;
  text.lines().rev().find_map(|line| {
    let mut parts = line.split(' ');
    (parts.next()? == hash).then(|| line.to_string())
  })
}

/// The hash of what `check` reads, as the tree and tools are now; `None` when git cannot
/// list the files.
pub fn inputs(root: &Path, check: &Check) -> Option<String> {
  let files = crate::common::files::repo(root, check.paths).ok()?;
  let mut data: Vec<u8> = Vec::new();
  for file in &files {
    data.extend_from_slice(file.as_bytes());
    data.push(0);
    match fs::read(root.join(file)) {
      Ok(bytes) => data.extend_from_slice(&bytes),
      // Listed but gone (deleted, not yet committed).
      Err(_) => data.extend_from_slice(b"\0missing"),
    }
    data.push(0);
  }
  for tool in check.tools {
    let args: Vec<String> = tool[1..].iter().map(|s| s.to_string()).collect();
    let (_, version) = process::capture(root, tool[0], &args, &[], process::QUICK);
    data.extend_from_slice(version.trim().as_bytes());
    data.push(0);
  }
  Some(crate::sha256::hex_digest(&data))
}

/// Notes that `check` passed for `hash`, in run `run` (its folder's name, or `-`).
pub fn record(root: &Path, check: &Check, hash: &str, run: &str) {
  let path = file(root, check);
  if let Some(dir) = path.parent() {
    let _ = fs::create_dir_all(dir);
  }
  let mut text = fs::read_to_string(&path).unwrap_or_default();
  text.push_str(&format!(
    "{} {} {}\n",
    hash,
    &timestamp(SystemTime::now())[..10],
    run
  ));
  let _ = fs::write(&path, text);
}

fn file(root: &Path, check: &Check) -> std::path::PathBuf {
  root
    .join(".dev")
    .join("proofs")
    .join(format!("{}.txt", check.name))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_proof_holds_until_what_the_check_reads_changes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let check = Check {
      name: "proof-test",
      paths: &["records/pins"],
      tools: &[],
    };
    let a = inputs(root, &check).unwrap();
    assert_eq!(inputs(root, &check).unwrap(), a);
    let other = Check {
      paths: &["records/policy"],
      ..check
    };
    assert_ne!(inputs(root, &other).unwrap(), a);
    let scratch = std::env::temp_dir().join(format!("mtindo-proof-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    record(&scratch, &check, &a, "run-1");
    assert!(holds(&scratch, &check, &a).is_some_and(|l| l.ends_with("run-1")));
    assert!(holds(&scratch, &check, "other").is_none());
    let _ = fs::remove_dir_all(&scratch);
  }
}
