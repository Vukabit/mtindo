//! The tools `flow.toml` pins: what is installed against what is pinned, `bootstrap`
//! installing or updating them, and the check each stage or command makes for the ones
//! it needs.

use std::path::Path;
use std::process::{Command, Stdio};

use super::config::{Flow, Tool, ToolKind};
use crate::common::process;

/// Where a tool stands on this machine.
#[derive(Debug, PartialEq)]
pub enum State {
  Ready,
  /// Installed at another version than pinned (or older than a system tool's oldest).
  Other(String),
  /// A toolchain without some of its components.
  Partial(Vec<String>),
  Missing,
}

/// `cargo xtask bootstrap [--for NAME] [--go]`: each tool's state and what would bring
/// it to its pin; with `go`, done, and the hooks turned on (not on CI).
pub fn bootstrap(root: &Path, flow: &Flow, only: Option<&str>, go: bool) -> Result<(), String> {
  let host = host();
  let mut failed = Vec::new();
  for tool in &flow.tools {
    if only.is_some_and(|o| !tool.needed_by.iter().any(|n| n == o)) {
      continue;
    }
    let state = state(tool, &host);
    let beside = tool.kind != ToolKind::Path || root.join(&tool.source).is_dir();
    let fix = if beside { fix(tool, &state) } else { None };
    let shown = match &state {
      State::Ready => "ok".to_string(),
      State::Other(found) => format!("{} installed", found),
      State::Partial(missing) => format!("missing {}", missing.join(", ")),
      State::Missing if tool.optional => "not installed (optional)".to_string(),
      State::Missing => "not installed".to_string(),
    };
    let pinned = if tool.version.is_empty() {
      tool.source.as_str()
    } else {
      &tool.version
    };
    println!("{:<11} {:<12} {}", tool.name, pinned, shown);
    let Some((program, args)) = fix else {
      if state != State::Ready {
        let how = match tool.kind {
          ToolKind::System => format!("install it from {}", tool.source),
          _ => format!("{} is not beside this repository: skipped", tool.source),
        };
        println!("{:<11} {}", "", how);
        if tool.kind == ToolKind::System && !tool.optional {
          failed.push(tool.name.clone());
        }
      }
      continue;
    };
    println!("{:<11} {} {}", "", program, args.join(" "));
    if go {
      let done = process::output(
        Command::new(&program).args(&args).current_dir(root),
        process::LONG,
      )
      .map_err(|e| format!("{} could not be run: {}", program, e))?;
      if done.code != 0 {
        failed.push(tool.name.clone());
      }
    }
  }
  if go && std::env::var_os("CI").is_none() && only.is_none() {
    super::setup(root)?;
  }
  if !go {
    println!("plan only: add --go to install");
  }
  if failed.is_empty() {
    Ok(())
  } else {
    Err(format!("not ready: {}", failed.join(", ")))
  }
}

/// The commands `needed_by` may name besides the stages: every command and subcommand of
/// `cargo xtask`, by its name, from the command tree itself.
pub fn commands() -> Vec<String> {
  let mut names = Vec::new();
  let mut stack = vec![crate::cli::tree()];
  while let Some(command) = stack.pop() {
    for sub in command.get_subcommands() {
      names.push(sub.get_name().to_string());
      stack.push(sub.clone());
    }
  }
  names.sort();
  names.dedup();
  names
}

/// The tools `user` (a stage or command) needs, each ready, or what to run.
pub fn require(flow: &Flow, user: &str) -> Result<(), String> {
  let host = host();
  let problems: Vec<String> = flow
    .tools
    .iter()
    .filter(|t| t.needed_by.iter().any(|n| n == user))
    .filter(|t| !t.optional)
    .filter_map(|t| match state(t, &host) {
      State::Ready => None,
      State::Other(found) => Some(format!(
        "{} {} is installed, flow.toml pins {}",
        t.name, found, t.version
      )),
      State::Partial(missing) => Some(format!("{} lacks {}", t.name, missing.join(", "))),
      State::Missing => Some(format!("{} is not installed", t.name)),
    })
    .collect();
  if problems.is_empty() {
    return Ok(());
  }
  Err(format!(
    "{} needs: {}; run `cargo xtask bootstrap`",
    user,
    problems.join("; ")
  ))
}

/// Whether version `found` is at least `oldest`.
fn at_least(found: &str, oldest: &str) -> bool {
  let numbers = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
  numbers(found) >= numbers(oldest)
}

/// The command that brings a tool to its pin, if one can.
fn fix(tool: &Tool, state: &State) -> Option<(String, Vec<String>)> {
  let owned = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
  match (tool.kind, state) {
    (_, State::Ready) => None,
    (ToolKind::Toolchain, State::Partial(missing)) => {
      let mut args = owned(&["component", "add", "--toolchain", &tool.version]);
      args.extend(missing.iter().cloned());
      Some(("rustup".to_string(), args))
    }
    (ToolKind::Toolchain, _) => {
      let mut args = owned(&[
        "toolchain",
        "install",
        &tool.version,
        "--profile",
        "minimal",
      ]);
      if !tool.components.is_empty() {
        args.push("--component".to_string());
        args.push(tool.components.join(","));
      }
      Some(("rustup".to_string(), args))
    }
    (ToolKind::Cargo, _) => Some((
      "cargo".to_string(),
      owned(&[
        "install",
        "--locked",
        "--version",
        &tool.version,
        &tool.name,
      ]),
    )),
    (ToolKind::Path, _) => Some((
      "cargo".to_string(),
      owned(&["install", "--locked", "--path", &tool.source]),
    )),
    (ToolKind::System, _) => None,
  }
}

/// This machine's target triple, as rustup names toolchains for it.
fn host() -> String {
  output("rustup", &["show"])
    .and_then(|t| {
      t.lines().find_map(|l| {
        l.strip_prefix("Default host:")
          .map(|h| h.trim().to_string())
      })
    })
    .unwrap_or_default()
}

/// A command's output, when it runs and succeeds.
fn output(program: &str, args: &[&str]) -> Option<String> {
  let done = process::output(
    Command::new(program)
      .args(args)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  )
  .ok()?;
  (done.code == 0).then(|| String::from_utf8_lossy(&done.stdout).into_owned())
}

fn state(tool: &Tool, host: &str) -> State {
  match tool.kind {
    ToolKind::Toolchain => {
      let name = format!("{}-{}", tool.version, host);
      let listed = output("rustup", &["toolchain", "list"]).unwrap_or_default();
      if !listed
        .lines()
        .any(|l| l.split_whitespace().next() == Some(name.as_str()))
      {
        return State::Missing;
      }
      let installed = output(
        "rustup",
        &[
          "component",
          "list",
          "--toolchain",
          &tool.version,
          "--installed",
        ],
      )
      .unwrap_or_default();
      let missing: Vec<String> = tool
        .components
        .iter()
        .filter(|c| {
          // `llvm-tools-preview` installs as `llvm-tools`.
          let base = c.trim_end_matches("-preview");
          !installed
            .lines()
            .any(|l| l.starts_with(&format!("{}-", c)) || l.starts_with(&format!("{}-", base)))
        })
        .cloned()
        .collect();
      if missing.is_empty() {
        State::Ready
      } else {
        State::Partial(missing)
      }
    }
    ToolKind::Cargo | ToolKind::System | ToolKind::Path => {
      let Some(found) = output(&tool.bin, &["--version"]).and_then(|t| version_in(&t)) else {
        return State::Missing;
      };
      let ready = match tool.kind {
        ToolKind::Cargo => found == tool.version,
        ToolKind::System => at_least(&found, &tool.version),
        // A path crate is ready once installed: its version is its own.
        _ => true,
      };
      if ready {
        State::Ready
      } else {
        State::Other(found)
      }
    }
  }
}

/// The first `N.N[.N]` in a `--version` line.
fn version_in(text: &str) -> Option<String> {
  text.lines().next()?.split_whitespace().find_map(|word| {
    let word = word.trim_start_matches('v');
    let parts: Vec<&str> = word.split('.').collect();
    (parts.len() >= 2
      && parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())))
    .then(|| word.to_string())
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_versions_are_read_and_compared() {
    assert_eq!(version_in("git-cliff 2.14.2\n").as_deref(), Some("2.14.2"));
    assert_eq!(
      version_in("gh version 2.93.0 (2026-05-27)").as_deref(),
      Some("2.93.0")
    );
    assert_eq!(version_in("cargo-fuzz 0.13.2").as_deref(), Some("0.13.2"));
    assert_eq!(version_in("nothing here"), None);
    assert!(at_least("2.93.0", "2.40.0"));
    assert!(!at_least("2.9.0", "2.40.0"));
  }
}
