//! `cargo xtask memst`: where a Rust program's time and memory go, function by function.
//!
//! Runs memst, a project of its own (`cargo install --path ../memst` puts `cargo-memst` on
//! the path): memst builds a shadow of the program's workspace with its recorder in it,
//! runs the binary (plain runs, a sampling run, a memory run, and a probe run when functions
//! are probed) and reports. Each run keeps its files in `.dev/runs/<UTC>-memst/`, its
//! `meta.txt` naming the memst version that made it.
//!
//! - `cargo xtask memst`: mto on the corpus's serde (`--input PATH...` for other inputs),
//!   with layout and formatting on and no configuration from outside; the totals are kept
//!   as `memst.<input>.*` series, which the ratchet checks.
//! - `cargo xtask memst PROJECT [--bin NAME] -- ARGS...`: any Cargo workspace's binary,
//!   run with ARGS.
//! - Either: `--function PATH,...` and `--file FILE,...` (timed exactly by probes, and
//!   what the views keep to), `--build release|debug`, `--repeat N`, `--no-time`,
//!   `--no-memory`, and the report's `--view`, `--by`, `--top`.
//! - `cargo xtask memst report RUN [...]`: a kept run's report again.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cli::{on, text, texts};
use crate::common::log::Logger;
use crate::common::{process, sample};

/// memst's binary, as `cargo install` names it.
const TOOL: &str = "cargo-memst";

/// The settings mto is profiled with: layout and formatting, nothing from outside.
const CONFIG: &str = "root = true\nlayout = true\nformat = true\n";

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "memst");
  log.info(&format!("cargo xtask {}", crate::cli::line()));
  if !process::on_path(TOOL) {
    let e = format!(
      "memst is not installed: `cargo install --path ../memst` puts {} on the path",
      TOOL
    );
    println!("{}", e);
    log.error(&e);
    return ExitCode::from(1);
  }
  if let Some(("report", sub)) = m.subcommand() {
    let mut args = vec!["report".to_string(), text(sub, "run").unwrap_or_default()];
    args.extend(report_flags(sub));
    return memst(root, &args).0;
  }
  crate::common::conditions::settle(root, "memst");
  let dir = sample::run_path(root, "memst")
    .parent()
    .unwrap_or(root)
    .to_path_buf();
  if let Err(e) = fs::create_dir_all(&dir) {
    println!("cannot make {}: {}", dir.display(), e);
    return ExitCode::from(1);
  }
  let mut args = vec!["profile".to_string()];
  let project = text(m, "project");
  let mut inputs: Vec<PathBuf> = Vec::new();
  let program_args = match &project {
    Some(project) => {
      args.push(project.clone());
      texts(m, "args")
    }
    None => {
      // mto, on the inputs given or the corpus's serde.
      inputs = texts(m, "input").iter().map(PathBuf::from).collect();
      if inputs.is_empty() {
        let serde = root.join("compat/cache/corpus/serde");
        inputs.push(if serde.is_dir() {
          serde
        } else {
          root.join("src")
        });
      }
      let config = dir.join("mto.toml");
      if let Err(e) = fs::write(&config, CONFIG) {
        println!("cannot write {}: {}", config.display(), e);
        return ExitCode::from(1);
      }
      args.extend(["--bin".to_string(), "mto".to_string()]);
      let mut mto = vec![
        "--dry-run".to_string(),
        "--no-history".to_string(),
        "-q".to_string(),
        "--config-path".to_string(),
        config.display().to_string(),
      ];
      mto.extend(inputs.iter().map(|p| p.display().to_string()));
      mto
    }
  };
  args.extend(["--out".to_string(), dir.display().to_string()]);
  for (flag, id) in [
    ("--bin", "bin"),
    ("--package", "package"),
    ("--build", "build"),
  ] {
    if let Some(value) = text(m, id) {
      args.extend([flag.to_string(), value]);
    }
  }
  if let Some(repeat) = m.get_one::<u32>("repeat") {
    args.extend(["--repeat".to_string(), repeat.to_string()]);
  }
  for id in ["no-time", "no-memory"] {
    if on(m, id) {
      args.push(format!("--{}", id));
    }
  }
  args.extend(report_flags(m));
  args.push("--".to_string());
  args.extend(program_args);
  let (code, ok) = memst(root, &args);
  if !ok {
    log.error("memst profile failed");
    return code;
  }
  if project.is_none() {
    record_metrics(root, &inputs, &dir);
  }
  println!(
    "\nkept in {} (cargo xtask memst report {})",
    relative(root, &dir),
    relative(root, &dir)
  );
  log.info(&format!("memst run kept in {}", dir.display()));
  code
}

/// The report's flags, as memst takes them.
fn report_flags(m: &clap::ArgMatches) -> Vec<String> {
  let mut out = Vec::new();
  if let Some(by) = text(m, "by") {
    out.extend(["--by".to_string(), by]);
  }
  if let Some(top) = m.get_one::<u32>("top") {
    out.extend(["--top".to_string(), top.to_string()]);
  }
  for id in ["view", "function", "file"] {
    let values = texts(m, id);
    if !values.is_empty() {
      out.extend([format!("--{}", id), values.join(",")]);
    }
  }
  out
}

/// Runs memst, its output shown as it comes; the exit code, and whether it succeeded.
fn memst(root: &Path, args: &[String]) -> (ExitCode, bool) {
  let tap: process::Tap = Box::new(|bytes: &[u8]| {
    let mut out = std::io::stdout();
    let _ = out.write_all(bytes);
    let _ = out.flush();
  });
  let (outcome, _) = process::run_tapped(
    root,
    TOOL,
    args,
    &[],
    process::HEAVY,
    None,
    Some(tap),
  );
  if outcome.timed_out {
    println!("memst timed out");
  }
  let ok = outcome.code == 0;
  (ExitCode::from(outcome.code.clamp(0, 255) as u8), ok)
}

/// The run's totals as series `memst.<input>.*`, the input named by its folder (`serde`
/// for the corpus's serde); the ratchet checks those it holds.
fn record_metrics(root: &Path, inputs: &[PathBuf], dir: &Path) {
  let label = match inputs {
    [one] => one
      .file_name()
      .map(|n| n.to_string_lossy().into_owned())
      .unwrap_or_else(|| "input".to_string()),
    _ => "several".to_string(),
  };
  let read = |name: &str| fs::read_to_string(dir.join(name)).unwrap_or_default();
  let field = |text: &str, key: &str| -> Vec<f64> {
    text
      .lines()
      .find_map(|l| l.strip_prefix(key))
      .map(|rest| {
        rest
          .split_whitespace()
          .filter_map(|v| v.parse().ok())
          .collect()
      })
      .unwrap_or_default()
  };
  let memory = read("memory.memst");
  let totals = field(&memory, "totals ");
  let time = field(&read("time.memst"), "elapsed_ns ").first().copied();
  let figures = [
    ("allocs", totals.first().copied(), "allocs"),
    ("reallocs", totals.get(2).copied(), "reallocs"),
    ("bytes", totals.get(3).copied(), "bytes"),
    ("peak_live", totals.get(5).copied(), "bytes"),
    ("live_end", totals.get(7).copied(), "bytes"),
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
