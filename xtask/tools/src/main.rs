//! The xtask commands that need the mtindo library. `cargo xtask` forwards to this binary,
//! so the runner itself never depends on mtindo and still builds when the library does not.

mod common;
mod compare;
mod config_docs;
mod corpus;
mod diff_fuzz;
mod doctor_bench;
mod perf;
mod probe;
mod scramble;
mod toml_test;
mod unformat;
mod upstream;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use xtask_core::{log, process};

// The allocator the mto binaries install, so that what is measured in process here (the
// perf benches above all) runs as mto does.
#[global_allocator]
static GLOBAL: mtindo::cache_alloc::Cached = mtindo::cache_alloc::Cached;

fn main() -> ExitCode {
  let args: Vec<String> = env::args().skip(1).collect();
  let root = project_root();
  match args.first().map(String::as_str) {
    Some("lexdiff") => compare::lexer(&root, &args[1..]),
    Some("parsecheck") => compare::parse(&root, &args[1..]),
    Some("corpus-check") => corpus::run(&root, &args[1..]),
    Some("docs") => config_docs::run(&root, &args[1..]),
    Some("corpus-speed") => corpus::speed(&args[1..]),
    Some("toml-test") => toml_test::run(&args[1..]),
    Some("diff") => compare::run(&args[1..]),
    Some("diff-fuzz") => diff_fuzz::run(&args[1..]),
    Some("diff-show") => compare::show(&args[1..]),
    Some("diff-why") => compare::why(&args[1..]),
    Some("doctor-bench") => doctor_bench::run(&args[1..]),
    Some("pairs") => compare::pairs(&args[1..]),
    Some("probe") => probe::run(&args[1..]),
    Some("upstream") => upstream::run(&args[1..]),
    Some("perf-scaling") => perf::scaling(&args[1..]),
    Some("perf-growth") => perf::growth(&args[1..]),
    Some("perf-depth") => perf::depth(&args[1..]),
    Some("perf-stages") => perf::stages(&args[1..]),
    Some("perf-walk") => perf::walk(&args[1..]),
    Some("perf-digest") => perf::digest(&args[1..]),
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
