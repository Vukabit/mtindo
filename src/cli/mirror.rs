//! `mto config mirror`: the formatting settings an `mto.toml` holds, written into a
//! `rustfmt.toml` beside it, for a project moving to mto while the standard formatter (an
//! editor's, `cargo fmt` in a script) still formats it: both then agree. Only the settings
//! the standard formatter's stable release takes are written; the rest are named. mto
//! writes the file it owns again, and never one it did not write. It writes the file both
//! tools read there (`.rustfmt.toml` when that is the one), and carries the settings of a
//! `rustfmt.toml` above that the new one would hide, so mirroring changes no one's output.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{Config, FileKind, Source};
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
  /// Lines carried from the standard formatter's file above, which the new one would hide.
  pub carried: Vec<String>,
  /// Setting lines the file held that writing it again removes or changes.
  pub replaced: Vec<String>,
}

/// The `rustfmt.toml` for `dir` from `config`'s formatting settings set in an mto.toml;
/// written unless `dry_run`. An error, with its exit code, for a file mto did not write.
pub fn mirror(config: &Config, dir: &Path, dry_run: bool) -> Result<Mirrored, (String, i32)> {
  let real = |p: &Path| crate::real_path(p).unwrap_or_else(|_| p.to_path_buf());
  // The file both tools read in `dir`: `.rustfmt.toml` before `rustfmt.toml`, as they look.
  let path = match dir.join(".rustfmt.toml") {
    dot if dot.is_file() => dot,
    _ => dir.join("rustfmt.toml"),
  };
  // The standard formatter's file the settings come from now, in a folder above: the new
  // one hides it from both tools, so what it sets is carried.
  let hidden = config
    .files()
    .iter()
    .find(|(kind, file)| *kind == FileKind::Standard && real(file) != real(&path))
    .map(|(_, file)| real(file));
  let mut written = Vec::new();
  let mut left_out = Vec::new();
  let mut carried = Vec::new();
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
    let from_hidden = hidden.as_ref().is_some_and(|h| real(path) == *h);
    if setting.file != File::Format || !(in_mto || from_hidden) {
      continue;
    }
    if from_hidden {
      carried.push(format!("{} = {}", setting.name, value.to_toml()));
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
  let existing = fs::read_to_string(&path).ok();
  if let Some(text) = &existing
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
    // Both resolved, as one folder can be named two ways (a Windows short name).
    let real = |p: &Path| crate::real_path(p).unwrap_or_else(|_| p.to_path_buf());
    let here = real(dir);
    let from: Vec<String> = from.iter().map(|p| relative(&real(p), &here)).collect();
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
  // What the file held that the new text does not: named, so a line added by hand is not
  // lost unseen (formatting settings belong in mto.toml).
  let replaced = existing
    .map(|text| {
      text
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter(|line| !written.iter().any(|w| w == line.trim()))
        .map(|line| line.trim().to_string())
        .collect()
    })
    .unwrap_or_default();
  Ok(Mirrored {
    path,
    written,
    left_out,
    carried,
    replaced,
  })
}

/// `path` as named from the folder `from` (`mto.toml`, `../rustfmt.toml`): the file is
/// committed and read on other machines, where an absolute path means nothing.
fn relative(path: &Path, from: &Path) -> String {
  let shared = from
    .ancestors()
    .find(|a| path.starts_with(a))
    .unwrap_or(Path::new(""));
  let up = from
    .strip_prefix(shared)
    .map_or(0, |rest| rest.components().count());
  let down = path.strip_prefix(shared).unwrap_or(path);
  let mut out = PathBuf::new();
  for _ in 0..up {
    out.push("..");
  }
  out.push(down);
  out.display().to_string().replace('\\', "/")
}
