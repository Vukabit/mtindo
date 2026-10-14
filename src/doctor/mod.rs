//! Layer 5: the build guard (`mto doctor`). It finds the fewest of a run's edits that
//! break the project's build, so they can be held back and the rest written.
//!
//! A run's edits are the units layout moved ([`crate::layout::Move`]) and the runs of
//! lines formatting changed ([`Hunk`]). Any of them can be held back: a moved unit stays
//! where it was, as if pinned, and changed lines stay as written. Holding back edits always
//! leaves a whole file, every item in it once.
//!
//! [`narrow`] finds what to hold back through a test the caller gives, which writes the
//! candidates and runs the command: first the fewest files whose edits break the build, by
//! delta debugging (Zeller and Hildebrandt's `ddmin`); then, within them, the one edit
//! whose hold makes the difference, by bisecting how many of them are held back in order,
//! moves first ([`decisive`]). It holds that edit back and starts again, trying the same
//! files first, until the build passes or the checks run out.

pub mod check;
pub mod guard;
pub mod holds;
pub mod report;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::history::sha256;
use crate::layout::Move;
use crate::options::layout::ItemKind;
use crate::text::source::normalize;
use crate::{Settings, shape};

/// One edit of one file, as [`narrow`] tries them.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Edit {
  /// Every edit of the file: what is held when the checks run out.
  All,
  /// The unit starting here moved.
  Move(usize),
  Lines(LinesId),
}

/// A file a run changes, and what is held back in it so far.
#[derive(Clone, Debug)]
pub struct File {
  pub path: PathBuf,
  pub source: String,
  pub settings: Arc<Settings>,
  pub held: Held,
}

impl File {
  /// The file's text holding back `held` on top of what it holds already; its source when
  /// it cannot be laid out.
  fn text(&self, more: &Held) -> String {
    let mut held = self.held.clone();
    held.all |= more.all;
    held.moves.extend(more.moves.iter().copied());
    held.lines.extend(more.lines.iter().cloned());
    match shape(&self.source, &self.settings, &held, None) {
      Ok(shaped) => shaped.text,
      Err(_) => self.source.clone(),
    }
  }

  /// The edits left to hold back: moves, then the runs of lines formatting changes over
  /// what layout gives with `moves_held`.
  fn edits(&self, moves_held: bool) -> (Vec<Move>, Vec<Hunk>, String) {
    let mut held = self.held.clone();
    let Ok(free) = shape(&self.source, &self.settings, &held, None) else {
      return (Vec::new(), Vec::new(), String::new());
    };
    if moves_held {
      held.moves.extend(free.moves.iter().map(|m| m.span.0));
    }
    let Ok(shaped) = shape(&self.source, &self.settings, &held, None) else {
      return (free.moves, Vec::new(), String::new());
    };
    // The changed lines still to be held: those formatting makes over what layout gives,
    // less those held already.
    let mut formatted = held.clone();
    formatted.lines.clear();
    let full = shape(&self.source, &self.settings, &formatted, None)
      .map(|s| s.text)
      .unwrap_or_default();
    let hunks = hunks(&shaped.laid_out, &full)
      .into_iter()
      .filter(|h| !held.lines.contains(&h.id))
      .collect();
    (free.moves, hunks, shaped.laid_out)
  }
}

/// An edit held back, and why.
#[derive(Clone, Debug)]
pub struct Found {
  pub file: usize,
  pub edit: Edit,
  /// What it was, in words.
  pub what: String,
  /// How to keep it for good without the guard.
  pub fix: String,
  /// The errors the build gave with it.
  pub errors: Vec<String>,
  /// What layout moved, when it is a move: for its content name.
  pub moved: Option<Move>,
}

/// The edits of one file to hold back.
#[derive(Clone, Debug, Default)]
pub struct Held {
  /// Every edit: the file stays as written.
  pub all: bool,
  /// Moved units, by where they start in the text layout reads.
  pub moves: HashSet<usize>,
  /// Moved units, by content (remembered holds).
  pub move_ids: Vec<MoveId>,
  /// Changed lines.
  pub lines: HashSet<LinesId>,
}

impl Held {
  pub fn is_empty(&self) -> bool {
    !self.all && self.moves.is_empty() && self.move_ids.is_empty() && self.lines.is_empty()
  }
}

/// A run of lines formatting changed: lines `old` of the text before it, which became lines
/// `new` of the text after it (half-open ranges of line numbers from 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
  pub old: (usize, usize),
  pub new: (usize, usize),
  pub id: LinesId,
}

/// A run of changed lines, named by the digests of the lines before and after.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LinesId {
  pub old: String,
  pub new: String,
}

/// A moved unit, named by content so it is found again: its kind, its name, and the digest
/// of its text as written.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MoveId {
  pub kind: ItemKind,
  pub identifier: String,
  pub digest: String,
}

/// How a narrowing ended.
#[derive(Debug)]
pub struct Narrowed {
  pub found: Vec<Found>,
  /// The texts last tried, one per file.
  pub texts: Vec<String>,
  /// Whether the build passed with them.
  pub passed: bool,
  /// The checks ran out before an edit was found, so files were held back whole.
  pub ran_out: bool,
}

/// What one try of a candidate showed: whether it broke the build, and the errors it
/// brought.
#[derive(Clone, Debug, Default)]
pub struct Tried {
  pub broke: bool,
  pub errors: Vec<String>,
}

/// `old` with the hunks `take` chooses made as in `new`, the others left as in `old`.
pub fn apply(old: &str, new: &str, hunks: &[Hunk], take: impl Fn(&Hunk) -> bool) -> String {
  let a: Vec<&str> = old.split_inclusive('\n').collect();
  let b: Vec<&str> = new.split_inclusive('\n').collect();
  let mut out = String::with_capacity(new.len());
  let mut at = 0;
  for hunk in hunks {
    out.extend(a[at..hunk.old.0].iter().copied());
    if take(hunk) {
      out.extend(b[hunk.new.0..hunk.new.1].iter().copied());
    } else {
      out.extend(a[hunk.old.0..hunk.old.1].iter().copied());
    }
    at = hunk.old.1;
  }
  out.extend(a[at..].iter().copied());
  out
}

/// The fewest of `items` that still fail, by Zeller and Hildebrandt's `ddmin`: `items` is
/// known to fail, as `known` says, and none of them to pass. `test` says whether a subset fails, and with
/// what; `None` from it (the checks ran out) ends the search with `None`. Each subset is
/// tried once.
pub fn ddmin<T: Clone + Eq + std::hash::Hash>(
  items: &[T],
  known: Tried,
  test: &mut dyn FnMut(&[T]) -> Option<Tried>,
) -> Option<(Vec<T>, Tried)> {
  let mut seen: HashMap<Vec<T>, Tried> = HashMap::new();
  let mut try_set = |set: &[T]| -> Option<Tried> {
    if let Some(known) = seen.get(set) {
      return Some(known.clone());
    }
    let tried = test(set)?;
    seen.insert(set.to_vec(), tried.clone());
    Some(tried)
  };
  let mut current = items.to_vec();
  let mut current_tried = known;
  let mut parts = 2;
  while current.len() >= 2 {
    let size = current.len().div_ceil(parts);
    let chunks: Vec<Vec<T>> = current.chunks(size).map(|c| c.to_vec()).collect();
    let mut reduced = false;
    for chunk in &chunks {
      let tried = try_set(chunk)?;
      if tried.broke {
        current = chunk.clone();
        current_tried = tried;
        parts = 2;
        reduced = true;
        break;
      }
    }
    if !reduced && chunks.len() > 2 {
      for skip in 0..chunks.len() {
        let rest: Vec<T> = chunks
          .iter()
          .enumerate()
          .filter(|(k, _)| *k != skip)
          .flat_map(|(_, c)| c.iter().cloned())
          .collect();
        let tried = try_set(&rest)?;
        if tried.broke {
          current = rest;
          current_tried = tried;
          parts = (parts - 1).max(2);
          reduced = true;
          break;
        }
      }
    }
    if !reduced {
      if parts >= current.len() {
        break;
      }
      parts = (parts * 2).min(current.len());
    }
  }
  Some((current, current_tried))
}

/// The edit to hold back next, of `count` in order: holding back the first `m` of them is
/// tried for `m` found by bisection, to the fewest that let the build pass; the last of
/// those is the one that made the difference. None held back is known to break the build,
/// as `known` says; all held back is known to pass when `all_pass`, and tried otherwise.
/// `Some(None)`: the build breaks even with all held back. `None`: the checks ran out.
/// About log2 `count` checks, where the fewest failing set would take many more when the
/// edits only break the build together.
pub fn decisive(
  count: usize,
  known: Tried,
  all_pass: bool,
  test: &mut dyn FnMut(usize) -> Option<Tried>,
) -> Option<Option<(usize, Tried)>> {
  if count == 0 {
    return Some(None);
  }
  if !all_pass && test(count)?.broke {
    return Some(None);
  }
  // Holding back `low` breaks the build; holding back `high` does not.
  let (mut low, mut high, mut broken) = (0, count, known);
  while high - low > 1 {
    let middle = low + (high - low) / 2;
    let tried = test(middle)?;
    if tried.broke {
      low = middle;
      broken = tried;
    } else {
      high = middle;
    }
  }
  Some(Some((high - 1, broken)))
}

/// The runs of changed lines between `old` and `new`.
pub fn hunks(old: &str, new: &str) -> Vec<Hunk> {
  let a: Vec<&str> = old.split_inclusive('\n').collect();
  let b: Vec<&str> = new.split_inclusive('\n').collect();
  let mut found = Vec::new();
  let (mut i, mut j) = (0, 0);
  let mut open: Option<(usize, usize)> = None;
  let close = |open: &mut Option<(usize, usize)>, i: usize, j: usize, found: &mut Vec<Hunk>| {
    if let Some((i0, j0)) = open.take() {
      found.push(Hunk {
        old: (i0, i),
        new: (j0, j),
        id: LinesId {
          old: sha256::hex(a[i0..i].concat().as_bytes()),
          new: sha256::hex(b[j0..j].concat().as_bytes()),
        },
      });
    }
  };
  for op in crate::diff_ops(&a, &b) {
    match op {
      crate::DiffOp::Equal => {
        close(&mut open, i, j, &mut found);
        i += 1;
        j += 1;
      }
      crate::DiffOp::Delete(_) => {
        open.get_or_insert((i, j));
        i += 1;
      }
      crate::DiffOp::Insert(_) => {
        open.get_or_insert((i, j));
        j += 1;
      }
    }
  }
  close(&mut open, i, j, &mut found);
  found
}

/// The word for a unit's kind, as the code says it.
pub fn kind_word(kind: &ItemKind) -> &'static str {
  match kind {
    ItemKind::ModDecl => "mod",
    ItemKind::UseDecl => "use",
    ItemKind::MacroDecl => "macro_rules!",
    ItemKind::MacroDef => "macro",
    ItemKind::ConstDecl => "const",
    ItemKind::StaticDecl => "static",
    ItemKind::TypeAlias => "type",
    ItemKind::TraitDecl => "trait",
    ItemKind::EnumDecl => "enum",
    ItemKind::StructDecl => "struct",
    ItemKind::Union => "union",
    ItemKind::ImplBlock => "impl",
    ItemKind::ExternBlock => "extern block",
    ItemKind::FnDecl => "fn",
    ItemKind::Unknown => "item",
  }
}

/// The content name of a moved unit of `source`.
pub fn move_id(source: &str, moved: &Move) -> MoveId {
  let text = normalize(source).map(|n| n.text).unwrap_or_default();
  let body = text.get(moved.span.0..moved.span.1).unwrap_or("");
  MoveId {
    kind: moved.kind.clone(),
    identifier: moved.identifier.clone(),
    digest: sha256::hex(body.as_bytes()),
  }
}

/// Finds edits of `files` to hold back until `test` passes. `test` writes the texts it is
/// given, one per file, and runs the command: `None` when no more checks may run. The texts
/// with nothing held back are known to break the build, as `first` says.
pub fn narrow(
  files: &mut [File],
  first: Tried,
  test: &mut dyn FnMut(&[String]) -> Option<Tried>,
) -> Narrowed {
  let mut broken = first;
  let mut found = Vec::new();
  let mut known_broken = true;
  // The files the last turn found breaking the build: tried first on the next turn.
  let mut suspects: Vec<usize> = Vec::new();
  let mut ran_out = false;
  // Each turn holds one edit back, or whole files, so there are at most this many.
  let most = files
    .iter()
    .map(|f| {
      let (moves, hunks, _) = f.edits(false);
      moves.len() + hunks.len() + 1
    })
    .sum::<usize>()
    + 1;
  for _ in 0..most {
    let full: Vec<String> = files.iter().map(|f| f.text(&Held::default())).collect();
    if !known_broken {
      match test(&full) {
        Some(tried) if !tried.broke => return done(found, full, true, ran_out),
        Some(tried) => broken = tried,
        None => return hold_all(files, found),
      }
    }
    known_broken = false;
    let changed: Vec<usize> = (0..files.len())
      .filter(|&i| full[i] != files[i].source)
      .collect();
    if changed.is_empty() {
      // Nothing left to hold back, and the build still breaks: not mto's doing.
      return done(found, full, false, ran_out);
    }
    // The fewest files whose edits break the build.
    let sources: Vec<String> = files.iter().map(|f| f.source.clone()).collect();
    let mut only = |subset: &[usize]| {
      let mut texts = sources.clone();
      for &i in subset {
        texts[i] = full[i].clone();
      }
      test(&texts)
    };
    // What broke the build last turn most likely still does.
    suspects.retain(|i| changed.contains(i));
    let again = if suspects.is_empty() {
      None
    } else {
      match only(&suspects) {
        Some(tried) if tried.broke => Some(Some((suspects.clone(), tried))),
        Some(_) => None,
        None => Some(None),
      }
    };
    let culprits = match again {
      Some(result) => result,
      None => ddmin(&changed, broken.clone(), &mut only),
    };
    let Some((culprit_files, in_files)) = culprits else {
      ran_out = true;
      suspects.clear();
      for i in changed {
        files[i].held.all = true;
        found.push(whole(i, &files[i]));
      }
      continue;
    };
    suspects = culprit_files.clone();
    // The texts with every other file as written.
    let with = |texts: &[(usize, String)]| {
      let mut all = sources.clone();
      for (i, text) in texts {
        all[*i] = text.clone();
      }
      all
    };
    let edits: Vec<(usize, Vec<Move>, Vec<Hunk>, String)> = culprit_files
      .iter()
      .map(|&i| {
        let (moves, _, _) = files[i].edits(false);
        let (_, hunks, laid) = files[i].edits(true);
        (i, moves, hunks, laid)
      })
      .collect();
    // Formatting alone, every move held back.
    let moves_held: Vec<(usize, String)> = edits
      .iter()
      .map(|(i, moves, _, _)| {
        let held = Held {
          moves: moves.iter().map(|m| m.span.0).collect(),
          ..Held::default()
        };
        (*i, files[*i].text(&held))
      })
      .collect();
    let Some(alone) = test(&with(&moves_held)) else {
      return hold_all(files, found);
    };
    if alone.broke {
      // The changed lines break it: find the fewest, every move held back.
      let lines: Vec<(usize, Hunk)> = edits
        .iter()
        .flat_map(|(i, _, hunks, _)| hunks.iter().map(move |h| (*i, h.clone())))
        .collect();
      let result = decisive(lines.len(), alone, false, &mut |count: usize| {
        let kept: HashSet<&LinesId> = lines[..count].iter().map(|(_, h)| &h.id).collect();
        let texts: Vec<(usize, String)> = edits
          .iter()
          .map(|(i, moves, _, _)| {
            let held = Held {
              moves: moves.iter().map(|m| m.span.0).collect(),
              lines: kept.iter().map(|id| (*id).clone()).collect(),
              ..Held::default()
            };
            (*i, files[*i].text(&held))
          })
          .collect();
        test(&with(&texts))
      });
      let Some(Some((k, tried))) = result else {
        ran_out |= result.is_none();
        hold_files(files, &culprit_files, &mut found);
        continue;
      };
      let (i, hunk) = &lines[k];
      let laid = &edits.iter().find(|e| e.0 == *i).expect("listed").3;
      hold(
        files,
        lines_found(*i, &files[*i], hunk, laid),
        tried,
        &mut found,
      );
    } else {
      let moves: Vec<(usize, Move)> = edits
        .iter()
        .flat_map(|(i, moves, _, _)| moves.iter().map(move |m| (*i, m.clone())))
        .collect();
      // Every move held back passes (`alone`), none held back breaks (`in_files`).
      let result = decisive(moves.len(), in_files, true, &mut |count: usize| {
        let kept: HashSet<(usize, usize)> =
          moves[..count].iter().map(|(i, m)| (*i, m.span.0)).collect();
        let texts: Vec<(usize, String)> = edits
          .iter()
          .map(|(i, file_moves, _, _)| {
            let held = Held {
              moves: file_moves
                .iter()
                .map(|m| m.span.0)
                .filter(|start| kept.contains(&(*i, *start)))
                .collect(),
              ..Held::default()
            };
            (*i, files[*i].text(&held))
          })
          .collect();
        test(&with(&texts))
      });
      let Some(Some((k, tried))) = result else {
        ran_out |= result.is_none();
        hold_files(files, &culprit_files, &mut found);
        continue;
      };
      let (i, moved) = &moves[k];
      hold(files, move_found(*i, &files[*i], moved), tried, &mut found);
    }
  }
  let full: Vec<String> = files.iter().map(|f| f.text(&Held::default())).collect();
  let passed = test(&full).is_some_and(|t| !t.broke);
  done(found, full, passed, ran_out)
}

fn done(found: Vec<Found>, texts: Vec<String>, passed: bool, ran_out: bool) -> Narrowed {
  Narrowed {
    found,
    texts,
    passed,
    ran_out,
  }
}

/// Holds back an edit, with the errors the build gave without it.
fn hold(files: &mut [File], mut first: Found, tried: Tried, found: &mut Vec<Found>) {
  first.errors = tried.errors;
  let file = &mut files[first.file];
  match &first.edit {
    Edit::All => file.held.all = true,
    Edit::Move(start) => {
      file.held.moves.insert(*start);
    }
    Edit::Lines(id) => {
      file.held.lines.insert(id.clone());
    }
  }
  found.push(first);
}

/// The checks ran out with no file narrowed down: every file is held back whole, and
/// whether that builds is not known.
fn hold_all(files: &mut [File], mut found: Vec<Found>) -> Narrowed {
  let mut texts = Vec::new();
  for (i, file) in files.iter_mut().enumerate() {
    if file.text(&Held::default()) != file.source {
      file.held.all = true;
      found.push(whole(i, file));
    }
    texts.push(file.source.clone());
  }
  Narrowed {
    found,
    texts,
    passed: false,
    ran_out: true,
  }
}

fn hold_files(files: &mut [File], which: &[usize], found: &mut Vec<Found>) {
  for &i in which {
    files[i].held.all = true;
    found.push(whole(i, &files[i]));
  }
}

fn lines_found(i: usize, file: &File, hunk: &Hunk, laid: &str) -> Found {
  let a: Vec<&str> = laid.split_inclusive('\n').collect();
  let shown: String = a[hunk.old.0..hunk.old.1]
    .iter()
    .take(3)
    .map(|l| format!("\n    {}", l.trim_end()))
    .collect();
  let range = if hunk.old.1 > hunk.old.0 + 1 {
    format!("lines {}–{}", hunk.old.0 + 1, hunk.old.1)
  } else {
    format!("line {}", hunk.old.0 + 1)
  };
  let _ = file;
  Found {
    file: i,
    edit: Edit::Lines(hunk.id.clone()),
    what: format!("formatted: {}{}", range, shown),
    fix: "`// mto: off` above these lines and `// mto: on` below keep them as written".into(),
    errors: Vec::new(),
    moved: None,
  }
}

fn move_found(i: usize, file: &File, moved: &Move) -> Found {
  let text = normalize(&file.source).map(|n| n.text).unwrap_or_default();
  let line = text[..moved.span.0.min(text.len())].matches('\n').count() + 1;
  let now = match &moved.before {
    Some((kind, name)) => format!(", now before {} {}", kind_word(kind), name),
    None => ", now last".into(),
  };
  Found {
    file: i,
    edit: Edit::Move(moved.span.0),
    what: format!(
      "moved: {} {} (line {}{})",
      kind_word(&moved.kind),
      moved.identifier,
      line,
      now
    ),
    fix: "`// mto: pin` on the line above it keeps it where it is".into(),
    errors: Vec::new(),
    moved: Some(moved.clone()),
  }
}

fn whole(i: usize, file: &File) -> Found {
  Found {
    file: i,
    edit: Edit::All,
    what: format!("every edit of {}", file.path.display()),
    fix: "`// mto: off` on its first line keeps the whole file as written".into(),
    errors: Vec::new(),
    moved: None,
  }
}

#[cfg(test)]
mod tests;
