//! `cargo xtask fuzz`: runs the fuzz targets and files what they find as incidents (see
//! [`crate::incidents`]), each with what the run said at the time.
//!
//! libFuzzer writes each input it fails on as `crash-<sha1>`, `oom-<sha1>`, `timeout-<sha1>`,
//! `leak-<sha1>` or `slow-unit-<sha1>`, a name that does not say which target, when, from
//! which commit or what went wrong. Runs here aim libFuzzer at a staging folder,
//! `.dev/incidents/.incoming/<target>/`, and save the run's failure output beside the input
//! it failed on. Each input staged there, any in cargo-fuzz's default
//! `fuzz/artifacts/<target>/`, and any libFuzzer wrote to the repository root when run by
//! hand there, is then filed: a `fuzz-crash`, `fuzz-oom`, `fuzz-timeout`, `fuzz-slow` or
//! `fuzz-leak` incident, or another occurrence of the one it matches.
//!
//! - `cargo xtask fuzz run TARGET [--time SECONDS]`: fuzz one target (60 s by default), then
//!   file what it found.
//! - `cargo xtask fuzz soak [--time SECONDS] [--targets A,B]`: every target side by side for
//!   24 hours (see [`crate::fuzz_soak`]), the machine kept awake, drawn live on a terminal.
//! - `cargo xtask fuzz collect`: file whatever is waiting.
//!
//! Replaying, minimizing and promoting are `cargo xtask incidents` commands.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use crate::common::log::Logger;
use crate::common::{process, sample, store, tree};
use crate::incidents::{self, Filed, Report};

/// The sanitizer's settings for every run and replay, before any the caller sets in
/// `ASAN_OPTIONS` (later settings win). By default it keeps freed memory for good and holds
/// 256 MB of it back to catch use after free; with the `format` target's churn that passed
/// 2.7 GB in ten minutes. Returning freed memory every half second and a 32 MB quarantine
/// kept a ten-minute run near 1 GB.
const ASAN_DEFAULTS: &str = "allocator_release_to_os_interval_ms=500:quarantine_size_mb=32";

/// The kinds of input libFuzzer writes, by their names' prefixes.
const KINDS: &[&str] = &["crash-", "oom-", "timeout-", "leak-", "slow-unit-"];

/// The longest a replay may take before it counts as a timeout.
const REPLAY_TIMEOUT: &str = "-timeout=60";

/// The memory a run may use: libFuzzer's own default, written out so that raising it is a
/// decision. The target's own memory stays near 50 MB (measured without the sanitizer);
/// the rest is the sanitizer's, which [`ASAN_DEFAULTS`] keeps within this.
const RSS_LIMIT_MB: &str = "-rss_limit_mb=2048";

/// Beside a staged input: the output of the run that found it.
const SAID_SUFFIX: &str = ".said";

/// Replays an input: whether it still fails (`fails`, `no longer fails` or `replay did not
/// run`), and the end of what the run printed.
pub type Replayer = dyn Fn(&Path, &str, &Path) -> (&'static str, Vec<String>);

/// How a target is fuzzed.
#[derive(Debug, PartialEq)]
pub struct Run {
  pub seconds: u32,
  /// cargo-fuzz's `--sanitizer`: `address` (the default) or `none`, to tell the
  /// sanitizer's own memory from the target's.
  pub sanitizer: String,
  /// Fuzz on past each find for the whole time (libFuzzer's fork mode), keeping every
  /// failing input, instead of stopping at the first.
  pub keep_going: bool,
}

impl Run {
  pub fn for_seconds(seconds: u32) -> Run {
    Run {
      seconds,
      sanitizer: "address".to_string(),
      keep_going: false,
    }
  }
}

/// Files every input waiting in the staging folders, cargo-fuzz's default artifacts folder
/// and the repository root; returns how many incidents were new or reopened.
pub fn collect(root: &Path, log: &Logger) -> usize {
  collect_with(root, log, &replay)
}

/// Where `target`'s corpus is kept: the store's fuzz area (`.dev/fuzz/TARGET`), every fuzz
/// run reading and growing it. A corpus left in cargo-fuzz's own folder (`fuzz/corpus/
/// TARGET`, where corpora were kept before) is moved there first.
pub fn corpus_dir(root: &Path, target: &str) -> PathBuf {
  let store = store::dir(root, store::FUZZ);
  let old = root.join("fuzz").join("corpus");
  // Every corpus moves at once, so none is left behind in the old folder.
  if let Ok(entries) = fs::read_dir(&old) {
    let _ = fs::create_dir_all(&store);
    for entry in entries.flatten() {
      let moved = store.join(entry.file_name());
      if entry.path().is_dir() && !moved.exists() {
        let _ = fs::rename(entry.path(), moved);
      }
    }
    // The old folder goes once it holds no corpus.
    let _ = fs::remove_dir(&old);
  }
  store.join(target)
}

/// Fuzzes `target` for `seconds` with libFuzzer's inputs staged for filing and the run's
/// failure output saved beside the one it failed on, without filing them, so that several
/// targets can run at once and [`collect`] once after. Returns libFuzzer's exit code and
/// the end of what the run said.
pub fn fuzz_target(root: &Path, target: &str, run: &Run, log: &Logger) -> (i32, Vec<String>) {
  fuzz_target_tapped(root, target, run, log, None)
}

/// [`fuzz_target`], with libFuzzer's output also handed to `tap` as it arrives.
pub fn fuzz_target_tapped(
  root: &Path,
  target: &str,
  run: &Run,
  log: &Logger,
  tap: Option<process::Tap>,
) -> (i32, Vec<String>) {
  let seconds = run.seconds;
  let incoming = incoming(root).join(target);
  if let Err(e) = fs::create_dir_all(&incoming) {
    return (
      1,
      vec![format!("cannot create {}: {}", incoming.display(), e)],
    );
  }
  let before: Vec<PathBuf> = staged(&incoming);
  let corpus = corpus_dir(root, target);
  if let Err(e) = fs::create_dir_all(&corpus) {
    return (
      1,
      vec![format!("cannot create {}: {}", corpus.display(), e)],
    );
  }
  let mut args: Vec<String> = vec![
    "+nightly".into(),
    "fuzz".into(),
    "run".into(),
    format!("--sanitizer={}", run.sanitizer),
    target.into(),
    corpus.display().to_string(),
    "--".into(),
    format!("-max_total_time={}", seconds),
    RSS_LIMIT_MB.into(),
    // libFuzzer joins the prefix and the name as given, so it ends with a separator.
    format!("-artifact_prefix={}/", incoming.display()),
  ];
  if run.keep_going {
    // One child fuzzes at a time and is started again after it fails; every failing input
    // is kept where the prefix says.
    args.extend(
      [
        "-fork=1",
        "-ignore_crashes=1",
        "-ignore_ooms=1",
        "-ignore_timeouts=1",
      ]
      .map(String::from),
    );
  }
  log.info(&format!(
    "fuzzing {} for {}s, sanitizer {}",
    target, seconds, run.sanitizer
  ));
  // Built first, apart, so the samples below are the fuzzing's alone (building peaks
  // well above it).
  let build: Vec<String> = vec![
    "+nightly".into(),
    "fuzz".into(),
    "build".into(),
    format!("--sanitizer={}", run.sanitizer),
    target.into(),
  ];
  let built = process::run(root, "cargo", &build, &asan_options(), process::LONG);
  if built.code != 0 {
    return (built.code, failure_lines(&built.output));
  }
  // The run checks the build is current, then fuzzes.
  let limit = Duration::from_secs(seconds.into()) + process::LONG;
  // The run's memory and CPU, for what libFuzzer's own `rss:` cannot show: its curve.
  let samples = sample::run_path(root, &format!("fuzz-{}-{}", target, run.sanitizer));
  let (outcome, used) = process::run_tapped(
    root,
    "cargo",
    &args,
    &asan_options(),
    limit,
    Some(&samples),
    tap,
  );
  let code = outcome.code;
  let mut said = failure_lines(&outcome.output);
  let memory = match used {
    Some(used) => format!(
      "memory: {} {} (samples in {})",
      used.describe(),
      sample::curve(&samples, 40),
      relative(root, &samples)
    ),
    None => "memory: not sampled".to_string(),
  };
  log.info(&format!("fuzzed {}: exit {}; {}", target, code, memory));
  // The run's speed, from libFuzzer's last status line, and its peak memory.
  if let Some(rate) = outcome
    .output
    .lines()
    .rev()
    .find_map(|l| l.split("exec/s: ").nth(1))
    .and_then(|rest| rest.split_whitespace().next())
    .and_then(|v| v.parse::<f64>().ok())
  {
    crate::metrics::record(root, &format!("fuzz.{}.exec_per_s", target), rate, "exec/s");
  }
  if let Some(used) = used {
    crate::metrics::record(
      root,
      &format!("fuzz.{}.{}.peak_rss", target, run.sanitizer),
      used.peak_rss_kb as f64 / 1024.0,
      "MB",
    );
  }
  said.push(memory);
  // What the run said goes beside each input it left (libFuzzer stops at the first crash).
  // A run that kept going said little of each, so [`collect`] replays them instead.
  for input in staged(&incoming)
    .into_iter()
    .filter(|p| !before.contains(p) && !run.keep_going)
  {
    let note = format!(
      "run: cargo {}\nexit: {}\n\n{}\n",
      args.join(" "),
      code,
      said.join("\n")
    );
    let _ = fs::write(format!("{}{}", input.display(), SAID_SUFFIX), note);
  }
  (code, said)
}

/// The incident kind of a libFuzzer input, by its name.
pub fn kind_of(name: &str) -> &'static str {
  match name.split('-').next().unwrap_or("") {
    "oom" => "fuzz-oom",
    "timeout" => "fuzz-timeout",
    "leak" => "fuzz-leak",
    "slow" => "fuzz-slow",
    _ => "fuzz-crash",
  }
}

/// Replays an input once: whether it still fails, and the end of what the run printed.
pub fn replay(root: &Path, target: &str, input: &Path) -> (&'static str, Vec<String>) {
  let args: Vec<String> = vec![
    "+nightly".into(),
    "fuzz".into(),
    "run".into(),
    target.into(),
    input.display().to_string(),
    "--".into(),
    RSS_LIMIT_MB.into(),
    REPLAY_TIMEOUT.into(),
  ];
  let (code, output) = process::capture(root, "cargo", &args, &asan_options(), process::LONG);
  // Only what libFuzzer says counts: a clean run reports `Executed <input> in`, a failing
  // one a panic or its own error. Anything else (a build error, no nightly toolchain) means
  // the replay did not run, which proves nothing either way.
  let failed = output.contains("panicked at") || output.contains("ERROR: libFuzzer");
  let status = if failed {
    "fails"
  } else if code == 0 && output.contains("Executed ") {
    "no longer fails"
  } else {
    "replay did not run"
  };
  (status, failure_lines(&output))
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "fuzz");
  log.info(&format!("cargo xtask {}", crate::cli::line()));
  match m.subcommand() {
    Some(("run", sub)) => {
      let target = crate::cli::text(sub, "target").unwrap_or_default();
      let run = run_of(sub);
      ExitCode::from(run_target(root, &target, &run, &log) as u8)
    }
    Some(("soak", sub)) => crate::fuzz_soak::run(root, sub, &log),
    Some(("collect", _)) => {
      let filed = collect(root, &log);
      println!(
        "{} new or reopened incident(s) (cargo xtask incidents)",
        filed
      );
      ExitCode::SUCCESS
    }
    Some(("coverage", sub)) => crate::coverage::run(root, sub),
    Some(("diff", sub)) => crate::fuzz_diff::run(root, sub),
    _ => unreachable!("clap requires a subcommand"),
  }
}

/// Fuzzes `target` for `seconds` (see [`fuzz_target`]), then files what it found. The exit
/// code is libFuzzer's (0 when nothing was found), clamped to a byte.
pub fn run_target(root: &Path, target: &str, run: &Run, log: &Logger) -> i32 {
  let seconds = run.seconds;
  let (code, said) = fuzz_target(root, target, run, log);
  if code == 0 {
    println!("fuzzed {} for {}s: nothing found", target, seconds);
    if let Some(memory) = said.last() {
      println!("{}", memory);
    }
  } else {
    println!("{}", said.join("\n"));
  }
  let filed = collect(root, log);
  if filed > 0 {
    println!(
      "{} new or reopened incident(s) (cargo xtask incidents)",
      filed
    );
  }
  code.clamp(0, 255)
}

/// `ASAN_OPTIONS` for a run: [`ASAN_DEFAULTS`], then the caller's.
fn asan_options() -> Vec<(String, String)> {
  let value = match std::env::var("ASAN_OPTIONS") {
    Ok(own) if !own.is_empty() => format!("{}:{}", ASAN_DEFAULTS, own),
    _ => ASAN_DEFAULTS.to_string(),
  };
  vec![("ASAN_OPTIONS".to_string(), value)]
}

/// [`collect`], replaying with `replayer`.
fn collect_with(root: &Path, log: &Logger, replayer: &Replayer) -> usize {
  let mut waiting: Vec<(String, PathBuf)> = Vec::new();
  for dir in [incoming(root), root.join("fuzz").join("artifacts")] {
    let Ok(targets) = fs::read_dir(&dir) else {
      continue;
    };
    for target in targets.flatten() {
      waiting.extend(
        staged(&target.path())
          .into_iter()
          .map(|p| (target.file_name().to_string_lossy().into_owned(), p)),
      );
    }
  }
  // libFuzzer run by hand from the repository root writes there; its target is unknown.
  if let Ok(files) = fs::read_dir(root) {
    for file in files.flatten() {
      let name = file.file_name().to_string_lossy().into_owned();
      if file.path().is_file() && KINDS.iter().any(|kind| name.starts_with(kind)) {
        waiting.push(("unknown-target".to_string(), file.path()));
      }
    }
  }
  waiting.sort();
  let tree = tree::identity(root);
  let run = std::env::var(crate::runs::RUN_DIR)
    .ok()
    .and_then(|d| {
      Path::new(&d)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    })
    .unwrap_or_default();
  let mut filed = 0;
  for (target, path) in waiting {
    let said_path = PathBuf::from(format!("{}{}", path.display(), SAID_SUFFIX));
    let Ok(bytes) = fs::read(&path) else {
      continue;
    };
    let name = path
      .file_name()
      .map(|n| n.to_string_lossy().into_owned())
      .unwrap_or_default();
    let (status, replayed) = replayer(root, &target, &path);
    let said = match fs::read_to_string(&said_path) {
      Ok(said) => said,
      // Found as a file, with nothing of its run: the replay is the nearest to what it said.
      Err(_) => format!(
        "(found as {}; what its run said was not kept: this is a replay)\n{}\n",
        name,
        replayed.join("\n")
      ),
    };
    let report = Report {
      kind: kind_of(&name),
      target: &target,
      said: &said,
      input: Some(&bytes),
      replay: "",
      tree: &tree,
      run: &run,
      key: None,
    };
    match incidents::file(root, &report) {
      Ok(result) => {
        if let Some((dir, mut incident)) = incidents::find(root, result.id()) {
          incident.replay = format!(
            "cargo xtask incidents replay {}",
            incidents::short(&incident)
          );
          incidents::note(
            root,
            &dir,
            &mut incident,
            &tree,
            &format!("replay: {}", status),
          );
        }
        log.info(&format!("filed {} as {:?}", path.display(), result));
        println!("{} {}: {}", verb(&result), result.id(), status);
        if !matches!(result, Filed::Again(_)) {
          filed += 1;
        }
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&said_path);
      }
      Err(e) => {
        log.error(&format!("cannot file {}: {}", path.display(), e));
        println!("cannot file {}: {}", path.display(), e);
      }
    }
  }
  filed
}

/// The end of a run's output worth keeping: from the panic or libFuzzer's error on, or the
/// last lines when there is neither; and every later line naming a check's rule, which
/// tells one failure from another (an input printed in a message can push it past the
/// first lines).
fn failure_lines(output: &str) -> Vec<String> {
  let lines: Vec<&str> = output.lines().collect();
  let from = lines
    .iter()
    .position(|l| l.contains("panicked at") || l.contains("ERROR: libFuzzer"))
    .unwrap_or(lines.len().saturating_sub(20));
  let mut kept: Vec<String> = lines[from..]
    .iter()
    .take(40)
    .map(|l| l.to_string())
    .collect();
  kept.extend(
    lines[(from + 40).min(lines.len())..]
      .iter()
      .filter(|l| l.contains("rule: "))
      .map(|l| l.to_string()),
  );
  kept
}

/// Where runs stage what libFuzzer writes, one folder per target.
fn incoming(root: &Path) -> PathBuf {
  incidents::root_dir(root).join(".incoming")
}

fn relative(root: &Path, path: &Path) -> String {
  path
    .strip_prefix(root)
    .unwrap_or(path)
    .display()
    .to_string()
}

/// `fuzz run`'s `--time` and `--sanitizer`.
fn run_of(m: &clap::ArgMatches) -> Run {
  Run {
    seconds: m.get_one::<u32>("time").copied().unwrap_or(60),
    sanitizer: crate::cli::text(m, "sanitizer").unwrap_or_else(|| "address".to_string()),
    keep_going: false,
  }
}

/// The inputs staged in a folder, notes aside, sorted.
fn staged(folder: &Path) -> Vec<PathBuf> {
  let mut files: Vec<PathBuf> = fs::read_dir(folder)
    .map(|files| {
      files
        .flatten()
        .map(|f| f.path())
        .filter(|p| p.is_file() && !p.display().to_string().ends_with(SAID_SUFFIX))
        .collect()
    })
    .unwrap_or_default();
  files.sort();
  files
}

fn verb(filed: &Filed) -> &'static str {
  match filed {
    Filed::New(_) => "new incident",
    Filed::Again(_) => "seen again",
    Filed::Reopened(_) => "reopened (was fixed)",
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A scratch repository root, removed when dropped.
  struct Scratch(PathBuf);

  impl Scratch {
    fn new(name: &str) -> Scratch {
      let dir = std::env::temp_dir().join(format!("xtask-fuzz-{}-{}", name, std::process::id()));
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
    (
      "fails",
      vec!["thread panicked at src/x.rs:1:1:".to_string()],
    )
  }

  #[test]
  fn test_collect_files_staged_inputs_as_incidents() {
    let scratch = Scratch::new("collect");
    let root = &scratch.0;
    let log = Logger::new(root, "fuzz");
    let staging = incoming(root).join("format");
    fs::create_dir_all(&staging).unwrap();
    let said = "thread '<unnamed>' panicked at src/fuzzing/format.rs:36:3:\nmeaning changed:\n";
    for (name, text) in [
      ("crash-0123456789abcdef", "fn f() {}"),
      ("crash-fedcba9876543210", "fn g() {}"),
    ] {
      fs::write(staging.join(name), text).unwrap();
      fs::write(staging.join(format!("{}{}", name, SAID_SUFFIX)), said).unwrap();
    }
    // Two inputs with one failure: one incident, two occurrences, both inputs kept.
    assert_eq!(collect_with(root, &log, &fails), 1);
    assert!(staged(&staging).is_empty());
    let all = incidents::all(root);
    assert_eq!(all.len(), 1);
    let (dir, incident) = &all[0];
    assert_eq!(
      (incident.kind.as_str(), incident.target.as_str()),
      ("fuzz-crash", "format")
    );
    assert_eq!(incident.occurrences.len(), 2);
    assert!(dir.join("input").exists() && dir.join("input-2").exists());
    assert_eq!(incident.history.last().unwrap().result, "replay: fails");
    assert!(incident.replay.ends_with(incidents::short(incident)));
  }

  #[test]
  fn test_corpora_move_into_the_store_once() {
    let root = std::env::temp_dir().join(format!("xtask-fuzz-corpus-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("fuzz/corpus/format")).unwrap();
    fs::create_dir_all(root.join("fuzz/corpus/lexer")).unwrap();
    fs::write(root.join("fuzz/corpus/format/a"), "x").unwrap();
    let dir = corpus_dir(&root, "format");
    assert_eq!(dir, store::dir(&root, store::FUZZ).join("format"));
    assert_eq!(fs::read_to_string(dir.join("a")).unwrap(), "x");
    assert!(store::dir(&root, store::FUZZ).join("lexer").is_dir());
    assert!(!root.join("fuzz/corpus").exists());
    // Again, nothing to move and nothing lost.
    assert_eq!(corpus_dir(&root, "format"), dir);
    assert!(dir.join("a").is_file());
    let _ = fs::remove_dir_all(&root);
  }

  #[test]
  fn test_failure_lines_start_at_the_panic() {
    let mut output: Vec<String> = (0..50).map(|n| format!("#{} NEW", n)).collect();
    output.push("thread '<unnamed>' panicked at src/x.rs:1:1:".to_string());
    output.push("boom".to_string());
    let said = failure_lines(&output.join("\n"));
    assert_eq!(said[0], "thread '<unnamed>' panicked at src/x.rs:1:1:");
    assert_eq!(said.len(), 2);
    let quiet: Vec<String> = (0..50).map(|n| format!("line {}", n)).collect();
    assert_eq!(failure_lines(&quiet.join("\n")).len(), 20);
  }

  #[test]
  fn test_inputs_left_at_the_root_are_filed_with_their_replay() {
    let scratch = Scratch::new("root");
    let root = &scratch.0;
    let log = Logger::new(root, "fuzz");
    fs::write(root.join("oom-fedcba9876543210"), b"x").unwrap();
    fs::write(root.join("README.md"), b"not a crash").unwrap();
    assert_eq!(collect_with(root, &log, &fails), 1);
    assert!(!root.join("oom-fedcba9876543210").exists());
    assert!(root.join("README.md").exists());
    let all = incidents::all(root);
    assert_eq!(all[0].1.kind, "fuzz-oom");
    assert_eq!(all[0].1.target, "unknown-target");
    let said = fs::read_to_string(all[0].0.join("said.txt")).unwrap();
    assert!(said.contains("this is a replay"), "{}", said);
  }

  #[test]
  fn test_kinds_by_name() {
    assert_eq!(kind_of("crash-abc"), "fuzz-crash");
    assert_eq!(kind_of("slow-unit-abc"), "fuzz-slow");
    assert_eq!(kind_of("oom-abc"), "fuzz-oom");
  }

  #[test]
  fn test_run_arguments() {
    let run = |words: &[&str]| {
      let mut all = vec!["fuzz", "run", "format"];
      all.extend_from_slice(words);
      let all: Vec<String> = all.iter().map(|w| w.to_string()).collect();
      match crate::cli::parse(&all) {
        crate::cli::Parsed::Run(_, m) => Some(run_of(m.subcommand().unwrap().1)),
        _ => None,
      }
    };
    assert_eq!(run(&[]), Some(Run::for_seconds(60)));
    let parsed = run(&["--sanitizer", "none", "--time", "300"]).unwrap();
    assert_eq!((parsed.seconds, parsed.sanitizer.as_str()), (300, "none"));
    assert_eq!(run(&["--sanitizer", "thread"]), None);
    assert_eq!(run(&["--time"]), None);
    assert_eq!(run(&["300"]), None);
  }
}
