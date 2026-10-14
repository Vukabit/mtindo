//! Layer 3: bug reports. When mto finds a bug in itself (the verifier stops a write, or
//! laying a file out panics), what it knows is kept as a report, in a folder beside history,
//! for `mto report` to show, save, send or prune.
//!
//! A report holds the smallest part of the file that still fails, never the whole file
//! (that is added only on request, `mto report save --with-file`), with what failed, a
//! backtrace for a panic, the command, the settings that differ from the defaults, and the
//! build (`version::details`). One report per distinct bug: the same failure on the same
//! smallest input under the same build is counted, not kept again.
//!
//! ```text
//! <state>/mtindo/reports/<id>/
//!     report.md   the report, as an issue's body
//!     meta        count, first and last seen (ms), and each file it was seen in
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::history::{now_millis, sha256};
use crate::verify::Violation;
use crate::version;

/// Reports not seen again for this long go when another is saved.
pub const KEEP_DAYS: u64 = 90;

/// The most files a report remembers it was seen in.
const SEEN_IN: usize = 10;

/// What mto found wrong with itself.
#[derive(Clone, Debug)]
pub enum Failure {
  /// Laying the file out panicked: what it said and where.
  Panicked { said: String },
  /// The verifier stopped the write: the rules the new text broke.
  Verifier { violations: Vec<Violation> },
}

/// One bug, as found on one file.
#[derive(Clone, Debug)]
pub struct Bug {
  pub failure: Failure,
  /// The file, as the run names it.
  pub file: String,
  /// The file, absolute; `None` for stdin.
  pub path: Option<PathBuf>,
  /// The smallest part of the file that still fails, when one was found.
  pub minimal: Option<String>,
  /// Where the panic came from, for a panic.
  pub backtrace: Option<String>,
  /// The hash (SHA-256) of the whole file as it was read, so that a file added later is
  /// known to be the same one.
  pub source_hash: String,
  /// The settings that differ from the defaults, one per line.
  pub settings: String,
  /// The command line.
  pub command: String,
}

impl Bug {
  /// The report as the terminal shows it when reports are not kept: what failed, the
  /// smallest part of the file that still fails, and to report it.
  pub fn text(&self) -> String {
    match &self.failure {
      Failure::Panicked { said } => {
        crate::verify::report::panic_report(&self.file, said, self.minimal.as_deref())
      }
      Failure::Verifier { violations } => {
        crate::verify::report::bug_report(&self.file, violations, self.minimal.as_deref())
      }
    }
  }

  /// What makes two bugs one: the build, the failure (a panic's message and place; the
  /// rules broken), and the smallest input.
  pub fn key(&self) -> String {
    let failure = match &self.failure {
      Failure::Panicked { said } => format!("panic {}", said),
      Failure::Verifier { violations } => {
        let mut rules: Vec<&str> = violations.iter().map(|v| v.rule.name()).collect();
        rules.sort_unstable();
        rules.dedup();
        format!("verifier {}", rules.join(" "))
      }
    };
    let minimal = self.minimal.as_deref().unwrap_or("");
    sha256::hex(format!("{}\n{}\n{}", version::FULL, failure, minimal).as_bytes())
  }

  /// A line naming the bug, for a list and an issue's title.
  pub fn title(&self) -> String {
    let what = match &self.failure {
      Failure::Panicked { said } => format!("panic: {}", said),
      Failure::Verifier { violations } => {
        let mut rules: Vec<&str> = violations.iter().map(|v| v.rule.name()).collect();
        rules.dedup();
        format!("verifier: {}", rules.join(", "))
      }
    };
    let mut title: String = what.chars().take(100).collect();
    if title.len() < what.len() {
      title.push('…');
    }
    title
  }

  /// The report as Markdown, ready to be an issue's body.
  pub fn markdown(&self) -> String {
    let mut out = format!("## {}\n\n", self.title());
    match &self.failure {
      Failure::Panicked { said } => {
        out.push_str(&format!(
          "mto failed (panicked) while laying out a file:\n\n```text\n{}\n```\n\n",
          said
        ));
      }
      Failure::Verifier { violations } => {
        out.push_str("The verifier stopped the write; the new text would have broken:\n\n");
        for violation in violations {
          out.push_str(&format!(
            "- `{}`: {}\n",
            violation.rule.name(),
            violation.detail
          ));
        }
        out.push('\n');
      }
    }
    match &self.minimal {
      Some(minimal) => out.push_str(&format!(
        "The smallest part of the file that still fails:\n\n```rust\n{}\n```\n\n",
        minimal.trim_end()
      )),
      None => out.push_str("No smaller part of the file was found that still fails.\n\n"),
    }
    out.push_str(&format!("Command:\n\n```text\n{}\n```\n\n", self.command));
    out.push_str(&format!(
      "Settings that differ from the defaults:\n\n```text\n{}\n```\n\n",
      if self.settings.trim().is_empty() {
        "(none)"
      } else {
        self.settings.trim_end()
      }
    ));
    out.push_str(&format!("Build:\n\n```text\n{}```\n", version::details()));
    if let Some(backtrace) = &self.backtrace {
      out.push_str(&format!(
        "\n<details><summary>Backtrace</summary>\n\n```text\n{}\n```\n\n</details>\n",
        backtrace.trim_end()
      ));
    }
    out
  }
}

/// A kept report, as listed.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
  pub id: String,
  pub title: String,
  pub count: u64,
  pub first: u64,
  pub last: u64,
  /// The files it was seen in, the first first.
  pub files: Vec<Seen>,
  pub dir: PathBuf,
}

impl Entry {
  /// The report's text (`report.md`).
  pub fn read(&self) -> io::Result<String> {
    fs::read_to_string(self.dir.join("report.md"))
  }
}

/// A file a bug was seen in: as the run named it, where it is (`None` for stdin), and the
/// hash (SHA-256) of its text then.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
  pub shown: String,
  pub path: Option<PathBuf>,
  pub hash: String,
}

/// The folder reports are kept in.
#[derive(Debug)]
pub struct Store {
  dir: PathBuf,
  /// One save at a time in this process: workers finding the same bug count it once each.
  saving: Mutex<()>,
}

impl Store {
  pub fn new(dir: PathBuf) -> Store {
    Store {
      dir,
      saving: Mutex::new(()),
    }
  }

  pub fn dir(&self) -> &Path {
    &self.dir
  }

  /// Keeps `bug`: a new report, or one more sighting of the same one. Reports not seen for
  /// [`KEEP_DAYS`] go. Gives the report's id.
  pub fn save(&self, bug: &Bug) -> io::Result<String> {
    let _one = self.saving.lock().unwrap_or_else(|e| e.into_inner());
    let id = bug.key()[..12].to_string();
    let dir = self.dir.join(&id);
    let now = now_millis();
    let mut entry = read_entry(&dir).unwrap_or(Entry {
      id: id.clone(),
      title: bug.title(),
      count: 0,
      first: now,
      last: now,
      files: Vec::new(),
      dir: dir.clone(),
    });
    entry.count += 1;
    entry.last = now;
    if !entry.files.iter().any(|seen| seen.shown == bug.file) && entry.files.len() < SEEN_IN {
      entry.files.push(Seen {
        shown: bug.file.clone(),
        path: bug.path.clone(),
        hash: bug.source_hash.clone(),
      });
    }
    fs::create_dir_all(&dir)?;
    if entry.count == 1 {
      fs::write(dir.join("report.md"), bug.markdown())?;
    }
    fs::write(dir.join("meta"), meta_text(&entry))?;
    self.prune_older(now.saturating_sub(KEEP_DAYS * 24 * 60 * 60 * 1000));
    Ok(id)
  }

  /// Every kept report, the most recently seen first.
  pub fn list(&self) -> Vec<Entry> {
    let mut entries: Vec<Entry> = fs::read_dir(&self.dir)
      .map(|read| {
        read
          .flatten()
          .filter_map(|e| read_entry(&e.path()))
          .collect()
      })
      .unwrap_or_default();
    entries.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.id.cmp(&b.id)));
    entries
  }

  /// The report whose id starts with `prefix`; an error saying why when none or several do.
  pub fn find(&self, prefix: &str) -> Result<Entry, String> {
    let matching: Vec<Entry> = self
      .list()
      .into_iter()
      .filter(|e| e.id.starts_with(prefix))
      .collect();
    match matching.len() {
      0 => Err(format!("no report `{}` (`mto report` lists them)", prefix)),
      1 => Ok(matching.into_iter().next().expect("one")),
      n => Err(format!(
        "`{}` names {} reports; give more of its id",
        prefix, n
      )),
    }
  }

  /// Removes every report; how many went.
  pub fn clear(&self) -> io::Result<usize> {
    let entries = self.list();
    for entry in &entries {
      fs::remove_dir_all(&entry.dir)?;
    }
    Ok(entries.len())
  }

  /// Removes the reports last seen before `cutoff` (ms).
  fn prune_older(&self, cutoff: u64) {
    for entry in self.list() {
      if entry.last < cutoff {
        let _ = fs::remove_dir_all(&entry.dir);
      }
    }
  }
}

fn meta_text(entry: &Entry) -> String {
  let mut out = format!(
    "title {}\ncount {}\nfirst {}\nlast {}\n",
    entry.title.replace('\n', " "),
    entry.count,
    entry.first,
    entry.last
  );
  // Tabs between: a path may hold spaces.
  for seen in &entry.files {
    let path = seen
      .path
      .as_ref()
      .map_or("-".to_string(), |p| p.display().to_string());
    out.push_str(&format!("file {}\t{}\t{}\n", seen.hash, path, seen.shown));
  }
  out
}

fn read_entry(dir: &Path) -> Option<Entry> {
  let text = fs::read_to_string(dir.join("meta")).ok()?;
  let id = dir.file_name()?.to_string_lossy().to_string();
  let mut entry = Entry {
    id,
    title: String::new(),
    count: 0,
    first: 0,
    last: 0,
    files: Vec::new(),
    dir: dir.to_path_buf(),
  };
  for line in text.lines() {
    let (key, value) = line.split_once(' ')?;
    match key {
      "title" => entry.title = value.to_string(),
      "count" => entry.count = value.parse().ok()?,
      "first" => entry.first = value.parse().ok()?,
      "last" => entry.last = value.parse().ok()?,
      "file" => {
        let mut fields = value.splitn(3, '\t');
        let (hash, path, shown) = (fields.next()?, fields.next()?, fields.next()?);
        entry.files.push(Seen {
          shown: shown.to_string(),
          path: (path != "-").then(|| PathBuf::from(path)),
          hash: hash.to_string(),
        });
      }
      _ => {}
    }
  }
  Some(entry)
}

#[cfg(test)]
mod tests;
