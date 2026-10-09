//! `cargo xtask test`: runs mtindo's gates and test suites and reports each task as passed,
//! failed or skipped with a reason.
//!
//! Exit codes: 0 when nothing failed (skips never fail a run), 1 when anything failed, 2
//! for bad arguments. Every run is logged to `logs/test.log` with each task's full output.
//! The last commit that passed a default or full run on a clean tree is kept in
//! `logs/.last_success`; after a failure the runner shows what changed since then.
//!
//! The runner does not depend on mtindo, so it builds and reports even when the library
//! does not compile. Ctrl-C stops the runner and the running task together.
//!
//! For the whole run the machine is kept awake ([`KeepAwake`]), and every task's time asleep
//! is measured. Fuzz tasks fail if the machine slept more than a minute: their time limit
//! is wall-clock time, so sleep would cut the fuzzing short while still passing.

mod tasks;

use std::collections::HashMap;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tasks::{
    Action, DEFAULT_FUZZ_TIME, DEFAULT_GROUPS, ENV_VARS, GROUPS, State, TEST_REGISTRY, Task,
};

use crate::common::awake::{self, KeepAwake, Stamp};
use crate::common::color::{HelpPalette, color_enabled};
use crate::common::log::Logger;
use crate::common::process;

/// Lines of a failed task's output shown in the terminal; the log has all of it.
const FAILURE_TAIL_LINES: usize = 60;

/// The command-line flags: (flags, value name, help). Parsing and -h both use this table.
const FLAGS: &[(&str, &str, &str)] = &[
    ("-h, --help", "", "Show this help and exit"),
    (
        "--group",
        "GROUP",
        "Run only this group; repeatable. Default: gates and tests.",
    ),
    (
        "--full",
        "",
        "Run every group, including heavy (network), fuzz (nightly) and bench.",
    ),
    (
        "--target",
        "ID",
        "Run one task by id, whatever its group (ids are listed below).",
    ),
    (
        "--fuzz-time",
        "SECONDS",
        "Seconds to fuzz each target. Default: 10.",
    ),
    (
        "--bless",
        "",
        "Regenerate the regression fixtures' expected outputs, then list what changed.",
    ),
    (
        "--list",
        "",
        "Show which tasks would run, without running them.",
    ),
];

/// More sleep than this during a fuzz task fails it.
const FUZZ_SLEEP_ALLOWED_SECONDS: u64 = 60;

/// The column where help descriptions start.
const HELP_COLUMN: usize = 28;

const STATE_FILE: &str = "logs/.last_success";

const USAGE: &str = "cargo xtask test [-h] [--group GROUP] [--full] [--target ID] [--fuzz-time SECONDS] [--bless] [--list]";

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
    target: String,
    fuzz_time: u32,
    bless: bool,
    list: bool,
    help: bool,
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
        help.lines()
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
    for (flags, _, _) in FLAGS {
        for flag in flags.split(", ") {
            if !help.contains(flag) {
                missing.push(format!("-h does not list flag {}", flag));
            }
        }
    }
    for (name, _) in ENV_VARS {
        if !help.contains(name) {
            missing.push(format!("-h does not list environment variable {}", name));
        }
    }
    for (id, _, _) in TEST_REGISTRY {
        if *id != "lib" && *id != "xtask" && !root.join("tests").join(format!("{}.rs", id)).exists()
        {
            missing.push(format!(
                "TEST_REGISTRY lists {}, but tests/{}.rs does not exist",
                id, id
            ));
        }
    }
    missing
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let options = match parse_args(args) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("usage: {}", USAGE);
            eprintln!("cargo xtask test: error: {}", e);
            return ExitCode::from(2);
        }
    };
    if options.help {
        print!("{}", render_help(root, color_enabled(false)));
        return ExitCode::SUCCESS;
    }
    let colors = Colors {
        on: color_enabled(false),
    };
    let log = Logger::new(root, "test");
    let (mut tasks, warnings) = tasks::build(root, options.bless, options.fuzz_time);
    for warning in warnings {
        println!("{} {}", colors.paint("93", "warning:"), warning);
        log.warning(&warning);
    }
    let groups = select(&mut tasks, &options);
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

    let commit = current_commit(root);
    let dirty = tree_is_dirty(root);
    let mut revision = "no git".to_string();
    if !commit.is_empty() {
        revision = commit[..7.min(commit.len())].to_string();
        if dirty {
            revision.push_str(" + uncommitted changes");
        }
    }
    log.info(&format!(
        "Starting run: groups={} target={} revision={}",
        groups.join(","),
        options.target,
        revision
    ));
    println!(
        "{}{}",
        colors.paint("1", "mtindo checks"),
        colors.paint("90", &format!("  ({} | {})", groups.join(", "), revision))
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
    let failed = print_summary(&tasks, elapsed, &log, &colors);
    if failed == 0 {
        let default_groups =
            groups.iter().any(|g| g == "gates") && groups.iter().any(|g| g == "tests");
        if options.target.is_empty() && !options.bless && default_groups {
            record_success(root, &commit, dirty, &log);
        }
        return ExitCode::SUCCESS;
    }
    print_changes_since_last_good(root, &commit, dirty, &log, &colors);
    ExitCode::from(1)
}

fn current_commit(root: &Path) -> String {
    git(root, &["rev-parse", "HEAD"])
        .map(|out| out.trim().to_string())
        .unwrap_or_default()
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
    let (code, output) = process::capture(root, "git", &args, &[]);
    (code == 0).then_some(output)
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        groups: Vec::new(),
        full: false,
        target: String::new(),
        fuzz_time: DEFAULT_FUZZ_TIME,
        bless: false,
        list: false,
        help: false,
    };
    let mut idx = 0;
    while idx < args.len() {
        let (flag, inline) = match args[idx].split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (args[idx].as_str(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            if let Some(value) = inline.clone() {
                return Ok(value);
            }
            idx += 1;
            args.get(idx)
                .cloned()
                .ok_or_else(|| format!("argument {}: expected one argument", name))
        };
        match flag {
            "-h" | "--help" => options.help = true,
            "--full" => options.full = true,
            "--bless" => options.bless = true,
            "--list" => options.list = true,
            "--group" => {
                let group = value("--group")?;
                if !GROUPS.iter().any(|(name, _)| *name == group) {
                    let names: Vec<&str> = GROUPS.iter().map(|(n, _)| *n).collect();
                    return Err(format!(
                        "argument --group: invalid choice: '{}' (choose from {})",
                        group,
                        names.join(", ")
                    ));
                }
                options.groups.push(group);
            }
            "--target" => options.target = value("--target")?,
            "--fuzz-time" => {
                let text = value("--fuzz-time")?;
                options.fuzz_time = text
                    .parse()
                    .map_err(|_| format!("argument --fuzz-time: invalid int value: '{}'", text))?;
            }
            other => return Err(format!("unrecognized arguments: {}", other)),
        }
        idx += 1;
    }
    Ok(options)
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

fn print_changes_since_last_good(
    root: &Path,
    commit: &str,
    dirty: bool,
    log: &Logger,
    colors: &Colors,
) {
    let last_good = fs::read_to_string(root.join(STATE_FILE))
        .map(|text| text.trim().to_string())
        .unwrap_or_default();
    if last_good.is_empty() {
        return;
    }
    if last_good == commit && !dirty {
        println!(
            "\nThis commit passed before with a clean tree; the failure likely comes from the environment."
        );
        return;
    }
    let Some(stat) = git(root, &["diff", "--stat", &last_good]) else {
        return;
    };
    let short = &last_good[..7.min(last_good.len())];
    println!(
        "\n{}",
        colors.paint(
            "93",
            &format!(
                "Changes since the last passing commit ({}), including uncommitted work:",
                short
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
    if let Some(full) = git(root, &["diff", &last_good]) {
        log.error(&format!("Full diff against {}:\n{}", short, full));
    }
    println!("{}", colors.paint("90", "Full diff in logs/test.log"));
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
        State::Pass | State::Fail => {
            line.push_str(&colors.paint("90", &format!(" ({})", seconds(task.seconds))))
        }
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
                        "    ... {} earlier lines in logs/test.log",
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

fn record_success(root: &Path, commit: &str, dirty: bool, log: &Logger) {
    if commit.is_empty() {
        return;
    }
    if dirty {
        log.info("All tasks passed, but the working tree has uncommitted changes; known-good commit not updated.");
        return;
    }
    let path = root.join(STATE_FILE);
    let written = fs::create_dir_all(root.join("logs"))
        .and_then(|_| fs::write(&path, format!("{}\n", commit)));
    match written {
        Ok(()) => log.info(&format!("Recorded known-good commit {}", commit)),
        Err(e) => log.warning(&format!("cannot write {}: {}", STATE_FILE, e)),
    }
}

/// The help screen. The text is fixed; only the colors depend on `color`.
fn render_help(root: &Path, color: bool) -> String {
    let p = HelpPalette::new(color, "cargo xtask test");
    let mut out = String::new();
    out.push_str(&format!(
        "{} {}\n",
        p.heading("usage:"),
        p.usage_line(USAGE)
    ));
    out.push_str(
        "\nRun mtindo's gates and test suites; report each as passed, failed or skipped with a reason.\n",
    );
    out.push_str(&format!("\n{}\n", p.heading("options:")));
    for (flags, value, help) in FLAGS {
        let mut label = String::from("  ");
        let mut width = 2;
        for (idx, flag) in flags.split(", ").enumerate() {
            if idx > 0 {
                label.push_str(", ");
                width += 2;
            }
            label.push_str(&if flag.starts_with("--") {
                p.long(flag)
            } else {
                p.short(flag)
            });
            width += flag.len();
        }
        if !value.is_empty() {
            label.push(' ');
            label.push_str(&p.value(value));
            width += 1 + value.len();
        }
        out.push_str(&p.row(&label, width, HELP_COLUMN, help));
    }
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
            "logs/test.log",
            "every run, with the full output of every task",
        ),
        (
            "logs/capabilities.log",
            "what cargo xtask capabilities read, fetched and wrote",
        ),
        (
            "logs/.last_success",
            "last commit that passed a default or full run on a clean tree",
        ),
        (
            "compat/rustfmt-surface.txt",
            "rustfmt's options and flags; update with cargo xtask capabilities refresh",
        ),
        (
            "compat/capabilities.txt",
            "mto's status for each; add new items with cargo xtask capabilities sync",
        ),
    ] {
        out.push_str(&format!("  {:<28}{}\n", path, info));
    }
    out.push_str(&format!("\n{}\n", p.heading("exit codes:")));
    out.push_str(
        "  0 nothing failed (skips never fail a run), 1 something failed, 2 bad arguments\n",
    );
    out
}

fn run_task(
    root: &Path,
    task: &mut Task,
    cache: &mut HashMap<&'static str, bool>,
    log: &Logger,
    colors: &Colors,
) {
    for tool in task.requires.clone() {
        if !tool_available(root, tool, cache, log) {
            task.state = State::Skip;
            task.note = format!("{} is not available", tool);
            return;
        }
    }
    log.info(&format!("Running [{}]: {}", task.id, describe(task)));
    let spinner = Spinner::start(&task.title, colors);
    let stamp = Stamp::now();
    let started = Instant::now();
    let (code, output) = match &task.action {
        Action::Command(program, args) => process::capture(root, program, args, &task.env),
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
    if code == 0 {
        task.state = State::Pass;
        log.info(&format!(
            "Passed [{}] in {}",
            task.id,
            seconds(task.seconds)
        ));
    } else {
        task.state = State::Fail;
        task.note = format!("exit code {}", code);
        log.error(&format!("Failed [{}] with exit code {}", task.id, code));
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
        process::capture(root, "cargo", &args, &env).0 == 0
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

fn tree_is_dirty(root: &Path) -> bool {
    git(root, &["status", "--porcelain"]).is_none_or(|out| !out.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
    }

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn test_help_lists_everything() {
        assert_eq!(check_help(root()), Vec::<String>::new());
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
                .contains("invalid choice: 'nope'")
        );
        assert!(parse_args(&args(&["--fuzz-time", "x"])).is_err());
        assert!(
            parse_args(&args(&["--target"]))
                .unwrap_err()
                .contains("expected one argument")
        );
        assert!(
            parse_args(&args(&["--frobnicate"]))
                .unwrap_err()
                .contains("unrecognized")
        );
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

    #[test]
    fn test_every_test_file_is_registered() {
        let (_, warnings) = tasks::build(root(), false, DEFAULT_FUZZ_TIME);
        assert!(warnings.is_empty(), "{:?}", warnings);
    }
}
