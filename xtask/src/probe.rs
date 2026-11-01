//! `cargo xtask probe`: one of the standard formatter's test pairs against every answer
//! key (`crate::sources`), or with `--scan` every pair, for where the keys disagree. Finds
//! the suites at stable and nightly, then runs the probe in `xtask-tools`, which links
//! mtindo.

use std::path::Path;
use std::process::ExitCode;

use crate::common::log::Logger;
use crate::sources;

/// Where the disagreements between answer keys are recorded.
pub const RECORD: &str = "records/policy/disagreements.txt";

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "probe");
  log.info(&format!("cargo xtask {}", crate::cli::line()));
  let stable = match sources::tests_dir(root, "stable", &log) {
    Ok(dir) => dir,
    Err(e) => {
      println!("{}", e);
      return ExitCode::from(1);
    }
  };
  let mut args: Vec<String> = vec![
    "--tests-stable".into(),
    stable.display().to_string(),
    "--surface".into(),
    root
      .join("records/upstream/rustfmt-surface.txt")
      .display()
      .to_string(),
    "--disagreements".into(),
    root.join(RECORD).display().to_string(),
  ];
  match sources::tests_dir(root, "nightly", &log) {
    Ok(dir) => args.extend(["--tests-nightly".into(), dir.display().to_string()]),
    Err(e) => println!("{} (tests@nightly is left out)", e),
  }
  if let Some(toolchain) = crate::cli::text(m, "toolchain") {
    args.extend(["--toolchain".into(), toolchain]);
  }
  if let Some(key) = crate::cli::text(m, "show") {
    args.extend(["--show".into(), key]);
  }
  for flag in ["scan", "record"] {
    if crate::cli::on(m, flag) {
      args.push(format!("--{}", flag));
    }
  }
  if let Some(case) = crate::cli::text(m, "case") {
    args.push(case);
  }
  let mut all = vec!["probe".to_string()];
  all.extend(args);
  crate::common::process::forward_to_tools(root, &all)
}
