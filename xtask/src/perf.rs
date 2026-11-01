//! `cargo xtask perf`: what mto costs beyond a whole corpus at once, kept as `perf.*` series.
//!
//! - `latency`: one small file, as an editor formats on save (`mto -`) and as `mto --check
//!   FILE`, against `mto --version` (starting the process and nothing else); the middle and
//!   the 99th percentile of many runs of the release `mto`.
//! - `scaling`: every 8th corpus file laid out and formatted in-process by 1, 2, 4 and 8
//!   threads at once (xtask-tools `perf-scaling`); the one-thread speed, and the speedup
//!   of each count over one.
//! - `growth`: generated files of doubling size in each way a file grows (xtask-tools
//!   `perf-growth`); how the time grows with the size, as an exponent (1 is linear).
//!
//! With no subcommand, all three.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use crate::common::process;
use crate::common::timing;
use crate::corpus;
use crate::metrics;

/// `scaling` takes every this-many-th corpus file.
const EVERY: usize = 8;

/// Runs of each kind of call `latency` times.
const LATENCY_RUNS: usize = 200;

/// How many passes `scaling` makes over the thread counts.
const PASSES: usize = 3;

/// A file such as an editor saves: a few items to order, some formatting to do.
const SMALL: &str = "use std::fmt;\nuse std::collections::HashMap;\n\n\
pub struct Counter { counts: HashMap<String, usize> }\n\n\
impl fmt::Display for Counter {\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        \
for (word, count) in &self.counts { writeln!(f, \"{}: {}\", word, count)?; }\n        Ok(())\n    }\n}\n\n\
impl Counter {\n    pub fn new() -> Self { Counter { counts: HashMap::new() } }\n\n    \
pub fn add(&mut self, text: &str) {\n        for word in text.split_whitespace() {\n            \
*self.counts.entry(word.to_lowercase()).or_insert(0) += 1;\n        }\n    }\n}\n\n\
fn helper(x: u32) -> u32 { x * 2 }\n\npub fn top(counter: &Counter, n: usize) -> Vec<(&String, &usize)> {\n    \
let mut all: Vec<_> = counter.counts.iter().collect();\n    all.sort_by(|a, b| b.1.cmp(a.1));\n    \
all.truncate(n);\n    all\n}\n";

/// The thread counts `scaling` compares.
const THREADS: &[usize] = &[1, 2, 4, 8];

/// One way `latency` calls mto: its name, arguments, stdin, and the exit code expected.
type Call = (
  &'static str,
  &'static [&'static str],
  Option<&'static [u8]>,
  i32,
);

/// A generated file `growth` timed: its size, bytes and seconds.
type Point = (f64, f64, f64);

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let which = m.subcommand_name();
  // Counted, not timed: neither the machine's quiet nor the other measures matter to it.
  if let Some(("instructions", sub)) = m.subcommand() {
    return crate::instructions::run(root, crate::cli::on(sub, "update"));
  }
  // Built first, so the build is not what the machine is busy with when timing starts.
  let tools = ["build", "--quiet", "--release", "--package", "xtask-tools"].map(String::from);
  if build_mto(root).is_err() || process::capture(root, "cargo", &tools, &[], process::LONG).0 != 0
  {
    println!("cannot build mto and xtask-tools");
    return ExitCode::from(1);
  }
  crate::common::conditions::settle(root, "cargo xtask perf");
  let mut breaches = Vec::new();
  let mut failed = false;
  if which.is_none() || which == Some("latency") {
    failed |= latency(root, &mut breaches).is_err();
  }
  if which.is_none() || which == Some("scaling") {
    failed |= scaling(root, &mut breaches).is_err();
  }
  if which.is_none() || which == Some("growth") {
    failed |= growth(root, &mut breaches).is_err();
  }
  for breach in &breaches {
    println!("ratchet: {}", breach);
  }
  if failed || !breaches.is_empty() {
    ExitCode::from(1)
  } else {
    ExitCode::SUCCESS
  }
}

fn build_mto(root: &Path) -> Result<PathBuf, ()> {
  let build = ["build", "--quiet", "--release", "--bin", "mto"].map(String::from);
  let (code, output) = process::capture(root, "cargo", &build, &[], process::LONG);
  if code != 0 {
    println!("cannot build mto:\n{}", output);
    return Err(());
  }
  Ok(
    root
      .join("target")
      .join("release")
      .join(format!("mto{}", std::env::consts::EXE_SUFFIX)),
  )
}

/// The corpus folders, fetched first if they are not.
fn corpus_dirs(root: &Path) -> Result<Vec<PathBuf>, ()> {
  let pins = corpus::load_pins(root).map_err(|e| println!("{}", e))?;
  let dirs: Vec<PathBuf> = pins
    .iter()
    .filter(|p| !p.opt_in)
    .map(|p| corpus::cache_dir(root).join(&p.name))
    .collect();
  if dirs.iter().any(|d| !d.is_dir()) {
    println!("the corpus is not fetched: run `cargo xtask corpus fetch` first");
    return Err(());
  }
  Ok(dirs)
}

/// How time grows with size over the three largest sizes: the slope of log time against
/// log size (the smaller sizes are mostly fixed costs).
fn exponent(points: &[Point]) -> f64 {
  let top = &points[points.len().saturating_sub(3)..];
  let logs: Vec<(f64, f64)> = top.iter().map(|(n, _, s)| (n.ln(), s.ln())).collect();
  let count = logs.len() as f64;
  let mean_x = logs.iter().map(|(x, _)| x).sum::<f64>() / count;
  let mean_y = logs.iter().map(|(_, y)| y).sum::<f64>() / count;
  let covariance: f64 = logs.iter().map(|(x, y)| (x - mean_x) * (y - mean_y)).sum();
  let variance: f64 = logs.iter().map(|(x, _)| (x - mean_x).powi(2)).sum();
  covariance / variance
}

fn growth(root: &Path, breaches: &mut Vec<String>) -> Result<(), ()> {
  let scratch = std::env::temp_dir().join("mtindo-perf-growth");
  let _ = fs::remove_dir_all(&scratch);
  let output = tools(
    root,
    &["perf-growth".to_string(), scratch.display().to_string()],
  )?;
  let _ = fs::remove_dir_all(&scratch);
  // Each kind's points, in order of size.
  let mut kinds: Vec<(String, Vec<Point>)> = Vec::new();
  for line in output.lines() {
    let words: Vec<&str> = line.split_whitespace().collect();
    let ["growth", kind, n, bytes, seconds] = words[..] else {
      continue;
    };
    let (Ok(n), Ok(bytes), Ok(seconds)) = (n.parse(), bytes.parse(), seconds.parse()) else {
      continue;
    };
    match kinds.iter_mut().find(|(k, _)| k == kind) {
      Some((_, points)) => points.push((n, bytes, seconds)),
      None => kinds.push((kind.to_string(), vec![(n, bytes, seconds)])),
    }
  }
  if kinds.is_empty() {
    println!("cannot read xtask-tools perf-growth:\n{}", output);
    return Err(());
  }
  println!("growth: time against size, doubling (1 is linear, 2 quadratic):");
  for (kind, points) in &kinds {
    let exponent = exponent(points);
    let (_, bytes, seconds) = points[points.len() - 1];
    let us_per_kb = seconds * 1e6 / (bytes / 1000.0);
    println!(
      "  {:<8} exponent {:.2}   {:>7.1} us per KB at {:.0} KB",
      kind,
      exponent,
      us_per_kb,
      bytes / 1000.0
    );
    keep(
      root,
      &format!("perf.growth.{}.exponent", kind),
      exponent,
      "",
      breaches,
    );
  }
  Ok(())
}

fn keep(root: &Path, name: &str, value: f64, unit: &str, breaches: &mut Vec<String>) {
  let value = (value * 1000.0).round() / 1000.0;
  breaches.extend(metrics::record_checked(root, name, value, unit));
}

fn latency(root: &Path, breaches: &mut Vec<String>) -> Result<(), ()> {
  let mto = build_mto(root)?;
  let scratch = std::env::temp_dir().join("mtindo-perf-latency");
  let _ = fs::remove_dir_all(&scratch);
  let project = scratch.join("project");
  let made = fs::create_dir_all(&project)
    .and_then(|_| fs::create_dir_all(scratch.join("state")))
    .and_then(|_| {
      fs::write(
        project.join("mto.toml"),
        "root = true\nlayout = true\nformat = true\n",
      )
    })
    .and_then(|_| fs::write(project.join("lib.rs"), SMALL));
  if let Err(e) = made {
    println!("cannot make {}: {}", project.display(), e);
    return Err(());
  }
  println!(
    "latency: one {}-byte file, {} runs each ({} {}):",
    SMALL.len(),
    LATENCY_RUNS,
    std::env::consts::OS,
    std::env::consts::ARCH
  );
  let ways: [Call; 3] = [
    ("version", &["--version"], None, 0),
    ("stdin", &["-"], Some(SMALL.as_bytes()), 0),
    ("check", &["--check", "lib.rs"], None, 1),
  ];
  for (name, args, input, expected) in ways {
    let mut times = Vec::with_capacity(LATENCY_RUNS);
    for _ in 0..LATENCY_RUNS {
      let mut command = Command::new(&mto);
      command
        .args(args)
        .current_dir(&project)
        .env("HOME", scratch.join("home"))
        .env("XDG_STATE_HOME", scratch.join("state"))
        .env("LOCALAPPDATA", scratch.join("state"))
        .env("CI", "1")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("MTINDO_CONFIG")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
      let started = Instant::now();
      let output = process::output_with_input(&mut command, input, process::QUICK);
      let took = started.elapsed().as_secs_f64() * 1000.0;
      match output {
        Ok(out) if out.code == expected => times.push(took),
        Ok(out) => {
          println!(
            "  {}: mto exited {}: {}",
            name,
            out.code,
            String::from_utf8_lossy(&out.stderr)
          );
          return Err(());
        }
        Err(e) => {
          println!("  {}: cannot run mto: {}", name, e);
          return Err(());
        }
      }
    }
    let p99 = timing::percentile(&times, 99);
    let median = timing::middle(times);
    println!(
      "  {:<8} {:>7.2} ms middle  {:>7.2} ms p99",
      name, median, p99
    );
    let name = format!("perf.latency.{}", name);
    keep(root, &format!("{}.median_ms", name), median, "ms", breaches);
    keep(root, &format!("{}.p99_ms", name), p99, "ms", breaches);
  }
  let _ = fs::remove_dir_all(&scratch);
  Ok(())
}

fn scaling(root: &Path, breaches: &mut Vec<String>) -> Result<(), ()> {
  let dirs = corpus_dirs(root)?;
  println!(
    "scaling: every {}th corpus file, laid out and formatted in-process, fastest of {} passes:",
    EVERY, PASSES
  );
  // Passes over every count in turn, so that a moment of other work weighs on one reading
  // of one count, not on all of a count's; each count's fastest is kept.
  let mut fastest = vec![f64::MAX; THREADS.len()];
  let mut bytes = 0.0;
  for _ in 0..PASSES {
    for (at, &threads) in THREADS.iter().enumerate() {
      let mut args = vec![
        "perf-scaling".to_string(),
        threads.to_string(),
        EVERY.to_string(),
      ];
      args.extend(dirs.iter().map(|d| d.display().to_string()));
      let output = tools(root, &args)?;
      let words: Vec<&str> = output
        .lines()
        .find(|l| l.starts_with("threads "))
        .map(|l| l.split_whitespace().collect())
        .unwrap_or_default();
      let field = |name: &str| -> Option<f64> {
        let at = words.iter().position(|w| *w == name)?;
        words.get(at + 1)?.parse().ok()
      };
      let (Some(read), Some(seconds)) = (field("bytes"), field("seconds")) else {
        println!("cannot read xtask-tools perf-scaling:\n{}", output);
        return Err(());
      };
      bytes = read;
      fastest[at] = fastest[at].min(seconds);
    }
  }
  let one = fastest[0];
  let mb_per_s = bytes / 1e6 / one;
  println!("  1 thread   {:>6.2} s   {:.2} MB/s", one, mb_per_s);
  keep(
    root,
    "perf.scaling.one_thread.mb_per_s",
    mb_per_s,
    "MB/s",
    breaches,
  );
  for (&threads, &seconds) in THREADS.iter().zip(&fastest).skip(1) {
    let speedup = one / seconds;
    println!(
      "  {} threads  {:>6.2} s   {:.2}x",
      threads, seconds, speedup
    );
    keep(
      root,
      &format!("perf.scaling.t{}.speedup", threads),
      speedup,
      "x",
      breaches,
    );
  }
  Ok(())
}

/// Runs xtask-tools, release, with `args`; its output.
fn tools(root: &Path, args: &[String]) -> Result<String, ()> {
  let mut all: Vec<String> = [
    "run",
    "--quiet",
    "--release",
    "--package",
    "xtask-tools",
    "--",
  ]
  .map(String::from)
  .to_vec();
  all.extend(args.iter().cloned());
  let (code, output) = process::capture(root, "cargo", &all, &[], process::LONG);
  if code != 0 {
    println!("xtask-tools {} failed:\n{}", args[0], output);
    return Err(());
  }
  Ok(output)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_exponent_of_linear_and_quadratic_growth() {
    let linear: Vec<_> = [1.0, 2.0, 4.0, 8.0]
      .iter()
      .map(|n| (*n, 0.0, n * 0.5))
      .collect();
    assert!((exponent(&linear) - 1.0).abs() < 1e-9);
    let quadratic: Vec<_> = [1.0, 2.0, 4.0, 8.0]
      .iter()
      .map(|n| (*n, 0.0, n * n))
      .collect();
    assert!((exponent(&quadratic) - 2.0).abs() < 1e-9);
  }
}
