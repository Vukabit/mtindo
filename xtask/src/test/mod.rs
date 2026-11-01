//! `cargo xtask test`: runs mtindo's gates and test suites and reports each task as passed,
//! failed or skipped with a reason.
//!
//! Exit codes: 0 when nothing failed, 1 when anything failed, 2 for bad arguments. Only a
//! task outside the chosen groups is skipped; a chosen task that cannot run (its tool is
//! missing) fails, and a task past its time limit is stopped and fails, so a green run
//! means every chosen task ran and passed. Every run is logged to `.dev/logs/test.log` with
//! each task's full output.
//!
//! Every green default or full run is added to `.dev/logs/last-green.tsv`, named by its exact
//! tree (`<commit>` or `<commit>-dirty.<digest>`); after a failure the runner shows the
//! last green tree and what changed since.
//!
//! The runner does not depend on mtindo, so it builds and reports even when the library
//! does not compile. Ctrl-C stops the runner and the running task together.
//!
//! For the whole run the machine is kept awake ([`KeepAwake`]), and every task's time asleep
//! is measured. Fuzz tasks fail if the machine slept more than a minute: their time limit
//! is wall-clock time, so sleep would cut the fuzzing short while still passing.

mod manual;
mod reference;
pub mod tasks;

use std::collections::HashMap;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use tasks::{
  Action, DEFAULT_FUZZ_TIME, DEFAULT_GROUPS, ENV_VARS, GROUPS, State, TEST_REGISTRY, Task,
};

use crate::common::awake::{self, KeepAwake, Stamp};
use crate::common::color::{HelpPalette, color_enabled};
use crate::common::log::{Logger, timestamp};
use crate::common::store;
use crate::common::{process, tree};

/// Lines of a failed task's output shown in the terminal; the log has all of it.
const FAILURE_TAIL_LINES: usize = 60;

/// More sleep than this during a fuzz task fails it.
const FUZZ_SLEEP_ALLOWED_SECONDS: u64 = 60;

/// One line per green default or full run: when, tree, groups, seconds.
const GREEN_FILE: &str = ".dev/logs/last-green.tsv";

const GREEN_HEADER: &str = "when\ttree\tgroups\tseconds\n";

/// Each task's latest result, whichever run gave it: `.dev/logs/last-test.tsv`.
const LAST_FILE: &str = ".dev/logs/last-test.tsv";

/// Terminal colors for the run report; empty strings when color is off.
struct Colors {
  on: bool,
}

impl Colors {
  fn paint(&self, code: &str, text: &str) -> String {
    if self.on {
      format!("\x1b[{}m{}\x1b[0m", code, text)
    } else {
      text.to_string()
    }
  }
}

#[derive(Debug, PartialEq)]
struct Options {
  groups: Vec<String>,
  full: bool,
  /// Only the tasks that failed when they last ran.
  failed: bool,
  target: String,
  fuzz_time: u32,
  bless: bool,
  list: bool,
}

/// A spinner with the running task's title, shown only on a terminal.
struct Spinner {
  running: Arc<AtomicBool>,
  handle: Option<JoinHandle<()>>,
}

impl Spinner {
  fn start(message: &str, colors: &Colors) -> Spinner {
    let running = Arc::new(AtomicBool::new(true));
    if !std::io::stdout().is_terminal() {
      return Spinner {
        running,
        handle: None,
      };
    }
    let flag = Arc::clone(&running);
    let message = message.to_string();
    let frames: Vec<String> = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
      .iter()
      .map(|f| colors.paint("96", f))
      .collect();
    let handle = thread::spawn(move || {
      let mut idx = 0;
      while flag.load(Ordering::Relaxed) {
        print!("\r\x1b[K{} {}", frames[idx], message);
        let _ = std::io::stdout().flush();
        idx = (idx + 1) % frames.len();
        thread::sleep(Duration::from_millis(100));
      }
    });
    Spinner {
      running,
      handle: Some(handle),
    }
  }

  fn stop(self) {
    self.running.store(false, Ordering::Relaxed);
    if let Some(handle) = self.handle {
      let _ = handle.join();
      print!("\r\x1b[K");
      let _ = std::io::stdout().flush();
    }
  }
}

/// The `help` gate: -h must list every group, task id, flag and environment variable.
pub fn check_help(root: &Path) -> Vec<String> {
  let help = render_help(root, false);
  let listed = |name: &str| {
    help
      .lines()
      .any(|line| line.trim_start().starts_with(name) && line.starts_with("  "))
  };
  let mut missing = Vec::new();
  for (group, _) in GROUPS {
    if !listed(group) {
      missing.push(format!("-h does not list group {}", group));
    }
  }
  let (tasks, _) = tasks::build(root, false, DEFAULT_FUZZ_TIME);
  for task in tasks.iter() {
    if !listed(&task.id) {
      missing.push(format!("-h does not list task {}", task.id));
    }
  }
  for arg in crate::cli::find(&["test".to_string()])
    .map(|c| c.get_arguments().cloned().collect::<Vec<_>>())
    .unwrap_or_default()
  {
    if let Some(long) = arg.get_long()
      && !help.contains(&format!("--{}", long))
    {
      missing.push(format!("-h does not list flag --{}", long));
    }
  }
  for (name, _) in ENV_VARS {
    if !help.contains(name) {
      missing.push(format!("-h does not list environment variable {}", name));
    }
  }
  for (id, _, _) in TEST_REGISTRY {
    if *id != "lib" && *id != "xtask" && !root.join("tests").join(format!("{}.rs", id)).exists() {
      missing.push(format!(
        "TEST_REGISTRY lists {}, but tests/{}.rs does not exist",
        id, id
      ));
    }
  }
  missing
}

/// The help screen. The text is fixed; only the colors depend on `color`.
pub fn render_help(root: &Path, color: bool) -> String {
  let p = HelpPalette::new(color, "cargo xtask test");
  let mut out = crate::cli::help(&["test".to_string()], color).unwrap_or_default();
  let defaults: Vec<&str> = DEFAULT_GROUPS.to_vec();
  out.push_str(&format!(
    "\n{}\n",
    p.heading(&format!("groups (default: {}):", defaults.join(", ")))
  ));
  for (name, info) in GROUPS {
    out.push_str(&format!("  {}{}\n", p.value(&format!("{:<7}", name)), info));
  }
  out.push_str(&format!(
    "\n{}\n",
    p.heading("tasks (id, group, what it checks; run one with --target ID):")
  ));
  let (tasks, _) = tasks::build(root, false, DEFAULT_FUZZ_TIME);
  for task in tasks.iter() {
    out.push_str(&format!(
      "  {}{:<7}{}\n",
      p.value(&format!("{:<26}", task.id)),
      task.group,
      task.title
    ));
  }
  out.push_str(&format!(
    "\n{}\n",
    p.heading("environment variables the runner sets:")
  ));
  for (name, info) in ENV_VARS {
    out.push_str(&format!(
      "  {}{}\n",
      p.value(&format!("{:<20}", name)),
      info
    ));
  }
  out.push_str(&format!("\n{}\n", p.heading("files:")));
  for (path, info) in [
    (
      ".dev/logs/test.log",
      "every run, with the full output of every task",
    ),
    (
      ".dev/logs/capabilities.log",
      "what cargo xtask capabilities read, fetched and wrote",
    ),
    (
      ".dev/logs/last-green.tsv",
      "every green default or full run, by exact tree",
    ),
    (
      "records/upstream/rustfmt-surface.txt",
      "rustfmt's options and flags; update with cargo xtask capabilities refresh",
    ),
    (
      "records/policy/capabilities.txt",
      "mto's status for each; add new items with cargo xtask capabilities sync",
    ),
  ] {
    out.push_str(&format!("  {:<28}{}\n", path, info));
  }
  out.push_str(&format!("\n{}\n", p.heading("exit codes:")));
  out.push_str("  0 every chosen task ran and passed, 1 something failed, 2 bad arguments\n");
  out
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let options = options_of(m);
  let colors = Colors {
    on: color_enabled(false),
  };
  let log = Logger::new(root, "test");
  let (mut tasks, warnings) = tasks::build(root, options.bless, options.fuzz_time);
  for warning in warnings {
    println!("{} {}", colors.paint("93", "warning:"), warning);
    log.warning(&warning);
  }
  let mut groups = select(&mut tasks, &options);
  if options.failed {
    let failed = last_failed(root);
    if failed.is_empty() {
      println!("Nothing failed when it last ran ({}).", LAST_FILE);
      return ExitCode::SUCCESS;
    }
    for task in tasks.iter_mut() {
      task.state = if failed.contains(&task.id) {
        State::Pending
      } else {
        State::Excluded
      };
    }
    groups = vec!["failed last time".to_string()];
  }
  if !options.target.is_empty() && !tasks.iter().any(|t| t.id == options.target) {
    eprintln!(
      "Unknown task id '{}'. Use --list to see the ids.",
      options.target
    );
    return ExitCode::from(2);
  }
  if options.list {
    print_plan(&tasks, &groups, &colors);
    return ExitCode::SUCCESS;
  }

  let revision = tree::identity(root);
  log.info(&format!(
    "Starting run: groups={} target={} revision={}",
    groups.join(","),
    options.target,
    revision
  ));
  let chosen = if options.target.is_empty() {
    groups.join(", ")
  } else {
    format!("target {}", options.target)
  };
  println!(
    "{}{}",
    colors.paint("1", "mtindo checks"),
    colors.paint("90", &format!("  ({} | {})", chosen, revision))
  );
  println!("========================================");

  let keep_awake = KeepAwake::start();
  log.info(&format!(
    "Keeping the machine awake with {}",
    keep_awake.how()
  ));
  let started = Instant::now();
  let mut tools = HashMap::new();
  for task in tasks.iter_mut() {
    match task.state {
      State::Excluded => continue,
      State::Pending => run_task(root, task, &mut tools, &log, &colors),
      _ => {}
    }
    print_task_line(task, &colors);
  }
  let elapsed = started.elapsed().as_secs_f64();
  drop(keep_awake);

  if options.bless {
    print_blessed_changes(root, &colors);
  }
  record_last(root, &tasks, &revision);
  let failed = print_summary(&tasks, elapsed, &log, &colors);
  if failed == 0 {
    let default_groups = groups.iter().any(|g| g == "gates") && groups.iter().any(|g| g == "tests");
    if options.target.is_empty() && !options.bless && default_groups {
      record_green(root, &revision, &groups, elapsed, &log);
    }
    return ExitCode::SUCCESS;
  }
  print_changes_since_last_green(root, &revision, &colors);
  ExitCode::from(1)
}

fn describe(task: &Task) -> String {
  match &task.action {
    Action::Check(_) => "(built-in check)".to_string(),
    Action::Command(program, args) => {
      let mut text = String::new();
      for (key, value) in task.env.iter() {
        text.push_str(&format!("{}={} ", key, value));
      }
      text.push_str(program);
      for arg in args {
        text.push(' ');
        text.push_str(arg);
      }
      text
    }
  }
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
  if !process::on_path("git") {
    return None;
  }
  let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
  let (code, output) = process::capture(root, "git", &args, &[], process::QUICK);
  (code == 0).then_some(output)
}

/// How to get a missing tool, for the failure message.
fn install_hint(tool: &str) -> &'static str {
  match tool {
    "clippy" | "rustfmt" | "cargo-fuzz" => {
      "Install it with: cargo xtask bootstrap --go (the versions flow.toml pins)"
    }
    "rustup" => "Install it from https://rustup.rs",
    _ => "Install it and make sure it is on PATH.",
  }
}

/// The ids of the tasks that failed when they last ran.
fn last_failed(root: &Path) -> Vec<String> {
  std::fs::read_to_string(root.join(LAST_FILE))
    .unwrap_or_default()
    .lines()
    .skip(1)
    .filter_map(|l| {
      let f: Vec<&str> = l.split('\t').collect();
      (f.get(2) == Some(&"failed")).then(|| f[0].to_string())
    })
    .collect()
}

/// The latest line of [`GREEN_FILE`]: when, and which tree.
fn last_green(root: &Path) -> Option<(String, String)> {
  let text = fs::read_to_string(root.join(GREEN_FILE)).ok()?;
  let line = text
    .lines()
    .skip(1)
    .filter(|l| !l.trim().is_empty())
    .last()?;
  let mut fields = line.split('\t');
  Some((fields.next()?.to_string(), fields.next()?.to_string()))
}

fn options_of(m: &clap::ArgMatches) -> Options {
  use crate::cli::{on, text, texts};
  Options {
    groups: texts(m, "group"),
    full: on(m, "full"),
    failed: on(m, "failed"),
    target: text(m, "target").unwrap_or_default(),
    fuzz_time: m
      .get_one::<u32>("fuzz-time")
      .copied()
      .unwrap_or(DEFAULT_FUZZ_TIME),
    bless: on(m, "bless"),
    list: on(m, "list"),
  }
}

fn print_blessed_changes(root: &Path, colors: &Colors) {
  println!(
    "\n{}",
    colors.paint(
      "93",
      "--bless rewrote the expected outputs. Review every change before committing:"
    )
  );
  match git(root, &["status", "--short", "--", "tests/fixtures"]) {
    Some(out) if !out.trim().is_empty() => println!("{}", out.trim_end()),
    _ => println!("  no fixture changed"),
  }
}

/// After a failure: the last green tree, and what changed since its commit.
fn print_changes_since_last_green(root: &Path, revision: &str, colors: &Colors) {
  let Some((when, last)) = last_green(root) else {
    return;
  };
  if last == revision {
    println!(
      "\nThis exact tree passed at {}; the failure likely comes from the environment.",
      when
    );
    return;
  }
  let commit = tree::commit_of(&last);
  let Some(stat) = git(root, &["diff", "--stat", commit]) else {
    return;
  };
  let since = if last.contains("-dirty.") {
    format!(
      "{} (green at {} with uncommitted changes of its own)",
      commit, when
    )
  } else {
    format!("{} (green at {})", commit, when)
  };
  println!(
    "\n{}",
    colors.paint(
      "93",
      &format!(
        "Changes since the last green tree, {}, including uncommitted work:",
        since
      )
    )
  );
  println!("{}", stat.trim_end());
  if let Some(untracked) = git(root, &["ls-files", "--others", "--exclude-standard"])
    && !untracked.trim().is_empty()
  {
    println!(
      "  plus {} untracked file(s)",
      untracked.trim().lines().count()
    );
  }
  println!(
    "{}",
    colors.paint("90", &format!("See it all with: git diff {}", commit))
  );
}

fn print_plan(tasks: &[Task], groups: &[String], colors: &Colors) {
  println!(
    "{}",
    colors.paint("1", &format!("Plan (groups: {})", groups.join(", ")))
  );
  for task in tasks {
    let marker = match task.state {
      State::Pending => colors.paint("92", "run "),
      State::Skip => colors.paint("93", "skip"),
      _ => colors.paint("90", "off "),
    };
    println!(
      "  {}  {:<6}  {:<26}{}",
      marker, task.group, task.id, task.title
    );
  }
}

/// Prints the totals and returns how many tasks failed.
fn print_summary(tasks: &[Task], elapsed: f64, log: &Logger, colors: &Colors) -> usize {
  let count = |state: State| tasks.iter().filter(|t| t.state == state).count();
  let (passed, failed, skipped) = (count(State::Pass), count(State::Fail), count(State::Skip));
  println!("========================================");
  let counts = format!(
    "{} passed, {} failed, {} skipped in {}",
    passed,
    failed,
    skipped,
    seconds(elapsed)
  );
  if failed == 0 {
    println!("{}", colors.paint("1;92", &format!("SUCCESS: {}", counts)));
  } else {
    println!("{}", colors.paint("1;91", &format!("FAILURE: {}", counts)));
    for task in tasks.iter().filter(|t| t.state == State::Fail) {
      println!(
        "  {} {}  (rerun: cargo xtask test --target {})",
        colors.paint("91", "✗"),
        task.id,
        task.id
      );
    }
  }
  log.info(&format!("Run complete: {}", counts));
  failed
}

fn print_task_line(task: &Task, colors: &Colors) {
  let mark = match task.state {
    State::Pass => colors.paint("92", "✓"),
    State::Fail => colors.paint("91", "✗"),
    _ => colors.paint("93", "-"),
  };
  let mut line = format!("{} {}", mark, task.title);
  match task.state {
    State::Pass => line.push_str(&colors.paint("90", &format!(" ({})", seconds(task.seconds)))),
    State::Fail => line.push_str(&colors.paint(
      "90",
      &format!(" ({}, {})", task.note, seconds(task.seconds)),
    )),
    State::Skip => line.push_str(&colors.paint("90", &format!(" (skipped: {})", task.note))),
    _ => {}
  }
  println!("{}", line);
  if task.state == State::Fail && !task.output.trim().is_empty() {
    let lines: Vec<&str> = task.output.trim_end().lines().collect();
    let shown = if lines.len() > FAILURE_TAIL_LINES {
      println!(
        "{}",
        colors.paint(
          "90",
          &format!(
            "    ... {} earlier lines in .dev/logs/test.log",
            lines.len() - FAILURE_TAIL_LINES
          )
        )
      );
      &lines[lines.len() - FAILURE_TAIL_LINES..]
    } else {
      &lines[..]
    };
    for output_line in shown {
      println!("    {}", output_line);
    }
  }
  let _ = std::io::stdout().flush();
}

fn record_green(root: &Path, revision: &str, groups: &[String], elapsed: f64, log: &Logger) {
  let path = root.join(GREEN_FILE);
  let mut text = fs::read_to_string(&path).unwrap_or_else(|_| GREEN_HEADER.to_string());
  text.push_str(&format!(
    "{}\t{}\t{}\t{:.1}\n",
    timestamp(SystemTime::now()),
    revision,
    groups.join(","),
    elapsed
  ));
  let written =
    fs::create_dir_all(store::dir(root, store::LOGS)).and_then(|_| fs::write(&path, text));
  match written {
    Ok(()) => log.info(&format!("Recorded green tree {}", revision)),
    Err(e) => log.warning(&format!("cannot write {}: {}", GREEN_FILE, e)),
  }
}

/// Adds this run's results to `.dev/logs/last-test.tsv`, each task's replacing what an
/// earlier run said of it; tasks it did not run keep theirs. In the order of the suite.
fn record_last(root: &Path, tasks: &[Task], tree: &str) {
  let header = "id\tgroup\tstate\tseconds\twhen\ttree\ttitle\tnote";
  let mut known: HashMap<String, String> = std::fs::read_to_string(root.join(LAST_FILE))
    .unwrap_or_default()
    .lines()
    .skip(1)
    .filter_map(|l| {
      l.split('\t')
        .next()
        .map(|id| (id.to_string(), l.to_string()))
    })
    .collect();
  let when = crate::common::log::timestamp(std::time::SystemTime::now());
  let clean = |t: &str| t.replace(['\t', '\n'], " ");
  for task in tasks {
    let state = match task.state {
      State::Pass => "passed",
      State::Fail => "failed",
      State::Skip => "skipped",
      State::Pending | State::Excluded => continue,
    };
    known.insert(
      task.id.clone(),
      format!(
        "{}\t{}\t{}\t{:.2}\t{}\t{}\t{}\t{}",
        task.id,
        task.group,
        state,
        task.seconds,
        when,
        tree,
        clean(&task.title),
        clean(&task.note)
      ),
    );
  }
  let mut text = format!("{}\n", header);
  for task in tasks {
    if let Some(line) = known.remove(&task.id) {
      text.push_str(&line);
      text.push('\n');
    }
  }
  let path = root.join(LAST_FILE);
  if let Some(dir) = path.parent() {
    let _ = std::fs::create_dir_all(dir);
  }
  let _ = std::fs::write(path, text);
}

fn run_task(
  root: &Path,
  task: &mut Task,
  cache: &mut HashMap<&'static str, bool>,
  log: &Logger,
  colors: &Colors,
) {
  for (tool, why) in task.optional.clone() {
    if !tool_available(root, tool, cache, log) {
      task.state = State::Skip;
      task.note = format!("{} is not installed ({})", tool, why);
      log.info(&format!("Skipped [{}]: {}", task.id, task.note));
      return;
    }
  }
  for tool in task.requires.clone() {
    if !tool_available(root, tool, cache, log) {
      task.state = State::Fail;
      task.note = format!("{} is not available", tool);
      task.output = format!(
        "This task needs {}, which is not available, so it could not run. {}\n",
        tool,
        install_hint(tool)
      );
      log.error(&format!("Failed [{}]: {} is not available", task.id, tool));
      return;
    }
  }
  log.info(&format!("Running [{}]: {}", task.id, describe(task)));
  let spinner = Spinner::start(&task.title, colors);
  let stamp = Stamp::now();
  let started = Instant::now();
  let mut timed_out = false;
  let (code, output) = match &task.action {
    Action::Command(program, args) => {
      let outcome = process::run(root, program, args, &task.env, task.limit);
      timed_out = outcome.timed_out;
      (outcome.code, outcome.output)
    }
    Action::Check(check) => {
      let problems = check(root);
      if problems.is_empty() {
        (0, "ok\n".to_string())
      } else {
        (1, problems.join("\n") + "\n")
      }
    }
  };
  task.seconds = started.elapsed().as_secs_f64();
  crate::metrics::record(
    root,
    &format!("task.{}.seconds", task.id),
    task.seconds,
    "s",
  );
  spinner.stop();
  log.debug(&format!("Output of [{}]:\n{}", task.id, output));
  task.output = output;
  let asleep = awake::asleep_seconds(stamp);
  let mut code = code;
  if asleep > 0 {
    log.warning(&format!(
      "The machine slept {} during [{}]",
      awake::duration(asleep),
      task.id
    ));
  }
  if asleep > FUZZ_SLEEP_ALLOWED_SECONDS && task.group == "fuzz" {
    code = 1;
    task.output.push_str(&format!(
            "\nThe machine slept for {} of this task, so it fuzzed for only about {}. \
             The time limit counts sleep; keep the machine awake (a laptop's lid open) and run again.\n",
            awake::duration(asleep),
            awake::duration(task.seconds as u64)
        ));
  }
  // The real-world check's breaks are incidents; its passing fixes them.
  if task.id == "domain_13_rw" {
    let filed = crate::incidents::from_real_world(root, code == 0, &task.output);
    if filed > 0 {
      log.info(&format!(
        "domain_13_rw: {} compile-break incident(s) filed or fixed",
        filed
      ));
    }
  }
  if code == 0 {
    task.state = State::Pass;
    log.info(&format!(
      "Passed [{}] in {}",
      task.id,
      seconds(task.seconds)
    ));
  } else {
    task.state = State::Fail;
    task.note = if timed_out {
      format!(
        "stopped at its time limit of {}",
        process::duration(task.limit)
      )
    } else {
      format!("exit code {}", code)
    };
    log.error(&format!("Failed [{}]: {}", task.id, task.note));
  }
}

fn seconds(value: f64) -> String {
  let rounded = (value * 100.0).round() / 100.0;
  format!("{}s", rounded)
}

/// Marks each task as pending, excluded (not shown) or skipped (shown, with how to run it).
/// Returns the groups that run.
fn select(tasks: &mut [Task], options: &Options) -> Vec<String> {
  let mut groups: Vec<String> = DEFAULT_GROUPS.iter().map(|g| g.to_string()).collect();
  let mut explicit = false;
  if options.full {
    groups = GROUPS.iter().map(|(g, _)| g.to_string()).collect();
  } else if !options.groups.is_empty() {
    groups = options.groups.clone();
    explicit = true;
  }
  for task in tasks.iter_mut() {
    if task.target_only && task.id != options.target {
      task.state = State::Excluded;
    } else if !options.target.is_empty() {
      task.state = if task.id == options.target {
        State::Pending
      } else {
        State::Excluded
      };
    } else if groups.iter().any(|g| g == task.group) {
      task.state = State::Pending;
    } else if explicit || DEFAULT_GROUPS.contains(&task.group) {
      task.state = State::Excluded;
    } else {
      task.state = State::Skip;
      task.note = format!("use --group {} or --full", task.group);
    }
  }
  groups
}

fn tool_available(
  root: &Path,
  tool: &'static str,
  cache: &mut HashMap<&'static str, bool>,
  log: &Logger,
) -> bool {
  if let Some(known) = cache.get(tool) {
    return *known;
  }
  let works = |args: &[&str]| {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let env = [("RUSTUP_AUTO_INSTALL".to_string(), "0".to_string())];
    process::capture(root, "cargo", &args, &env, process::QUICK).0 == 0
  };
  let available = match tool {
    "rustfmt" => works(&["fmt", "--version"]),
    "clippy" => works(&["clippy", "--version"]),
    "cargo-fuzz" => works(&["+nightly", "fuzz", "--version"]),
    other => process::on_path(other),
  };
  cache.insert(tool, available);
  log.info(&format!("Tool check: {} available={}", tool, available));
  available
}

#[cfg(test)]
mod tests {
  use super::*;

  fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
  }

  /// The options `cargo xtask test ARGS` parses to.
  fn parse_args(words: &[String]) -> Result<Options, String> {
    let mut all = vec!["test".to_string()];
    all.extend(words.iter().cloned());
    match crate::cli::parse(&all) {
      crate::cli::Parsed::Run(_, m) => Ok(options_of(&m)),
      crate::cli::Parsed::Help(_) => Err("help".to_string()),
      crate::cli::Parsed::Error(e) => Err(e),
      crate::cli::Parsed::Json => Err("json".to_string()),
    }
  }

  fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
  }

  #[test]
  fn test_every_test_file_is_registered() {
    let (_, warnings) = tasks::build(root(), false, DEFAULT_FUZZ_TIME);
    assert!(warnings.is_empty(), "{:?}", warnings);
  }

  #[test]
  fn test_help_is_identical_apart_from_color() {
    let colored = render_help(root(), true);
    let mut stripped = String::new();
    let mut rest = colored.as_str();
    while let Some(idx) = rest.find('\x1b') {
      stripped.push_str(&rest[..idx]);
      rest = &rest[idx..];
      rest = &rest[rest.find('m').unwrap() + 1..];
    }
    stripped.push_str(rest);
    assert_eq!(stripped, render_help(root(), false));
  }

  #[test]
  fn test_help_lists_everything() {
    assert_eq!(check_help(root()), Vec::<String>::new());
  }

  #[test]
  fn test_parse_args() {
    let options = parse_args(&args(&[
      "--group",
      "heavy",
      "--group=fuzz",
      "--fuzz-time",
      "60",
      "--bless",
    ]))
    .unwrap();
    assert_eq!(options.groups, vec!["heavy", "fuzz"]);
    assert_eq!(options.fuzz_time, 60);
    assert!(options.bless && !options.full && !options.list);
    assert_eq!(
      parse_args(&args(&["--target=lexdiff"])).unwrap().target,
      "lexdiff"
    );
    assert!(
      parse_args(&args(&["--group", "nope"]))
        .unwrap_err()
        .contains("nope")
    );
    assert!(parse_args(&args(&["--fuzz-time", "x"])).is_err());
    assert!(parse_args(&args(&["--target"])).is_err());
    assert!(
      parse_args(&args(&["--frobnicate"]))
        .unwrap_err()
        .contains("--frobnicate")
    );
    assert_eq!(parse_args(&args(&["-h"])).unwrap_err(), "help");
  }

  #[test]
  fn test_selection_by_group_full_and_target() {
    let (mut tasks, _) = tasks::build(root(), false, DEFAULT_FUZZ_TIME);
    let defaults = parse_args(&[]).unwrap();
    let groups = select(&mut tasks, &defaults);
    assert_eq!(groups, vec!["gates", "tests"]);
    let state = |tasks: &[Task], id: &str| tasks.iter().find(|t| t.id == id).unwrap().state;
    assert_eq!(state(&tasks, "clippy"), State::Pending);
    assert_eq!(state(&tasks, "unicode"), State::Skip);

    assert_eq!(state(&tasks, "fuzz-24h"), State::Excluded);
    let full = parse_args(&args(&["--full"])).unwrap();
    select(&mut tasks, &full);
    assert_eq!(state(&tasks, "fuzz-24h"), State::Excluded);
    assert_eq!(state(&tasks, "corpus"), State::Pending);

    let heavy = parse_args(&args(&["--group", "heavy"])).unwrap();
    select(&mut tasks, &heavy);
    assert_eq!(state(&tasks, "clippy"), State::Excluded);
    assert_eq!(state(&tasks, "unicode"), State::Pending);

    let target = parse_args(&args(&["--target", "lexdiff"])).unwrap();
    select(&mut tasks, &target);
    assert_eq!(
      tasks.iter().filter(|t| t.state == State::Pending).count(),
      1
    );
    let soak = parse_args(&args(&["--target", "fuzz-24h"])).unwrap();
    select(&mut tasks, &soak);
    assert_eq!(state(&tasks, "fuzz-24h"), State::Pending);
  }
}
