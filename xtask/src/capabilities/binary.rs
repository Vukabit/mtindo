//! Reads rustfmt's surface from installed binaries; works offline.
//!
//! - `rustfmt --print-config default` lists every option it does not hide.
//! - `rustfmt --help=config` lists only the stable options, with their values in brackets
//!   (`[Auto|Windows|Unix|Native]`), on stable and nightly toolchains alike.
//! - `rustfmt --help` and `cargo fmt --help` list the flags. A nightly binary also shows its
//!   unstable flags without marking them, so only a stable binary reports flags as stable.
//!
//! Every command runs with `RUSTUP_AUTO_INSTALL=0`, so reading never installs a toolchain.

use std::process::Command;

use super::surface::{Found, Kind, Reading};
use crate::common::log::Logger;

/// The local readings: the default `rustfmt`, plus `+nightly` when rustup has a nightly
/// toolchain that differs from the default.
pub fn local_readings(log: &Logger) -> Vec<Reading> {
  let mut readings = Vec::new();
  let default = match read("local", None) {
    Ok(reading) => reading,
    Err(e) => {
      log.warning(&format!("no local rustfmt: {}", e));
      println!("local: no rustfmt found ({})", e);
      return readings;
    }
  };
  let default_version = default.description.clone();
  readings.push(default);
  let has_nightly = run("rustup", None, &["toolchain", "list"])
    .map(|list| list.lines().any(|line| line.starts_with("nightly")))
    .unwrap_or(false);
  if has_nightly && !default_version.contains("-nightly") {
    match read("local-nightly", Some("nightly")) {
      Ok(reading) => readings.push(reading),
      Err(e) => {
        log.warning(&format!("rustfmt +nightly unreadable: {}", e));
        println!("local-nightly: skipped ({})", e);
      }
    }
  }
  readings
}

/// Long flag names from a `--help` screen: `-v, --verbose  ...` or `--emit [files|stdout]`.
pub fn parse_flags(help: &str) -> Vec<String> {
  let mut flags = Vec::new();
  for line in help.lines() {
    let mut words = line.split_whitespace();
    let mut word = words.next().unwrap_or("");
    if word.len() == 3 && word.starts_with('-') && word.ends_with(',') {
      word = words.next().unwrap_or("");
    }
    let Some(name) = word.strip_prefix("--") else {
      continue;
    };
    let name: String = name
      .chars()
      .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
      .collect();
    if !name.is_empty() && !flags.contains(&format!("--{}", name)) {
      flags.push(format!("--{}", name));
    }
  }
  flags
}

/// (option, values) for each option in `--help=config`; values only for bracketed lists.
pub fn parse_help_config(help: &str) -> Vec<(String, Vec<String>)> {
  let mut options = Vec::new();
  for line in help.lines() {
    if !line.contains(" Default: ") {
      continue;
    }
    let mut words = line.split_whitespace();
    let Some(name) = words.next() else {
      continue;
    };
    let mut values = Vec::new();
    if let Some(rest) = words.next()
      && let Some(list) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']'))
    {
      values = list.split('|').map(str::to_string).collect();
    }
    options.push((name.to_string(), values));
  }
  options
}

/// Option names from `--print-config default` (`name = value` lines).
pub fn parse_print_config(config: &str) -> Vec<String> {
  config
    .lines()
    .filter_map(|line| line.split_once(" = "))
    .map(|(name, _)| name.trim().to_string())
    .filter(|name| {
      !name.is_empty()
        && name
          .chars()
          .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    })
    .collect()
}

/// The short commit hash in a version such as `rustfmt 1.9.0-stable (88d9e12ae1 2026-08-18)`.
pub fn short_commit(version: &str) -> Option<String> {
  let inside = version.split_once('(')?.1;
  let hash = inside.split_whitespace().next()?;
  (hash.len() >= 7 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_string())
}

/// The rust-lang/rust commit a local binary was built from, if its toolchain's `rustc -vV`
/// names one that matches the short hash in the binary's version.
pub fn toolchain_commit(reading: &Reading) -> Option<String> {
  let toolchain = if reading.label == "local-nightly" {
    Some("nightly")
  } else {
    None
  };
  let short = short_commit(&reading.description)?;
  let verbose = run("rustc", toolchain, &["-vV"]).ok()?;
  let full = verbose
    .lines()
    .find_map(|line| line.strip_prefix("commit-hash: "))?
    .trim()
    .to_string();
  full.starts_with(&short).then_some(full)
}

fn read(label: &str, toolchain: Option<&str>) -> Result<Reading, String> {
  let version = run("rustfmt", toolchain, &["--version"])?;
  let version = version.trim().to_string();
  let stable_binary = !version.contains("-nightly") && !version.contains("-dev");
  let mut items = Vec::new();
  let stable_options = parse_help_config(&run("rustfmt", toolchain, &["--help=config"])?);
  for name in parse_print_config(&run("rustfmt", toolchain, &["--print-config", "default"])?) {
    let stable = stable_options.iter().any(|(n, _)| *n == name);
    items.push(Found::new(Kind::Option, name, stable));
  }
  for (name, values) in stable_options.iter() {
    for value in values {
      items.push(Found::new(
        Kind::Value,
        format!("{}={}", name, value),
        false,
      ));
    }
  }
  for flag in parse_flags(&run("rustfmt", toolchain, &["--help"])?) {
    items.push(Found::new(
      Kind::Flag,
      format!("rustfmt:{}", flag),
      stable_binary,
    ));
  }
  for flag in parse_flags(&run("cargo", toolchain, &["fmt", "--help"])?) {
    items.push(Found::new(
      Kind::Flag,
      format!("cargo-fmt:{}", flag),
      stable_binary,
    ));
  }
  Ok(Reading {
    label: label.to_string(),
    description: format!("{}, installed binary", version),
    items,
  })
}

fn run(program: &str, toolchain: Option<&str>, args: &[&str]) -> Result<String, String> {
  let mut command = Command::new(program);
  command.env("RUSTUP_AUTO_INSTALL", "0");
  if let Some(toolchain) = toolchain {
    command.arg(format!("+{}", toolchain));
  }
  command.args(args);
  let output = command
    .output()
    .map_err(|e| format!("cannot run {}: {}", program, e))?;
  if !output.status.success() {
    return Err(format!(
      "`{} {}` failed: {}",
      program,
      args.join(" "),
      String::from_utf8_lossy(&output.stderr).trim()
    ));
  }
  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_parse_flags_from_both_help_styles() {
    let rustfmt = "Options:\n        --check         Run in 'check' mode. Exits with 0 if input is\n                        formatted correctly.\n        --emit [files|stdout|coverage|checkstyle|json]\n    -v, --verbose       Print verbose output\n    -h, --help [=TOPIC] Show this message\n        --config [key1=val1,key2=val2...]\n";
    assert_eq!(
      parse_flags(rustfmt),
      vec!["--check", "--emit", "--verbose", "--help", "--config"]
    );
    let cargo = "Options:\n  -q, --quiet\n      --version\n  -p, --package <package>...\n      --manifest-path <manifest-path>\n  -h, --help\n";
    assert_eq!(
      parse_flags(cargo),
      vec![
        "--quiet",
        "--version",
        "--package",
        "--manifest-path",
        "--help"
      ]
    );
  }

  #[test]
  fn test_parse_help_config_values() {
    let help = "Configuration Options:\n                          max_width <unsigned integer> Default: 100\n                                    Maximum width of each line\n\n                      newline_style [Auto|Windows|Unix|Native] Default: Auto\n                                    Unix or Windows line endings\n";
    assert_eq!(
      parse_help_config(help),
      vec![
        ("max_width".to_string(), vec![]),
        (
          "newline_style".to_string(),
          vec![
            "Auto".to_string(),
            "Windows".to_string(),
            "Unix".to_string(),
            "Native".to_string()
          ]
        ),
      ]
    );
  }

  #[test]
  fn test_parse_print_config_names() {
    let config = "max_width = 100\nnewline_style = \"Auto\"\nignore = []\n\n[weird]\n";
    assert_eq!(
      parse_print_config(config),
      vec!["max_width", "newline_style", "ignore"]
    );
  }

  #[test]
  fn test_short_commit() {
    assert_eq!(
      short_commit("rustfmt 1.9.0-stable (88d9e12ae1 2026-08-18)").as_deref(),
      Some("88d9e12ae1")
    );
    assert_eq!(short_commit("rustfmt 1.9.0-stable"), None);
  }
}
