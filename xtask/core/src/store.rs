//! The store: `.dev/` at the repository root, ignored by git, where the commands keep what
//! they measure, find and fetch. Each area is named here with what it holds and what of it
//! is kept, and paths into it come from [`dir`], so the layout has one owner.
//!
//! ```text
//! .dev/
//!     runs/       a folder per command run: its record, samples, output, manifest
//!     proofs/     the checks that passed, by a hash of what they read
//!     incidents/  findings from fuzzing and comparisons, with their life cycle
//!     metrics/    every measured number over time
//!     logs/       each command's log, and the last test run's results
//!     cache/      what is fetched or built again when missing (corpus, releases)
//!     fuzz/       each fuzz target's corpus, by target
//!     upstream/   cases where the reference formatter is wrong, kept for reports
//! ```

use std::fs;
use std::path::{Path, PathBuf};

pub const AREAS: &[Area] = &[
  Area {
    name: RUNS,
    holds: "a folder per command run: record, samples, output, manifest",
    kept: "the newest 50 of 30 days, those running and those a finding names",
  },
  Area {
    name: PROOFS,
    holds: "the checks that passed, by a hash of what they read",
    kept: "every pass, a line each",
  },
  Area {
    name: INCIDENTS,
    holds: "findings from fuzzing and comparisons, with their life cycle",
    kept: "until closed by `cargo xtask incidents`",
  },
  Area {
    name: METRICS,
    holds: "every measured number over time",
    kept: "all of it",
  },
  Area {
    name: LOGS,
    holds: "each command's log, and the last test run's results",
    kept: "each log up to 5 MiB, with three older files",
  },
  Area {
    name: CACHE,
    holds: "what is fetched or built again when missing",
    kept: "the pinned versions; removing any of it costs a download",
  },
  Area {
    name: FUZZ,
    holds: "each fuzz target's corpus: the inputs that reach new code, grown by every run",
    kept: "all of it: what the fuzzers have learned; removing it starts them over",
  },
  Area {
    name: UPSTREAM,
    holds: "cases where the reference formatter is wrong, kept for reports",
    kept: "all of it",
  },
];

pub const CACHE: &str = "cache";

pub const FUZZ: &str = "fuzz";

pub const INCIDENTS: &str = "incidents";

pub const LOGS: &str = "logs";

pub const METRICS: &str = "metrics";

pub const PROOFS: &str = "proofs";

pub const RUNS: &str = "runs";

pub const UPSTREAM: &str = "upstream";

/// One area of the store.
pub struct Area {
  pub name: &'static str,
  pub holds: &'static str,
  pub kept: &'static str,
}

/// The folder of the area `name`, one of [`AREAS`].
pub fn dir(repo: &Path, name: &str) -> PathBuf {
  debug_assert!(
    AREAS.iter().any(|a| a.name == name),
    "{} is not an area of the store",
    name
  );
  root(repo).join(name)
}

/// The store's root.
pub fn root(repo: &Path) -> PathBuf {
  repo.join(".dev")
}

/// `bytes` for reading: KB, MB or GB.
pub fn shown_bytes(bytes: u64) -> String {
  let b = bytes as f64;
  if b >= 1e9 {
    format!("{:.1} GB", b / 1e9)
  } else if b >= 1e6 {
    format!("{:.1} MB", b / 1e6)
  } else {
    format!("{:.0} KB", b / 1e3)
  }
}

/// The bytes and files under `path`.
pub fn size(path: &Path) -> (u64, usize) {
  let (mut bytes, mut files) = (0, 0);
  let mut stack = vec![path.to_path_buf()];
  while let Some(here) = stack.pop() {
    if here.is_file() {
      bytes += fs::metadata(&here).map_or(0, |m| m.len());
      files += 1;
      continue;
    }
    for entry in fs::read_dir(&here).into_iter().flatten().flatten() {
      match entry.metadata() {
        Ok(m) if m.is_dir() => stack.push(entry.path()),
        Ok(m) => {
          bytes += m.len();
          files += 1;
        }
        Err(_) => {}
      }
    }
  }
  (bytes, files)
}

/// What is in the store but in no area: left by commands that are gone.
pub fn unknown(repo: &Path) -> Vec<PathBuf> {
  let mut out: Vec<PathBuf> = fs::read_dir(root(repo))
    .into_iter()
    .flatten()
    .flatten()
    .filter(|e| {
      let name = e.file_name();
      !AREAS.iter().any(|a| name == a.name)
    })
    .map(|e| e.path())
    .collect();
  out.sort();
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Every `.dev/<area>` the xtask crates name is an area here, and paths into the store
  /// come from [`dir`], so the layout stays written down in one place.
  #[test]
  fn test_the_crates_name_only_known_areas() {
    let xtask = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut stack = vec![
      xtask.join("src"),
      xtask.join("core/src"),
      xtask.join("tools/src"),
    ];
    let mut found = Vec::new();
    while let Some(path) = stack.pop() {
      if path.is_dir() {
        stack.extend(fs::read_dir(&path).unwrap().flatten().map(|e| e.path()));
        continue;
      }
      if path.extension().is_none_or(|e| e != "rs") {
        continue;
      }
      let text = fs::read_to_string(&path).unwrap();
      let here = path.ends_with("core/src/store.rs");
      for (n, line) in text.lines().enumerate() {
        if !here && line.contains("join(\".dev") {
          found.push(format!("{}:{}: use store::dir", path.display(), n + 1));
        }
        for (at, _) in line.match_indices(".dev/") {
          let name: String = line[at + 5..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
          if !name.is_empty() && !AREAS.iter().any(|a| a.name == name) {
            found.push(format!("{}:{}: .dev/{}", path.display(), n + 1, name));
          }
        }
      }
    }
    assert!(
      found.is_empty(),
      "name the area in xtask/core/src/store.rs:\n{}",
      found.join("\n")
    );
  }

  #[test]
  fn test_what_is_in_no_area_is_unknown() {
    let repo = std::env::temp_dir().join(format!("mtindo-store-{}", std::process::id()));
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(dir(&repo, RUNS).join("a")).unwrap();
    fs::write(dir(&repo, RUNS).join("a/run.toml"), "12345").unwrap();
    fs::create_dir_all(root(&repo).join("hub")).unwrap();
    assert_eq!(unknown(&repo), vec![root(&repo).join("hub")]);
    assert_eq!(size(&dir(&repo, RUNS)), (5, 1));
    assert_eq!(shown_bytes(1_200_000_000), "1.2 GB");
    let _ = fs::remove_dir_all(&repo);
  }
}
