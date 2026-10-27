//! The edits the build guard holds back, remembered so later runs hold them back again
//! without running the command: `holds.txt` in the doctor's folder, one per line.
//!
//! A hold names its file and what it is about by content, not by place: a moved item by its
//! kind, name and the digest of its text; changed lines by the digests of the lines before
//! and after. So it is found again after other edits move it, and lapses when the text it
//! is about changes, or when mto's build does.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{Held, LinesId, MoveId};
use crate::options::layout::ItemKind;

/// The build holds are kept for: they lapse with a new one, as it may lay files out
/// differently, though the version be the same (a build between releases).
pub const VERSION: &str = crate::version::FULL;

/// What a hold is about.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
  Move(MoveId),
  Lines(LinesId),
}

/// One remembered hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hold {
  pub path: PathBuf,
  pub version: String,
  pub kind: Kind,
  /// What the edit was, in words (`moved: mod generated, from line 40`).
  pub what: String,
  /// How to keep it for good without the guard.
  pub fix: String,
  /// The run that found it.
  pub run: String,
}

/// Every remembered hold, as read from the doctor's folder.
#[derive(Debug, Default)]
pub struct Holds {
  path: Option<PathBuf>,
  pub list: Vec<Hold>,
}

impl Holds {
  /// The holds kept in `dir`; those of another build of mto have lapsed and are left out.
  pub fn load(dir: &Path) -> Holds {
    let path = dir.join("holds.txt");
    let list: Vec<Hold> = fs::read_to_string(&path)
      .map(|text| text.lines().filter_map(parse).collect())
      .unwrap_or_default();
    Holds {
      path: Some(path),
      list: list.into_iter().filter(|h| h.version == VERSION).collect(),
    }
  }

  /// What is held in `path`, for laying it out.
  pub fn held_for(&self, path: &Path) -> Held {
    let mut held = Held::default();
    if self.list.is_empty() {
      return held;
    }
    let path = key(path);
    for hold in self.list.iter().filter(|h| h.path == path) {
      match &hold.kind {
        Kind::Move(id) => held.move_ids.push(id.clone()),
        Kind::Lines(id) => {
          held.lines.insert(id.clone());
        }
      }
    }
    held
  }

  /// Writes the holds back, through a temporary file; none leaves no file.
  pub fn save(&self) -> io::Result<()> {
    let Some(path) = &self.path else {
      return Ok(());
    };
    if self.list.is_empty() {
      return match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
      };
    }
    if let Some(dir) = path.parent() {
      fs::create_dir_all(dir)?;
    }
    let text: String = self.list.iter().map(line).collect();
    let temporary = path.with_extension("txt.new");
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path)
  }

  /// Drops the holds of `path` that were not found in it: the text they were about is gone.
  pub fn lapse(&mut self, path: &Path, found: &[Kind]) -> bool {
    let before = self.list.len();
    if before == 0 {
      return false;
    }
    let path = key(path);
    self
      .list
      .retain(|h| h.path != path || found.contains(&h.kind));
    self.list.len() != before
  }
}

/// The path a file's holds are kept under: its real one, so that they are found however the
/// file is named (through a link, or in another case where names ignore case).
pub fn key(path: &Path) -> PathBuf {
  crate::real_path(path)
    .or_else(|_| std::path::absolute(path))
    .unwrap_or_else(|_| path.to_path_buf())
}

fn escape(text: &str) -> String {
  text
    .replace('\\', "\\\\")
    .replace('\t', "\\t")
    .replace('\n', "\\n")
    .replace('\r', "\\r")
}

fn line(hold: &Hold) -> String {
  let mut fields = vec![
    match hold.kind {
      Kind::Move(_) => "move".to_string(),
      Kind::Lines(_) => "lines".to_string(),
    },
    hold.version.clone(),
    hold.path.to_string_lossy().into_owned(),
    hold.run.clone(),
  ];
  match &hold.kind {
    Kind::Move(id) => {
      fields.push(format!("{:?}", id.kind));
      fields.push(id.identifier.clone());
      fields.push(id.digest.clone());
    }
    Kind::Lines(id) => {
      fields.push(id.old.clone());
      fields.push(id.new.clone());
    }
  }
  fields.push(hold.what.clone());
  fields.push(hold.fix.clone());
  let fields: Vec<String> = fields.iter().map(|f| escape(f)).collect();
  format!("{}\n", fields.join("\t"))
}

fn parse(line: &str) -> Option<Hold> {
  let fields: Vec<String> = line.split('\t').map(unescape).collect();
  let field = |i: usize| fields.get(i).cloned();
  let (kind, rest) = match fields.first()?.as_str() {
    "move" => (
      Kind::Move(MoveId {
        kind: ItemKind::from_name(&field(4)?),
        identifier: field(5)?,
        digest: field(6)?,
      }),
      7,
    ),
    "lines" => (
      Kind::Lines(LinesId {
        old: field(4)?,
        new: field(5)?,
      }),
      6,
    ),
    _ => return None,
  };
  Some(Hold {
    version: field(1)?,
    path: PathBuf::from(field(2)?),
    run: field(3)?,
    kind,
    what: field(rest)?,
    fix: field(rest + 1)?,
  })
}

fn unescape(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let mut chars = text.chars();
  while let Some(c) = chars.next() {
    if c != '\\' {
      out.push(c);
      continue;
    }
    match chars.next() {
      Some('t') => out.push('\t'),
      Some('n') => out.push('\n'),
      Some('r') => out.push('\r'),
      Some(other) => out.push(other),
      None => out.push('\\'),
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A file's holds are found however it is named: here through a linked folder.
  #[cfg(unix)]
  #[test]
  fn test_holds_are_found_through_a_link() {
    let dir = std::env::temp_dir().join(format!("mtindo-holds-link-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("real")).unwrap();
    fs::write(dir.join("real/lib.rs"), "").unwrap();
    std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
    let lines = Kind::Lines(LinesId {
      old: "cd".into(),
      new: "ef".into(),
    });
    let mut holds = Holds::default();
    holds.list.push(Hold {
      path: key(&dir.join("real/lib.rs")),
      version: VERSION.into(),
      kind: lines,
      what: String::new(),
      fix: String::new(),
      run: String::new(),
    });
    assert_eq!(holds.held_for(&dir.join("link/lib.rs")).lines.len(), 1);
    assert!(holds.lapse(&dir.join("link/lib.rs"), &[]));
    assert!(holds.list.is_empty());
    let _ = fs::remove_dir_all(&dir);
  }

  #[test]
  fn test_holds_read_back_and_lapse() {
    let dir = std::env::temp_dir().join(format!("mtindo-holds-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let moved = Kind::Move(MoveId {
      kind: ItemKind::ModDecl,
      identifier: "generated".into(),
      digest: "ab".into(),
    });
    let lines = Kind::Lines(LinesId {
      old: "cd".into(),
      new: "ef".into(),
    });
    let hold = |kind: Kind, version: &str| Hold {
      path: key(Path::new("/work/src/lib.rs")),
      version: version.into(),
      kind,
      what: "moved: mod generated\twith a tab".into(),
      fix: "// mto: pin\nabove it".into(),
      run: "0192".into(),
    };
    let mut holds = Holds::load(&dir);
    assert!(holds.list.is_empty());
    holds.list.push(hold(moved.clone(), VERSION));
    holds.list.push(hold(lines.clone(), VERSION));
    holds.list.push(hold(lines.clone(), "0.0.0-old"));
    holds.save().unwrap();
    let read = Holds::load(&dir);
    assert_eq!(read.list.len(), 2);
    assert_eq!(read.list[0], hold(moved.clone(), VERSION));
    let held = read.held_for(Path::new("/work/src/lib.rs"));
    assert_eq!(held.move_ids.len(), 1);
    assert_eq!(held.lines.len(), 1);
    assert!(read.held_for(Path::new("/work/src/main.rs")).is_empty());
    let mut read = read;
    assert!(read.lapse(Path::new("/work/src/lib.rs"), &[lines]));
    assert_eq!(read.list.len(), 1);
    read.list.clear();
    read.save().unwrap();
    assert!(!dir.join("holds.txt").exists());
    let _ = fs::remove_dir_all(&dir);
  }
}
