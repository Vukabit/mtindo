//! Development tasks for mtindo, run with `cargo xtask <command>`.
//!
//! This crate is never published and is not part of the `mto` binary. It does not depend
//! on mtindo, so the test runner builds when the library does not; commands that need
//! mtindo are forwarded to `xtask-tools` (`xtask/tools`). Every command that keeps a record
//! logs to `.dev/logs/NAME.log`, and every command but help and the record readers leaves a run
//! record under `.dev/runs/` (see [`runs`]); `--detach` runs it in the background.

mod bench;
mod capabilities;
mod cli;
mod common;
mod corpus;
mod coverage;
mod diff;
mod doctor;
mod fuzz;
mod fuzz_diff;
mod fuzz_soak;
mod gates;
mod incidents;
mod io;
mod memst;
mod metrics;
mod pairs;
mod papers;
mod perf;
mod race;
mod runs;
mod settings;
mod sha256;
mod status;
mod test;
mod toml_test;
mod unicode;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::ArgMatches;
use common::color::color_enabled;

const EXIT_CODES: &[(&str, &str)] = &[
  ("0", "success"),
  ("1", "a check failed"),
  ("2", "bad arguments"),
];

/// Each command's entry point, by name; [`cli::tree`] defines what it accepts.
fn dispatch(root: &Path, name: &str, m: &ArgMatches) -> ExitCode {
  match name {
    "test" => test::run(root, m),
    "capabilities" => capabilities::run(root, m),
    "corpus" => corpus::run(root, m),
    "diff" => diff::run(root, m),
    // docs/generated/config/ is written by xtask-tools from the settings table; the metrics papers
    // here, from the recorded data.
    "docs" if m.subcommand_name() == Some("metrics") => papers::run(root, m),
    "docs" => {
      let check = m.subcommand().is_some_and(|(_, sub)| cli::on(sub, "check"));
      let args = if check {
        vec!["--check".to_string()]
      } else {
        Vec::new()
      };
      forward(root, "docs-config", args)
    }
    "doctor" => doctor::run(root, m),
    "fuzz" => fuzz::run(root, m),
    "gates" => gates::run(root, m),
    "incidents" => incidents::run(root, m),
    "io" => io::run(root, m),
    "perf" => perf::run(root, m),
    "lexdiff" => {
      let mut args = vec![
        "--edition".to_string(),
        cli::text(m, "edition").unwrap_or_default(),
      ];
      args.extend(cli::texts(m, "paths"));
      forward(root, "lexdiff", args)
    }
    "memst" => memst::run(root, m),
    "metrics" => metrics::run(root, m),
    "pairs" => pairs::run(root, m),
    "parsecheck" => forward(root, "parsecheck", cli::texts(m, "paths")),
    "race" => race::run(root, m),
    "runs" => runs::run(root, m),
    "settings" => settings::run(root, m),
    "status" => status::run(root, m),
    "toml-test" => toml_test::run(root, m),
    "tui" => {
      let mut args = Vec::new();
      if cli::on(m, "check") {
        args.push("--check".to_string());
      }
      if let Some(screen) = cli::text(m, "render") {
        args.extend(["--render".to_string(), screen]);
        args.extend([
          "--size".to_string(),
          cli::text(m, "size").unwrap_or_default(),
        ]);
      }
      common::process::forward(
        root,
        &["run", "--quiet", "--release", "--package", "hub", "--"],
        &args,
      )
    }
    "unicode" => unicode::run(root, m),
    other => unreachable!("{} is in the tree but has no entry point", other),
  }
}

/// Runs a command of `xtask-tools`, the binary that links mtindo, with what was given.
fn forward(root: &Path, command: &str, args: Vec<String>) -> ExitCode {
  let mut all = vec![command.to_string()];
  all.extend(args);
  common::process::forward_to_tools(root, &all)
}

fn main() -> ExitCode {
  let args: Vec<String> = env::args().skip(1).collect();
  let root = project_root();
  match cli::parse(&args) {
    cli::Parsed::Help(path) => match render_help(&root, &path, color_enabled(false)) {
      Some(text) => {
        print!("{}", text);
        ExitCode::SUCCESS
      }
      None => {
        eprintln!("cargo xtask: no command `{}`", path.join(" "));
        ExitCode::from(2)
      }
    },
    cli::Parsed::Json => {
      print!("{}", cli::json());
      ExitCode::SUCCESS
    }
    cli::Parsed::Error(message) => {
      eprint!("{}", message);
      eprintln!("Run 'cargo xtask help' for usage.");
      ExitCode::from(2)
    }
    cli::Parsed::Run(name, m) => {
      if runs::supervised(&args) {
        runs::start(&root, &args)
      } else {
        dispatch(&root, &name, &m)
      }
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

/// The help for `path`: `cargo xtask` itself with its gates and exit codes, the test
/// runner with its groups and tasks, or any command as the tree describes it. Fixed text;
/// only the colors depend on `color`.
fn render_help(root: &Path, path: &[String], color: bool) -> Option<String> {
  if path == ["test"] {
    return Some(test::render_help(root, color));
  }
  let mut out = cli::help(path, color)?;
  if path.is_empty() {
    let p = common::color::HelpPalette::new(color, "cargo xtask");
    out.push_str(&format!("\n{}\n", p.heading("gates:")));
    for (name, help, _) in gates::GATES {
      let label = format!("  {}", p.value(name));
      out.push_str(&p.row(&label, 2 + name.len(), 30, help));
    }
    out.push_str(&format!("\n{}\n", p.heading("exit codes:")));
    for (code, meaning) in EXIT_CODES {
      out.push_str(&format!("  {}  {}\n", p.value(code), meaning));
    }
  }
  Some(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn root() -> PathBuf {
    project_root()
  }

  #[test]
  fn test_every_command_has_an_entry_point() {
    // Each command's help renders, and dispatch knows each name (an unknown one panics).
    for command in cli::tree().get_subcommands() {
      if command.get_name() == "help" {
        continue;
      }
      let path = vec![command.get_name().to_string()];
      assert!(render_help(&root(), &path, false).is_some());
      let source = include_str!("main.rs");
      assert!(
        source.contains(&format!("\"{}\" =>", command.get_name())),
        "{} has no entry point",
        command.get_name()
      );
    }
  }

  #[test]
  fn test_help_lists_every_command_and_gate() {
    let help = render_help(&root(), &[], false).unwrap();
    for command in cli::tree().get_subcommands() {
      assert!(
        help.contains(&format!("\n  {}", command.get_name())),
        "{} missing from help",
        command.get_name()
      );
    }
    for (name, _, _) in gates::GATES {
      assert!(
        help.contains(&format!("\n  {}", name)),
        "gate {} missing from help",
        name
      );
    }
  }
}
