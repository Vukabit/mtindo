//! `mto config update` and `mto config reset`: a folder's mto.toml brought up to every
//! setting mto has, or back to what `mto init` writes.
//!
//! Update adds each of mto's settings the file does not set, at its default (what applies
//! while it is not set, so nothing a run does changes), with the line `mto init` writes
//! above it, where `mto init` puts it among the settings already there. Nothing the file
//! holds is changed or moved: values, comments, blank lines, the standard formatter's
//! settings and any table stay as they are. Reset writes the file `mto init` writes.

use std::path::{Path, PathBuf};

use crate::options::Setting;

/// What an update or a reset wrote, or would write.
pub struct Rewritten {
  pub path: PathBuf,
  /// The file as it was, and as it is to be.
  pub before: String,
  pub after: String,
  /// The settings added (update), or whose value changes (reset), as `name = value`.
  pub lines: Vec<String>,
  /// What a reset takes out, as `mto init` does not write it: other settings, as
  /// `name = value`, and tables, as `[name]`.
  pub removed: Vec<String>,
}

/// The mto.toml in `dir`: `mto.toml`, else `.mto.toml`.
pub fn file_in(dir: &Path) -> Option<PathBuf> {
  ["mto.toml", ".mto.toml"]
    .iter()
    .map(|name| dir.join(name))
    .find(|path| path.is_file())
}

/// The reset of the mto.toml in `dir`; an error, with its exit code, when there is none or
/// it cannot be read.
pub fn reset(dir: &Path) -> Result<Rewritten, (String, i32)> {
  let (path, before) = read(dir)?;
  let after = crate::init_template();
  let set = top_level(&before);
  let lines = crate::template_settings()
    .into_iter()
    .filter_map(|setting| {
      let value = crate::init_value(setting);
      let was = set
        .iter()
        .find(|(name, _)| name == setting.name)
        .map(|(_, v)| v.as_str());
      (was != Some(value)).then(|| format!("{} = {}", setting.name, value))
    })
    .collect();
  let written = top_level(&after);
  let mut removed: Vec<String> = set
    .iter()
    .filter(|(name, _)| !written.iter().any(|(kept, _)| kept == name))
    .map(|(name, value)| format!("{} = {}", name, value))
    .collect();
  removed.extend(tables(&before));
  Ok(Rewritten {
    path,
    before,
    after,
    lines,
    removed,
  })
}

/// The keys `text` sets at its top level (before any `[table]`), with their values as
/// written.
pub fn top_level(text: &str) -> Vec<(String, String)> {
  let mut set = Vec::new();
  let lines: Vec<&str> = text.lines().collect();
  let mut at = 0;
  while at < lines.len() {
    let trimmed = lines[at].trim_start();
    if trimmed.starts_with('[') {
      break;
    }
    let end = value_end(&lines, at);
    if !trimmed.starts_with('#')
      && let Some((key, _)) = trimmed.split_once('=')
    {
      let key = key.trim().trim_matches('"');
      if !key.is_empty()
        && key
          .chars()
          .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
      {
        let whole = lines[at..=end].join("\n");
        let value = whole.split_once('=').map_or("", |(_, v)| v);
        set.push((key.to_string(), crate::compact_value(value)));
      }
    }
    at = end + 1;
  }
  set
}

/// The update of the mto.toml in `dir`; an error, with its exit code, when there is none
/// or it cannot be read.
pub fn update(dir: &Path) -> Result<Rewritten, (String, i32)> {
  let (path, before) = read(dir)?;
  let (after, added) = updated(&before);
  Ok(Rewritten {
    path,
    before,
    after,
    lines: added,
    removed: Vec::new(),
  })
}

/// `text` with every mto setting it does not set added at its default, and the lines added.
pub fn updated(text: &str) -> (String, Vec<String>) {
  let settings = crate::template_settings();
  let set = top_level(text);
  let is_set = |setting: &Setting| set.iter().any(|(name, _)| name == setting.name);
  let missing: Vec<&Setting> = settings.iter().copied().filter(|s| !is_set(s)).collect();
  if missing.is_empty() {
    return (text.to_string(), Vec::new());
  }
  let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
  // The top level ends at the first table: settings added go before it.
  let mut end = lines
    .iter()
    .position(|l| l.trim_start().starts_with('['))
    .unwrap_or(lines.len());
  // Where setting `name` starts and ends (an array may take several lines).
  let line_of = |lines: &[String], end: usize, name: &str| {
    let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
    lines[..end]
      .iter()
      .position(|l| {
        let l = l.trim_start();
        !l.starts_with('#')
          && l
            .split_once('=')
            .is_some_and(|(key, _)| key.trim().trim_matches('"') == name)
      })
      .map(|start| (start, value_end(&borrowed, start)))
  };
  let mut added = Vec::new();
  for (at, setting) in settings.iter().enumerate() {
    if is_set(setting) {
      continue;
    }
    let block: Vec<String> = [String::new(), format!("# {}", setting.summary)]
      .into_iter()
      .chain(
        crate::setting_line(setting.name, setting.default)
          .lines()
          .map(str::to_string),
      )
      .collect();
    // After the nearest setting before it in `mto init`'s order that the file holds (or one
    // added just now), else before the nearest after it, above that one's comment lines.
    let after = settings[..at]
      .iter()
      .rev()
      .find_map(|s| line_of(&lines, end, s.name));
    let insert = match after {
      Some((_, last)) => last + 1,
      None => match settings[at + 1..]
        .iter()
        .find_map(|s| line_of(&lines, end, s.name))
      {
        Some((mut line, _)) => {
          while line > 0 && lines[line - 1].trim_start().starts_with('#') {
            line -= 1;
          }
          // The block's blank line goes after it, before the setting it comes above.
          lines.splice(
            line..line,
            block[1..].iter().cloned().chain([String::new()]),
          );
          end += block.len();
          added.push(format!("{} = {}", setting.name, setting.default));
          continue;
        }
        None => end,
      },
    };
    lines.splice(insert..insert, block.iter().cloned());
    end += block.len();
    added.push(format!("{} = {}", setting.name, setting.default));
  }
  let mut out = lines.join("\n");
  out.push('\n');
  (out, added)
}

/// The last line of the setting starting on line `at`: itself, or the line closing an array
/// it opens across lines (`key = [` to `]`).
pub fn value_end(lines: &[&str], at: usize) -> usize {
  let opens = |line: &str| {
    line
      .split_once('=')
      .map(|(_, v)| v.trim())
      .is_some_and(|v| v.starts_with('[') && v.matches('[').count() > v.matches(']').count())
  };
  if lines[at].trim_start().starts_with('#') || !opens(lines[at]) {
    return at;
  }
  let mut depth: i32 = 0;
  for (i, line) in lines.iter().enumerate().skip(at) {
    let code = line.split('#').next().unwrap_or("");
    depth += code.matches('[').count() as i32 - code.matches(']').count() as i32;
    if depth <= 0 {
      return i;
    }
  }
  lines.len() - 1
}

fn read(dir: &Path) -> Result<(PathBuf, String), (String, i32)> {
  let Some(path) = file_in(dir) else {
    return Err((
      format!("{} has no mto.toml; `mto init` writes one", dir.display()),
      2,
    ));
  };
  let text = std::fs::read_to_string(&path)
    .map_err(|e| (format!("cannot read {}: {}", path.display(), e), 3))?;
  Ok((path, text))
}

/// The tables `text` holds, as their headers are written (`[doctor]`).
fn tables(text: &str) -> Vec<String> {
  let lines: Vec<&str> = text.lines().collect();
  let mut found = Vec::new();
  let mut at = 0;
  while at < lines.len() {
    let trimmed = lines[at].trim();
    if trimmed.starts_with('[') {
      found.push(trimmed.split('#').next().unwrap_or("").trim().to_string());
      at += 1;
    } else {
      at = value_end(&lines, at) + 1;
    }
  }
  found
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_an_update_adds_each_missing_setting_at_its_default_and_changes_nothing_else() {
    let text = "# mine\nlayout = true\n\n# Sort items of the same kind by name\nsort_alphabetically = false\n\n[override]\nx = 1\n";
    let (after, added) = updated(text);
    // What was there stays, in order, the table last.
    let kept: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    let mut rest = after.lines().filter(|l| !l.is_empty());
    for line in kept {
      assert!(rest.any(|l| l == line), "{} lost:\n{}", line, after);
    }
    assert!(after.trim_end().ends_with("[override]\nx = 1"), "{}", after);
    // Every setting is named once, the ones set as they were, the rest at their default.
    for setting in crate::template_settings() {
      let lines: Vec<&str> = after
        .lines()
        .filter(|l| l.starts_with(&format!("{} = ", setting.name)))
        .collect();
      assert_eq!(lines.len(), 1, "{}:\n{}", setting.name, after);
    }
    assert!(after.contains("\nsort_alphabetically = false\n"));
    assert!(after.contains("\nkeep_paragraphs = false\n"), "{}", after);
    assert!(!added.iter().any(|l| l.starts_with("layout =")));
    // A second update adds nothing.
    assert_eq!(updated(&after), (after.clone(), Vec::new()));
  }

  #[test]
  fn test_an_update_of_a_file_naming_every_setting_changes_nothing() {
    let template = crate::init_template();
    assert_eq!(updated(&template), (template.clone(), Vec::new()));
  }
}
