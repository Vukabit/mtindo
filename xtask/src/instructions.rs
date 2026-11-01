//! `cargo xtask speed instructions [--update]`: how many instructions the release mto runs
//! to check each file of a fixed workload (`xtask-tools perf-workload`: a large file of
//! each way a file grows), counted by Cachegrind. A count is the same from run to run on
//! the same build and machine, so a CI runner, too noisy for wall time, still catches a
//! few percent of slowdown.
//!
//! ```text
//! records/results/instructions.txt
//!   target x86_64-unknown-linux-gnu
//!   count KIND INSTRUCTIONS
//! ```
//!
//! Counts are compared on the target the record names (Valgrind's, Linux); a count more
//! than [`TOLERANCE`] above its record fails, one as far below asks for `--update`.
//! Elsewhere, or without Valgrind, the measure says so and passes.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::common::process;

/// The record, from the repository root.
pub const RECORD: &str = "records/results/instructions.txt";

/// How far above its record a count may be.
pub const TOLERANCE: f64 = 0.03;

pub fn run(root: &Path, update: bool) -> ExitCode {
  if !cfg!(target_os = "linux") || !process::on_path("valgrind") {
    println!(
      "instructions: counted on Linux with Valgrind, which this machine lacks; CI counts them"
    );
    return ExitCode::SUCCESS;
  }
  let build = ["build", "--quiet", "--release", "--bin", "mto"].map(String::from);
  if process::capture(root, "cargo", &build, &[], process::LONG).0 != 0 {
    println!("cannot build mto");
    return ExitCode::from(1);
  }
  let mto = root.join("target/release/mto");
  let scratch = std::env::temp_dir().join(format!("mtindo-instructions-{}", std::process::id()));
  let _ = fs::remove_dir_all(&scratch);
  let workload = scratch.join("workload");
  let args: Vec<String> = [
    "run",
    "--quiet",
    "--release",
    "--package",
    "xtask-tools",
    "--",
    "perf-workload",
  ]
  .iter()
  .map(|s| s.to_string())
  .chain([workload.display().to_string()])
  .collect();
  let (code, said) = process::capture(root, "cargo", &args, &[], process::LONG);
  if code != 0 {
    println!("cannot write the workload:\n{}", said);
    return ExitCode::from(1);
  }
  let kinds: Vec<String> = said
    .lines()
    .filter_map(|l| l.strip_prefix("workload "))
    .filter_map(|rest| rest.split_whitespace().next())
    .map(str::to_string)
    .collect();
  let mut counts = BTreeMap::new();
  for kind in &kinds {
    match count(&mto, &workload.join(format!("{}.rs", kind)), &scratch) {
      Ok(n) => {
        counts.insert(kind.clone(), n);
      }
      Err(e) => {
        println!("instructions: {}: {}", kind, e);
        let _ = fs::remove_dir_all(&scratch);
        return ExitCode::from(1);
      }
    }
  }
  let _ = fs::remove_dir_all(&scratch);
  let target = format!("{}-unknown-linux-gnu", std::env::consts::ARCH);
  if update {
    let _ = fs::write(root.join(RECORD), record_text(&target, &counts));
    println!("recorded in {}", RECORD);
  }
  let recorded = parse(&fs::read_to_string(root.join(RECORD)).unwrap_or_default());
  let (report, failed) = compare(&target, &counts, &recorded);
  print!("{}", report);
  if failed {
    ExitCode::from(1)
  } else {
    ExitCode::SUCCESS
  }
}

/// Each count against its record: the report, and whether one grew past the tolerance.
fn compare(
  target: &str,
  counts: &BTreeMap<String, u64>,
  (recorded_target, recorded): &(String, BTreeMap<String, u64>),
) -> (String, bool) {
  let mut out = format!("instructions to check each workload file ({}):\n", target);
  let comparable = recorded_target == target;
  let mut failed = false;
  for (kind, n) in counts {
    let note = match recorded.get(kind) {
      Some(was) if comparable => {
        let change = *n as f64 / *was as f64 - 1.0;
        if change > TOLERANCE {
          failed = true;
          format!("{:+.1}% over the record {}: slower", change * 100.0, was)
        } else if change < -TOLERANCE {
          format!(
            "{:+.1}% under the record {}: record it with --update",
            change * 100.0,
            was
          )
        } else {
          format!("{:+.1}% of the record", change * 100.0)
        }
      }
      _ => "not recorded for this target".to_string(),
    };
    out.push_str(&format!("  {:<10} {:>15}  {}\n", kind, n, note));
  }
  (out, failed)
}

/// The instructions mto runs to check `file`, from Cachegrind's summary.
fn count(mto: &Path, file: &Path, scratch: &Path) -> Result<u64, String> {
  let home = scratch.join("home");
  let _ = fs::create_dir_all(&home);
  let done = process::output(
    Command::new("valgrind")
      .args([
        "--tool=cachegrind",
        "--cache-sim=no",
        "--cachegrind-out-file=/dev/null",
      ])
      .arg(mto)
      .args(["--check", "--skip-children", "--quiet"])
      .arg(file)
      .current_dir(scratch)
      // No user configuration reaches the run: the count is of the defaults.
      .env("HOME", &home)
      .env("XDG_CONFIG_HOME", home.join(".config"))
      .env("XDG_STATE_HOME", home.join(".state"))
      .stdout(Stdio::null())
      .stderr(Stdio::piped()),
    process::LONG,
  )
  .map_err(|e| format!("cannot run valgrind: {}", e))?;
  refs(&String::from_utf8_lossy(&done.stderr))
    .ok_or_else(|| "Cachegrind gave no count (is valgrind installed with cachegrind?)".to_string())
}

/// The record's target and counts.
fn parse(text: &str) -> (String, BTreeMap<String, u64>) {
  let mut target = String::new();
  let mut counts = BTreeMap::new();
  for line in text.lines() {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
      ["target", t] => target = t.to_string(),
      ["count", kind, n] => {
        if let Ok(n) = n.parse() {
          counts.insert(kind.to_string(), n);
        }
      }
      _ => {}
    }
  }
  (target, counts)
}

fn record_text(target: &str, counts: &BTreeMap<String, u64>) -> String {
  let mut out = String::from(
    "# Instructions the release mto runs to check each file of the fixed workload, counted\n\
     # by Cachegrind (`cargo xtask speed instructions --update` writes it). Compared on the\n\
     # target named, where a count may not grow more than 3%.\n",
  );
  out.push_str(&format!("target {}\n", target));
  for (kind, n) in counts {
    out.push_str(&format!("count {} {}\n", kind, n));
  }
  out
}

/// The `I refs:` count of Cachegrind's summary, `==123== I refs:   1,234,567`.
fn refs(stderr: &str) -> Option<u64> {
  stderr.lines().find_map(|line| {
    let rest = line
      .split("I refs:")
      .nth(1)
      .or_else(|| line.split("I   refs:").nth(1))?;
    rest.trim().replace(',', "").parse().ok()
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_cachegrind_summaries_are_read() {
    assert_eq!(
      refs("==42== \n==42== I refs:        1,234,567\n"),
      Some(1234567)
    );
    assert_eq!(refs("==42== I   refs:      89\n"), Some(89));
    assert_eq!(refs("nothing"), None);
  }

  #[test]
  fn test_counts_hold_to_their_record() {
    let counts: BTreeMap<String, u64> = [("items".to_string(), 1040), ("line".to_string(), 960)]
      .into_iter()
      .collect();
    let text = record_text(
      "x86_64-unknown-linux-gnu",
      &[("items".to_string(), 1000), ("line".to_string(), 1000)]
        .into_iter()
        .collect(),
    );
    let recorded = parse(&text);
    let (report, failed) = compare("x86_64-unknown-linux-gnu", &counts, &recorded);
    assert!(failed, "{}", report);
    assert!(
      report.contains("+4.0% over the record 1000: slower"),
      "{}",
      report
    );
    assert!(
      report.contains("-4.0% under the record 1000: record it with --update"),
      "{}",
      report
    );
    let (other, failed) = compare("aarch64-unknown-linux-gnu", &counts, &recorded);
    assert!(
      !failed && other.contains("not recorded for this target"),
      "{}",
      other
    );
  }
}
