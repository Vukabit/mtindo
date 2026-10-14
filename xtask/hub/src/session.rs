//! The session: where the hub was and what it showed, kept in `<state>/session.json` so a
//! restart (or a reload of the config) comes back to it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::expr::{Row, Value};

/// The parts of a session `[hub] persist` may name.
pub const PARTS: &[&str] = &["place", "selection", "terminals"];

/// The session file's version; a file of another is ignored.
const VERSION: u32 = 1;

/// A screen and the pages opened over it, each with its row.
pub type Place = (String, Vec<(String, Json)>);

/// Which hub keeps the session of a state folder: the first opened on it. Another opened
/// while that one runs (a hub in its own terminal, or in a second window) neither takes up
/// nor saves the session, so the two never write over each other's.
pub struct Owner {
  path: PathBuf,
  owns: bool,
}

impl Owner {
  /// Claims the session of `dir`, unless a hub still running holds it.
  pub fn claim(dir: &Path) -> Owner {
    let path = dir.join("session.owner");
    let _ = std::fs::create_dir_all(dir);
    let mine = std::process::id().to_string();
    for _ in 0..2 {
      match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
      {
        Ok(mut file) => {
          use std::io::Write;
          let owns = file.write_all(mine.as_bytes()).is_ok();
          return Owner { path, owns };
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
          let holder = std::fs::read_to_string(&path).unwrap_or_default();
          if holder.trim().parse::<u32>().is_ok_and(running) {
            return Owner { path, owns: false };
          }
          // Left by a hub that ended without letting go.
          let _ = std::fs::remove_file(&path);
        }
        // A folder that cannot be written keeps no session anyway.
        Err(_) => return Owner { path, owns: true },
      }
    }
    Owner { path, owns: false }
  }

  pub fn owns(&self) -> bool {
    self.owns
  }
}

impl Drop for Owner {
  fn drop(&mut self) {
    let mine = std::process::id().to_string();
    if self.owns && std::fs::read_to_string(&self.path).is_ok_and(|t| t.trim() == mine) {
      let _ = std::fs::remove_file(&self.path);
    }
  }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Session {
  pub version: u32,
  // place: where the hub was, and the way back and forward.
  pub screen: Option<String>,
  #[serde(default)]
  pub pages: Vec<(String, Json)>,
  /// The pane focused on each screen or page, by its id.
  #[serde(default)]
  pub focus: BTreeMap<String, String>,
  #[serde(default)]
  pub behind: Vec<Place>,
  #[serde(default)]
  pub ahead: Vec<Place>,
  // selection: what was selected, how far panes were scrolled, the search.
  #[serde(default)]
  pub selected: BTreeMap<String, Json>,
  #[serde(default)]
  pub selection: BTreeMap<String, usize>,
  #[serde(default)]
  pub scroll: BTreeMap<String, usize>,
  pub search: Option<String>,
  // terminals: each terminal widget's tabs.
  #[serde(default)]
  pub terminals: BTreeMap<String, Tabs>,
}

impl Session {
  pub fn new() -> Session {
    Session {
      version: VERSION,
      ..Session::default()
    }
  }

  /// The session saved at `path`, if there is one this hub can read.
  pub fn load(path: &Path) -> Option<Session> {
    let text = std::fs::read_to_string(path).ok()?;
    let session: Session = serde_json::from_str(&text).ok()?;
    (session.version == VERSION).then_some(session)
  }

  /// A session from its JSON text; a new one if the text is not one.
  #[cfg(test)]
  pub fn load_from_str(text: &str) -> Session {
    serde_json::from_str(text).unwrap_or_else(|_| Session::new())
  }

  /// Writes the session to `path`, through a temporary file so a crash leaves the old one
  /// whole. Whether it changed what was there.
  pub fn save(&self, path: &Path, last: &mut String) -> bool {
    let Ok(text) = serde_json::to_string_pretty(self) else {
      return false;
    };
    if text == *last {
      return false;
    }
    if let Some(dir) = path.parent() {
      let _ = std::fs::create_dir_all(dir);
    }
    let temporary = path.with_extension("json.new");
    if std::fs::write(&temporary, &text).is_ok() && std::fs::rename(&temporary, path).is_ok() {
      *last = text;
      return true;
    }
    false
  }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Tab {
  /// The name given it, if any.
  pub name: Option<String>,
  /// The tmux session its program runs in, when its widget keeps it (`keep = "tmux"`).
  #[serde(default)]
  pub session: Option<String>,
  /// Its screen as the escape sequences that draw it again; empty for a full-screen
  /// program's, which would not make sense under a new shell.
  #[serde(default)]
  pub screen: String,
}

/// A terminal widget's tabs as they were.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Tabs {
  pub at: usize,
  pub tabs: Vec<Tab>,
}

pub fn from_json(json: &Json) -> Value {
  match json {
    Json::Null => Value::Null,
    Json::Bool(b) => Value::Bool(*b),
    Json::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
    Json::String(s) => Value::Str(s.clone()),
    Json::Array(items) if items.iter().all(Json::is_object) && !items.is_empty() => {
      Value::Rows(items.iter().map(row_from_json).collect())
    }
    Json::Array(items) => Value::List(items.iter().map(from_json).collect()),
    Json::Object(_) => Value::Rows(vec![row_from_json(json)]),
  }
}

pub fn row_from_json(json: &Json) -> Row {
  match json {
    Json::Object(map) => map.iter().map(|(k, v)| (k.clone(), from_json(v))).collect(),
    _ => Row::new(),
  }
}

pub fn row_to_json(row: &Row) -> Json {
  Json::Object(row.iter().map(|(k, v)| (k.clone(), to_json(v))).collect())
}

/// Whether two rows are the same thing: by the first of `id`, `hash`, `path`, `name`,
/// `_path` both have, else by every field. A row found again after a restart is matched
/// so even when its other fields changed.
pub fn same_row(a: &Row, b: &Row) -> bool {
  for key in ["id", "hash", "path", "name", "_path"] {
    if let (Some(x), Some(y)) = (a.get(key), b.get(key)) {
      return x == y;
    }
  }
  a == b
}

pub fn to_json(value: &Value) -> Json {
  match value {
    Value::Null => Json::Null,
    Value::Bool(b) => Json::Bool(*b),
    Value::Num(n) => serde_json::Number::from_f64(*n).map_or(Json::Null, Json::Number),
    Value::Str(s) => Json::String(s.clone()),
    Value::List(l) => Json::Array(l.iter().map(to_json).collect()),
    Value::Rows(rows) => Json::Array(rows.iter().map(row_to_json).collect()),
  }
}

/// Whether a process with this id runs.
fn running(pid: u32) -> bool {
  let Ok(pid) = libc::pid_t::try_from(pid) else {
    return false;
  };
  // Signal 0 only asks whether the process is there.
  // SAFETY: kill with signal 0 sends nothing.
  let found = unsafe { libc::kill(pid, 0) } == 0;
  found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_second_hub_on_a_folder_keeps_no_session() {
    let dir = std::env::temp_dir().join(format!("hub-owner-{}", std::process::id()));
    let first = Owner::claim(&dir);
    assert!(first.owns());
    // This process runs, so its claim stands against a second.
    let second = Owner::claim(&dir);
    assert!(!second.owns());
    drop(second);
    assert!(dir.join("session.owner").exists());
    drop(first);
    assert!(!dir.join("session.owner").exists());
    // A claim left by a process that has ended is taken over.
    std::fs::write(dir.join("session.owner"), "999999999").unwrap();
    assert!(Owner::claim(&dir).owns());
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn test_a_session_is_saved_whole_and_only_when_changed() {
    let dir = std::env::temp_dir().join(format!("hub-session-{}", std::process::id()));
    let path = dir.join("session.json");
    let mut session = Session::new();
    session.screen = Some("jobs".into());
    let mut last = String::new();
    assert!(session.save(&path, &mut last));
    assert!(!session.save(&path, &mut last));
    let read = Session::load(&path).unwrap();
    assert_eq!(read.screen.as_deref(), Some("jobs"));
    std::fs::write(&path, "{\"version\": 99}").unwrap();
    assert!(Session::load(&path).is_none());
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn test_rows_go_through_json_and_are_found_again() {
    let mut row = Row::new();
    row.insert("id".into(), Value::Str("a".into()));
    row.insert("n".into(), Value::Num(2.5));
    row.insert("on".into(), Value::Bool(true));
    row.insert(
      "words".into(),
      Value::List(vec![Value::Str("x".into()), Value::Str("y".into())]),
    );
    let back = row_from_json(&row_to_json(&row));
    assert_eq!(back, row);
    let mut moved = back.clone();
    moved.insert("n".into(), Value::Num(9.0));
    assert!(same_row(&row, &moved));
    moved.insert("id".into(), Value::Str("b".into()));
    assert!(!same_row(&row, &moved));
  }
}
