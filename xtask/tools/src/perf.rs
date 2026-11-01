//! The in-process half of `cargo xtask perf`: what laying out and formatting costs with more
//! threads, and as inputs grow. Each file goes through `mtindo::run_file` as a `--check`
//! does (read, layout, formatting, both verifiers, the diff), with layout and formatting on
//! at their defaults.
//!
//! - `perf-scaling N EVERY DIR...`: every EVERY-th file under the folders, laid out by N
//!   threads at once, each taking the next file as `mto` does; prints
//!   `threads N files F bytes B seconds S`.
//! - `perf-growth SCRATCH`: generated files of doubling size in each way a file can grow;
//!   prints `growth KIND SIZE BYTES SECONDS`.
//! - `perf-workload DIR`: one file of each kind at the largest size growth uses, written to
//!   DIR as `KIND.rs`, the same bytes every time: the input whose instructions `cargo xtask
//!   perf instructions` counts; prints `workload KIND BYTES`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mtindo::options::format::FormatConfig;
use mtindo::{LayoutConfig, RunOptions, Settings, WriteContext, run_file};

use crate::corpus::collect;

/// The ways a file grows: a generator of a file of size `n`, and the smallest `n`. Each is
/// generated at five sizes, doubling, so the time should double too; the largest stays
/// within how deep mto follows a tree (`mtindo::syntax::MAX_DEPTH`).
const KINDS: &[(&str, Generator, usize)] = &[
  ("items", items, 128),
  ("nesting", nesting, 8),
  ("line", line, 128),
  ("chain", chain, 32),
  ("match", arms, 128),
];

/// A size's time is the fastest of runs adding up to at least this.
const MIN_TOTAL: Duration = Duration::from_millis(200);

/// How many sizes each kind is generated at.
const STEPS: u32 = 5;

/// Makes a file of size `n`.
type Generator = fn(usize) -> String;

/// The deepest tree among the files under the folders, and how many files reach each
/// tenth of it.
pub fn depth(args: &[String]) -> ExitCode {
  let mut files = Vec::new();
  for dir in args {
    collect(Path::new(dir), &mut files);
  }
  let mut depths: Vec<(usize, PathBuf)> = files
    .into_iter()
    .filter_map(|file| {
      let text = fs::read_to_string(&file).ok()?;
      let parsed = mtindo::syntax::parse(&text, mtindo::syntax::Edition::E2021);
      Some((parsed.depth(), file))
    })
    .collect();
  depths.sort();
  for (depth, file) in depths.iter().rev().take(5) {
    println!("depth {} {}", depth, file.display());
  }
  if let Some((p99, _)) = depths.get(depths.len() * 99 / 100) {
    println!("p99 {}", p99);
  }
  ExitCode::SUCCESS
}

/// `perf-digest DIR...`: each file under the folders laid out and formatted as `--check`
/// would, with both verifiers; prints `PATH HASH` of the result (or of why it was refused),
/// for proving that a change to mto's speed changed nothing it writes.
pub fn digest(args: &[String]) -> ExitCode {
  let mut files = Vec::new();
  for dir in args {
    collect(Path::new(dir), &mut files);
  }
  files.sort();
  let settings = settings();
  for file in files {
    let _parses = mtindo::syntax::ParseCache::open();
    let Ok(source) = fs::read_to_string(&file) else {
      continue;
    };
    let outcome = match mtindo::shape(&source, &settings, &Default::default(), None) {
      Err(reason) => format!("refused: {}", reason),
      Ok(shaped) => {
        let layout = shaped.laid_out != source
          && !mtindo::verify::verify_layout(&source, &shaped.laid_out, shaped.edition).is_empty();
        let formatted = shaped.text != shaped.laid_out
          && !mtindo::verify::format::verify_format(&shaped.laid_out, &shaped.text, shaped.edition)
            .is_empty();
        format!("{} {} {}", layout, formatted, shaped.text)
      }
    };
    println!(
      "{} {}",
      file.display(),
      mtindo::history::sha256::hex(outcome.as_bytes())
    );
  }
  ExitCode::SUCCESS
}

pub fn growth(args: &[String]) -> ExitCode {
  let Some(scratch) = args.first().map(PathBuf::from) else {
    eprintln!("perf-growth SCRATCH");
    return ExitCode::from(2);
  };
  if fs::create_dir_all(&scratch).is_err() {
    eprintln!("cannot make {}", scratch.display());
    return ExitCode::from(1);
  }
  let settings = settings();
  let opts = check();
  for (kind, generate, smallest) in KINDS {
    for n in (0..STEPS).map(|step| smallest << step) {
      let file = scratch.join(format!("{}_{}.rs", kind, n));
      let text = generate(n);
      if fs::write(&file, &text).is_err() {
        eprintln!("cannot write {}", file.display());
        return ExitCode::from(1);
      }
      let seconds = fastest(|| {
        let _ = run_file(&file, &settings, &opts, WriteContext::default());
      });
      println!("growth {} {} {} {:.6}", kind, n, text.len(), seconds);
    }
  }
  ExitCode::SUCCESS
}

pub fn scaling(args: &[String]) -> ExitCode {
  let number = |at: usize| args.get(at).and_then(|n| n.parse::<usize>().ok());
  let (Some(threads), Some(every)) = (number(0), number(1)) else {
    eprintln!("perf-scaling THREADS EVERY DIR...");
    return ExitCode::from(2);
  };
  let mut found = Vec::new();
  for dir in &args[2..] {
    collect(Path::new(dir), &mut found);
  }
  found.sort();
  let files: Vec<PathBuf> = found.into_iter().step_by(every.max(1)).collect();
  let bytes: u64 = files
    .iter()
    .filter_map(|f| fs::metadata(f).ok())
    .map(|m| m.len())
    .sum();
  let settings = settings();
  let opts = check();
  // Read once first, so that every count of threads finds the files cached alike.
  let _ = once(&files, threads, &settings, &opts);
  let seconds = once(&files, threads, &settings, &opts);
  println!(
    "threads {} files {} bytes {} seconds {:.4}",
    threads,
    files.len(),
    bytes,
    seconds
  );
  ExitCode::SUCCESS
}

/// `perf-stages EVERY DIR...`: every EVERY-th file under the folders through each stage of
/// a `--check` in turn, on one thread, with the parses shared as `run_file` shares them;
/// prints `stage NAME SECONDS` for reading, layout, formatting, each verifier and the diff.
pub fn stages(args: &[String]) -> ExitCode {
  use mtindo::syntax::ParseCache;
  let Some(every) = args.first().and_then(|n| n.parse::<usize>().ok()) else {
    eprintln!("perf-stages EVERY DIR...");
    return ExitCode::from(2);
  };
  let mut found = Vec::new();
  for dir in &args[1..] {
    collect(Path::new(dir), &mut found);
  }
  found.sort();
  let layout_only = Settings {
    layout: Some(LayoutConfig::default()),
    format: None,
    edition: mtindo::syntax::Edition::E2021,
  };
  let format = FormatConfig::default();
  let names = [
    "read",
    "layout",
    "format",
    "verify.layout",
    "verify.format",
    "diff",
  ];
  let mut took = [Duration::ZERO; 6];
  let mut bytes = 0;
  for file in found.into_iter().step_by(every.max(1)) {
    let _parses = ParseCache::open();
    let mut now = Instant::now();
    let mut lap = |at: usize, took: &mut [Duration; 6]| {
      took[at] += now.elapsed();
      now = Instant::now();
    };
    let Ok(source) = fs::read_to_string(&file) else {
      continue;
    };
    lap(0, &mut took);
    bytes += source.len();
    let shaped = mtindo::shape(&source, &layout_only, &Default::default(), None);
    lap(1, &mut took);
    let Ok(shaped) = shaped else {
      continue;
    };
    let (laid_out, edition) = (shaped.laid_out, shaped.edition);
    let formatted = mtindo::format::file::format_file(&laid_out, edition, &format);
    lap(2, &mut took);
    if laid_out != source {
      let _ = mtindo::verify::verify_layout(&source, &laid_out, edition);
    }
    lap(3, &mut took);
    if formatted != laid_out {
      let _ = mtindo::verify::format::verify_format(&laid_out, &formatted, edition);
    }
    lap(4, &mut took);
    if formatted != source {
      let _ = mtindo::check_diff(&source, &formatted, "f", false);
    }
    lap(5, &mut took);
  }
  println!("bytes {}", bytes);
  for (name, took) in names.iter().zip(took) {
    println!("stage {} {:.4}", name, took.as_secs_f64());
  }
  ExitCode::SUCCESS
}

/// `perf-walk TIMES CONFIG DIR`: the walk `mto --config-path CONFIG DIR` (`-`: `mto DIR`) makes before any
/// file is laid out (folders read, each folder's configuration resolved), TIMES times;
/// prints `walk FILES SECONDS DIGEST` for the fastest, the digest of what it found.
pub fn walk(args: &[String]) -> ExitCode {
  use mtindo::config::{Environment, Overrides, Resolver};
  let (Some(times), Some(config), Some(dir)) = (
    args.first().and_then(|n| n.parse::<usize>().ok()),
    args.get(1),
    args.get(2),
  ) else {
    eprintln!("perf-walk TIMES CONFIG DIR");
    return ExitCode::from(2);
  };
  // `-`: no configuration file named, each folder's own found as `mto DIR` finds it.
  let overrides = Overrides {
    config_path: (config != "-").then(|| PathBuf::from(config)),
    ..Overrides::default()
  };
  let mut best = f64::MAX;
  let mut files = 0;
  let mut found = String::new();
  for _ in 0..times {
    let started = Instant::now();
    let mut resolver = match Resolver::new(&overrides, Environment::from_process()) {
      Ok(resolver) => resolver,
      Err(e) => {
        eprintln!("{}", e);
        return ExitCode::from(1);
      }
    };
    let opts = check();
    match mtindo::collect_jobs(&[PathBuf::from(dir)], &mut resolver, &opts) {
      Ok(collected) => {
        files = collected.jobs.len();
        // What the walk found, to compare walks by: each file with its settings, and the
        // folders passed over.
        found = mtindo::history::sha256::hex(
          format!("{:?} {:?}", collected.jobs, collected.nested).as_bytes(),
        );
      }
      Err(e) => {
        eprintln!("{}", e);
        return ExitCode::from(1);
      }
    }
    best = best.min(started.elapsed().as_secs_f64());
  }
  println!("walk {} {:.4} {}", files, best, found);
  ExitCode::SUCCESS
}

/// `perf-workload DIR`: each kind's largest file, written to DIR.
pub fn workload(args: &[String]) -> ExitCode {
  let Some(dir) = args.first().map(PathBuf::from) else {
    eprintln!("perf-workload DIR");
    return ExitCode::from(2);
  };
  if fs::create_dir_all(&dir).is_err() {
    eprintln!("cannot make {}", dir.display());
    return ExitCode::from(1);
  }
  for (kind, generate, smallest) in KINDS {
    let text = generate(smallest << (STEPS - 1));
    if fs::write(dir.join(format!("{}.rs", kind)), &text).is_err() {
      eprintln!("cannot write {}", dir.display());
      return ExitCode::from(1);
    }
    println!("workload {} {}", kind, text.len());
  }
  ExitCode::SUCCESS
}

/// A `match` with `n` arms.
fn arms(n: usize) -> String {
  let mut text = String::from("fn pick(x: u32) -> u32 {\n    match x {\n");
  for i in 0..n {
    text.push_str(&format!("{0} => {{ x+{0} }}\n", i));
  }
  text.push_str("_ => 0,\n    }\n}\n");
  text
}

/// A method chain of `n` calls on one line.
fn chain(n: usize) -> String {
  let mut text = String::from("fn long() {\n    let v = start()");
  for i in 0..n {
    text.push_str(&format!(".step_{}(x)", i));
  }
  text.push_str(";\n}\n");
  text
}

fn check() -> RunOptions {
  RunOptions {
    check: true,
    quiet: true,
    ..RunOptions::default()
  }
}

/// The fastest of as many runs of `f` as take [`MIN_TOTAL`], and at least three.
fn fastest(f: impl FnMut()) -> f64 {
  xtask_core::timing::fastest_of(3, MIN_TOTAL, f)
}

// The generators: each writes items in reverse order where it can, so layout moves them,
// and spacing formatting changes, so every stage has work.

/// `n` functions with ordinary bodies.
fn items(n: usize) -> String {
  let mut text = String::new();
  for i in (0..n).rev() {
    text.push_str(&format!(
      "pub fn f{0}(x: u32, y: &[u32]) -> u32 {{\n  let z = y.iter().map(|v| v + x).sum::<u32>();\n  if z > {0} {{ z - {0} }} else {{ z }}\n}}\n\n",
      i
    ));
  }
  text
}

/// One call with `n` arguments written on a single line.
fn line(n: usize) -> String {
  let args: Vec<String> = (0..n).map(|i| format!("arg_{}", i)).collect();
  format!(
    "fn wide() {{\n    let v = function_name({});\n}}\n",
    args.join(",")
  )
}

/// Blocks nested `n` deep inside one function, half as `if`s and half as closures in
/// calls, with a statement at each level.
fn nesting(n: usize) -> String {
  let mut text = String::from("fn deep(x: u32) -> u32 {\n");
  for i in 0..n {
    if i % 2 == 0 {
      text.push_str(&format!("let a{} = x;\nif x > {} {{\n", i, i));
    } else {
      text.push_str(&format!("let b{} = x;\ncall(|x| {{\n", i));
    }
  }
  text.push_str("x\n");
  for i in (0..n).rev() {
    if i % 2 == 0 {
      text.push_str("} else { 0 }\n");
    } else {
      text.push_str("})\n");
    }
  }
  text.push_str("}\n");
  text
}

/// Seconds for `threads` threads to take every file, each the next not yet taken.
fn once(files: &[PathBuf], threads: usize, settings: &Settings, opts: &RunOptions) -> f64 {
  let next = AtomicUsize::new(0);
  let started = Instant::now();
  std::thread::scope(|scope| {
    for _ in 0..threads {
      scope.spawn(|| {
        loop {
          let i = next.fetch_add(1, Ordering::Relaxed);
          let Some(file) = files.get(i) else {
            break;
          };
          let _ = run_file(file, settings, opts, WriteContext::default());
        }
      });
    }
  });
  started.elapsed().as_secs_f64()
}

fn settings() -> Settings {
  Settings {
    layout: Some(LayoutConfig::default()),
    format: Some(FormatConfig::default()),
    edition: mtindo::syntax::Edition::E2021,
  }
}
