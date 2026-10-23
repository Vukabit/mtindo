//! `cargo xtask io bench`: what writing costs, on this machine and file system, kept as
//! `io.*` series.
//!
//! A project of generated files, every one of which mto rewrites, is laid out by the
//! release `mto` with `safety.sync = "batch"` (the default) with history and without, and
//! with `"full"` with history; median of three fresh runs each, against a `--check` of the
//! same files that writes nothing. Generated, so it needs no network and runs the same on
//! every platform CI builds for: the figures show what each platform's way of syncing
//! (`src/durable`) costs per file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

use crate::common::process;
use crate::metrics;

/// Files in the generated project unless `--files` says otherwise; the series are kept
/// for this size only.
const DEFAULT_FILES: u32 = 600;

/// Files in a folder of the generated project.
const PER_FOLDER: usize = 25;

const WAYS: &[Way] = &[
  Way {
    name: "batch.history",
    sync: "batch",
    history: true,
  },
  Way {
    name: "batch.plain",
    sync: "batch",
    history: false,
  },
  Way {
    name: "full.history",
    sync: "full",
    history: true,
  },
];

/// One way of running mto: its series name, `safety.sync`, and whether history is on.
struct Way {
  name: &'static str,
  sync: &'static str,
  history: bool,
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let Some(("bench", sub)) = m.subcommand() else {
    eprintln!("cargo xtask io bench; see `cargo xtask io -h`");
    return ExitCode::from(2);
  };
  let files = sub
    .get_one::<u32>("files")
    .copied()
    .unwrap_or(DEFAULT_FILES) as usize;
  // A run's fixed costs weigh differently on another number of files, so only the
  // default size is kept.
  let keep = files == DEFAULT_FILES as usize;
  let build = ["build", "--quiet", "--release", "--bin", "mto"].map(String::from);
  let (code, output) = process::capture(root, "cargo", &build, &[], process::LONG);
  if code != 0 {
    println!("cannot build mto:\n{}", output);
    return ExitCode::from(1);
  }
  let mto = root
    .join("target")
    .join("release")
    .join(format!("mto{}", std::env::consts::EXE_SUFFIX));
  crate::common::conditions::settle(root, "the write bench");
  let scratch = std::env::temp_dir().join("mtindo-io-bench");
  println!(
    "writing {} generated files ({} {}, in {}), middle of three runs:",
    files,
    std::env::consts::OS,
    std::env::consts::ARCH,
    scratch.display()
  );
  let mut breaches = Vec::new();
  match time(&mto, &scratch, files, None) {
    Ok(seconds) => report(root, "check", seconds, files, keep, &mut breaches),
    Err(e) => {
      println!("cannot time a check: {}", e);
      return ExitCode::from(1);
    }
  }
  for way in WAYS {
    match time(&mto, &scratch, files, Some(way)) {
      Ok(seconds) => report(root, way.name, seconds, files, keep, &mut breaches),
      Err(e) => {
        println!("cannot time {}: {}", way.name, e);
        return ExitCode::from(1);
      }
    }
  }
  let _ = fs::remove_dir_all(&scratch);
  for breach in &breaches {
    println!("{}", breach);
  }
  if breaches.is_empty() {
    ExitCode::SUCCESS
  } else {
    ExitCode::from(1)
  }
}

/// `files` files in folders of [`PER_FOLDER`], each with its items in reverse order, so
/// that mto rewrites every one.
fn generate(project: &Path, files: usize) -> std::io::Result<()> {
  fs::create_dir_all(project)?;
  fs::write(
    project.join("mto.toml"),
    "root = true\nlayout = true\nformat = false\n",
  )?;
  for i in 0..files {
    let folder: PathBuf = project.join(format!("m{}", i / PER_FOLDER));
    if i % PER_FOLDER == 0 {
      fs::create_dir_all(&folder)?;
    }
    let mut text = String::new();
    for f in (0..12).rev() {
      text.push_str(&format!(
        "fn f{0}_{1}(x: u32) -> u32 {{\n    let y = x + {1};\n    y * 2\n}}\n\n",
        i, f
      ));
    }
    text.push_str(&format!("struct S{};\n", i));
    fs::write(folder.join(format!("f{}.rs", i)), text)?;
  }
  Ok(())
}

fn report(
  root: &Path,
  name: &str,
  seconds: f64,
  files: usize,
  keep: bool,
  breaches: &mut Vec<String>,
) {
  let per_file = 1000.0 * seconds / files as f64;
  println!(
    "  {:<14} {:>6.2} s  {:>6.3} ms per file",
    name, seconds, per_file
  );
  if !keep {
    return;
  }
  let round = |x: f64| (x * 1000.0).round() / 1000.0;
  metrics::record(
    root,
    &format!("io.write.{}.seconds", name),
    round(seconds),
    "s",
  );
  breaches.extend(metrics::record_checked(
    root,
    &format!("io.write.{}.ms_per_file", name),
    round(per_file),
    "ms",
  ));
}

/// The middle of three runs of mto over a fresh project: one writing it the `way` given,
/// or a `--check` with none.
fn time(mto: &Path, scratch: &Path, files: usize, way: Option<&Way>) -> Result<f64, String> {
  let mut runs = Vec::new();
  for _ in 0..3 {
    let project = scratch.join("project");
    let state = scratch.join("state");
    let _ = fs::remove_dir_all(scratch);
    generate(&project, files).map_err(|e| e.to_string())?;
    fs::create_dir_all(&state).map_err(|e| e.to_string())?;
    let config = scratch.join("config.toml");
    let sync = way.map_or("batch", |w| w.sync);
    fs::write(&config, format!("[safety]\nsync = \"{}\"\n", sync)).map_err(|e| e.to_string())?;
    let mut command = Command::new(mto);
    command
      .current_dir(&project)
      .env("MTINDO_CONFIG", &config)
      .env("XDG_STATE_HOME", &state)
      .env("LOCALAPPDATA", &state)
      .env("CI", "1");
    match way {
      None => command.args(["--check", "-q", "."]),
      Some(w) if !w.history => command.args(["--no-history", "--allow-dirty", "-q", "."]),
      Some(_) => command.args(["-q", "."]),
    };
    let start = Instant::now();
    let output = process::output(&mut command, process::LONG).map_err(|e| e.to_string())?;
    let seconds = start.elapsed().as_secs_f64();
    // A check finds every file would change (1); a run writes them all (0).
    let expected = if way.is_none() { 1 } else { 0 };
    if output.code != expected {
      return Err(format!(
        "mto exited {}: {}",
        output.code,
        String::from_utf8_lossy(&output.stderr)
      ));
    }
    runs.push(seconds);
  }
  runs.sort_by(|a, b| a.total_cmp(b));
  Ok(runs[1])
}
