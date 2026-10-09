//! Development tasks for mtindo, run with `cargo xtask <command>`.
//!
//! This crate is never published and is not part of the `mto` binary.

mod gates;
mod lexdiff;
mod sha256;
mod unicode;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <command> [args]

Commands:
  gates [all|deps|layering]   Check repository rules (default: all)
      deps       mtindo has no runtime dependencies; xtask uses only allowed ones
      layering   modules under src/ only use layers below them
  lexdiff [--edition YEAR] [PATH...]
                              Compare mto's lexer with rustc's own on .rs files
                              (default: the repository's own sources; edition 2021)
  unicode [--version X.Y.Z] [--check]
                              Regenerate src/text/unicode/*_tables.rs from the Unicode
                              Character Database (default version: the toolchain's
                              char::UNICODE_VERSION); --check only compares
  help                        Show this message

Exit codes: 0 success, 1 a check failed, 2 bad arguments.
";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let root = project_root();
    match args.first().map(String::as_str) {
        Some("gates") => gates::run(&root, &args[1..]),
        Some("lexdiff") => lexdiff::run(&root, &args[1..]),
        Some("unicode") => unicode::run(&root, &args[1..]),
        Some("help") | Some("-h") | Some("--help") => {
            print!("{}", USAGE);
            ExitCode::SUCCESS
        }
        None => {
            eprint!("{}", USAGE);
            ExitCode::from(2)
        }
        Some(other) => {
            eprintln!("Unknown xtask command `{}`.\n", other);
            eprint!("{}", USAGE);
            ExitCode::from(2)
        }
    }
}

fn project_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest_dir)
}
