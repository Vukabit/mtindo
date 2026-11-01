//! `cargo xtask doctor bench`: what the build guard (`mto doctor`) costs and how well it
//! narrows, kept as `doctor.*` series (`.dev/metrics/`), some held by the ratchet.
//!
//! - **In mto** (from `xtask-tools doctor-bench`, which links mtindo): shaping speed with
//!   and without remembered holds, and narrowings on corpus files with planted culprits and
//!   a pretend build: checks taken, mto's own time between them, and whether only the
//!   culprits were held back.
//! - **On real crates**: a corpus crate copied aside, laid out without the guard and then
//!   with it, checking with `cargo check --offline`, its target folder kept warm as in
//!   daily work. What the guard adds to a run is about two checks of the crate.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use crate::common::process;
use crate::common::timing;
use crate::metrics;

/// The crates guarded runs are timed on, small, medium and large.
const GUARDED: &[&str] = &["itoa", "serde", "tokio"];

/// The corpus crates the in-process figures are measured on.
const SHAPED: &[&str] = &["serde", "tokio", "clap", "syn", "regex"];

struct Figures {
  /// One warm `cargo check` of the crate as written: the check before a run.
  check: f64,
  /// One `cargo check` right after mto laid the crate out: the check after a run, which
  /// compiles again what changed.
  rebuild: f64,
  unguarded: f64,
  guarded: f64,
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let Some(("bench", sub)) = m.subcommand() else {
    eprintln!("cargo xtask doctor bench; see `cargo xtask doctor -h`");
    return ExitCode::from(2);
  };
  let trials = sub.get_one::<u32>("trials").copied().unwrap_or(10);
  let corpus = crate::corpus::cache_dir(root);
  if !SHAPED.iter().all(|c| corpus.join(c).is_dir()) {
    eprintln!("the corpus is not checked out: run `cargo xtask corpus check` first");
    return ExitCode::from(2);
  }
  let build = ["build", "--quiet", "--release", "--package", "xtask-tools"].map(String::from);
  if process::capture(root, "cargo", &build, &[], process::LONG).0 != 0 {
    eprintln!("cannot build xtask-tools");
    return ExitCode::from(1);
  }
  crate::common::conditions::settle(root, "the doctor bench");
  let mut failed = false;
  let mut breaches = Vec::new();
  // In mto.
  let mut args: Vec<String> = [
    "run",
    "--quiet",
    "--release",
    "--package",
    "xtask-tools",
    "--",
    "doctor-bench",
    "--trials",
  ]
  .iter()
  .map(|s| s.to_string())
  .collect();
  args.push(trials.to_string());
  args.extend(SHAPED.iter().map(|c| corpus.join(c).display().to_string()));
  let (code, output) = process::capture(root, "cargo", &args, &[], process::LONG);
  for line in output.lines() {
    match line.strip_prefix("metric ") {
      Some(figure) => {
        let words: Vec<&str> = figure.split_whitespace().collect();
        if let [name, value, rest @ ..] = words.as_slice()
          && let Ok(value) = value.parse::<f64>()
        {
          let unit = rest.first().copied().unwrap_or("");
          breaches.extend(metrics::record_checked(root, name, value, unit));
        }
      }
      None => println!("{}", line),
    }
  }
  if code != 0 {
    println!("xtask-tools doctor-bench failed (exit {})", code);
    failed = true;
  }
  // On real crates.
  let (code, built) = process::capture(
    root,
    "cargo",
    &["build", "--quiet", "--release", "--bin", "mto"]
      .iter()
      .map(|s| s.to_string())
      .collect::<Vec<_>>(),
    &[],
    process::LONG,
  );
  if code != 0 {
    println!("cannot build mto:\n{}", built);
    return ExitCode::from(1);
  }
  let mto = root.join("target").join("release").join("mto");
  println!("guarded runs (cargo check --offline, target folder warm; middle of three):");
  for name in GUARDED {
    match guarded(&mto, &corpus.join(name), name) {
      Ok(figures) => {
        let overhead = figures.guarded - figures.unguarded;
        println!(
          "  {:<6} mto {:>5.2} s, guarded {:>5.2} s: +{:.2} s, for checks of {:.2} s before \
           and {:.2} s after ({:.0}% of the added time)",
          name,
          figures.unguarded,
          figures.guarded,
          overhead,
          figures.check,
          figures.rebuild,
          100.0 * (figures.check + figures.rebuild) / overhead.max(1e-3)
        );
        let series = |what: &str| format!("doctor.real.{}.{}", name, what);
        metrics::record(root, &series("check.seconds"), round(figures.check), "s");
        metrics::record(
          root,
          &series("rebuild.seconds"),
          round(figures.rebuild),
          "s",
        );
        metrics::record(
          root,
          &series("unguarded.seconds"),
          round(figures.unguarded),
          "s",
        );
        metrics::record(
          root,
          &series("guarded.seconds"),
          round(figures.guarded),
          "s",
        );
        metrics::record(root, &series("overhead.seconds"), round(overhead), "s");
      }
      Err(why) => println!("  {:<8} not measured: {}", name, why),
    }
  }
  for breach in &breaches {
    println!("{}", breach);
  }
  if failed || !breaches.is_empty() {
    ExitCode::from(1)
  } else {
    ExitCode::SUCCESS
  }
}

/// Copies the files under `from` to `to`, leaving out `.git` and `target`.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
  fs::create_dir_all(to)?;
  for entry in fs::read_dir(from)? {
    let entry = entry?;
    let name = entry.file_name();
    if name == ".git" || name == "target" {
      continue;
    }
    let path: PathBuf = entry.path();
    let kind = entry.file_type()?;
    if kind.is_dir() {
      copy_tree(&path, &to.join(&name))?;
    } else if kind.is_file() {
      fs::copy(&path, to.join(&name))?;
    }
  }
  Ok(())
}

/// Times mto on a copy of `crate_dir` without the guard and with it.
fn guarded(mto: &Path, crate_dir: &Path, name: &str) -> Result<Figures, String> {
  // Outside the repository, so cargo does not take the copy for a member of its
  // workspace.
  let scratch = std::env::temp_dir().join("mtindo-doctor-bench").join(name);
  let copy = scratch.join("crate");
  let state = scratch.join("state");
  let _ = fs::remove_dir_all(&copy);
  let _ = fs::remove_dir_all(&state);
  fs::create_dir_all(&state).map_err(|e| e.to_string())?;
  let config = scratch.join("config.toml");
  fs::write(
    &config,
    "[doctor]\nenabled = true\ncommand = [\"cargo\", \"check\", \"--offline\", \"--quiet\"]\n",
  )
  .map_err(|e| e.to_string())?;
  let env: Vec<(String, String)> = [
    ("MTINDO_CONFIG", config.display().to_string()),
    ("XDG_STATE_HOME", state.display().to_string()),
    // Kept apart from the copy, so it stays warm when the copy is replaced.
    (
      "CARGO_TARGET_DIR",
      scratch.join("target").display().to_string(),
    ),
  ]
  .iter()
  .map(|(k, v)| (k.to_string(), v.clone()))
  .collect();
  let fresh = || -> Result<(), String> {
    let _ = fs::remove_dir_all(&copy);
    copy_tree(crate_dir, &copy).map_err(|e| e.to_string())?;
    fs::write(
      copy.join("mto.toml"),
      "root = true\nlayout = true\nformat = false\n",
    )
    .map_err(|e| e.to_string())
  };
  let check = |timed: bool| -> Result<f64, String> {
    let started = Instant::now();
    let (code, output) = process::capture(
      &copy,
      "cargo",
      &["check", "--offline", "--quiet"]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
      &env,
      process::LONG,
    );
    if code != 0 {
      let last = output.lines().rev().find(|l| !l.trim().is_empty());
      return Err(format!(
        "cargo check fails{}: {}",
        if timed { "" } else { " before mto" },
        last.unwrap_or("")
      ));
    }
    Ok(started.elapsed().as_secs_f64())
  };
  let lay_out = |guard: bool| -> Result<f64, String> {
    let mut args: Vec<String> = vec!["-q".into(), ".".into()];
    if !guard {
      args.push("--unguarded".into());
    }
    let started = Instant::now();
    let (code, output) = process::capture(
      &copy,
      &mto.display().to_string(),
      &args,
      &env,
      process::LONG,
    );
    // Exit 3: files with syntax errors, left alone.
    if code != 0 && code != 3 {
      return Err(format!("mto exited with {}: {}", code, output.trim()));
    }
    Ok(started.elapsed().as_secs_f64())
  };
  // Writing files dominates a run's time and varies from run to run (each file is synced
  // to disk), so each figure is the middle of three, each on a fresh copy with the target
  // folder warm on the crate as written.
  let mut times = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
  for _ in 0..3 {
    fresh()?;
    check(false)?;
    times.0.push(check(true)?);
    times.1.push(lay_out(false)?);
    times.3.push(check(true)?);
    fresh()?;
    check(false)?;
    times.2.push(lay_out(true)?);
  }
  Ok(Figures {
    check: timing::middle(times.0),
    rebuild: timing::middle(times.3),
    unguarded: timing::middle(times.1),
    guarded: timing::middle(times.2),
  })
}

fn round(value: f64) -> f64 {
  (value * 1000.0).round() / 1000.0
}
