//! `cargo xtask diff`: how much of the Rust style mto reproduces (stage 3, F1). Fetches the
//! corpus and the standard formatter's pinned test suite, then runs the harness in
//! `xtask-tools` (which links mtindo), comparing against `records/results/compatibility.txt`.
//! `--update` records new numbers; `--markdown` prints a shareable report.

use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::capabilities::{fetch as surface_fetch, load_surface};
use crate::common::log::Logger;
use crate::common::store;
use crate::common::{process, proof};
use crate::corpus;

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "diff");
  log.info(&format!("cargo xtask {}", crate::cli::line()));
  let args: Vec<String> = ["update", "markdown"]
    .iter()
    .filter(|f| crate::cli::on(m, f))
    .map(|f| format!("--{}", f))
    .collect();
  if corpus::fetch(root, false, &log) != 0 {
    return ExitCode::from(1);
  }
  let tests = match tests_dir(root, &log) {
    Ok(dir) => dir,
    Err(e) => {
      println!("{}", e);
      return ExitCode::from(1);
    }
  };
  let reference = match process::output(
    Command::new("rustfmt")
      .arg("--version")
      .env("RUSTUP_AUTO_INSTALL", "0")
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  ) {
    Ok(out) if out.code == 0 => String::from_utf8_lossy(&out.stdout).trim().to_string(),
    _ => {
      println!("the standard formatter (rustfmt) could not be run");
      return ExitCode::from(1);
    }
  };
  let pins = match corpus::load_pins(root) {
    Ok(pins) => pins,
    Err(e) => {
      println!("{}", e);
      return ExitCode::from(1);
    }
  };
  let mut forwarded: Vec<String> = vec![
    "diff".into(),
    "--tests".into(),
    tests.display().to_string(),
    "--reference".into(),
    reference,
    "--cache".into(),
    store::dir(root, store::CACHE)
      .join("diff")
      .display()
      .to_string(),
    "--record".into(),
    root
      .join("records/results/compatibility.txt")
      .display()
      .to_string(),
    "--meaning-log".into(),
    store::dir(root, store::LOGS)
      .join("diff-meaning.txt")
      .display()
      .to_string(),
    "--broke-log".into(),
    store::dir(root, store::LOGS)
      .join("diff-broke.txt")
      .display()
      .to_string(),
    "--exceptions".into(),
    root
      .join("records/policy/exceptions.txt")
      .display()
      .to_string(),
  ];
  forwarded.extend(args.iter().cloned());
  for pin in pins.iter().filter(|p| !p.opt_in) {
    forwarded.push(format!(
      "{}={}",
      pin.name,
      corpus::cache_dir(root).join(&pin.name).display()
    ));
  }
  log.info("running xtask-tools diff (release build)");
  let findings = store::dir(root, store::LOGS).join("diff-findings.tsv");
  let _ = std::fs::remove_file(&findings);
  let done = process::output(
    Command::new("cargo")
      .args([
        "run",
        "--quiet",
        "--release",
        "--package",
        "xtask-tools",
        "--",
      ])
      .args(&forwarded)
      .current_dir(root),
    process::HEAVY,
  );
  match done {
    Ok(done) => {
      if done.timed_out {
        println!(
          "the diff was stopped at its time limit of {}",
          process::duration(process::HEAVY)
        );
      }
      // The run's numbers become series.
      let tallies = store::dir(root, store::LOGS).join("diff-tallies.tsv");
      if let Ok(text) = std::fs::read_to_string(&tallies) {
        for line in text.lines() {
          let f: Vec<&str> = line.split('\t').collect();
          let n = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
          if let Some(set) = f.first() {
            let share = |a: f64, b: f64| if b > 0.0 { 100.0 * a / b } else { 0.0 };
            crate::metrics::record(
              root,
              &format!("compat.{}.files", set),
              share(n(1), n(2)),
              "%",
            );
            crate::metrics::record(
              root,
              &format!("compat.{}.lines", set),
              share(n(3), n(4)),
              "%",
            );
            crate::metrics::record(root, &format!("compat.{}.skipped", set), n(5), "files");
          }
        }
        let _ = std::fs::remove_file(&tallies);
      }
      // The run's findings become incidents; on a complete run, those no longer found
      // are fixed.
      if let Ok(text) = std::fs::read_to_string(&findings) {
        let inputs = store::dir(root, store::LOGS).join("diff-inputs");
        let counts = crate::incidents::from_diff(root, &text, &inputs);
        println!(
          "incidents: {} new, {} reopened, {} seen again, {} fixed (cargo xtask incidents)",
          counts.new, counts.reopened, counts.again, counts.fixed
        );
        log.info(&format!("incidents from the diff: {:?}", counts));
      }
      // A passing run is a proof for what it read (taken after `--update` wrote the
      // record), so the release check need not run it again.
      if done.code == 0
        && !done.timed_out
        && let Some(hash) = proof::inputs(root, &proof::COMPAT)
      {
        let run = std::env::var_os(crate::runs::RUN_DIR)
          .and_then(|d| {
            Path::new(&d)
              .file_name()
              .map(|n| n.to_string_lossy().into_owned())
          })
          .unwrap_or_else(|| "-".to_string());
        proof::record(root, &proof::COMPAT, &hash, &run);
      }
      ExitCode::from(done.code.clamp(0, 255) as u8)
    }
    Err(e) => {
      println!("cannot run cargo: {}", e);
      ExitCode::from(1)
    }
  }
}

/// The pinned stable release's `tests` folder, from the capabilities cache.
fn tests_dir(root: &Path, log: &Logger) -> Result<std::path::PathBuf, String> {
  let surface = load_surface(root)?;
  let description = surface
        .sources
        .iter()
        .find(|(label, _)| label == "stable")
        .map(|(_, d)| d.clone())
        .ok_or("no stable release in records/upstream/rustfmt-surface.txt; run `cargo xtask capabilities refresh` online")?;
  let commit = description
    .split([' ', ','])
    .find(|w| w.len() == 40 && w.chars().all(|c| c.is_ascii_hexdigit()))
    .ok_or("the stable source names no commit")?
    .to_string();
  let checkout = surface_fetch::cached_checkout(root, &commit, log)?.ok_or(
    "the standard formatter's tests are not cached; run `cargo xtask capabilities refresh` online",
  )?;
  Ok(checkout.join("tests"))
}
