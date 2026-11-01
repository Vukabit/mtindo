//! Helpers shared by the tools.

use std::path::{Path, PathBuf};

use mtindo::syntax::Edition;

/// The edition to parse `file` with: its header's, else its crate's.
pub fn edition_for(file: &Path, text: &str) -> Edition {
  header_edition(text).unwrap_or_else(|| {
    Edition::from_year(xtask_core::edition::of_crate(file)).unwrap_or(Edition::E2015)
  })
}

/// The edition a `// rustfmt-edition: YEAR` header in the first lines asks for.
pub fn header_edition(text: &str) -> Option<Edition> {
  text.lines().take(20).find_map(|line| {
    let line = line.trim();
    let year = xtask_core::harness::header(line)?.strip_prefix("edition:")?;
    Edition::from_year(year.trim())
  })
}

/// Every `.rs` file of the repository, tracked or new and not ignored by git, sorted: what
/// the checks over the repository's own code read when given no paths, so that a new
/// folder is covered without being listed anywhere.
pub fn repo_rust_files(root: &Path) -> Result<Vec<PathBuf>, String> {
  Ok(
    xtask_core::files::repo(root, &["*.rs"])?
      .iter()
      .map(|line| root.join(line))
      // Listed but deleted in the working tree.
      .filter(|path| path.is_file())
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_edition_from_a_header_comment() {
    let text = "// rustfmt-edition: 2018\nfn f() {}\n";
    assert_eq!(
      edition_for(Path::new("/nowhere/x.rs"), text),
      Edition::E2018
    );
    assert_eq!(
      edition_for(Path::new("/nowhere/x.rs"), "fn f() {}\n"),
      Edition::E2015
    );
  }

  #[test]
  fn test_repo_rust_files_cover_every_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap();
    let files = repo_rust_files(&root).unwrap();
    for expected in [
      "src/lib.rs",
      "xtask/src/main.rs",
      "xtask/tools/src/main.rs",
      "fuzz/fuzz_targets/format.rs",
    ] {
      assert!(files.contains(&root.join(expected)), "{} missing", expected);
    }
    assert!(
      files
        .iter()
        .all(|f| f.extension().is_some_and(|e| e == "rs"))
    );
    assert!(!files.iter().any(|f| f.starts_with(root.join("target"))));
  }
}
