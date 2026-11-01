//! The answer keys mto is compared with, and where each comes from:
//!
//! - `tests@stable`, `tests@nightly`: the expected files of rustfmt's own test suite at the
//!   commit of the stable or nightly release recorded in
//!   `records/upstream/rustfmt-surface.txt`, from the cached checkout.
//! - `run@stable`, `run@nightly`: the installed rustfmt of that toolchain, run on the input.
//!
//! The suite at stable is the default key for test pairs, unstable settings included; the
//! others are shown beside it.

use std::path::{Path, PathBuf};

use crate::capabilities::{fetch as surface_fetch, load_surface};
use crate::common::log::Logger;

/// The `tests/` folder of rustfmt's suite at the release `channel` ("stable" or
/// "nightly") names in the recorded surface.
pub fn tests_dir(root: &Path, channel: &str, log: &Logger) -> Result<PathBuf, String> {
  let surface = load_surface(root)?;
  let description = surface
    .sources
    .iter()
    .find(|(label, _)| label == channel)
    .map(|(_, d)| d.clone())
    .ok_or(format!(
      "no {} release in records/upstream/rustfmt-surface.txt; run `cargo xtask capabilities \
       refresh` online",
      channel
    ))?;
  let commit = description
    .split([' ', ','])
    .find(|w| w.len() == 40 && w.chars().all(|c| c.is_ascii_hexdigit()))
    .ok_or(format!("the {} source names no commit", channel))?
    .to_string();
  let checkout = surface_fetch::cached_checkout(root, &commit, log)?.ok_or(format!(
    "the standard formatter's tests at {} are not cached; run `cargo xtask capabilities \
     refresh` online",
    channel
  ))?;
  Ok(checkout.join("tests"))
}
