//! Which tree a result belongs to: the commit, and with uncommitted changes, which ones.

use std::fs;
use std::path::Path;

use super::process;

/// The commit part of an identity: `b117865` from `b117865-dirty.0b74e185`.
pub fn commit_of(identity: &str) -> &str {
  identity.split("-dirty.").next().unwrap_or(identity)
}

/// The commit checked out (short hash); with uncommitted changes, `<commit>-dirty.<digest>`,
/// the digest covering the diff against HEAD and the untracked files, so that a result
/// recorded on a dirty tree says exactly which tree it was. `unknown` without git.
pub fn identity(root: &Path) -> String {
  let git = |a: &[&str]| {
    let args: Vec<String> = a.iter().map(|s| s.to_string()).collect();
    process::capture(root, "git", &args, &[], process::QUICK)
  };
  let (code, hash) = git(&["rev-parse", "--short", "HEAD"]);
  if code != 0 {
    return "unknown".to_string();
  }
  let (_, status) = git(&["status", "--porcelain"]);
  if status.trim().is_empty() {
    return hash.trim().to_string();
  }
  let (_, diff) = git(&["diff", "HEAD", "--binary"]);
  let (_, untracked) = git(&["ls-files", "--others", "--exclude-standard"]);
  let mut changes = diff.into_bytes();
  for file in untracked.lines() {
    changes.extend_from_slice(file.as_bytes());
    changes.extend(fs::read(root.join(file)).unwrap_or_default());
  }
  let digest = crate::sha256::hex_digest(&changes);
  format!("{}-dirty.{}", hash.trim(), &digest[..8])
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_commit_of() {
    assert_eq!(commit_of("b117865-dirty.0b74e185"), "b117865");
    assert_eq!(commit_of("b117865"), "b117865");
  }

  #[test]
  fn test_identity_names_this_checkout() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let id = identity(root);
    assert!(id.len() >= 7, "{}", id);
    let commit = commit_of(&id);
    assert!(commit.chars().all(|c| c.is_ascii_hexdigit()), "{}", id);
    if let Some(digest) = id.strip_prefix(&format!("{}-dirty.", commit)) {
      assert_eq!(digest.len(), 8);
    }
  }
}
