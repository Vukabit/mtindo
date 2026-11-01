//! `cargo xtask flow`: how changes reach mtindo's history and its releases, from one
//! file, `flow.toml`. The git hooks in `.githooks/` are one line each, calling
//! `cargo xtask flow hook NAME`; CI runs the same commands, so every check is written
//! once here.
//!
//! - `setup` turns the hooks on for this clone (`core.hooksPath`).
//! - `hook commit-msg` checks a message: `kind(scope): subject`, the kinds and sections
//!   `flow.toml` lists, the scope covering the commit's files, a body only where the
//!   change earns one.
//! - `hook prepare-commit-msg` starts the message with the scope the staged files are in.
//! - `hook pre-commit` and `hook pre-push` run those stages' checks (`[stages]`): the
//!   staged files, and the commits and refs being pushed.
//! - `check STAGE` runs a stage's checks on the whole tree, as CI does.
//! - `messages RANGE` checks every commit in a range.
//! - `release VERSION` cuts a release (a plan unless `--go`); `yank VERSION` prints how
//!   to take one back.
//! - `package TARGET` and `prepare TAG` are the release workflow's steps.

pub(crate) mod checks;
pub(crate) mod config;
mod git;
mod glob;
mod message;
mod publish;
mod release;
mod tools;

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use config::Flow;

use crate::cli;

/// `cargo xtask bootstrap`: the tools flow.toml pins, installed or updated.
pub fn bootstrap(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let outcome = config::load(root).and_then(|flow| {
    tools::bootstrap(
      root,
      &flow,
      cli::text(m, "for").as_deref(),
      cli::on(m, "go"),
    )
  });
  match outcome {
    Ok(()) => ExitCode::SUCCESS,
    Err(e) => {
      eprintln!("{}", e);
      ExitCode::from(1)
    }
  }
}

/// Each kind `flow.toml` lists, with its changelog heading (`None`: left out).
pub fn kinds(root: &Path) -> Result<Vec<(String, Option<String>)>, String> {
  Ok(config::load(root)?.kinds)
}

/// The tools a command (`changelog`, `release`) needs, each ready.
pub fn require(root: &Path, user: &str) -> Result<(), String> {
  tools::require(&config::load(root)?, user)
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let Some((name, sub)) = m.subcommand() else {
    return ExitCode::from(2);
  };
  let outcome = match name {
    "setup" => setup(root),
    "hook" => {
      let args = cli::texts(sub, "args");
      match cli::text(sub, "name").as_deref() {
        Some("commit-msg") => commit_msg(root, &args),
        Some("prepare-commit-msg") => prepare_commit_msg(root, &args),
        Some("pre-commit") => pre_commit(root),
        Some("pre-push") => pre_push(root),
        Some(other) => Err(format!("no hook `{}`", other)),
        None => Err("which hook?".to_string()),
      }
    }
    "messages" => config::load(root).and_then(|flow| {
      messages_in(
        root,
        &flow,
        &[range_args(&cli::text(sub, "range").unwrap_or_default())],
      )
    }),
    "release" => config::load(root).and_then(|flow| {
      release::release(
        root,
        &flow,
        &cli::text(sub, "version").unwrap_or_default(),
        cli::on(sub, "go"),
      )
    }),
    "yank" => release::yank(&cli::text(sub, "version").unwrap_or_default()),
    "package" => config::load(root).and_then(|flow| {
      publish::package(root, &flow, &cli::text(sub, "target").unwrap_or_default())
    }),
    "prepare" => config::load(root).and_then(|flow| {
      publish::prepare(
        root,
        &flow,
        &cli::text(sub, "tag").unwrap_or_default(),
        cli::on(sub, "dry-run"),
      )
    }),
    "check" => check(
      root,
      &cli::text(sub, "stage").unwrap_or_default(),
      cli::text(sub, "range"),
    ),
    other => Err(format!("no flow command `{}`", other)),
  };
  match outcome {
    Ok(()) => ExitCode::SUCCESS,
    Err(e) => {
      if !e.is_empty() {
        eprintln!("{}", e);
      }
      ExitCode::from(1)
    }
  }
}

/// `check STAGE [--range A..B]`: a stage's checks on the whole tree, the range's commit
/// messages too (CI's push or pull request).
pub(crate) fn check(root: &Path, stage: &str, range: Option<String>) -> Result<(), String> {
  let flow = config::load(root)?;
  let names = flow
    .stage(stage)
    .ok_or(format!("no stage `{}` (flow.toml [stages])", stage))?;
  let scope = checks::Scope {
    stage: stage.to_string(),
    files: None,
    staged: false,
    ranges: range.iter().map(|r| range_args(r)).collect(),
    pushed: Vec::new(),
  };
  tools::require(&flow, stage)?;
  checks::run_stage(root, &flow, names, &scope)
}

/// The hooks in `.githooks/` turned on for this clone.
pub(super) fn setup(root: &Path) -> Result<(), String> {
  config::load(root)?;
  git::git(root, &["config", "core.hooksPath", ".githooks"])?;
  let hooks: Vec<String> = fs::read_dir(root.join(".githooks"))
    .map_err(|e| format!(".githooks: {}", e))?
    .filter_map(|e| e.ok())
    .map(|e| e.file_name().to_string_lossy().into_owned())
    .collect();
  println!("hooks: core.hooksPath = .githooks ({})", hooks.join(", "));
  Ok(())
}

/// `commit-msg FILE`: the message git is about to commit.
fn commit_msg(root: &Path, args: &[String]) -> Result<(), String> {
  let file = args.first().ok_or("commit-msg: no message file")?;
  // A merge's message is git's own.
  if git::git_path(root, "MERGE_HEAD")?.is_file() {
    return Ok(());
  }
  let flow = config::load(root)?;
  let raw = fs::read_to_string(root.join(file)).map_err(|e| format!("{}: {}", file, e))?;
  let text = message::cleaned(&raw);
  // Squashed into another before it is pushed (`messages` refuses it there).
  if message::is_autosquash(&text) {
    return Ok(());
  }
  let (mut files, mut lines) = git::staged(root)?;
  // Nothing staged: rewording the last commit (`--amend`), whose change it describes.
  if files.is_empty()
    && let Ok((amended, amended_lines)) = git::changed_by(root, "HEAD")
  {
    (files, lines) = (amended, amended_lines);
  }
  let tree = git::tree_files(root, None)?;
  let known = |p: &str| git::names(&tree, &files, p);
  report(
    "commit-msg",
    &text,
    message::problems(
      &text,
      &flow,
      &message::Change {
        files: &files,
        changed_lines: lines,
        known: &known,
      },
    ),
  )
}

/// Every commit in `ranges`, as pushing and CI check them: merges pass, commits made to
/// be squashed do not.
fn messages_in(root: &Path, flow: &Flow, ranges: &[Vec<String>]) -> Result<(), String> {
  let mut report = Vec::new();
  let mut count = 0;
  // History from before the rules is left as it was written.
  let before = match &flow.commit.since {
    Some(since) => vec![
      "--not".to_string(),
      git::git(
        root,
        &["rev-parse", "--verify", &format!("{}^{{commit}}", since)],
      )
      .map_err(|_| format!("flow.toml: commit.since, {}, is not a commit here", since))?
      .trim()
      .to_string(),
    ],
    None => Vec::new(),
  };
  for range in ranges {
    let range: Vec<String> = range.iter().chain(&before).cloned().collect();
    for (sha, merge) in git::commits(root, &range)? {
      count += 1;
      if merge {
        continue;
      }
      let text = message::cleaned(&git::message(root, &sha)?);
      // A GitHub bot (Dependabot) always writes a body; only its first line is held to
      // the rules.
      let author = git::git(root, &["log", "-1", "--format=%an", &sha]).unwrap_or_default();
      let text = if author.trim().ends_with("[bot]") {
        text.lines().next().unwrap_or("").to_string()
      } else {
        text
      };
      let short = &sha[..sha.len().min(10)];
      let first = text.lines().next().unwrap_or("").to_string();
      if message::is_autosquash(&text) {
        report.push(format!(
          "{}  {}\n  - squash it into the commit it fixes before pushing",
          short, first
        ));
        continue;
      }
      let (files, lines) = git::changed_by(root, &sha)?;
      let tree = git::tree_files(root, Some(&sha))?;
      let known = |p: &str| git::names(&tree, &files, p);
      let found = message::problems(
        &text,
        flow,
        &message::Change {
          files: &files,
          changed_lines: lines,
          known: &known,
        },
      );
      if !found.is_empty() {
        let mut entry = format!("{}  {}", short, first);
        for problem in found {
          entry.push_str(&format!("\n  - {}", problem));
        }
        report.push(entry);
      }
    }
  }
  if report.is_empty() {
    Ok(())
  } else {
    Err(format!(
      "{} of {} commit(s):\n{}",
      report.len(),
      count,
      report.join("\n")
    ))
  }
}

/// `pre-commit`: its stage's checks on what is staged.
fn pre_commit(root: &Path) -> Result<(), String> {
  let flow = config::load(root)?;
  let (files, _) = git::staged(root)?;
  let scope = checks::Scope {
    stage: "pre-commit".to_string(),
    files: Some(files),
    staged: true,
    ranges: Vec::new(),
    pushed: Vec::new(),
  };
  tools::require(&flow, "pre-commit")?;
  checks::run_stage(
    root,
    &flow,
    flow.stage("pre-commit").unwrap_or_default(),
    &scope,
  )
}

/// `pre-push`: its stage's checks on the commits and refs being pushed, which git gives
/// on stdin a line each (`local-ref local-sha remote-ref remote-sha`).
fn pre_push(root: &Path) -> Result<(), String> {
  let flow = config::load(root)?;
  let mut input = String::new();
  std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
    .map_err(|e| format!("pre-push: {}", e))?;
  let zero = |sha: &str| sha.chars().all(|c| c == '0');
  let mut ranges = Vec::new();
  let mut pushed = Vec::new();
  for line in input.lines() {
    let words: Vec<&str> = line.split_whitespace().collect();
    let [_, local, remote_ref, remote] = words.as_slice() else {
      continue;
    };
    // A ref deleted: nothing to check.
    if zero(local) {
      continue;
    }
    pushed.push((remote_ref.to_string(), local.to_string()));
    // A new branch: its commits no remote has yet.
    ranges.push(if zero(remote) {
      vec![
        local.to_string(),
        "--not".to_string(),
        "--remotes".to_string(),
      ]
    } else {
      vec![format!("{}..{}", remote, local)]
    });
  }
  let scope = checks::Scope {
    stage: "pre-push".to_string(),
    files: None,
    staged: false,
    ranges,
    pushed,
  };
  tools::require(&flow, "pre-push")?;
  checks::run_stage(
    root,
    &flow,
    flow.stage("pre-push").unwrap_or_default(),
    &scope,
  )
}

/// `prepare-commit-msg FILE [SOURCE [SHA]]`: a plain `git commit`'s message started with
/// the sections the staged files are in.
fn prepare_commit_msg(root: &Path, args: &[String]) -> Result<(), String> {
  let file = args.first().ok_or("prepare-commit-msg: no message file")?;
  // A message given (`-m`, `-F`), a template, a merge, a squash or an amend: left alone.
  if args.len() > 1 {
    return Ok(());
  }
  let flow = config::load(root)?;
  let (files, _) = git::staged(root)?;
  let scope = message::suggested_scope(&flow, &files);
  let path = root.join(file);
  let written = fs::read_to_string(&path).unwrap_or_default();
  let kinds: Vec<&str> = flow.kinds.iter().map(|(k, _)| k.as_str()).collect();
  let start = format!(
    "kind({}): \n\n# kinds: {}\n# sections of the staged files: {}\n",
    scope,
    kinds.join(" "),
    if scope.is_empty() {
      "none: name a path"
    } else {
      &scope
    }
  );
  fs::write(&path, format!("{}{}", start, written)).map_err(|e| format!("{}: {}", file, e))
}

/// `A..B` as `rev-list` takes it; `B --not --remotes` style ranges pass through.
fn range_args(range: &str) -> Vec<String> {
  range.split_whitespace().map(String::from).collect()
}

/// The commits `flow release` made and marked for `hold` to let through.
fn release_mark(root: &Path) -> Vec<String> {
  git::git_path(root, "flow-release")
    .ok()
    .and_then(|path| fs::read_to_string(path).ok())
    .map(|text| text.split_whitespace().map(String::from).collect())
    .unwrap_or_default()
}

/// A hook's verdict on one message.
fn report(hook: &str, text: &str, found: Vec<String>) -> Result<(), String> {
  if found.is_empty() {
    return Ok(());
  }
  eprintln!("{}  x message", hook);
  eprintln!("  {}", text.lines().next().unwrap_or(""));
  for problem in &found {
    eprintln!("  - {}", problem);
  }
  Err("commit refused".to_string())
}
