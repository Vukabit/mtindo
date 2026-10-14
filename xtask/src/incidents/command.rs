//! `cargo xtask incidents`: the board, one incident, and what can be done with it.
//!
//! - `list [--kind KIND] [--status STATUS]`: the incidents, open first.
//! - `show ID`: one incident's page.
//! - `mark ID STATUS [--note TEXT]`: set its status (`open`, `flaky`, `fixed`, `wont-fix`).
//! - `replay ID`: run its input again and say what happened.
//! - `recheck [ID]`: replay every fuzz incident (or one) at this tree: one that no longer
//!   fails is marked fixed here, a fixed one that fails again is reopened. Exits 1 while
//!   any still fails or could not be replayed.
//! - `minimize ID`: shrink its input with libFuzzer, keeping it failing (`minimized`).
//! - `promote ID`: keep its input as a regression test under `tests/fuzz_regressions/`,
//!   once it no longer fails, and record the test as its guard.
//! - `board`: write `.dev/incidents/INCIDENTS.md` and every incident's page again.
//!
//! Incidents other than fuzz ones are rechecked by what found them: `cargo xtask diff`
//! for meaning, unstable, panic and compat-break incidents, the `domain_13_rw` task for
//! compile breaks.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::{Incident, all, find, mark, note, short};
use crate::cli::{text, texts};
use crate::common::log::Logger;
use crate::common::{process, tree};
use crate::fuzz::{self, Replayer};

/// Where promoted inputs live, one folder per fuzz target.
pub const REGRESSIONS: &str = "tests/fuzz_regressions";

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "incidents");
  log.info(&format!("cargo xtask {}", crate::cli::line()));
  migrate(root, &log);
  match m.subcommand() {
    None => list(root, &[], &[]),
    Some(("list", sub)) => list(root, &texts(sub, "kind"), &texts(sub, "status")),
    Some(("board", _)) => {
      for (dir, incident) in all(root) {
        let _ = super::save(root, &dir, &incident);
      }
      let _ = super::write_board(root);
      println!("{}", super::root_dir(root).join("INCIDENTS.md").display());
      ExitCode::SUCCESS
    }
    Some(("recheck", sub)) => recheck(root, text(sub, "id").as_deref(), &log, &fuzz::replay),
    Some((action, sub)) => {
      let id = text(sub, "id").unwrap_or_default();
      let Some((dir, incident)) = find(root, &id) else {
        eprintln!("no incident matches `{}` (cargo xtask incidents list)", id);
        return ExitCode::from(2);
      };
      match action {
        "show" => {
          print!(
            "{}",
            fs::read_to_string(dir.join("README.md")).unwrap_or_default()
          );
          ExitCode::SUCCESS
        }
        "mark" => {
          let mut incident = incident;
          let status = text(sub, "status").unwrap_or_default();
          let here = tree::identity(root);
          mark(root, &dir, &mut incident, &status, &here);
          if let Some(words) = text(sub, "note") {
            note(
              root,
              &dir,
              &mut incident,
              &here,
              &format!("note: {}", words),
            );
          }
          println!("{} is {}", short(&incident), incident.status);
          ExitCode::SUCCESS
        }
        "replay" => replay_one(root, &dir, &incident),
        "minimize" => minimize(root, &dir, incident, &log),
        _ => promote(root, &dir, incident, &log, &fuzz::replay),
      }
    }
  }
}

fn file_len(path: &Path) -> u64 {
  fs::metadata(path).map_or(0, |m| m.len())
}

/// The input an incident is replayed with: the minimized one if any, else its first.
fn input_of(dir: &Path) -> PathBuf {
  let minimized = dir.join("minimized");
  if minimized.exists() {
    minimized
  } else {
    dir.join("input")
  }
}

fn list(root: &Path, kinds: &[String], statuses: &[String]) -> ExitCode {
  let mut incidents: Vec<(PathBuf, Incident)> = all(root)
    .into_iter()
    .filter(|(_, i)| kinds.is_empty() || kinds.contains(&i.kind))
    .filter(|(_, i)| statuses.is_empty() || statuses.contains(&i.status))
    .collect();
  incidents.reverse();
  let rank = |s: &str| match s {
    "open" => 0,
    "flaky" => 1,
    "fixed" => 2,
    _ => 3,
  };
  incidents.sort_by_key(|(_, i)| rank(&i.status));
  if incidents.is_empty() {
    println!("no incidents");
    return ExitCode::SUCCESS;
  }
  println!("STATUS    KIND              ID         TARGET              SEEN  WHAT IT SAID");
  for (_, i) in &incidents {
    let title: String = i.title.chars().take(70).collect();
    println!(
      "{:<9} {:<17} {:<10} {:<19} {:>4}  {}",
      i.status,
      i.kind,
      short(i),
      i.target,
      i.occurrences.len(),
      title
    );
  }
  ExitCode::SUCCESS
}

/// Moves crashes kept the old way (`.dev/incidents/fuzz/<target>/<when>-<kind>-<hash>/`
/// with `info.txt`, and `INDEX.tsv`) into incidents, once.
fn migrate(root: &Path, log: &Logger) {
  let old = super::root_dir(root).join("fuzz");
  if !old.join("INDEX.tsv").exists() {
    return;
  }
  let mut moved = 0;
  for target in fs::read_dir(&old).into_iter().flatten().flatten() {
    if !target.path().is_dir() {
      continue;
    }
    let target_name = target.file_name().to_string_lossy().into_owned();
    for crash in fs::read_dir(target.path()).into_iter().flatten().flatten() {
      let info = fs::read_to_string(crash.path().join("info.txt")).unwrap_or_default();
      let Ok(input) = fs::read(crash.path().join("input")) else {
        continue;
      };
      let field = |name: &str| {
        info
          .lines()
          .find_map(|l| l.strip_prefix(&format!("{}:", name)))
          .map(|v| v.trim().to_string())
          .unwrap_or_default()
      };
      let said: String = info
        .split("what it said when it crashed:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\nfirst bytes").next())
        .unwrap_or("")
        .lines()
        .map(|l| l.strip_prefix("  ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n");
      let kind = fuzz::kind_of(&format!("{}-x", field("kind")));
      let commit = field("commit");
      let tree = commit.split(' ').next().unwrap_or("").to_string();
      let report = super::Report {
        kind,
        target: &target_name,
        said: &said,
        input: Some(&input),
        replay: "",
        tree: &tree,
        run: "",
        key: None,
      };
      let Ok(filed) = super::file(root, &report) else {
        continue;
      };
      if let Some((dir, mut incident)) = find(root, filed.id()) {
        incident.replay = format!("cargo xtask incidents replay {}", short(&incident));
        incident.occurrences[0].when = field("crashed");
        let status = field("status");
        note(
          root,
          &dir,
          &mut incident,
          &tree,
          &format!("kept before incidents existed; last replay: {}", status),
        );
        if status.starts_with("no longer fails") {
          let at = status.rsplit(" at ").next().unwrap_or("").to_string();
          mark(root, &dir, &mut incident, "fixed", &at);
        }
      }
      moved += 1;
    }
  }
  let _ = fs::remove_dir_all(&old);
  log.info(&format!(
    "moved {} crash(es) kept the old way into incidents",
    moved
  ));
  println!("moved {} crash(es) kept the old way into incidents", moved);
}

/// Shrinks a fuzz incident's input with libFuzzer's minimizer, which keeps the input
/// failing; the result is `minimized`.
fn minimize(root: &Path, dir: &Path, mut incident: Incident, log: &Logger) -> ExitCode {
  if !replayable(&incident) {
    println!("only fuzz incidents with a known target can be minimized");
    return ExitCode::from(2);
  }
  let input = dir.join("input");
  let out = dir.join("minimized");
  let args: Vec<String> = vec![
    "+nightly".into(),
    "fuzz".into(),
    "tmin".into(),
    incident.target.clone(),
    input.display().to_string(),
    "--".into(),
    format!("-exact_artifact_path={}", out.display()),
    "-rss_limit_mb=2048".into(),
  ];
  println!(
    "minimizing {} ({} bytes)...",
    short(&incident),
    file_len(&input)
  );
  let (code, output) = process::capture(root, "cargo", &args, &[], process::LONG);
  log.info(&format!("tmin exit {}:\n{}", code, output));
  if !out.exists() {
    println!(
      "minimizing did not produce an input (exit {}); see .dev/logs/incidents.log",
      code
    );
    return ExitCode::from(1);
  }
  let result = format!(
    "minimized from {} to {} bytes",
    file_len(&input),
    file_len(&out)
  );
  println!("{}", result);
  note(root, dir, &mut incident, &tree::identity(root), &result);
  ExitCode::SUCCESS
}

/// Keeps a fixed fuzz incident's input as a regression test: copied to
/// `tests/fuzz_regressions/<target>/<signature>.bin`, where `tests/fuzz_regressions.rs` runs
/// every file through its target's checks.
fn promote(
  root: &Path,
  dir: &Path,
  mut incident: Incident,
  log: &Logger,
  replayer: &Replayer,
) -> ExitCode {
  if !replayable(&incident) {
    println!("only fuzz incidents with a known target can be promoted");
    return ExitCode::from(2);
  }
  let input = input_of(dir);
  let (status, _) = replayer(root, &incident.target, &input);
  if status != "no longer fails" {
    println!(
      "{} {} at this tree: fix it first, then promote it",
      short(&incident),
      status
    );
    return ExitCode::from(1);
  }
  let here = tree::identity(root);
  let stem = incident.signature[..16.min(incident.signature.len())].to_string();
  let folder = root.join(REGRESSIONS).join(&incident.target);
  // Every input the incident kept, and the minimized one: each showed the failure.
  let mut inputs: Vec<PathBuf> = fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .map(|e| e.path())
    .filter(|p| {
      p.file_name()
        .is_some_and(|n| n == "minimized" || n.to_string_lossy().starts_with("input"))
    })
    .collect();
  inputs.sort();
  for (n, input) in inputs.iter().enumerate() {
    let name = if n == 0 {
      format!("{}.bin", stem)
    } else {
      format!("{}-{}.bin", stem, n + 1)
    };
    let copied = fs::create_dir_all(&folder).and_then(|_| fs::copy(input, folder.join(&name)));
    if let Err(e) = copied {
      println!("cannot copy to {}: {}", folder.join(&name).display(), e);
      return ExitCode::from(1);
    }
  }
  let name = format!("{}*.bin", stem);
  let relative = format!("{}/{}/{}", REGRESSIONS, incident.target, name);
  incident.regression_test = relative.clone();
  if incident.status != "fixed" {
    mark(root, dir, &mut incident, "fixed", &here);
  }
  note(
    root,
    dir,
    &mut incident,
    &here,
    &format!("promoted to {}", relative),
  );
  log.info(&format!("promoted {} to {}", incident.id, relative));
  println!(
    "promoted {} to {} (add it to git; `cargo test --test fuzz_regressions` runs it)",
    short(&incident),
    relative
  );
  ExitCode::SUCCESS
}

/// Replays every fuzz incident (or the one `only` names) and moves its status with the
/// result.
fn recheck(root: &Path, only: Option<&str>, log: &Logger, replayer: &Replayer) -> ExitCode {
  let here = tree::identity(root);
  let (mut fails, mut fixed, mut unknown) = (0, 0, 0);
  for (dir, mut incident) in all(root) {
    if !replayable(&incident) || incident.status == "wont-fix" {
      continue;
    }
    if only.is_some_and(|id| !incident.id.ends_with(id) && !incident.signature.starts_with(id)) {
      continue;
    }
    let (status, _) = replayer(root, &incident.target, &input_of(&dir));
    match status {
      "fails" => {
        fails += 1;
        if incident.status == "fixed" {
          mark(root, &dir, &mut incident, "open", &here);
        }
      }
      "no longer fails" => {
        fixed += 1;
        if incident.status == "open" || incident.status == "flaky" {
          mark(root, &dir, &mut incident, "fixed", &here);
        }
      }
      _ => unknown += 1,
    }
    note(
      root,
      &dir,
      &mut incident,
      &here,
      &format!("recheck: {}", status),
    );
    println!(
      "{:<10} {:<14} {:<9} {}",
      short(&incident),
      incident.kind,
      incident.status,
      status
    );
  }
  let summary = format!(
    "{} still fail, {} no longer fail, {} could not be replayed",
    fails, fixed, unknown
  );
  println!("{}", summary);
  log.info(&format!("recheck: {}", summary));
  // A replay that did not run proves nothing, so it does not pass either.
  if fails + unknown > 0 {
    ExitCode::from(1)
  } else {
    ExitCode::SUCCESS
  }
}

fn replay_one(root: &Path, dir: &Path, incident: &Incident) -> ExitCode {
  if !replayable(incident) {
    println!(
      "{} is replayed by what found it ({}), not here",
      short(incident),
      incident.kind
    );
    return ExitCode::from(2);
  }
  let (status, said) = fuzz::replay(root, &incident.target, &input_of(dir));
  println!("{}", said.join("\n"));
  println!("{}: {}", short(incident), status);
  let mut incident = incident.clone();
  note(
    root,
    dir,
    &mut incident,
    &tree::identity(root),
    &format!("replay: {}", status),
  );
  if status == "no longer fails" {
    ExitCode::SUCCESS
  } else {
    ExitCode::from(1)
  }
}

fn replayable(incident: &Incident) -> bool {
  incident.kind.starts_with("fuzz-") && incident.target != "unknown-target"
}

#[cfg(test)]
mod tests {
  use super::*;

  struct Scratch(PathBuf);

  impl Scratch {
    fn new(name: &str) -> Scratch {
      let dir = std::env::temp_dir().join(format!("xtask-incmd-{}-{}", name, std::process::id()));
      let _ = fs::remove_dir_all(&dir);
      fs::create_dir_all(&dir).unwrap();
      Scratch(dir)
    }
  }

  impl Drop for Scratch {
    fn drop(&mut self) {
      let _ = fs::remove_dir_all(&self.0);
    }
  }

  fn fails(_: &Path, _: &str, _: &Path) -> (&'static str, Vec<String>) {
    ("fails", vec!["panicked at x".to_string()])
  }

  fn filed(root: &Path) -> (PathBuf, Incident) {
    let report = super::super::Report {
      kind: "fuzz-crash",
      target: "format",
      said: "thread 'x' panicked at src/a.rs:1:1:\nboom\n",
      input: Some(b"input bytes"),
      replay: "",
      tree: "t1",
      run: "",
      key: None,
    };
    let id = super::super::file(root, &report).unwrap().id().to_string();
    find(root, &id).unwrap()
  }

  fn fixed(_: &Path, _: &str, _: &Path) -> (&'static str, Vec<String>) {
    ("no longer fails", vec!["Executed input".to_string()])
  }

  #[test]
  fn test_old_crash_folders_become_incidents() {
    let scratch = Scratch::new("migrate");
    let root = &scratch.0;
    let log = Logger::new(root, "incidents");
    let old = super::super::root_dir(root).join("fuzz");
    let crash = old.join("format").join("20261011-162040-crash-20b00cad");
    fs::create_dir_all(&crash).unwrap();
    fs::write(old.join("INDEX.tsv"), "crashed\ttarget\n").unwrap();
    fs::write(crash.join("input"), b"bytes").unwrap();
    fs::write(
      crash.join("info.txt"),
      "target:   format\nkind:     crash\ncrashed:  2026-10-11T16:20:40.752Z\n\
       commit:   0769733-dirty (when filed)\nstatus:   no longer fails at 7bb39cb\n\n\
       what it said when it crashed:\n  thread 'x' panicked at src/a.rs:3:1:\n  boom\n\n\
       first bytes, hex:\n  62\n",
    )
    .unwrap();
    migrate(root, &log);
    assert!(!old.exists());
    let all = all(root);
    assert_eq!(all.len(), 1);
    let incident = &all[0].1;
    assert_eq!(
      (incident.kind.as_str(), incident.status.as_str()),
      ("fuzz-crash", "fixed")
    );
    assert_eq!(incident.location, "src/a.rs:3");
    assert_eq!(incident.occurrences[0].when, "2026-10-11T16:20:40.752Z");
    assert_eq!(incident.occurrences[0].tree, "0769733-dirty");
    assert_eq!(incident.fixed_in, "7bb39cb");
  }

  #[test]
  fn test_promote_only_what_no_longer_fails() {
    let scratch = Scratch::new("promote");
    let root = &scratch.0;
    let log = Logger::new(root, "incidents");
    let (dir, incident) = filed(root);
    assert_eq!(
      promote(root, &dir, incident.clone(), &log, &fails),
      ExitCode::from(1)
    );
    assert_eq!(
      promote(root, &dir, incident.clone(), &log, &fixed),
      ExitCode::SUCCESS
    );
    let now = find(root, &incident.id).unwrap().1;
    assert_eq!(now.status, "fixed");
    let first = now.regression_test.replace('*', "");
    assert_eq!(fs::read(root.join(first)).unwrap(), b"input bytes");
    assert!(
      now
        .regression_test
        .starts_with("tests/fuzz_regressions/format/")
    );
  }

  #[test]
  fn test_recheck_moves_status_both_ways() {
    let scratch = Scratch::new("recheck");
    let root = &scratch.0;
    let log = Logger::new(root, "incidents");
    let (_, incident) = filed(root);
    assert_eq!(recheck(root, None, &log, &fixed), ExitCode::SUCCESS);
    let now = find(root, &incident.id).unwrap().1;
    assert_eq!(now.status, "fixed");
    assert!(!now.fixed_in.is_empty());
    assert_eq!(recheck(root, None, &log, &fails), ExitCode::from(1));
    assert_eq!(find(root, &incident.id).unwrap().1.status, "open");
  }
}
