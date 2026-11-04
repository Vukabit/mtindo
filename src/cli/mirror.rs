//! `mto config mirror`: the formatting settings an `mto.toml` holds, written into a
//! `rustfmt.toml` beside it, for a project moving to mto while the standard formatter (an
//! editor's, `cargo fmt` in a script) still formats it: both then agree. Only the settings
//! the standard formatter's stable release takes are written; the rest are named. mto
//! writes the file it owns again, and never one it did not write.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{Config, Source};
use crate::options::{File, Stability};

/// The first line of a `rustfmt.toml` mto wrote.
const HEADER: &str = "# Written by `mto config mirror` from";

/// What a mirror wrote, or would write.
pub struct Mirrored {
  pub path: PathBuf,
  /// `key = value` lines written.
  pub written: Vec<String>,
  /// Settings left out: the standard formatter's stable release does not take them.
  pub left_out: Vec<String>,
}

/// The `rustfmt.toml` for `dir` from `config`'s formatting settings set in an mto.toml;
/// written unless `dry_run`. An error, with its exit code, for a file mto did not write.
pub fn mirror(config: &Config, dir: &Path, dry_run: bool) -> Result<Mirrored, (String, i32)> {
  let mut written = Vec::new();
  let mut left_out = Vec::new();
  let mut from = Vec::new();
  let mut values: Vec<_> = config.values().collect();
  values.sort_by_key(|(setting, _, _)| setting.name);
  for (setting, value, source) in values {
    let Source::File { path, .. } = source else {
      continue;
    };
    let in_mto = path
      .file_name()
      .is_some_and(|name| name == "mto.toml" || name == ".mto.toml");
    if setting.file != File::Format || !in_mto {
      continue;
    }
    let stable = setting
      .channels
      .is_some_and(|c| c.stable == Stability::Stable);
    let line = format!("{} = {}", setting.name, value.to_toml());
    if stable {
      written.push(line);
    } else {
      left_out.push(line);
    }
    if !from.contains(path) {
      from.push(path.clone());
    }
  }
  let path = dir.join("rustfmt.toml");
  if let Ok(text) = fs::read_to_string(&path)
    && !text.starts_with(HEADER)
  {
    return Err((
      format!(
        "{} exists and mto did not write it; remove it first, or keep the formatting \
         settings in it rather than in mto.toml",
        path.display()
      ),
      3,
    ));
  }
  if !dry_run {
    // Named from the folder, as the file is committed and read elsewhere.
    let here = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let from: Vec<String> = from
      .iter()
      .map(|p| p.strip_prefix(&here).unwrap_or(p).display().to_string())
      .collect();
    let mut text = format!(
      "{} {}; run it again after changing them there.\n",
      HEADER,
      if from.is_empty() {
        "no mto.toml formatting settings".to_string()
      } else {
        from.join(", ")
      }
    );
    for line in &written {
      text.push_str(line);
      text.push('\n');
    }
    crate::write_atomic(&path, &text)
      .map_err(|e| (format!("cannot write {}: {}", path.display(), e), 3))?;
  }
  Ok(Mirrored {
    path,
    written,
    left_out,
  })
}
