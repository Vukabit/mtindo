//! `cargo xtask speed <MEASURE> [ARGS...]`: every speed measurement behind one command,
//! each still run by the command that has always made it (and timed through
//! `xtask_core::timing`), with ARGS passed on:
//!
//! - `throughput`: the corpus against the standard formatter, stable and nightly (`corpus
//!   bench`);
//! - `latency`, `scaling`, `growth`: one small file, thread counts, doubling sizes (`perf`);
//! - `instructions`: instructions counted on a fixed workload, by Cachegrind (`perf`);
//! - `io`: what writing files costs, by way of syncing (`io bench`);
//! - `features`: the standard formatter and mto feature by feature, live (`race`);
//! - `doctor`: what the build guard costs (`doctor bench`).
//!
//! Memory is memst's (`cargo xtask memst`).

use std::path::Path;
use std::process::ExitCode;

/// Each measure and the command that makes it.
pub const MEASURES: &[(&str, &[&str], &str)] = &[
  (
    "throughput",
    &["corpus", "bench"],
    "the corpus against the standard formatter, stable and nightly",
  ),
  (
    "latency",
    &["perf", "latency"],
    "one small file, many times: middle and p99",
  ),
  (
    "scaling",
    &["perf", "scaling"],
    "corpus files laid out by 1, 2, 4 and 8 threads",
  ),
  (
    "growth",
    &["perf", "growth"],
    "generated files of doubling size",
  ),
  (
    "timings",
    &["perf"],
    "latency, scaling and growth, one after another",
  ),
  (
    "instructions",
    &["perf", "instructions"],
    "instructions counted on a fixed workload (Linux, Cachegrind)",
  ),
  (
    "io",
    &["io", "bench"],
    "what writing files costs, by way of syncing",
  ),
  (
    "features",
    &["race"],
    "the standard formatter and mto feature by feature, drawn live",
  ),
  ("doctor", &["doctor", "bench"], "what the build guard costs"),
];

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let measure = crate::cli::text(m, "measure").unwrap_or_default();
  let Some((_, command, _)) = MEASURES.iter().find(|(name, _, _)| *name == measure) else {
    eprintln!("no measure `{}`", measure);
    return ExitCode::from(2);
  };
  let mut args: Vec<String> = command.iter().map(|s| s.to_string()).collect();
  args.extend(crate::cli::texts(m, "args"));
  crate::run_internal(root, &args[0], &args[1..])
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Every measure names a command xtask takes.
  #[test]
  fn test_every_measure_is_a_command() {
    for (name, command, _) in MEASURES {
      let args: Vec<String> = command.iter().map(|s| s.to_string()).collect();
      assert!(crate::cli::takes(&args), "{}: {:?}", name, command);
    }
  }
}
