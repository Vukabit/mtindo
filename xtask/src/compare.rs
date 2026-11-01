//! `cargo xtask compare [SOURCE...] [-- ARGS...]`: mto held against an answer key, by
//! source, through the one engine in `xtask/tools/src/compare/`. Each family of sources
//! still has its command, which compare runs with ARGS:
//!
//! - the recorded sets (`tests`, `tests-with-settings`, `corpus`, `corpus-unformatted`;
//!   all four by default): `diff`, which keeps `records/results/compatibility.txt`; some of
//!   them only are reported, never recorded;
//! - `pairs`: scrambled pairs, also built with `cargo check` (`pairs`);
//! - `lexer`: files held against rustc's lexer (`lexdiff`);
//! - `parse`: files parsed alone (`parsecheck`).
//!
//! A run takes the sources of one family.

use std::path::Path;
use std::process::ExitCode;

/// Every source: its name, its family's command, and what it is.
pub const SOURCES: &[(&str, &str, &str)] = &[
  (
    "tests",
    "diff",
    "the standard formatter's test pairs, default settings",
  ),
  (
    "tests-with-settings",
    "diff",
    "its test pairs that set options",
  ),
  ("corpus", "diff", "the corpus as fetched"),
  (
    "corpus-unformatted",
    "diff",
    "the corpus with its whitespace scrambled",
  ),
  (
    "pairs",
    "pairs",
    "corpus files scrambled by a seed, items and whitespace",
  ),
  (
    "lexer",
    "lexdiff",
    "files held against rustc's lexer, token for token",
  ),
  (
    "parse",
    "parsecheck",
    "files parsed alone, for the grammar's gaps",
  ),
];

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let names = crate::cli::texts(m, "sources");
  let rest = crate::cli::texts(m, "args");
  let args = match command_for(&names, &rest) {
    Ok(args) => args,
    Err(e) => {
      eprintln!("{}", e);
      return ExitCode::from(2);
    }
  };
  crate::run_internal(root, &args[0], &args[1..])
}

/// The command a run of `names` is, with `rest` passed on.
fn command_for(names: &[String], rest: &[String]) -> Result<Vec<String>, String> {
  let family = |name: &str| {
    SOURCES
      .iter()
      .find(|(n, _, _)| *n == name)
      .map(|(_, f, _)| *f)
  };
  let mut families: Vec<&str> = names.iter().filter_map(|n| family(n)).collect();
  families.dedup();
  let command = match families.as_slice() {
    [] => "diff",
    [one] => one,
    _ => {
      return Err(format!(
        "{} come from different commands; compare one family at a time",
        names.join(", ")
      ));
    }
  };
  let mut args = vec![command.to_string()];
  if command == "diff" && !names.is_empty() {
    args.extend(["--sources".to_string(), names.join(",")]);
  }
  args.extend(rest.iter().cloned());
  Ok(args)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_each_family_runs_its_command() {
    assert_eq!(command_for(&[], &[]).unwrap(), words(&["diff"]));
    assert_eq!(
      command_for(&words(&["tests", "corpus"]), &words(&["--markdown"])).unwrap(),
      words(&["diff", "--sources", "tests,corpus", "--markdown"])
    );
    assert_eq!(
      command_for(&words(&["pairs"]), &words(&["--seed", "7"])).unwrap(),
      words(&["pairs", "--seed", "7"])
    );
    assert_eq!(
      command_for(&words(&["lexer"]), &[]).unwrap(),
      words(&["lexdiff"])
    );
    assert!(command_for(&words(&["tests", "pairs"]), &[]).is_err());
  }

  /// Every command a source names is one xtask takes, and every recorded set is diff's.
  #[test]
  fn test_every_source_names_a_command() {
    for (name, command, _) in SOURCES {
      let args = vec![command.to_string()];
      assert!(crate::cli::takes(&args), "{}: {}", name, command);
      assert_eq!(
        *command == "diff",
        crate::diff::SETS.contains(name),
        "{}",
        name
      );
    }
  }

  fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| w.to_string()).collect()
  }
}
