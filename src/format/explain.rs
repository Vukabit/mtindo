//! Why code is left as written: each place the formatter gives up on laying something
//! out says so here, for `mto debug layout`. Nothing is kept unless a caller asks
//! ([`collect`]), so a run that formats pays one check per place.
//!
//! A note holds the text it is about rather than an offset: the pre-passes rewrite the
//! text before the layout reads it, and macro bodies are formatted as texts of their own,
//! so an offset would point into a text the caller never sees. [`locate`] finds each note's
//! text in the file.

use std::cell::RefCell;

/// The most characters of a note's code shown.
const SHOWN: usize = 80;

/// One place left as written: the node kind, its first line as written, the start of its
/// code with each run of whitespace one space (to show, and to find it by), and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
  pub kind: String,
  pub text: String,
  pub shown: String,
  pub reason: String,
}

thread_local! {
  static NOTES: RefCell<Option<Vec<Note>>> = const { RefCell::new(None) };
}

/// Whether notes are being kept, so that a caller builds a reason only then.
pub fn active() -> bool {
  NOTES.with(|notes| notes.borrow().is_some())
}

/// Runs `f`, keeping the notes made while it runs.
pub fn collect<T>(f: impl FnOnce() -> T) -> (T, Vec<Note>) {
  let before = NOTES.with(|notes| notes.replace(Some(Vec::new())));
  let out = f();
  let notes = NOTES
    .with(|notes| notes.replace(before))
    .unwrap_or_default();
  (out, notes)
}

/// The line and column (from 1) in `source` where each note's code is found, whitespace
/// aside, searching on past the one before, then from the start; `None` for one not found
/// (code the pre-passes rewrote).
pub fn locate(source: &str, notes: &[Note]) -> Vec<Option<(usize, usize)>> {
  // The source without whitespace, and where each of its characters is.
  let mut bare = String::with_capacity(source.len());
  let mut at = Vec::with_capacity(source.len());
  for (i, c) in source.char_indices() {
    if !c.is_whitespace() {
      bare.push(c);
      at.extend(std::iter::repeat_n(i, c.len_utf8()));
    }
  }
  let mut from = 0;
  notes
    .iter()
    .map(|note| {
      let needle: String = note
        .shown
        .trim_end_matches('…')
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(SHOWN / 2)
        .collect();
      if needle.is_empty() {
        return None;
      }
      let found = bare[from..]
        .find(&needle)
        .map(|i| from + i)
        .or_else(|| bare.find(&needle))?;
      from = found + needle.len();
      let offset = at[found];
      let line_start = source[..offset].rfind('\n').map_or(0, |i| i + 1);
      let line = source[..offset].matches('\n').count() + 1;
      Some((line, source[line_start..offset].chars().count() + 1))
    })
    .collect()
}

/// Keeps a note when [`collect`] asked for them: `text` is what is left as written.
pub fn note(kind: &str, text: &str, reason: impl FnOnce() -> String) {
  NOTES.with(|notes| {
    if let Some(notes) = notes.borrow_mut().as_mut() {
      let text = text.trim_start();
      let first = text.lines().next().unwrap_or("").trim_end();
      let mut shown = String::new();
      for word in text.split_whitespace() {
        if shown.chars().count() >= SHOWN {
          break;
        }
        if !shown.is_empty() {
          shown.push(' ');
        }
        shown.push_str(word);
      }
      if shown.chars().count() > SHOWN {
        shown = shown.chars().take(SHOWN - 1).collect::<String>() + "…";
      }
      let note = Note {
        kind: kind.to_string(),
        text: first.to_string(),
        shown,
        reason: reason(),
      };
      if !notes.contains(&note) {
        notes.push(note);
      }
    }
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_notes_kept_only_while_collected() {
    note("LET_STMT", "let x = 1;", || "not kept".to_string());
    let ((), notes) = collect(|| {
      note("LET_STMT", "  let x =\n\t 1;\n  more", || {
        "kept".to_string()
      });
      note("LET_STMT", "  let x =\n\t 1;\n  more", || {
        "kept".to_string()
      });
    });
    assert_eq!(
      notes,
      vec![Note {
        kind: "LET_STMT".to_string(),
        text: "let x =".to_string(),
        shown: "let x = 1; more".to_string(),
        reason: "kept".to_string(),
      }]
    );
    assert!(!active());
  }

  #[test]
  fn test_notes_located_in_order() {
    let source = "fn f() {\n    let x = 1;\n}\nfn g() {\n    let x = 1;\n}\n";
    let note = |text: &str| Note {
      kind: "LET_STMT".to_string(),
      text: text.to_string(),
      shown: text.to_string(),
      reason: String::new(),
    };
    let found = locate(
      source,
      &[
        note("fn g() {"),
        note("let x = 1;"),
        note("missing"),
        note("let x  =\n 1;"),
      ],
    );
    assert_eq!(found, vec![Some((4, 1)), Some((5, 5)), None, Some((2, 5))]);
  }
}
