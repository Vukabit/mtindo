//! `mto __complete SLOT`: what only the machine knows, for the shells' scripts to complete
//! (see [`Live`]). One `value<TAB>description` line each, the likeliest first; nothing,
//! and success, when there is nothing to offer or it cannot be read, so a shell never
//! shows an error mid-word. Hidden: not in help or the manual, and its lines are the
//! scripts' to read, not a person's.
//!
//! A script newer than the mto it runs is safe: an older mto takes `__complete` for a path
//! that does not exist and stops before it reads anything (exit code 2, its error thrown
//! away by the script). A change to the lines' form takes a new word (`__complete2`), so
//! an older script never misreads them.

use std::path::Path;

use super::history::{shorten, utc};
use super::spec::Live;
use crate::history::{History, id};
use crate::report::Store;

/// How many runs and commits are offered: the newest, which are the ones asked for.
const NEWEST: usize = 20;

/// The lines `mto __complete` prints for `live`.
pub fn lines(live: Live, history: Option<&History>, reports: Option<&Store>, dir: &Path) -> String {
  let found = match live {
    Live::Runs => history.map(runs).unwrap_or_default(),
    Live::Reports => reports.map(report_ids).unwrap_or_default(),
    Live::Revs => revs(dir),
  };
  let mut out = String::new();
  for (value, description) in found {
    // A tab or line break in either would split the line where the scripts do not.
    let clean = |text: &str| text.replace(['\t', '\n', '\r'], " ");
    out.push_str(&format!("{}\t{}\n", clean(&value), clean(&description)));
  }
  out
}

/// The kept reports' ids, the most recently seen first, each with what broke, where, how
/// often and when last.
fn report_ids(store: &Store) -> Vec<(String, String)> {
  store
    .list()
    .into_iter()
    .map(|entry| {
      let what = entry
        .title
        .strip_prefix("verifier: ")
        .unwrap_or(&entry.title);
      let file = entry.files.first().map_or(String::new(), |seen| {
        format!(" in {}", tail(&seen.shown, 32))
      });
      let when = utc(entry.last);
      (
        entry.id[..6].to_string(),
        format!(
          "{}{}, {}, last {}",
          what,
          file,
          times(entry.count),
          &when[..16]
        ),
      )
    })
    .collect()
}

/// `HEAD`, the branches and tags (the most recently committed to first), then the newest
/// commits, each with what it is.
fn revs(dir: &Path) -> Vec<(String, String)> {
  let git = |args: &[&str]| super::git_files::git(dir, args).unwrap_or_default();
  let refs = git(&[
    "for-each-ref",
    "--sort=-committerdate",
    "--format=%(refname:short)%09%(refname)",
    "refs/heads",
    "refs/tags",
    "refs/remotes",
  ]);
  if refs.is_empty() && git(&["rev-parse", "--git-dir"]).is_empty() {
    return Vec::new();
  }
  let mut out = vec![("HEAD".to_string(), "the commit checked out".to_string())];
  for line in refs.lines() {
    let Some((short, full)) = line.split_once('\t') else {
      continue;
    };
    let kind = if full.starts_with("refs/heads/") {
      "branch"
    } else if full.starts_with("refs/tags/") {
      "tag"
    } else if full.ends_with("/HEAD") {
      continue;
    } else {
      "remote branch"
    };
    out.push((short.to_string(), kind.to_string()));
  }
  let count = format!("-{}", NEWEST);
  for line in git(&["log", &count, "--format=%h%x09%s"]).lines() {
    if let Some((hash, subject)) = line.split_once('\t') {
      out.push((hash.to_string(), shorten(subject, 60)));
    }
  }
  out
}

/// `last`, then the newest runs' handles, each with when it ran, how many files it wrote
/// and its command.
fn runs(history: &History) -> Vec<(String, String)> {
  let Ok(ids) = history.ids() else {
    return Vec::new();
  };
  if ids.is_empty() {
    return Vec::new();
  }
  let mut out = vec![("last".to_string(), "this project's newest run".to_string())];
  for run_id in ids.iter().rev().take(NEWEST) {
    let Ok(run) = history.read(run_id) else {
      continue;
    };
    let when = utc(id::millis(run_id));
    let files = match run.entries.len() {
      1 => "1 file".to_string(),
      n => format!("{} files", n),
    };
    out.push((
      id::handle(run_id, &ids),
      format!("{}  {}  {}", &when[..16], files, shorten(&run.command, 50)),
    ));
  }
  out
}

/// `path`'s end, at most about `width` characters: whole components after `.../`, or the
/// file's name alone when that is longer.
fn tail(path: &str, width: usize) -> String {
  if path.chars().count() <= width {
    return path.to_string();
  }
  let parts: Vec<&str> = path.split(['/', '\\']).collect();
  let mut kept = parts[parts.len() - 1].to_string();
  for part in parts[..parts.len() - 1].iter().rev() {
    if kept.chars().count() + part.chars().count() + 5 > width {
      break;
    }
    kept = format!("{}/{}", part, kept);
  }
  format!(".../{}", kept)
}

fn times(count: u64) -> String {
  match count {
    1 => "seen once".to_string(),
    n => format!("seen {} times", n),
  }
}
