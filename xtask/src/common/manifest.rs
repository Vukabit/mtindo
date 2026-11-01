//! Run manifests: what produced a run, so that when a number moves between two runs the
//! difference names its cause (the formatter changed, the toolchain did, a pin or a
//! record did, another machine ran it).
//!
//! ```text
//! .dev/runs/<id>/manifest.txt   KEY = VALUE, one per line, sorted
//! ```
//!
//! Keys: `tree`, `rustc`, `rustfmt`, `machine`, `inputs.<check>` (a proof's input hash)
//! and `record.<path>` (a hash of each file under records/ but the READMEs and history).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::common::{process, proof, tree};

pub const FILE: &str = "manifest.txt";

/// What differs between two manifests: (key, before, after), a missing side as `-`.
pub fn differences(
  before: &BTreeMap<String, String>,
  after: &BTreeMap<String, String>,
) -> Vec<(String, String, String)> {
  let mut keys: Vec<&String> = before.keys().chain(after.keys()).collect();
  keys.sort();
  keys.dedup();
  keys
    .into_iter()
    .filter_map(|k| {
      let (a, b) = (before.get(k), after.get(k));
      (a != b).then(|| {
        (
          k.clone(),
          a.cloned().unwrap_or_else(|| "-".to_string()),
          b.cloned().unwrap_or_else(|| "-".to_string()),
        )
      })
    })
    .collect()
}

/// A run's manifest, empty when it has none.
pub fn read(dir: &Path) -> BTreeMap<String, String> {
  fs::read_to_string(dir.join(FILE))
    .unwrap_or_default()
    .lines()
    .filter_map(|l| l.split_once(" = "))
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// What produced a run, as the tree and tools are now.
pub fn take(root: &Path) -> BTreeMap<String, String> {
  let mut out = BTreeMap::new();
  out.insert("tree".to_string(), tree::identity(root));
  for tool in ["rustc", "rustfmt"] {
    let (_, version) =
      process::capture(root, tool, &["--version".to_string()], &[], process::QUICK);
    out.insert(tool.to_string(), version.trim().to_string());
  }
  out.insert("machine".to_string(), crate::metrics::machine());
  if let Some(hash) = proof::inputs(root, &proof::COMPAT) {
    out.insert(format!("inputs.{}", proof::COMPAT.name), hash);
  }
  let mut stack = vec![root.join("records")];
  while let Some(dir) = stack.pop() {
    for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
      let path = entry.path();
      if path.is_dir() {
        stack.push(path);
        continue;
      }
      let rel = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
      if rel.ends_with("README.md") || rel == crate::papers::history::FILE {
        continue;
      }
      if let Ok(bytes) = fs::read(&path) {
        out.insert(
          format!("record.{}", rel),
          crate::sha256::hex_digest(&bytes)[..16].to_string(),
        );
      }
    }
  }
  out
}

/// Writes the manifest into a run's folder.
pub fn write(root: &Path, dir: &Path) {
  let text: String = take(root)
    .into_iter()
    .map(|(k, v)| format!("{} = {}\n", k, v))
    .collect();
  let _ = fs::write(dir.join(FILE), text);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_differences_name_what_changed() {
    let a: BTreeMap<String, String> = [("tree", "a"), ("rustfmt", "1.9"), ("record.x", "1")]
      .iter()
      .map(|(k, v)| (k.to_string(), v.to_string()))
      .collect();
    let mut b = a.clone();
    b.insert("rustfmt".to_string(), "1.10".to_string());
    b.remove("record.x");
    assert_eq!(
      differences(&a, &b),
      vec![
        ("record.x".to_string(), "1".to_string(), "-".to_string()),
        ("rustfmt".to_string(), "1.9".to_string(), "1.10".to_string()),
      ]
    );
  }

  #[test]
  fn test_this_repository_gives_a_full_manifest() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let m = take(root);
    for key in [
      "tree",
      "rustc",
      "rustfmt",
      "machine",
      "inputs.compat",
      "record.records/pins/corpus.txt",
    ] {
      assert!(m.contains_key(key), "{} missing", key);
    }
    assert!(
      !m.keys()
        .any(|k| k.ends_with("README.md") || k.ends_with("history.txt"))
    );
  }
}
