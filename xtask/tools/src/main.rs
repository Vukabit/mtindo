//! The xtask commands that need the mtindo library. `cargo xtask` forwards to this binary,
//! so the runner itself never depends on mtindo and still builds when the library does not.

mod cases;
mod common;
mod corpus;
mod diff;
mod lexdiff;
// The same logger as xtask, compiled from its file: logs/NAME.log at the repository root.
#[allow(dead_code)]
#[path = "../../src/common/log.rs"]
mod log;
mod parsecheck;
// The same child-process helpers as xtask, with their time limits.
#[allow(dead_code)]
#[path = "../../src/common/process.rs"]
mod process;
mod toml_test;
mod unformat;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
  let args: Vec<String> = env::args().skip(1).collect();
  let root = project_root();
  match args.first().map(String::as_str) {
    Some("lexdiff") => lexdiff::run(&root, &args[1..]),
    Some("parsecheck") => parsecheck::run(&root, &args[1..]),
    Some("corpus-check") => corpus::run(&root, &args[1..]),
    Some("corpus-speed") => corpus::speed(&args[1..]),
    Some("toml-test") => toml_test::run(&args[1..]),
    Some("diff") => diff::run(&args[1..]),
    // Prints a file with its whitespace scrambled, as `diff` scrambles it.
    Some("unformat") if args.len() == 2 => {
      let path = Path::new(&args[1]);
      let text = std::fs::read_to_string(path).unwrap_or_default();
      match unformat::unformat(&text, common::edition_for(path, &text)) {
        Some(messy) => {
          print!("{}", messy);
          ExitCode::SUCCESS
        }
        None => ExitCode::from(1),
      }
    }
    Some("cases") if args.len() == 2 => {
      let summary = cases::run(Path::new(&args[1]));
      println!(
        "cases {} {} {} {}",
        summary.total, summary.configured, summary.passed, summary.unreadable
      );
      ExitCode::SUCCESS
    }
    _ => {
      eprintln!("xtask-tools: run through `cargo xtask`; see `cargo xtask help`.");
      ExitCode::from(2)
    }
  }
}

fn project_root() -> PathBuf {
  let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
  manifest_dir
    .ancestors()
    .nth(2)
    .map(Path::to_path_buf)
    .unwrap_or(manifest_dir)
}
