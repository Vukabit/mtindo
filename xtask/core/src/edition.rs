//! Which edition a file is written in.

use std::fs;
use std::path::Path;

/// The editions, oldest first.
pub const YEARS: &[&str] = &["2015", "2018", "2021", "2024"];

/// The edition of the crate `file` belongs to: `edition = "YEAR"` in the nearest
/// `Cargo.toml` that sets one (workspace ones included), else 2015, rustfmt's default.
pub fn of_crate(file: &Path) -> &'static str {
  let mut dir = file.parent();
  while let Some(current) = dir {
    if let Ok(manifest) = fs::read_to_string(current.join("Cargo.toml")) {
      for line in manifest.lines() {
        if let Some(value) = line.trim().strip_prefix("edition") {
          let year = value
            .trim()
            .trim_start_matches('=')
            .trim()
            .trim_matches('"');
          if let Some(known) = YEARS.iter().find(|y| **y == year) {
            return known;
          }
        }
      }
      if manifest.contains("[package]") && !manifest.contains("edition.workspace") {
        return "2015";
      }
    }
    dir = current.parent();
  }
  "2015"
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_file_takes_its_crates_edition() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(of_crate(&root.join("src/lib.rs")), "2024");
    assert_eq!(of_crate(Path::new("/nowhere/x.rs")), "2015");
  }
}
