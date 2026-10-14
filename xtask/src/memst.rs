//! `cargo xtask memst`: where mto's memory and time go, function by function.
//!
//! Builds `xtask/memst` with frame pointers forced on (in `target/memst/`, apart from the
//! usual builds), runs mto's pipeline on the given paths under its recorder, then prints
//! the report. Each run keeps its files in `.dev/runs/<UTC>-memst/`:
//!
//! ```text
//! mto.toml      the settings used: layout and formatting on, nothing from outside
//! profile.memst every allocation stack and CPU-time sample, aggregated (see memst)
//! samples.tsv   the process's memory and CPU once a second, as for any run
//! report.txt    what was printed
//! ```
//!
//! - `cargo xtask memst [PATH...] [--view NAME|all] [--by bytes|allocs|time] [--top N]`:
//!   profile PATH (default: the corpus's serde, else this repository's `src/`).
//! - `cargo xtask memst report FILE [--view ...]`: the report of a kept profile.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cli::{text, texts};
use crate::common::log::Logger;
use crate::common::{process, sample};

/// The settings profiled: layout and formatting, with no configuration from outside.
const CONFIG: &str = "root = true\nlayout = true\nformat = true\n";

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "memst");
  log.info(&format!("cargo xtask {}", crate::cli::line()));
  let report_args = |m: &clap::ArgMatches| {
    let mut flags = vec!["--view".to_string(), text(m, "view").unwrap_or_default()];
    flags.extend(["--by".to_string(), text(m, "by").unwrap_or_default()]);
    let top = m.get_one::<u32>("top").copied().unwrap_or(15);
    flags.extend(["--top".to_string(), top.to_string()]);
    flags
  };
  if let Err(e) = build(root, &log) {
    println!("{}", e);
    log.error(&e);
    return ExitCode::from(1);
  }
  if m.subcommand().is_none() {
    crate::common::conditions::settle(root, "memst");
  }
  if let Some(("report", sub)) = m.subcommand() {
    let file = text(sub, "file").unwrap_or_default();
    return match report(root, Path::new(&file), &report_args(sub)) {
      Ok(text) => {
        print!("{}", text);
        ExitCode::SUCCESS
      }
      Err(e) => {
        println!("{}", e);
        ExitCode::from(1)
      }
    };
  }
  let report_args = report_args(m);
  let rest = texts(m, "paths");
  let mut paths: Vec<PathBuf> = rest.iter().map(PathBuf::from).collect();
  if paths.is_empty() {
    let serde = root.join("compat/cache/corpus/serde");
    paths.push(if serde.is_dir() {
      serde
    } else {
      root.join("src")
    });
  }
  let samples = sample::run_path(root, "memst");
  let dir = samples.parent().unwrap_or(root).to_path_buf();
  let config = dir.join("mto.toml");
  let profile = dir.join("profile.memst");
  if let Err(e) = fs::create_dir_all(&dir).and_then(|_| fs::write(&config, CONFIG)) {
    println!("cannot write {}: {}", config.display(), e);
    return ExitCode::from(1);
  }
  let mut run_args = vec![
    "run".to_string(),
    "--out".to_string(),
    profile.display().to_string(),
    "--config-path".to_string(),
    config.display().to_string(),
  ];
  run_args.extend(paths.iter().map(|p| p.display().to_string()));
  println!(
    "profiling mto on {}...",
    paths
      .iter()
      .map(|p| p.display().to_string())
      .collect::<Vec<_>>()
      .join(" ")
  );
  let (outcome, used) = process::run_sampled(
    root,
    &binary(root).display().to_string(),
    &run_args,
    &[],
    process::HEAVY,
    Some(&samples),
  );
  print!("{}", outcome.output);
  if let Some(used) = used.filter(|u| u.samples > 0) {
    println!("process memory: {}", used.describe());
  }
  if outcome.code != 0 {
    log.error(&format!("memst run failed: {}", outcome.output));
    return ExitCode::from(1);
  }
  record_metrics(root, &paths, &profile);
  match report(root, &profile, &report_args) {
    Ok(text) => {
      print!("\n{}", text);
      let _ = fs::write(dir.join("report.txt"), &text);
      println!(
        "\nkept in {} (cargo xtask memst report {} --view NAME)",
        relative(root, &dir),
        relative(root, &profile)
      );
      log.info(&format!("memst run kept in {}", dir.display()));
      ExitCode::SUCCESS
    }
    Err(e) => {
      println!("{}", e);
      ExitCode::from(1)
    }
  }
}

fn binary(root: &Path) -> PathBuf {
  root.join("target/memst/release/memst")
}

/// Builds memst with frame pointers in every function, so each stack can be walked.
fn build(root: &Path, log: &Logger) -> Result<(), String> {
  println!("building memst (release, frame pointers forced on)...");
  let env = [
    (
      "RUSTFLAGS".to_string(),
      "-C force-frame-pointers=yes".to_string(),
    ),
    (
      "CARGO_TARGET_DIR".to_string(),
      root.join("target/memst").display().to_string(),
    ),
  ];
  let args: Vec<String> = ["build", "--quiet", "--release", "--package", "memst"]
    .iter()
    .map(|s| s.to_string())
    .collect();
  let outcome = process::run(root, "cargo", &args, &env, process::LONG);
  if outcome.code != 0 {
    return Err(format!("cannot build memst:\n{}", outcome.output));
  }
  log.info("memst built");
  Ok(())
}

/// The profile's totals as series `memst.<input>.*`, the input named by its folder
/// (`serde` for the corpus's serde); the ratchet checks those it holds.
fn record_metrics(root: &Path, paths: &[PathBuf], profile: &Path) {
  let label = match paths {
    [one] => one
      .file_name()
      .map(|n| n.to_string_lossy().into_owned())
      .unwrap_or_else(|| "input".to_string()),
    _ => "several".to_string(),
  };
  let text = fs::read_to_string(profile).unwrap_or_default();
  let field = |key: &str| {
    text
      .lines()
      .find_map(|l| l.strip_prefix(key))
      .map(|rest| {
        rest
          .split_whitespace()
          .map(str::to_string)
          .collect::<Vec<_>>()
      })
      .unwrap_or_default()
  };
  let totals = field("totals ");
  let number = |i: usize| totals.get(i).and_then(|v| v.parse::<f64>().ok());
  let time = field("time_elapsed_ns ")
    .first()
    .and_then(|v| v.parse::<f64>().ok());
  let figures = [
    ("allocs", number(0), "allocs"),
    ("reallocs", number(2), "reallocs"),
    ("bytes", number(3), "bytes"),
    ("peak_live", number(5), "bytes"),
    ("live_end", number(7), "bytes"),
    ("time_ms", time.map(|ns| ns / 1e6), "ms"),
  ];
  for (name, value, unit) in figures {
    if let Some(value) = value
      && let Some(breach) =
        crate::metrics::record_checked(root, &format!("memst.{}.{}", label, name), value, unit)
    {
      println!("ratchet: {} (filed as an incident)", breach);
    }
  }
}

fn relative(root: &Path, path: &Path) -> String {
  path
    .strip_prefix(root)
    .unwrap_or(path)
    .display()
    .to_string()
}

fn report(root: &Path, profile: &Path, flags: &[String]) -> Result<String, String> {
  let mut args = vec!["report".to_string(), profile.display().to_string()];
  args.extend(flags.iter().cloned());
  let outcome = process::run(
    root,
    &binary(root).display().to_string(),
    &args,
    &[],
    process::LONG,
  );
  if outcome.code != 0 {
    return Err(outcome.output);
  }
  Ok(outcome.output)
}
