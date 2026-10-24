//! The report of a narrowing, in Markdown, for the user and for a bug report.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::Narrowed;
use super::check::{Check, Said};

/// What a report tells.
pub struct Report<'a> {
  pub run: &'a str,
  /// The command line of the run.
  pub command: &'a str,
  pub check: &'a Check,
  pub before: &'a Said,
  pub after: &'a Said,
  pub narrowed: &'a Narrowed,
  /// The files of the run, in the order the narrowing numbers them.
  pub paths: &'a [PathBuf],
  /// The last failure seen, for when nothing could be held back.
  pub last_failure: &'a Said,
}

/// Removes from `dir` the reports of runs history no longer keeps, so the reports stay
/// within history's limits. A report is named by its run's id; `latest.md`, for a run not
/// recorded, is replaced by the next one and stays. The report of a run a remembered hold
/// names stays while the hold does, and one younger than history's grace period stays, since
/// a run going on at the same time may not be recorded yet. Returns how many went, and their
/// bytes.
pub fn prune(
  dir: &Path,
  runs: &HashSet<String>,
  held: &HashSet<String>,
) -> io::Result<(usize, u64)> {
  let entries = match fs::read_dir(dir) {
    Ok(entries) => entries,
    Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((0, 0)),
    Err(e) => return Err(e),
  };
  let (mut count, mut bytes) = (0, 0);
  for entry in entries {
    let entry = entry?;
    let name = entry.file_name().to_string_lossy().to_string();
    let Some(run) = name.strip_suffix(".md") else {
      continue;
    };
    if !is_run_id(run) || runs.contains(run) || held.contains(run) {
      continue;
    }
    let meta = entry.metadata()?;
    let young = meta
      .modified()
      .ok()
      .and_then(|m| m.elapsed().ok())
      .is_none_or(|age| age < crate::history::GRACE);
    if young {
      continue;
    }
    fs::remove_file(entry.path())?;
    count += 1;
    bytes += meta.len();
  }
  Ok((count, bytes))
}

pub fn render(report: &Report) -> String {
  let mut out = format!(
    "# mto doctor: run {}\n\n",
    if report.run.is_empty() {
      "(not recorded)"
    } else {
      report.run
    }
  );
  out.push_str(&format!("- mto {}\n", crate::version::short()));
  out.push_str(&format!("- Run: `{}`\n", report.command));
  out.push_str(&format!(
    "- Check: `{}` in `{}`\n",
    report.check.line(),
    report.check.dir.display()
  ));
  out.push_str(&format!("- Before the run: {}\n", report.before.summary()));
  out.push_str(&format!("- After it: {}\n", report.after.summary()));
  let narrowed = report.narrowed;
  if !narrowed.passed {
    out.push_str(
      "\nThe check still fails with every edit held back, so the run did not break \
       the build: the command fails on its own, or reads something else that changed. \
       The files are as they were before the run.\n",
    );
    if let Some(first) = &report.last_failure.first {
      out.push_str(&format!("\nIts last failure:\n\n```text\n{}```\n", first));
    }
    return out;
  }
  out.push_str(&format!("\n## Held back ({})\n", narrowed.found.len()));
  if narrowed.ran_out {
    out.push_str(
      "\nThe checks ran out (`doctor.max_checks`) before every edit was narrowed \
       down, so every changed file is held back whole: the files are as they were \
       before the run, which the check before passed.\n",
    );
  } else {
    out.push_str("\nEvery other edit of the run is written, and the check passes with them.\n");
  }
  for (n, found) in narrowed.found.iter().enumerate() {
    let (what, shown) = found.what.split_once('\n').unwrap_or((&found.what, ""));
    out.push_str(&format!("\n### {}. {}\n\n", n + 1, what));
    out.push_str(&format!(
      "- File: `{}`\n",
      report.paths[found.file].display()
    ));
    if !shown.is_empty() {
      out.push_str(&format!(
        "- Lines as formatted:\n\n```text\n{}\n```\n\n",
        shown
      ));
    }
    if !found.errors.is_empty() {
      out.push_str("- The errors the build gave with it:\n\n```text\n");
      for error in &found.errors {
        out.push_str(error);
        out.push('\n');
      }
      out.push_str("```\n\n");
    }
    out.push_str(&format!("- To keep it without the guard: {}.\n", found.fix));
  }
  out.push_str(
    "\nA held edit is held again on later runs, until mto or the text changes; \
     `mto doctor holds` lists them and `mto doctor release` drops one.\n",
  );
  out
}

/// Whether `name` is a run id (a UUID), so that no other file is removed.
fn is_run_id(name: &str) -> bool {
  name.len() == 36
    && name.char_indices().all(|(i, c)| {
      if [8, 13, 18, 23].contains(&i) {
        c == '-'
      } else {
        c.is_ascii_hexdigit()
      }
    })
}
