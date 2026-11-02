//! A guarded run: the files are written once the check before has run, checked again, and
//! narrowed down when they broke the build.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::check::{Check, Said};
use super::holds::{self, Hold, Holds};
use super::{Edit, File, Held, Narrowed, Tried, move_id, narrow, report};
use crate::history::{self, Entry, History, Recorder, State};
use crate::{Settings, c_green, c_red, c_reset, c_yellow, write_atomic};
use crate::{errln, outln};

/// How a guarded run ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
  /// The files are written and, as far as the check can tell, the build is as before.
  Written,
  /// Some files could not be written; the report says which.
  Failed,
  /// The build fails with every edit held back, or failed before any: the files are as
  /// they were.
  NotMto,
}

/// How the guard runs: its command, its limit, and where it keeps holds and reports.
#[derive(Clone, Debug)]
pub struct Guard {
  pub check: Check,
  pub max_checks: usize,
  /// The doctor's folder.
  pub dir: PathBuf,
  pub quiet: bool,
}

/// A file a run is to write.
#[derive(Clone, Debug)]
pub struct Planned {
  pub path: PathBuf,
  pub source: String,
  pub text: String,
  pub settings: Arc<Settings>,
  /// What remembered holds keep back in it already.
  pub held: Held,
}

/// Writes `planned`, the check before having said `before`, and checks the build again;
/// narrows down a break and holds back what it finds.
pub fn finish(
  guard: &Guard,
  before: Said,
  planned: Vec<Planned>,
  recorder: &mut Option<Recorder>,
  remembered: &mut Holds,
  command: &str,
) -> Ended {
  let note = |recorder: &mut Option<Recorder>, line: &str| {
    if let Some(recorder) = recorder.as_mut() {
      let _ = recorder.note(line);
    }
  };
  // No baseline, no guard: a check that already fails before mto changes anything cannot
  // tell what this run would break (a crate that never builds hides every crate after
  // it), so nothing is written rather than the run looking checked.
  if !before.passed && !planned.is_empty() {
    note(
      recorder,
      &format!("check before {}; nothing written", before.summary()),
    );
    let why = match (&before.stopped, before.errors.iter().next()) {
      (Some(stopped), _) => stopped.clone(),
      (None, Some(error)) => error.clone(),
      (None, None) => "it fails without an error the compiler's way".to_string(),
    };
    errln!(
      "{}doctor: `{}` already fails before mto changes anything ({}), so it cannot tell \
       whether this run breaks the build; nothing was written. Fix the build, give the \
       guard a command that passes (`mto doctor on -- COMMAND`), or pass --unguarded.{}",
      c_red(),
      guard.check.line(),
      why,
      c_reset(true)
    );
    return Ended::NotMto;
  }
  let mut ended = Ended::Written;
  let mut written: Vec<String> = Vec::with_capacity(planned.len());
  let results = write_all(&planned, recorder);
  for (file, result) in planned.iter().zip(results) {
    match result {
      Ok(()) => {
        written.push(file.text.clone());
        if !guard.quiet {
          outln!(
            "{}Formatted:{} {}",
            c_green(),
            c_reset(false),
            file.path.display()
          );
        }
      }
      Err(e) => {
        errln!("{}{}{}", c_red(), e, c_reset(true));
        written.push(file.source.clone());
        ended = Ended::Failed;
      }
    }
  }
  if planned.is_empty() {
    return ended;
  }
  note(recorder, &format!("check before {}", before.summary()));
  let after = guard.check.run();
  note(recorder, &format!("check after {}", after.summary()));
  let mark = |said: &Said| if said.passed { "✓" } else { "✗" };
  if !after.broke(&before) {
    say(
      guard,
      &format!("doctor: before {}  after {}", mark(&before), mark(&after)),
    );
    return ended;
  }
  let first_error = after
    .new_errors(&before)
    .next()
    .cloned()
    .unwrap_or_default();
  say(
    guard,
    &format!("doctor: before {}  after ✗ {}", mark(&before), first_error),
  );
  note(recorder, "doctor open");
  let mut files: Vec<File> = planned
    .iter()
    .map(|p| File {
      path: p.path.clone(),
      source: p.source.clone(),
      settings: p.settings.clone(),
      held: p.held.clone(),
    })
    .collect();
  let mut checks = 0;
  let mut on_disk = written;
  let mut last_failure = after.clone();
  let first = Tried {
    broke: true,
    errors: after.new_errors(&before).cloned().collect(),
  };
  let narrowed = {
    let mut test = |texts: &[String]| -> Option<Tried> {
      if checks >= guard.max_checks {
        return None;
      }
      for (i, text) in texts.iter().enumerate() {
        if *text != on_disk[i]
          && write(&planned[i].path, &planned[i].source, text, recorder).is_ok()
        {
          on_disk[i] = text.clone();
        }
      }
      checks += 1;
      let said = guard.check.run();
      let broke = said.broke(&before);
      if broke {
        last_failure = said.clone();
      }
      Some(Tried {
        broke,
        errors: said.new_errors(&before).cloned().collect(),
      })
    };
    // A run written without its remembered holds (`--unguarded`) was checked with them
    // made: the texts holding them back are tried first.
    let checked = files
      .iter()
      .zip(&planned)
      .all(|(file, p)| file.text(&Held::default()) == p.text);
    narrow(&mut files, checked.then_some(first), &mut test)
  };
  // The texts it ended on, written.
  for (i, text) in narrowed.texts.iter().enumerate() {
    if *text != on_disk[i] && write(&planned[i].path, &planned[i].source, text, recorder).is_ok() {
      on_disk[i] = text.clone();
    }
  }
  let edits: usize = planned.iter().map(edits_of).sum();
  say(
    guard,
    &format!(
      "doctor: {}, {}: {}",
      count(planned.len(), "file"),
      count(edits, "edit"),
      count(checks, "check")
    ),
  );
  let run = recorder
    .as_ref()
    .map(|r| r.id().to_string())
    .unwrap_or_default();
  if !narrowed.passed {
    // Every edit held back and still broken: put the files back as they were.
    for (i, file) in planned.iter().enumerate() {
      if on_disk[i] != file.source {
        let _ = write(&file.path, &file.source, &file.source, recorder);
      }
    }
    note(recorder, "doctor closed");
    say(
      guard,
      &format!(
        "doctor: `{}` still fails with every edit held back, so mto did not break it; \
         the files are as they were",
        guard.check.line()
      ),
    );
    write_report(
      guard,
      &run,
      command,
      &before,
      &after,
      &narrowed,
      &planned,
      &last_failure,
    );
    return Ended::NotMto;
  }
  remember(&narrowed, &planned, remembered, &run);
  if let Err(e) = remembered.save() {
    errln!(
      "{}warning: cannot keep the holds in {}: {}{}",
      c_yellow(true),
      guard.dir.display(),
      e,
      c_reset(true)
    );
  }
  note(recorder, "doctor closed");
  if narrowed.found.is_empty() {
    say(
      guard,
      "doctor: the edits it already holds back were enough (`mto doctor holds`)",
    );
  } else {
    say(
      guard,
      &format!("doctor: held back {}:", count(narrowed.found.len(), "edit")),
    );
  }
  for found in &narrowed.found {
    say(
      guard,
      &format!(
        "  {}: {}",
        planned[found.file].path.display(),
        found.what.replace('\n', "\n  ")
      ),
    );
  }
  if narrowed.ran_out {
    // Out of checks, what is left unchecked is not written: every file is as before the
    // run, which the check before passed.
    say(
      guard,
      &format!(
        "doctor: out of checks (doctor.max_checks = {}); every file is as it was, and \
         nothing is written",
        guard.max_checks
      ),
    );
  } else {
    say(guard, "doctor: the rest is written; check ✓");
  }
  write_report(
    guard,
    &run,
    command,
    &before,
    &after,
    &narrowed,
    &planned,
    &last_failure,
  );
  ended
}

/// Puts back the files of every recent run whose narrowing was cut short, as they were
/// before the run, when they still hold what it last wrote. Returns what it did, in
/// words.
pub fn recover(history: &History) -> Vec<String> {
  let mut said = Vec::new();
  let Ok(ids) = history.ids() else {
    return said;
  };
  for id in ids.iter().rev().take(20) {
    let Ok(run) = history.read(id) else {
      continue;
    };
    if !run.doctor_open {
      continue;
    }
    let (mut put_back, mut left) = (0, 0);
    for entry in &run.entries {
      match history::Run::state(entry) {
        State::AsWritten => match history.load(&entry.before) {
          Ok(before) if write_atomic(&entry.path, &String::from_utf8_lossy(&before)).is_ok() => {
            put_back += 1
          }
          _ => left += 1,
        },
        State::AsBefore => {}
        State::Changed | State::Missing => left += 1,
      }
    }
    let _ = history.append(id, "doctor closed");
    said.push(format!(
      "doctor: run {} was cut short while narrowing; put {} back as before it{}",
      history::id::handle(id, &ids),
      count(put_back, "file"),
      if left > 0 {
        format!(", and left {} changed since", count(left, "file"))
      } else {
        String::new()
      }
    ));
  }
  said
}

/// Writes `text` to `path`, its contents stored in history and added to the run's record
/// first, so the record always says what the file holds.
pub fn write(
  path: &Path,
  source: &str,
  text: &str,
  recorder: &mut Option<Recorder>,
) -> Result<(), String> {
  if let Some(recorder) = recorder.as_mut() {
    let history = recorder.history().clone();
    let entry = entry(&history, path, source, text)?;
    recorder
      .record(&entry)
      .and_then(|_| recorder.order())
      .map_err(|e| format!("Left {} alone: cannot record it: {}", path.display(), e))?;
  }
  write_atomic(path, text).map_err(|e| format!("Cannot write {}: {}", path.display(), e))
}

/// `n` things, as words: `1 edit`, `3 edits`.
fn count(n: usize, thing: &str) -> String {
  format!("{} {}{}", n, thing, if n == 1 { "" } else { "s" })
}

/// How many edits the run makes to a file: units moved, and runs of lines formatting
/// changed.
fn edits_of(file: &Planned) -> usize {
  let Ok(shaped) = crate::shape(&file.source, &file.settings, &file.held, None) else {
    return 0;
  };
  shaped.moves.len() + super::hunks(&shaped.laid_out, &shaped.text).len()
}

/// The record of `path` going from `source` to `text`, the contents before stored in
/// history.
fn entry(history: &History, path: &Path, source: &str, text: &str) -> Result<Entry, String> {
  let stored = (|| {
    Ok::<_, std::io::Error>(Entry {
      path: std::path::absolute(path).unwrap_or_else(|_| path.into()),
      before: history.store(source.as_bytes())?,
      after: crate::history::sha256::hex(text.as_bytes()),
    })
  })();
  stored.map_err(|e| {
    format!(
      "Left {} alone: cannot record it in history ({}): {}",
      path.display(),
      history.dir().display(),
      e
    )
  })
}

/// Keeps the edits found, by content, for later runs.
fn remember(narrowed: &Narrowed, planned: &[Planned], holds: &mut Holds, run: &str) {
  for found in &narrowed.found {
    let file = &planned[found.file];
    let kind = match (&found.edit, &found.moved) {
      (Edit::Move(_), Some(moved)) => holds::Kind::Move(move_id(&file.source, moved)),
      (Edit::Lines(id), _) => holds::Kind::Lines(id.clone()),
      // A file held back whole is held for this run only.
      _ => continue,
    };
    let path = holds::key(&file.path);
    if holds.list.iter().any(|h| h.path == path && h.kind == kind) {
      continue;
    }
    holds.list.push(Hold {
      path,
      version: holds::VERSION.to_string(),
      kind,
      what: found.what.clone(),
      fix: found.fix.clone(),
      run: run.to_string(),
    });
  }
}

fn say(guard: &Guard, line: &str) {
  if !guard.quiet {
    errln!("{}", line);
  }
}

/// Writes every planned file as [`write`] does, on as many threads as the machine has, as
/// an ordinary run writes them: each file's contents are stored, then every file is added
/// to the record, and only then are the files replaced.
fn write_all(planned: &[Planned], recorder: &mut Option<Recorder>) -> Vec<Result<(), String>> {
  let workers = std::thread::available_parallelism()
    .map_or(1, |n| n.get())
    .min(planned.len().max(1));
  let in_parallel = |work: &(dyn Fn(usize) -> Result<(), String> + Sync)| {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Result<(), String>>> = (0..planned.len())
      .map(|_| std::sync::Mutex::new(Ok(())))
      .collect();
    std::thread::scope(|scope| {
      for _ in 0..workers {
        scope.spawn(|| {
          loop {
            let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if at >= planned.len() {
              break;
            }
            *results[at].lock().expect("not poisoned") = work(at);
          }
        });
      }
    });
    results
      .into_iter()
      .map(|r| r.into_inner().expect("not poisoned"))
      .collect::<Vec<_>>()
  };
  let mut results: Vec<Result<(), String>> = match recorder.as_mut() {
    None => vec![Ok(()); planned.len()],
    Some(recorder) => {
      let history = recorder.history().clone();
      let entries: Vec<std::sync::Mutex<Option<Entry>>> = planned
        .iter()
        .map(|_| std::sync::Mutex::new(None))
        .collect();
      let stored = in_parallel(&|at| {
        let file = &planned[at];
        let entry = entry(&history, &file.path, &file.source, &file.text)?;
        *entries[at].lock().expect("not poisoned") = Some(entry);
        Ok(())
      });
      let mut recorded: Vec<Result<(), String>> = stored
        .into_iter()
        .zip(entries)
        .zip(planned)
        .map(|((stored, entry), file)| {
          stored?;
          let entry = entry.into_inner().expect("not poisoned").expect("stored");
          recorder.record(&entry).map_err(|e| {
            format!(
              "Left {} alone: cannot record it: {}",
              file.path.display(),
              e
            )
          })
        })
        .collect();
      // Every file is recorded before any is replaced.
      if let Err(e) = recorder.order() {
        for (result, file) in recorded.iter_mut().zip(planned) {
          if result.is_ok() {
            *result = Err(format!(
              "Left {} alone: cannot record it: {}",
              file.path.display(),
              e
            ));
          }
        }
      }
      recorded
    }
  };
  let ready: Vec<bool> = results.iter().map(Result::is_ok).collect();
  let replaced = in_parallel(&|at| {
    if !ready[at] {
      return Ok(());
    }
    let file = &planned[at];
    write_atomic(&file.path, &file.text)
      .map_err(|e| format!("Cannot write {}: {}", file.path.display(), e))
  });
  for (result, replaced) in results.iter_mut().zip(replaced) {
    if result.is_ok() {
      *result = replaced;
    }
  }
  results
}

#[allow(clippy::too_many_arguments)]
fn write_report(
  guard: &Guard,
  run: &str,
  command: &str,
  before: &Said,
  after: &Said,
  narrowed: &Narrowed,
  planned: &[Planned],
  last_failure: &Said,
) {
  let text = report::render(&report::Report {
    run,
    command,
    check: &guard.check,
    before,
    after,
    narrowed,
    paths: &planned.iter().map(|p| p.path.clone()).collect::<Vec<_>>(),
    last_failure,
  });
  let name = if run.is_empty() { "latest" } else { run };
  let path = guard.dir.join("reports").join(format!("{}.md", name));
  let saved =
    std::fs::create_dir_all(guard.dir.join("reports")).and_then(|_| std::fs::write(&path, text));
  match saved {
    Ok(()) => say(guard, &format!("report: {}", path.display())),
    Err(e) => errln!(
      "{}warning: cannot write the report {}: {}{}",
      c_yellow(true),
      path.display(),
      e,
      c_reset(true)
    ),
  }
}
