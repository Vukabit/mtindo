//! What `cargo xtask test` runs: the groups, the tasks in run order, and the tools each
//! task needs. `-h` is generated from these tables, and the `help` gate checks it.

use std::fs;
use std::path::Path;

use crate::{capabilities, gates};

pub const DEFAULT_FUZZ_TIME: u32 = 10;

pub const DEFAULT_GROUPS: &[&str] = &["gates", "tests"];

pub const ENV_BLESS: &str = "MTINDO_BLESS";

pub const ENV_HEAVY: &str = "MTINDO_HEAVY_TESTS";

/// Environment variables the runner sets for tasks. Shown by -h.
pub const ENV_VARS: &[(&str, &str)] = &[
  (
    ENV_BLESS,
    "set to 1 for the snapshots task with --bless: rewrite expected outputs",
  ),
  (
    ENV_HEAVY,
    "set to 1 for domain_13_rw: run the real-world crate check instead of skipping it",
  ),
];

/// Every group, with what it covers. The help gate checks that each one is in `-h`.
pub const GROUPS: &[(&str, &str)] = &[
  (
    "gates",
    "Repository rules: clippy, dependencies, layering, docs, capabilities, rustfmt, mto on itself, help text",
  ),
  (
    "tests",
    "Every test suite in tests/, plus library, binary and xtask unit tests",
  ),
  (
    "heavy",
    "Real-world crates still compile after formatting, the corpus and speed checks, the TOML conformance suite, and the settings and Unicode tables match their generators; needs network access",
  ),
  (
    "fuzz",
    "Each fuzz target for --fuzz-time seconds; needs nightly Rust and cargo-fuzz",
  ),
  (
    "bench",
    "Criterion benchmarks; slow, and report numbers rather than pass/fail",
  ),
];

/// Human titles for the test files, in run order. A test file missing from this table
/// still runs (under its file name) and triggers a warning, so nothing is skipped silently.
pub const TEST_REGISTRY: &[(&str, &str, &str)] = &[
  ("lib", "Library and binary unit tests", "tests"),
  (
    "xtask",
    "xtask unit tests: runner, gates, capabilities, logging, lexdiff",
    "tests",
  ),
  (
    "domain_1_unit",
    "Unit: config values, imports, item metadata",
    "tests",
  ),
  ("domain_2_golden", "Golden layouts", "tests"),
  ("snapshots", "Regression fixtures", "tests"),
  (
    "syntax_fixtures",
    "Parser fixtures: exact trees and error recovery",
    "tests",
  ),
  ("integration", "Block extraction", "tests"),
  (
    "domain_3_proptest",
    "Property tests: no panics, no lost lines, idempotence",
    "tests",
  ),
  (
    "domain_6_io",
    "File I/O: atomic writes, permissions, symlinks",
    "tests",
  ),
  (
    "domain_7_ecosystem",
    "Config lookup and ignored directories",
    "tests",
  ),
  (
    "domain_8_cli",
    "Command line: check, dry run, exit codes, stdin, history and restore",
    "tests",
  ),
  ("domain_9_config_hierarchy", "Config hierarchy", "tests"),
  (
    "domain_10_polyrepo",
    "Multiple repos, rollback, symlink cycles",
    "tests",
  ),
  (
    "domain_11_matrix",
    "Every settings combination is idempotent",
    "tests",
  ),
  (
    "domain_12_scale",
    "500-file repository still compiles after formatting",
    "tests",
  ),
  (
    "domain_13_rw",
    "15 real crates still compile after layout, in three configurations (network)",
    "heavy",
  ),
];

/// How long `fuzz-24h` runs each target.
const FUZZ_24H_SECONDS: u32 = 24 * 60 * 60;

/// What a task does: run a program, or a check inside this process that returns its
/// problems (none means it passed).
pub enum Action {
  Command(String, Vec<String>),
  Check(fn(&Path) -> Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
  Pending,
  Excluded,
  Skip,
  Pass,
  Fail,
}

pub struct Task {
  pub id: String,
  pub title: String,
  pub group: &'static str,
  pub action: Action,
  pub env: Vec<(String, String)>,
  pub requires: Vec<&'static str>,
  /// Runs only when named with `--target`, never as part of a group or `--full`.
  pub target_only: bool,
  pub state: State,
  pub seconds: f64,
  pub note: String,
  pub output: String,
}

impl Task {
  fn new(id: &str, title: &str, group: &'static str, action: Action) -> Task {
    Task {
      id: id.to_string(),
      title: title.to_string(),
      group,
      action,
      env: Vec::new(),
      requires: Vec::new(),
      target_only: false,
      state: State::Pending,
      seconds: 0.0,
      note: String::new(),
      output: String::new(),
    }
  }

  fn requires(mut self, tools: &[&'static str]) -> Task {
    self.requires = tools.to_vec();
    self
  }

  fn target_only(mut self) -> Task {
    self.target_only = true;
    self
  }

  fn with_env(mut self, key: &str, value: &str) -> Task {
    self.env.push((key.to_string(), value.to_string()));
    self
  }
}

/// Every task, in run order. Test files missing from [`TEST_REGISTRY`] are added at the
/// end of the tests and returned as warnings.
pub fn build(root: &Path, bless: bool, fuzz_time: u32) -> (Vec<Task>, Vec<String>) {
  let mut tasks = Vec::new();
  let mut warnings = Vec::new();
  tasks.push(
    Task::new(
      "clippy",
      "Clippy, warnings are errors",
      "gates",
      command(
        "cargo",
        &[
          "clippy",
          "--workspace",
          "--all-targets",
          "--quiet",
          "--",
          "-D",
          "warnings",
        ],
      ),
    )
    .requires(&["clippy"]),
  );
  tasks.push(Task::new(
    "deps",
    "No runtime dependencies",
    "gates",
    Action::Check(gate("deps")),
  ));
  tasks.push(Task::new(
    "layering",
    "Modules only use lower layers",
    "gates",
    Action::Check(gate("layering")),
  ));
  tasks.push(Task::new(
    "docs",
    "Public Markdown stays within 80 columns",
    "gates",
    Action::Check(gate("docs")),
  ));
  tasks.push(Task::new(
    "naming",
    "The reference formatter is named only where the name does work",
    "gates",
    Action::Check(gate("naming")),
  ));
  tasks.push(Task::new(
    "capabilities",
    "Every rustfmt option, value and flag has a status",
    "gates",
    Action::Check(capabilities::check),
  ));
  tasks.push(
    Task::new(
      "rustfmt",
      "rustfmt agrees with the repo layout",
      "gates",
      command("cargo", &["fmt", "--all", "--check"]),
    )
    .requires(&["rustfmt"]),
  );
  tasks.push(Task::new(
    "self-format",
    "mto formats itself",
    "gates",
    command(
      "cargo",
      &["run", "--quiet", "--bin", "mto", "--", "--check", "."],
    ),
  ));
  if root.join("fuzz").exists() {
    tasks.push(Task::new(
      "fuzz-build",
      "Fuzz crate compiles",
      "gates",
      command(
        "cargo",
        &["check", "--quiet", "--manifest-path", "fuzz/Cargo.toml"],
      ),
    ));
  }
  tasks.push(Task::new(
    "help",
    "-h lists every group, task, flag and variable",
    "gates",
    Action::Check(super::check_help),
  ));
  tasks.push(Task::new(
    "man",
    "mto --man documents every key, value, flag and command, and nothing else",
    "gates",
    Action::Check(super::manual::check_manual),
  ));

  tasks.push(Task::new(
    "build",
    "Build test binaries",
    "tests",
    command("cargo", &["test", "--workspace", "--no-run", "--quiet"]),
  ));
  let test_files = stems(&root.join("tests"));
  for (id, title, group) in TEST_REGISTRY {
    let action = match *id {
      "lib" => command("cargo", &["test", "--lib", "--bins", "--quiet"]),
      "xtask" => command(
        "cargo",
        &[
          "test",
          "--package",
          "xtask",
          "--package",
          "xtask-tools",
          "--quiet",
        ],
      ),
      _ if test_files.iter().any(|f| f == id) => {
        command("cargo", &["test", "--test", id, "--quiet"])
      }
      _ => continue,
    };
    let group: &'static str = group;
    let mut task = Task::new(id, title, group, action);
    if *id == "snapshots" {
      task = task.requires(&["rustc"]);
      if bless {
        task = task.with_env(ENV_BLESS, "1");
      }
    }
    if *id == "domain_13_rw" {
      task = task.requires(&["git"]).with_env(ENV_HEAVY, "1");
    }
    tasks.push(task);
  }
  for stem in test_files.iter() {
    if !TEST_REGISTRY.iter().any(|(id, _, _)| id == stem) {
      warnings.push(format!(
        "tests/{}.rs is not in TEST_REGISTRY; running it under its file name.",
        stem
      ));
      tasks.push(Task::new(
        stem,
        stem,
        "tests",
        command("cargo", &["test", "--test", stem, "--quiet"]),
      ));
    }
  }
  let tools = |name: &str| {
    command(
      "cargo",
      &["run", "--quiet", "--package", "xtask-tools", "--", name],
    )
  };
  tasks.push(Task::new(
    "lexdiff",
    "Lexer agrees with rustc's lexer on the repo's sources",
    "tests",
    tools("lexdiff"),
  ));
  tasks.push(Task::new(
    "parsecheck",
    "The repo's own sources parse without errors",
    "tests",
    tools("parsecheck"),
  ));
  let this_binary = std::env::current_exe()
    .map(|p| p.display().to_string())
    .unwrap_or_else(|_| "xtask".to_string());
  tasks.push(
    Task::new(
      "corpus",
      "Pinned corpus: round trip, valid code, tokens, verifier, speed",
      "heavy",
      Action::Command(this_binary.clone(), vec!["corpus".into(), "check".into()]),
    )
    .requires(&["git"]),
  );
  tasks.push(
    Task::new(
      "speed",
      "Speed target: mto at least 2x rustfmt's throughput (S2-1); layout per core",
      "heavy",
      Action::Command(this_binary.clone(), vec!["corpus".into(), "bench".into()]),
    )
    .requires(&["git"]),
  );
  tasks.push(
    Task::new(
      "diff",
      "Compatibility with the Rust style stays at or above compat/compatibility.txt",
      "heavy",
      Action::Command(this_binary.clone(), vec!["diff".into()]),
    )
    .requires(&["git", "rustfmt"]),
  );
  tasks.push(
    Task::new(
      "settings",
      "src/options/rustfmt.rs matches the pinned rustfmt releases",
      "heavy",
      Action::Command(
        this_binary.clone(),
        vec!["settings".into(), "--check".into()],
      ),
    )
    .requires(&["git"]),
  );
  tasks.push(
    Task::new(
      "toml-test",
      "TOML parser passes the TOML 1.0.0 conformance suite",
      "heavy",
      Action::Command(this_binary.clone(), vec!["toml-test".into()]),
    )
    .requires(&["git"]),
  );
  tasks.push(
    Task::new(
      "unicode",
      "Unicode tables match the generator",
      "heavy",
      Action::Command(
        this_binary.clone(),
        vec!["unicode".into(), "--check".into()],
      ),
    )
    .requires(&["curl"]),
  );
  // The oldest Rust the package says it builds with (`rust-version`) must build it.
  if let Some(version) = rust_version(root) {
    let toolchain = format!("+{}", version);
    tasks.push(
      Task::new(
        "msrv",
        &format!("Builds with Rust {}, the package's rust-version", version),
        "heavy",
        command(
          "cargo",
          &[&toolchain, "check", "--quiet", "--lib", "--bins"],
        ),
      )
      .requires(&["rustup"]),
    );
  }
  // Through `cargo xtask fuzz run`, which keeps what a run finds under fuzz/crashes/.
  for target in stems(&root.join("fuzz").join("fuzz_targets")) {
    tasks.push(
      Task::new(
        &target,
        &format!("Fuzz {} for {}s", target, fuzz_time),
        "fuzz",
        Action::Command(
          this_binary.clone(),
          vec![
            "fuzz".into(),
            "run".into(),
            target.clone(),
            "--time".into(),
            fuzz_time.to_string(),
          ],
        ),
      )
      .requires(&["cargo-fuzz"]),
    );
  }
  tasks.push(
    Task::new(
      "fuzz-24h",
      "Fuzz every target side by side for 24 hours (--target only)",
      "fuzz",
      Action::Check(fuzz_24h),
    )
    .requires(&["cargo-fuzz"])
    .target_only(),
  );
  if root.join("benches").exists() {
    tasks.push(Task::new(
      "bench",
      "Benchmarks",
      "bench",
      command("cargo", &["bench", "--quiet"]),
    ));
  }
  (tasks, warnings)
}

fn command(program: &str, args: &[&str]) -> Action {
  Action::Command(
    program.to_string(),
    args.iter().map(|a| a.to_string()).collect(),
  )
}

/// The final stage 1 gate: the lexer and parser targets fuzzed at the same time, each for
/// [`FUZZ_24H_SECONDS`]. A crash in either is a problem, with the end of its output. Like
/// every fuzz task, it fails if the machine slept (the runner keeps it awake and checks).
fn fuzz_24h(root: &Path) -> Vec<String> {
  let targets = stems(&root.join("fuzz").join("fuzz_targets"));
  let log = crate::common::log::Logger::new(root, "fuzz");
  let run = |target: &str| {
    let code = crate::fuzz::run_target(root, target, FUZZ_24H_SECONDS, &log);
    (target.to_string(), (code, String::new()))
  };
  let results = std::thread::scope(|scope| {
    let handles: Vec<_> = targets
      .iter()
      .map(|target| scope.spawn(|| run(target)))
      .collect();
    handles
      .into_iter()
      .filter_map(|h| h.join().ok())
      .collect::<Vec<_>>()
  });
  let mut problems = Vec::new();
  for (target, (code, output)) in results {
    if code != 0 {
      let tail: Vec<&str> = output.lines().rev().take(30).collect();
      let tail: Vec<&str> = tail.into_iter().rev().collect();
      problems.push(format!(
        "fuzz target {} failed (exit {}):\n{}",
        target,
        code,
        tail.join("\n")
      ));
    }
  }
  problems
}

fn gate(name: &str) -> fn(&Path) -> Vec<String> {
  gates::GATES
    .iter()
    .find(|(n, _, _)| *n == name)
    .map(|(_, _, check)| *check)
    .unwrap_or(|_| vec!["unknown gate".to_string()])
}

/// `rust-version` from the root `Cargo.toml`'s `[package]`.
fn rust_version(root: &Path) -> Option<String> {
  let manifest = fs::read_to_string(root.join("Cargo.toml")).ok()?;
  manifest.lines().find_map(|line| {
    let value = line.trim().strip_prefix("rust-version")?.trim();
    Some(
      value
        .strip_prefix('=')?
        .trim()
        .trim_matches('"')
        .to_string(),
    )
  })
}

/// File stems of the `.rs` files directly in `dir`, sorted.
fn stems(dir: &Path) -> Vec<String> {
  let mut stems: Vec<String> = fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .map(|entry| entry.path())
    .filter(|path| path.extension().is_some_and(|e| e == "rs"))
    .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
    .collect();
  stems.sort();
  stems
}
