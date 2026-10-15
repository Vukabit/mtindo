//! `pairs`: the engine of `cargo xtask pairs`, the scrambled-pair suite. Files of each crate
//! named are drawn by the seed and scrambled ([`crate::scramble`]); each scrambled file is
//! formatted by mto (formatting only, as `cargo xtask diff` scores it) and by the standard
//! formatter (as the harness runs it: the crate's edition and nearest configuration file,
//! an empty home folder), and scored by files and lines matching.
//!
//! ```text
//! pairs --seed N --out DIR [--files N] NAME=DIR...
//! ```
//!
//! writes `DIR/scrambled/NAME/FILE`, `DIR/mto/NAME/FILE` and `DIR/standard/NAME/FILE` (the
//! scrambled text where the standard formatter refused it), for `cargo xtask pairs` to
//! build the copies it compiles, and prints one line per file drawn:
//!
//! ```text
//! file<TAB>NAME<TAB>FILE<TAB>MOVED<TAB>MATCHED<TAB>LINES<TAB>SAME
//! refused<TAB>NAME<TAB>FILE<TAB>WHY      (the standard formatter refused or did not run)
//! skipped<TAB>NAME<TAB>FILE<TAB>WHY      (not scrambled: syntax errors, carriage returns)
//! scrambler-bug<TAB>NAME<TAB>FILE<TAB>WHAT
//! panic<TAB>NAME<TAB>FILE<TAB>MESSAGE    (mto panicked)
//! ```
//!
//! The files a seed draws from a crate, and how each is scrambled, depend only on the seed,
//! the crate and the number of files, so one crate of a run can be replayed alone.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::format::file::format_file;
use mtindo::options::format::FormatConfig;

use crate::common::edition_for;
use crate::diff::{Answer, common_lines, edition_year, fence, panic_message, run_standard};
use crate::scramble::{Outcome, Random, scramble};

/// One file to scramble and score.
struct Job {
  name: String,
  dir: PathBuf,
  relative: PathBuf,
  seed: u64,
}

struct Options {
  seed: u64,
  out: PathBuf,
  /// Files drawn per crate; 0 for every file.
  files: usize,
  crates: Vec<(String, PathBuf)>,
}

pub fn run(args: &[String]) -> ExitCode {
  let options = match parse_args(args) {
    Ok(options) => options,
    Err(e) => {
      eprintln!("xtask-tools pairs: {}", e);
      return ExitCode::from(2);
    }
  };
  let mut jobs = Vec::new();
  for (name, dir) in &options.crates {
    if let Some(downloads) = dir.parent().and_then(Path::parent) {
      fence(downloads);
    }
    for relative in draw(dir, name, options.seed, options.files) {
      let seed = file_seed(options.seed, name, &relative);
      jobs.push(Job {
        name: name.clone(),
        dir: dir.clone(),
        relative,
        seed,
      });
    }
  }
  let home = options.out.join("home");
  let _ = fs::create_dir_all(&home);
  let next = AtomicUsize::new(0);
  let lines: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
  let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
  std::thread::scope(|scope| {
    for _ in 0..threads {
      scope.spawn(|| {
        loop {
          let at = next.fetch_add(1, Ordering::Relaxed);
          let Some(job) = jobs.get(at) else {
            break;
          };
          let line = pair(job, &options.out, &home);
          lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((at, line));
        }
      });
    }
  });
  let mut lines = lines.into_inner().unwrap_or_else(|e| e.into_inner());
  lines.sort();
  for (_, line) in lines {
    println!("{}", line);
  }
  ExitCode::SUCCESS
}

/// mto's formatting settings for files in `folder`, as the harness resolves them: the
/// stable ones, since the answer is the stable standard formatter's.
fn config_for(folder: &Path) -> FormatConfig {
  Resolver::new(&Overrides::default(), Environment::default())
    .ok()
    .and_then(|mut r| r.config_for(folder).ok())
    .map(|c| c.stable_only().format())
    .unwrap_or_default()
}

/// The files `seed` draws from the crate in `dir`: `count` of them (all for 0), as paths
/// relative to `dir`, sorted.
fn draw(dir: &Path, name: &str, seed: u64, count: usize) -> Vec<PathBuf> {
  let mut files = Vec::new();
  crate::corpus::collect(dir, &mut files);
  let mut files: Vec<PathBuf> = files
    .into_iter()
    .filter_map(|f| f.strip_prefix(dir).ok().map(Path::to_path_buf))
    .collect();
  files.sort();
  if count > 0 && count < files.len() {
    let mut random = Random::new(seed ^ fnv(name.as_bytes()));
    random.shuffle(&mut files);
    files.truncate(count);
    files.sort();
  }
  files
}

/// The seed one file is scrambled with: from the run's seed, the crate and the file alone.
fn file_seed(seed: u64, name: &str, relative: &Path) -> u64 {
  let mut key = seed.to_le_bytes().to_vec();
  key.extend(name.as_bytes());
  key.push(0);
  key.extend(relative.to_string_lossy().as_bytes());
  fnv(&key)
}

/// FNV-1a.
fn fnv(bytes: &[u8]) -> u64 {
  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  for byte in bytes {
    hash ^= u64::from(*byte);
    hash = hash.wrapping_mul(0x0100_0000_01b3);
  }
  hash
}

/// Scrambles one file, formats it both ways and scores it: its line of the output.
fn pair(job: &Job, out: &Path, home: &Path) -> String {
  let file = job.dir.join(&job.relative);
  let shown = job.relative.display().to_string();
  let line = |what: &str, rest: &str| {
    format!(
      "{}\t{}\t{}\t{}",
      what,
      job.name,
      shown,
      rest.lines().next().unwrap_or("").trim()
    )
  };
  let Ok(text) = fs::read_to_string(&file) else {
    return line("skipped", "not UTF-8");
  };
  let edition = edition_for(&file, &text);
  let (messy, moved) = match scramble(&text, edition, job.seed) {
    Outcome::Scrambled { text, moved } => (text, moved),
    Outcome::Skipped(why) => return line("skipped", &why),
    Outcome::Bug(what) => return line("scrambler-bug", &what),
  };
  let write = |side: &str, text: &str| {
    let path = out.join(side).join(&job.name).join(&job.relative);
    if let Some(parent) = path.parent() {
      let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, text);
  };
  write("scrambled", &messy);
  let folder = file.parent().unwrap_or(Path::new("."));
  let answer = match run_standard(&messy, edition_year(edition), folder, home) {
    Answer::Text(answer) => answer,
    Answer::Refused(why) | Answer::NotRun(why) => {
      write("standard", &messy);
      return line("refused", &why);
    }
  };
  write("standard", &answer);
  let config = config_for(folder);
  let output = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    format_file(&messy, edition, &config)
  })) {
    Ok(output) => output,
    Err(payload) => return line("panic", &panic_message(&*payload)),
  };
  write("mto", &output);
  let want: Vec<&str> = answer.split('\n').collect();
  let got: Vec<&str> = output.split('\n').collect();
  let same = output == answer;
  let matched = if same {
    want.len()
  } else {
    common_lines(&got, &want)
  };
  format!(
    "file\t{}\t{}\t{}\t{}\t{}\t{}",
    job.name,
    shown,
    moved,
    matched,
    want.len(),
    u8::from(same)
  )
}

fn parse_args(args: &[String]) -> Result<Options, String> {
  let mut options = Options {
    seed: 0,
    out: PathBuf::new(),
    files: 0,
    crates: Vec::new(),
  };
  let mut seeded = false;
  let mut at = 0;
  while at < args.len() {
    let value = || {
      args
        .get(at + 1)
        .ok_or(format!("{} needs a value", args[at]))
    };
    match args[at].as_str() {
      "--seed" => {
        options.seed = value()?.parse().map_err(|_| "--seed takes a number")?;
        seeded = true;
      }
      "--out" => options.out = PathBuf::from(value()?),
      "--files" => options.files = value()?.parse().map_err(|_| "--files takes a number")?,
      other => {
        let (name, dir) = other
          .split_once('=')
          .ok_or(format!("unexpected `{}`", other))?;
        options.crates.push((name.to_string(), PathBuf::from(dir)));
        at += 1;
        continue;
      }
    }
    at += 2;
  }
  if !seeded || options.out.as_os_str().is_empty() || options.crates.is_empty() {
    return Err("usage: pairs --seed N --out DIR [--files N] NAME=DIR...".to_string());
  }
  Ok(options)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
  }

  #[test]
  fn test_a_seed_draws_the_same_files_and_another_seed_others() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let first = draw(&dir, "tools", 1, 4);
    assert_eq!(first.len(), 4);
    assert_eq!(first, draw(&dir, "tools", 1, 4));
    assert!((2..10).any(|seed| draw(&dir, "tools", seed, 4) != first));
    assert!(draw(&dir, "tools", 1, 0).len() > 4);
    // A file's scramble does not depend on which others were drawn with it.
    let file = Path::new("main.rs");
    assert_eq!(file_seed(1, "tools", file), file_seed(1, "tools", file));
    assert_ne!(file_seed(1, "tools", file), file_seed(2, "tools", file));
  }

  #[test]
  fn test_arguments() {
    let options = parse_args(&args(&[
      "--seed", "42", "--out", "/o", "--files", "3", "a=/x", "b=/y",
    ]))
    .unwrap();
    assert_eq!(options.seed, 42);
    assert_eq!(options.files, 3);
    assert_eq!(options.crates[1], ("b".to_string(), PathBuf::from("/y")));
    assert!(parse_args(&args(&["--out", "/o", "a=/x"])).is_err());
    assert!(parse_args(&args(&["--seed", "x", "--out", "/o", "a=/x"])).is_err());
    assert!(parse_args(&args(&["--seed", "1", "--out", "/o", "stray"])).is_err());
  }
}
