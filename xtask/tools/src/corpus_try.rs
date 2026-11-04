//! `cargo xtask corpus try`: settings tried on the whole corpus, in process.
//!
//! Each file goes through mto as a real run would take it with those settings over the
//! crate's own configuration (layout, formatting and the verifier), then through again
//! ([`mtindo::settle`]), and every outcome is counted apart: as written, settled, left
//! alone, the check failing, a panic, a second run changing the output. Nothing is written,
//! filed or kept in history, and no user configuration is read. With `--each`, each named
//! setting is tried alone and then all together. With `--record NAME` the totals go to
//! `records/results/toggles/NAME.txt`, the proof a layout setting passed the corpus.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;

use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::settle::{Outcome, Tally, settle};

/// What one try is: a name and the settings over every file's own.
struct Try {
  name: String,
  settings: Vec<(String, String)>,
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
  let mut base: Vec<(String, String)> = Vec::new();
  let mut each: Vec<String> = Vec::new();
  let mut runs = 2;
  let mut record: Option<String> = None;
  let mut repos: Vec<(String, PathBuf)> = Vec::new();
  let mut at = 0;
  while at < args.len() {
    let arg = args[at].as_str();
    let value = args.get(at + 1).cloned().unwrap_or_default();
    match arg {
      "--config" => {
        base.extend(pairs(&value));
        at += 1;
      }
      "--each" => {
        each.extend(
          pairs(&value)
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v)),
        );
        at += 1;
      }
      "--runs" => {
        runs = value.parse().unwrap_or(2);
        at += 1;
      }
      "--record" => {
        record = Some(value);
        at += 1;
      }
      "--layout-only" => base.push(("format".to_string(), "false".to_string())),
      other => {
        if let Some((name, dir)) = other.split_once('=') {
          repos.push((name.to_string(), PathBuf::from(dir)));
        }
      }
    }
    at += 1;
  }
  if !base.iter().any(|(key, _)| key == "layout") {
    base.insert(0, ("layout".to_string(), "true".to_string()));
  }
  let mut tries = Vec::new();
  for setting in &each {
    let mut settings = base.clone();
    settings.extend(pairs(setting));
    tries.push(Try {
      name: setting.clone(),
      settings,
    });
  }
  if each.len() != 1 {
    let mut settings = base.clone();
    for setting in &each {
      settings.extend(pairs(setting));
    }
    tries.push(Try {
      name: if each.is_empty() {
        shown(&base)
      } else {
        "all together".to_string()
      },
      settings,
    });
  }
  let mut files: Vec<(String, PathBuf)> = Vec::new();
  for (name, dir) in &repos {
    let mut found = Vec::new();
    crate::corpus::collect(dir, &mut found);
    found.sort();
    files.extend(found.into_iter().map(|path| (name.clone(), path)));
  }
  let mut clean = true;
  let mut recorded = String::new();
  for one in &tries {
    println!("{} ({} files)", one.name, files.len());
    let (per_crate, failures) = try_all(&files, &one.settings, runs);
    let mut total = Tally::default();
    println!(
      "  {:<16} {:>6} {:>8} {:>6} {:>10} {:>12} {:>9} {:>9}",
      "CRATE", "FILES", "CHANGED", "MOVES", "LEFT ALONE", "CHECK FAILED", "PANICKED", "UNSETTLED"
    );
    for (name, tally) in &per_crate {
      println!("  {}", row(name, tally));
      add(&mut total, tally);
    }
    println!("  {}", row("total", &total));
    for (file, outcome) in failures.iter().take(20) {
      println!("  {}: {}", file, describe(outcome));
    }
    if failures.len() > 20 {
      println!("  ... and {} more", failures.len() - 20);
    }
    if !failures.is_empty() {
      println!(
        "  replay one: mto debug settle FILE --config {}",
        shown(&one.settings)
      );
    }
    clean &= total.clean();
    let _ = writeln!(
      recorded,
      "try  {}\nsettings  {}\ntotal  {}",
      one.name,
      shown(&one.settings),
      row("", &total).trim()
    );
  }
  if let Some(name) = record {
    let path = root
      .join("records/results/toggles")
      .join(format!("{}.txt", name));
    let header = format!(
      "# `cargo xtask corpus try` on the pinned corpus ({} runs a file), for `{}`.\n\
       # Columns: files, changed, moves, left alone, check failed, panicked, unsettled.\n",
      runs, name
    );
    let written = std::fs::create_dir_all(path.parent().expect("a folder"))
      .and_then(|_| std::fs::write(&path, format!("{}{}", header, recorded)));
    match written {
      Ok(()) => println!("recorded in {}", path.display()),
      Err(e) => {
        println!("cannot write {}: {}", path.display(), e);
        return ExitCode::FAILURE;
      }
    }
  }
  if clean {
    ExitCode::SUCCESS
  } else {
    ExitCode::FAILURE
  }
}

fn add(total: &mut Tally, one: &Tally) {
  total.files += one.files;
  total.as_written += one.as_written;
  total.settled += one.settled;
  total.left_alone += one.left_alone;
  total.check_failed += one.check_failed;
  total.panicked += one.panicked;
  total.unsettled += one.unsettled;
  total.moves += one.moves;
}

fn describe(outcome: &Outcome) -> String {
  match outcome {
    Outcome::CheckFailed(said) => format!("the check failed: {}", said),
    Outcome::Panicked(said) => format!("panicked: {}", said),
    Outcome::Unsettled { run } => format!("run {} changed it again", run),
    other => other.id().to_string(),
  }
}

/// `key=value,...` as pairs; a comma inside `[...]` belongs to a list value.
fn pairs(text: &str) -> Vec<(String, String)> {
  let mut parts = Vec::new();
  let (mut depth, mut start) = (0i32, 0);
  for (at, c) in text.char_indices() {
    match c {
      '[' => depth += 1,
      ']' => depth -= 1,
      ',' if depth == 0 => {
        parts.push(&text[start..at]);
        start = at + 1;
      }
      _ => {}
    }
  }
  parts.push(&text[start..]);
  parts
    .into_iter()
    .filter_map(|pair| pair.split_once('='))
    .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
    .collect()
}

fn row(name: &str, tally: &Tally) -> String {
  format!(
    "{:<16} {:>6} {:>8} {:>6} {:>10} {:>12} {:>9} {:>9}",
    name,
    tally.files,
    tally.settled,
    tally.moves,
    tally.left_alone,
    tally.check_failed,
    tally.panicked,
    tally.unsettled
  )
}

fn shown(settings: &[(String, String)]) -> String {
  settings
    .iter()
    .map(|(key, value)| format!("{}={}", key, value))
    .collect::<Vec<_>>()
    .join(",")
}

/// Every file of `files` settled with `settings`, several at a time: a tally per crate and
/// the files that failed, with how.
fn try_all(
  files: &[(String, PathBuf)],
  settings: &[(String, String)],
  runs: usize,
) -> (BTreeMap<String, Tally>, Vec<(String, Outcome)>) {
  let next = Mutex::new(0usize);
  let tallies: Mutex<BTreeMap<String, Tally>> = Mutex::new(BTreeMap::new());
  let failures: Mutex<Vec<(String, Outcome)>> = Mutex::new(Vec::new());
  let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
  std::thread::scope(|scope| {
    for _ in 0..workers {
      scope.spawn(|| {
        // No user configuration: only the crate's files and the settings tried.
        let overrides = Overrides {
          settings: settings.to_vec(),
          ..Overrides::default()
        };
        let Ok(mut resolver) = Resolver::new(&overrides, Environment::default()) else {
          return;
        };
        loop {
          let at = {
            let mut next = next.lock().unwrap();
            let at = *next;
            *next += 1;
            at
          };
          let Some((name, path)) = files.get(at) else {
            break;
          };
          let Ok(text) = std::fs::read_to_string(path) else {
            continue;
          };
          let dir = path.parent().unwrap_or(Path::new("."));
          let resolved = resolver.config_for(dir).ok();
          let Some(file_settings) = resolved.as_deref().and_then(mtindo::Settings::of) else {
            continue;
          };
          let done = settle(&text, &file_settings, runs);
          if matches!(
            done.outcome,
            Outcome::CheckFailed(_) | Outcome::Panicked(_) | Outcome::Unsettled { .. }
          ) {
            failures
              .lock()
              .unwrap()
              .push((path.display().to_string(), done.outcome.clone()));
          }
          tallies
            .lock()
            .unwrap()
            .entry(name.clone())
            .or_default()
            .add(&done);
        }
      });
    }
  });
  let mut failures = failures.into_inner().unwrap();
  failures.sort_by(|a, b| a.0.cmp(&b.0));
  (tallies.into_inner().unwrap(), failures)
}
