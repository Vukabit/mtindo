//! `cargo xtask fuzz soak`: every fuzz target side by side for a long run, 24 hours by
//! default, then what they found filed as incidents (see [`crate::fuzz`]).
//!
//! The machine is kept awake for the whole run ([`KeepAwake`]), with the display on when the
//! run is watched, and a run the machine slept through anyway fails. Each target fuzzes on
//! past its finds for the whole time (libFuzzer's fork mode), keeping every input it fails
//! on; filing them groups the ones that fail the same way into one incident. With
//! `--restart`, libFuzzer stops a target at its first find instead and the soak starts it
//! again for the time left, up to [`RESTARTS`] times.
//!
//! Ctrl-C stops every target (they get it too) and still files what they found; a second
//! Ctrl-C ends the soak at once, leaving the finds staged for `cargo xtask fuzz collect`.
//!
//! On a terminal the run is drawn live: each target's executions, speed, coverage, corpus,
//! memory and finds, redrawn every [`FRAME`]. Elsewhere it prints one line per target every
//! [`EVERY`].

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::common::awake::{self, KeepAwake};
use crate::common::log::Logger;
use crate::fuzz::{self, Run};

/// How long a soak fuzzes each target unless `--time` says otherwise.
pub const SOAK_SECONDS: u32 = 24 * 60 * 60;

/// How often a run that is not on a terminal prints where it is.
const EVERY: Duration = Duration::from_secs(30 * 60);

/// How often the live view is redrawn.
const FRAME: Duration = Duration::from_secs(2);

/// A target with less time left than this is not started again.
const LEAST_RESTART: u64 = 60;

/// The most times one target is started again after libFuzzer stopped it at a find.
const RESTARTS: u32 = 10;

/// Set by the first Ctrl-C.
static STOPPED: AtomicBool = AtomicBool::new(false);

/// How a soak shows its progress.
#[derive(Clone, Copy, PartialEq)]
pub enum View {
  /// Redrawn in place on a terminal.
  Live,
  /// A line per target now and then.
  Lines,
  /// Nothing, for a caller that draws its own progress.
  Quiet,
}

/// One target's progress, from libFuzzer's status lines.
#[derive(Clone, Default)]
struct Progress {
  /// Executions so far in this start (`#N`), and in the starts before it.
  execs: u64,
  execs_before: u64,
  exec_per_s: u64,
  cov: u64,
  features: u64,
  /// The corpus as libFuzzer writes it: `inputs/size`.
  corpus: String,
  rss_mb: u64,
  /// Inputs it failed on in the starts before this one.
  found: u32,
  /// Inputs it failed on in this start, from fork mode's status lines.
  crashes: u32,
  /// `building`, `fuzzing`, `restarting`, `done`, `stopped` or `failed`.
  state: &'static str,
  /// When its first status line came, after the build: when its fuzzing time starts.
  since: Option<Instant>,
  /// The rest of a line libFuzzer has not finished writing.
  partial: String,
}

impl Progress {
  /// Takes in one piece of libFuzzer's output.
  fn feed(&mut self, bytes: &[u8]) {
    self.partial.push_str(&String::from_utf8_lossy(bytes));
    while let Some(end) = self.partial.find('\n') {
      let line: String = self.partial.drain(..=end).collect();
      self.line(line.trim_end());
    }
  }

  /// Reads one line: a status line (`#N  NEW  cov: ... exec/s: ... rss: ...`, or fork
  /// mode's `#N: cov: ... oom/timeout/crash: A/B/C ...`) updates the figures, anything else
  /// is ignored.
  fn line(&mut self, line: &str) {
    let Some(rest) = line.strip_prefix('#') else {
      return;
    };
    let mut words = rest.split_whitespace();
    let Some(execs) = words
      .next()
      .and_then(|n| n.trim_end_matches(':').parse::<u64>().ok())
    else {
      return;
    };
    self.execs = execs;
    self.state = "fuzzing";
    self.since.get_or_insert_with(Instant::now);
    let words: Vec<&str> = words.collect();
    for pair in words.windows(2) {
      let number = || {
        pair[1]
          .trim_end_matches(|c: char| c.is_ascii_alphabetic())
          .parse::<u64>()
          .ok()
      };
      match pair[0] {
        "cov:" => self.cov = number().unwrap_or(self.cov),
        "ft:" => self.features = number().unwrap_or(self.features),
        "corp:" => self.corpus = pair[1].to_string(),
        "exec/s:" => self.exec_per_s = number().unwrap_or(self.exec_per_s),
        "rss:" => self.rss_mb = number().unwrap_or(self.rss_mb),
        "oom/timeout/crash:" => {
          self.crashes = pair[1]
            .split('/')
            .filter_map(|n| n.parse::<u32>().ok())
            .sum()
        }
        _ => {}
      }
    }
  }

  fn total_execs(&self) -> u64 {
    self.execs_before + self.execs
  }

  fn total_found(&self) -> u32 {
    self.found + self.crashes
  }
}

/// `fuzz soak`'s arguments: run it, print the outcome, and fail when it found anything or a
/// target or the machine let it down.
pub fn run(root: &Path, m: &clap::ArgMatches, log: &Logger) -> ExitCode {
  let seconds = m.get_one::<u32>("time").copied().unwrap_or(SOAK_SECONDS);
  let keep_going = !m.get_flag("restart");
  let mut targets = targets(root);
  let wanted = crate::cli::texts(m, "targets");
  if !wanted.is_empty() {
    if let Some(unknown) = wanted.iter().find(|w| !targets.contains(w)) {
      println!(
        "no fuzz target {}; there are: {}",
        unknown,
        targets.join(", ")
      );
      return ExitCode::from(2);
    }
    targets.retain(|t| wanted.contains(t));
  }
  let view = if std::io::stdout().is_terminal() {
    View::Live
  } else {
    View::Lines
  };
  let awake = if view == View::Live {
    KeepAwake::start_watched()
  } else {
    KeepAwake::start()
  };
  println!(
    "fuzz soak: {} target(s) for {} each, side by side; kept awake by {}",
    targets.len(),
    awake::duration(seconds.into()),
    awake.how()
  );
  log.info(&format!(
    "soak of {} for {}s, kept awake by {}",
    targets.join(", "),
    seconds,
    awake.how()
  ));
  // A handler, not an ignored signal: an ignored SIGINT would be inherited by the fuzzers.
  // SAFETY: installs a handler that only swaps an atomic and calls `_exit`.
  unsafe {
    libc::signal(
      libc::SIGINT,
      on_interrupt as *const () as libc::sighandler_t,
    );
  }
  let problems = soak(root, &targets, seconds, keep_going, view, log);
  drop(awake);
  if problems.is_empty() {
    println!("soaked every target: nothing found");
    ExitCode::SUCCESS
  } else {
    for problem in &problems {
      println!("{}", problem);
    }
    ExitCode::FAILURE
  }
}

/// Fuzzes every one of `targets` at the same time for `seconds`, each fuzzing on past its
/// finds when `keep_going` and otherwise started again after one while time is left, then
/// files what they found. Returns the problems: a target that failed other than by finding
/// an input, the incidents filed, and sleep.
pub fn soak(
  root: &Path,
  targets: &[String],
  seconds: u32,
  keep_going: bool,
  view: View,
  log: &Logger,
) -> Vec<String> {
  let started = Instant::now();
  let stamp = awake::Stamp::now();
  let progress: Vec<Arc<Mutex<Progress>>> = targets
    .iter()
    .map(|_| {
      Arc::new(Mutex::new(Progress {
        state: "building",
        ..Progress::default()
      }))
    })
    .collect();
  let failures: Vec<Vec<String>> = thread::scope(|scope| {
    let workers: Vec<_> = targets
      .iter()
      .zip(&progress)
      .map(|(target, progress)| {
        scope.spawn(move || soak_target(root, target, seconds, keep_going, progress, log))
      })
      .collect();
    if view != View::Quiet {
      watch(targets, &progress, seconds, started, view, || {
        workers.iter().all(|w| w.is_finished())
      });
    }
    workers
      .into_iter()
      .map(|w| {
        w.join()
          .unwrap_or_else(|_| vec!["it panicked in the runner".to_string()])
      })
      .collect()
  });
  let mut problems = Vec::new();
  if STOPPED.load(Ordering::SeqCst) {
    println!("stopped by Ctrl-C: filing what the targets found (Ctrl-C again to leave it staged)");
    problems.push(format!(
      "stopped by Ctrl-C after {}",
      awake::duration(started.elapsed().as_secs())
    ));
  }
  let filed = fuzz::collect(root, log);
  for (target, said) in targets.iter().zip(failures) {
    if !said.is_empty() {
      problems.push(format!(
        "fuzz target {} failed:\n{}",
        target,
        said.join("\n")
      ));
    }
  }
  if filed > 0 {
    problems.push(format!(
      "{} new or reopened incident(s) (cargo xtask incidents)",
      filed
    ));
  }
  let slept = awake::asleep_seconds(stamp);
  if slept > 60 {
    problems.push(format!(
      "the machine slept for {} of the run, so the targets fuzzed less than asked",
      awake::duration(slept)
    ));
  }
  if view != View::Quiet {
    println!("{}", summary(targets, &progress, started));
  }
  problems
}

/// The fuzz targets, from `fuzz/fuzz_targets/`, sorted.
pub fn targets(root: &Path) -> Vec<String> {
  let mut stems: Vec<String> = fs::read_dir(root.join("fuzz").join("fuzz_targets"))
    .into_iter()
    .flatten()
    .flatten()
    .map(|entry| entry.path())
    .filter(|path| path.extension().is_some_and(|e| e == "rs"))
    .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
    .collect();
  stems.sort();
  stems
}

/// `987`, `12.3k`, `4.56M`, `1.23G`.
fn count(n: u64) -> String {
  match n {
    0..1_000 => n.to_string(),
    1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
    1_000_000..1_000_000_000 => format!("{:.2}M", n as f64 / 1e6),
    _ => format!("{:.2}G", n as f64 / 1e9),
  }
}

/// The live view: how far the run is, then one row per target.
fn frame(
  targets: &[String],
  progress: &[Arc<Mutex<Progress>>],
  seconds: u32,
  started: Instant,
) -> String {
  let found: u32 = progress
    .iter()
    .map(|p| p.lock().unwrap().total_found())
    .sum();
  // The run's clock starts when the first target starts fuzzing, after its build.
  let first = progress
    .iter()
    .filter_map(|p| p.lock().unwrap().since)
    .min();
  let mut out = match first {
    Some(first) => {
      let ran = first.elapsed().as_secs().min(seconds.into());
      format!(
        "fuzz soak  {} in, {} left  ({:.1}%)  found {}\n\n",
        awake::duration(ran),
        awake::duration(u64::from(seconds) - ran),
        100.0 * ran as f64 / f64::from(seconds.max(1)),
        found
      )
    }
    None => format!(
      "fuzz soak  building the targets ({} since the start)\n\n",
      awake::duration(started.elapsed().as_secs())
    ),
  };
  out.push_str(&format!(
    "{:<10} {:>10} {:>8} {:>7} {:>8} {:>13} {:>7} {:>5}  {}\n",
    "target", "execs", "exec/s", "cov", "ft", "corpus", "rss", "found", "state"
  ));
  for (target, p) in targets.iter().zip(progress) {
    let p = p.lock().unwrap();
    out.push_str(&format!(
      "{:<10} {:>10} {:>8} {:>7} {:>8} {:>13} {:>7} {:>5}  {}\n",
      target,
      count(p.total_execs()),
      p.exec_per_s,
      p.cov,
      p.features,
      p.corpus,
      // Fork mode's status lines leave the memory out.
      if p.rss_mb == 0 {
        "-".to_string()
      } else {
        format!("{}MB", p.rss_mb)
      },
      p.total_found(),
      p.state
    ));
  }
  out
}

extern "C" fn on_interrupt(_: libc::c_int) {
  if STOPPED.swap(true, Ordering::SeqCst) {
    // SAFETY: `_exit` is safe to call from a signal handler.
    unsafe { libc::_exit(130) }
  }
}

/// Fuzzes one target until its time is up, on past each find when `keep_going` and
/// otherwise started again after one; returns what it said when it failed other than by
/// finding an input.
fn soak_target(
  root: &Path,
  target: &str,
  seconds: u32,
  keep_going: bool,
  progress: &Arc<Mutex<Progress>>,
  log: &Logger,
) -> Vec<String> {
  let time = Duration::from_secs(seconds.into());
  // Until fuzzing starts the whole time is left; the build does not count against it.
  let ends = |p: &Progress| p.since.map_or(Instant::now() + time, |since| since + time);
  let mut starts = 0;
  loop {
    let left = {
      let p = progress.lock().unwrap();
      ends(&p).saturating_duration_since(Instant::now()).as_secs()
    };
    let sink = Arc::clone(progress);
    let tap: crate::common::process::Tap = Box::new(move |bytes| sink.lock().unwrap().feed(bytes));
    let run = Run {
      keep_going,
      ..Run::for_seconds(left.max(1) as u32)
    };
    let (code, said) = fuzz::fuzz_target_tapped(root, target, &run, log, Some(tap));
    // A find ends with a panic or a sanitizer's or libFuzzer's own error; anything else
    // that stops a run early (a build that fails, a missing toolchain) is a failure.
    let found = said
      .iter()
      .any(|l| l.contains("panicked at") || l.contains("ERROR:"));
    let mut p = progress.lock().unwrap();
    p.execs_before += p.execs;
    p.execs = 0;
    // A run that kept going counted its finds as it went; one that stopped found one.
    p.found += p.crashes.max(u32::from(found && code != 0));
    p.crashes = 0;
    if code == 0 {
      p.state = "done";
      return Vec::new();
    }
    if STOPPED.load(Ordering::SeqCst) {
      p.state = "stopped";
      return Vec::new();
    }
    if !found {
      p.state = "failed";
      return said;
    }
    starts += 1;
    let left = ends(&p).saturating_duration_since(Instant::now()).as_secs();
    if starts > RESTARTS || left < LEAST_RESTART {
      p.state = "done";
      return Vec::new();
    }
    p.state = "restarting";
    log.info(&format!(
      "{} found an input; starting it again for the {}s left",
      target, left
    ));
  }
}

/// What each target did, once the run is over.
fn summary(targets: &[String], progress: &[Arc<Mutex<Progress>>], started: Instant) -> String {
  let mut out = format!(
    "fuzz soak over after {}:\n",
    awake::duration(started.elapsed().as_secs())
  );
  for (target, p) in targets.iter().zip(progress) {
    let p = p.lock().unwrap();
    out.push_str(&format!(
      "  {:<10} {} executions, cov {}, ft {}, corpus {}, {} found, {}\n",
      target,
      count(p.total_execs()),
      p.cov,
      p.features,
      p.corpus,
      p.total_found(),
      p.state
    ));
  }
  out.trim_end().to_string()
}

/// Shows the run until `over` says every target is done.
fn watch(
  targets: &[String],
  progress: &[Arc<Mutex<Progress>>],
  seconds: u32,
  started: Instant,
  view: View,
  over: impl Fn() -> bool,
) {
  let mut first = true;
  let mut last_line = Instant::now();
  while !over() {
    match view {
      View::Live => {
        let mut out = String::from(if first { "\x1b[H\x1b[2J" } else { "\x1b[H" });
        for line in frame(targets, progress, seconds, started).lines() {
          out.push_str(line);
          out.push_str("\x1b[K\n");
        }
        out.push_str("\x1b[J");
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
        first = false;
      }
      View::Lines if last_line.elapsed() >= EVERY => {
        println!("{}", frame(targets, progress, seconds, started));
        last_line = Instant::now();
      }
      _ => {}
    }
    thread::sleep(if view == View::Live {
      FRAME
    } else {
      Duration::from_secs(5)
    });
  }
  if view == View::Live && !first {
    print!("\x1b[H\x1b[2J");
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_counts() {
    assert_eq!(count(987), "987");
    assert_eq!(count(12_345), "12.3k");
    assert_eq!(count(4_560_000), "4.56M");
    assert_eq!(count(1_230_000_000), "1.23G");
  }

  #[test]
  fn test_executions_add_up_across_starts() {
    let mut p = Progress::default();
    p.feed(b"#1000\tNEW cov: 1 ft: 1 corp: 1/1b exec/s: 10 rss: 1Mb\n");
    p.execs_before += p.execs;
    p.execs = 0;
    p.feed(b"#250\tNEW cov: 1 ft: 1 corp: 1/1b exec/s: 10 rss: 1Mb\n");
    assert_eq!(p.total_execs(), 1250);
  }

  #[test]
  fn test_status_lines_update_the_progress_across_pieces() {
    let mut p = Progress::default();
    p.feed(b"INFO: Seed: 123\n#2\tINITED cov: 10 ft: 20 corp: 1/1b exec/s: 0 rss: 41Mb\n#40");
    assert_eq!((p.execs, p.cov, p.features, p.rss_mb), (2, 10, 20, 41));
    assert_eq!(p.state, "fuzzing");
    // The rest of a line arrives in the next piece.
    p.feed(b"96\tpulse  cov: 15 ft: 33 corp: 4/96b lim: 4 exec/s: 1365 rss: 52Mb\n");
    assert_eq!((p.execs, p.cov, p.features), (4096, 15, 33));
    assert_eq!(
      (p.corpus.as_str(), p.exec_per_s, p.rss_mb),
      ("4/96b", 1365, 52)
    );
    // Fork mode's status line: the count ends in a colon, and the finds are counted.
    p.feed(b"#90000: cov: 16 ft: 40 corp: 9 exec/s: 1500 oom/timeout/crash: 1/0/3 time: 60s job: 4 dft_time: 0\n");
    assert_eq!((p.execs, p.cov, p.total_found()), (90000, 16, 4));
    p.execs = 4096;
    // Lines that are not status lines change nothing.
    p.feed(b"#hash is not a count\nthread 'main' panicked at src/a.rs:1:1\n");
    assert_eq!(p.execs, 4096);
  }
}
