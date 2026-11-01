//! `cargo xtask sync [rustfmt|unicode|docs|changelog] [--check]`: every file written from
//! upstream, brought up to date in the order they depend on one another, or with `--check`
//! only compared. Each part is still its own command, which sync runs:
//!
//! - `rustfmt`: re-reads the latest stable and nightly releases and the local binaries
//!   (`capabilities refresh`), adds their new items to the capabilities record as missing
//!   (`capabilities sync`), regenerates the settings table from them (`settings`), then the
//!   docs written from that table. `--check` says whether a newer release than the
//!   recorded one is out (online), and whether the record and the table are current.
//! - `unicode`: the Unicode tables from the Unicode Character Database (`unicode`).
//! - `docs`: everything under docs/generated/ (`docs`).
//! - `changelog`: CHANGELOG.md's newest section from the commits (`changelog`).
//!
//! With no part, all of them, the docs last; `--check` with no part leaves the changelog
//! out, as its newest section is written at release and is behind the commits until then.
//! A newer nightly than recorded is reported, not failed: nightly moves every day.

use std::path::Path;
use std::process::ExitCode;

use crate::common::log::Logger;

/// The parts, in the order `sync` runs them all.
pub const PARTS: &[&str] = &["rustfmt", "unicode", "changelog", "docs"];

/// One step of a part: a command of this binary, or the check for newer releases.
#[derive(Debug, PartialEq)]
enum Step {
  Command(Vec<&'static str>),
  NewReleases,
}

impl Step {
  fn name(&self) -> String {
    match self {
      Step::Command(args) => args.join(" "),
      Step::NewReleases => "newer rustfmt releases than recorded".to_string(),
    }
  }
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "sync");
  log.info(&format!("cargo xtask {}", crate::cli::line()));
  let check = crate::cli::on(m, "check");
  let parts: Vec<&str> = match crate::cli::text(m, "part") {
    Some(part) => vec![PARTS.iter().copied().find(|p| *p == part).unwrap_or("")],
    // The changelog's newest section is written at release, so between releases it is
    // always behind the commits: checked when named, not with everything.
    None if check => PARTS
      .iter()
      .copied()
      .filter(|p| *p != "changelog")
      .collect(),
    None => PARTS.to_vec(),
  };
  let mut failed: Vec<String> = Vec::new();
  for part in &parts {
    for step in steps(part, check, parts.len() > 1) {
      let ok = match &step {
        Step::Command(args) => {
          println!("\n== {}", args.join(" "));
          let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
          crate::run_internal(root, &args[0], &args[1..]) == ExitCode::SUCCESS
        }
        Step::NewReleases => {
          println!("\n== newer rustfmt releases than recorded");
          match crate::capabilities::newer_releases(root, &log) {
            Ok(newer) if newer.is_empty() => {
              println!("the recorded stable and nightly releases are the latest");
              true
            }
            Ok(newer) => {
              for (_, line) in &newer {
                println!("{}", line);
              }
              println!("bring them in with `cargo xtask sync rustfmt`");
              // Nightly moves every day: a newer one is news, not a failure. A newer
              // stable is the reference moving on.
              let stable = newer.iter().any(|(channel, _)| *channel == "stable");
              if !stable {
                println!("(only nightly moved on: not a failure)");
              }
              !stable
            }
            Err(e) => {
              println!("cannot tell ({}); offline?", e);
              false
            }
          }
        }
      };
      if !ok {
        failed.push(step.name());
        // A write stops at the first failure, since what follows reads what it wrote; a
        // check goes on, to report everything out of date at once.
        if !check {
          break;
        }
      }
    }
    if !check && !failed.is_empty() {
      break;
    }
  }
  let verb = if check { "out of date" } else { "failed" };
  if failed.is_empty() {
    println!(
      "\nsync{}: {} {}",
      if check { " --check" } else { "" },
      parts.join(", "),
      if check {
        "current"
      } else {
        "brought up to date"
      }
    );
    ExitCode::SUCCESS
  } else {
    println!("\nsync: {} {}", verb, failed.join("; "));
    log.error(&format!("{}: {}", verb, failed.join("; ")));
    ExitCode::from(1)
  }
}

/// A part's steps, in order. `docs` is left out of `rustfmt` when every part runs, since
/// the docs part runs last.
fn steps(part: &str, check: bool, all: bool) -> Vec<Step> {
  let docs = |list: &mut Vec<Step>| {
    if !all {
      list.push(Step::Command(if check {
        vec!["docs", "--check"]
      } else {
        vec!["docs"]
      }));
    }
  };
  match (part, check) {
    ("rustfmt", false) => {
      let mut list = vec![
        Step::Command(vec!["capabilities", "refresh"]),
        Step::Command(vec!["capabilities", "sync"]),
        Step::Command(vec!["settings"]),
      ];
      docs(&mut list);
      list
    }
    ("rustfmt", true) => {
      let mut list = vec![
        Step::NewReleases,
        Step::Command(vec!["capabilities", "check"]),
        Step::Command(vec!["settings", "--check"]),
      ];
      docs(&mut list);
      list
    }
    ("unicode", false) => vec![Step::Command(vec!["unicode"])],
    ("unicode", true) => vec![Step::Command(vec!["unicode", "--check"])],
    ("docs", false) => vec![Step::Command(vec!["docs"])],
    ("docs", true) => vec![Step::Command(vec!["docs", "--check"])],
    ("changelog", false) => vec![Step::Command(vec!["changelog"])],
    ("changelog", true) => vec![Step::Command(vec!["changelog", "--check"])],
    _ => Vec::new(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Every command a step runs is one of xtask's, with arguments it takes.
  #[test]
  fn test_every_step_is_a_command_xtask_takes() {
    for part in PARTS {
      for check in [false, true] {
        for step in steps(part, check, false) {
          if let Step::Command(args) = step {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            assert!(crate::cli::takes(&args), "{:?}", args);
          }
        }
      }
    }
  }

  #[test]
  fn test_rustfmt_runs_its_chain_in_order_and_docs_once() {
    assert_eq!(
      steps("rustfmt", false, false),
      vec![
        Step::Command(vec!["capabilities", "refresh"]),
        Step::Command(vec!["capabilities", "sync"]),
        Step::Command(vec!["settings"]),
        Step::Command(vec!["docs"]),
      ]
    );
    // With every part, the docs come once, from the docs part at the end.
    assert!(!steps("rustfmt", false, true).contains(&Step::Command(vec!["docs"])));
    assert_eq!(PARTS.last(), Some(&"docs"));
    assert_eq!(steps("rustfmt", true, false)[0], Step::NewReleases);
    for part in PARTS {
      assert!(!steps(part, true, false).is_empty(), "{}", part);
    }
  }
}
