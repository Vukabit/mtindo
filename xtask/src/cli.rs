//! Every xtask command, defined once with clap: the parser, `-h` for each command, the
//! help gate and the TUI's argument forms all read this tree.
//!
//! clap parses; the help text is drawn here, from the same definitions, in mto's help
//! palette (headings, short and long flags, values each their own color) and at a fixed
//! width, so it is the same on every terminal.

use clap::builder::PossibleValue;
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};

use crate::common::color::HelpPalette;
use crate::gates;
use crate::test::tasks::GROUPS;

/// The column where help descriptions start.
const HELP_COLUMN: usize = 30;

/// What [`parse`] found.
pub enum Parsed {
  /// Run this command with these matches.
  Run(String, ArgMatches),
  /// Print this help.
  Help(Vec<String>),
  /// Bad arguments: the message to print.
  Error(String),
  /// The whole tree as JSON.
  Json,
}

/// The command at `path` in the tree.
pub fn find(path: &[String]) -> Option<Command> {
  let mut command = tree();
  // Settles what clap infers (how many values each argument takes) before it is read.
  command.build();
  for name in path {
    command = command.find_subcommand(name)?.clone();
  }
  Some(command)
}

/// The help for the command at `path` (empty: `cargo xtask` itself), fixed text, colored
/// when `color`.
pub fn help(path: &[String], color: bool) -> Option<String> {
  let command = find(path)?;
  let prefix = std::iter::once("cargo xtask".to_string())
    .chain(path.iter().cloned())
    .collect::<Vec<_>>()
    .join(" ");
  let p = HelpPalette::new(color, "cargo xtask");
  let mut out = format!(
    "{} {}\n",
    p.heading("usage:"),
    p.usage_line(&usage(&command, &prefix))
  );
  if let Some(about) = command.get_about() {
    out.push_str(&format!("\n{}\n", about));
  }
  let positionals: Vec<&Arg> = command.get_positionals().collect();
  if !positionals.is_empty() {
    out.push_str(&format!("\n{}\n", p.heading("arguments:")));
    for arg in positionals {
      let name = value_word(arg);
      let label = format!("  {}", p.value(&name));
      out.push_str(&p.row(&label, 2 + name.len(), HELP_COLUMN, &arg_help(arg)));
    }
  }
  let options: Vec<&Arg> = command
    .get_arguments()
    .filter(|a| !a.is_positional())
    .collect();
  if !options.is_empty() {
    out.push_str(&format!("\n{}\n", p.heading("options:")));
    for arg in options {
      let mut label = String::from("  ");
      let mut width = 2;
      if let Some(short) = arg.get_short() {
        label.push_str(&p.short(&format!("-{}", short)));
        width += 2;
        if arg.get_long().is_some() {
          label.push_str(", ");
          width += 2;
        }
      }
      if let Some(long) = arg.get_long() {
        label.push_str(&p.long(&format!("--{}", long)));
        width += 2 + long.len();
      }
      if arg.get_action().takes_values() {
        let value = arg
          .get_value_names()
          .and_then(|v| v.first())
          .map(|v| v.to_string())
          .unwrap_or_default();
        label.push(' ');
        label.push_str(&p.value(&value));
        width += 1 + value.len();
      }
      out.push_str(&p.row(&label, width, HELP_COLUMN, &arg_help(arg)));
    }
  }
  let subs: Vec<&Command> = command.get_subcommands().collect();
  if !subs.is_empty() {
    out.push_str(&format!("\n{}\n", p.heading("commands:")));
    for sub in subs {
      let words = usage(sub, "");
      let words = words.trim();
      let label = if words.is_empty() {
        format!("  {}", p.long(sub.get_name()))
      } else {
        format!("  {} {}", p.long(sub.get_name()), p.value(words))
      };
      let width = 2 + sub.get_name().len() + if words.is_empty() { 0 } else { 1 + words.len() };
      let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
      out.push_str(&p.row(&label, width, HELP_COLUMN, &about));
    }
  }
  Some(out)
}

/// The whole tree, `cargo xtask` at the root.
/// The commands the front doors run but the command line no longer offers (`compare`
/// runs `diff`, `pairs`, `lexdiff` and `parsecheck`; `speed` runs `perf`, `race`, `io`,
/// `corpus bench` and `doctor`; `sync` runs `settings`, `unicode` and `capabilities
/// refresh|sync`), by name, as their entry points read their arguments; `None` for a name
/// that is not one of them.
pub fn internal(name: &str) -> Option<Command> {
  let command = match name {
    "diff" => diff(),
    "pairs" => pairs(),
    "lexdiff" => lexdiff(),
    "parsecheck" => parsecheck(),
    "perf" => perf(),
    "race" => race(),
    "io" => io(),
    "doctor" => doctor(),
    "settings" => settings(),
    "unicode" => unicode(),
    "corpus" => corpus_all(),
    "capabilities" => capabilities_all(),
    _ => return None,
  };
  Some(command.no_binary_name(true))
}

/// The whole tree as JSON: each command's name, about and arguments, and its commands
/// below.
pub fn json() -> String {
  let mut command = tree();
  command.build();
  let mut out = String::new();
  json_command(&command, &mut out);
  out.push('\n');
  out
}

/// The command line after `cargo xtask`, for logs.
pub fn line() -> String {
  std::env::args().skip(1).collect::<Vec<_>>().join(" ")
}

/// A flag's value; false when the command has no such flag.
pub fn on(m: &ArgMatches, id: &str) -> bool {
  m.try_get_one::<bool>(id).ok().flatten() == Some(&true)
}

/// Parses `cargo xtask`'s arguments.
pub fn parse(args: &[String]) -> Parsed {
  let mut all = vec!["cargo xtask".to_string()];
  all.extend(args.iter().cloned());
  let matches = match tree().try_get_matches_from(all) {
    Ok(matches) => matches,
    // `-h` asks for help even where the arguments are incomplete (`fuzz run -h`).
    Err(_) if args.iter().any(|a| a == "-h" || a == "--help") => {
      return Parsed::Help(command_path(args));
    }
    Err(e) => return Parsed::Error(e.to_string()),
  };
  // The deepest command asked for help, if any did.
  let mut path = Vec::new();
  let mut at = &matches;
  let mut help = at.get_flag("help");
  while let Some((name, sub)) = at.subcommand() {
    path.push(name.to_string());
    at = sub;
    help |= sub.try_get_one::<bool>("help").ok().flatten() == Some(&true);
  }
  if path.first().map(String::as_str) == Some("help") && on(at, "json") {
    return Parsed::Json;
  }
  if path.first().map(String::as_str) == Some("help") {
    let target: Vec<String> = at
      .get_one::<String>("command")
      .map(|c| vec![c.clone()])
      .unwrap_or_default();
    return Parsed::Help(target);
  }
  if help || path.is_empty() {
    return Parsed::Help(path);
  }
  let (name, sub) = matches.subcommand().expect("a command");
  Parsed::Run(name.to_string(), sub.clone())
}

/// `docs/generated/commands.md`: every command, its usage and its arguments, as `help`
/// shows them, within the 80 columns public Markdown keeps to.
pub fn reference() -> String {
  let mut out = String::from(
    "# xtask commands\n\n\
     Generated by `cargo xtask docs` from xtask's command tree; do not edit.\n\
     `cargo xtask help COMMAND` shows the same. CONTRIBUTING.md and\n\
     MAINTAINING.md say when each is used.\n",
  );
  walk(&tree(), "cargo xtask", &mut out);
  out
}

#[cfg(test)]
/// Whether `args` (a command's name, then its arguments) is a command `run_internal` runs:
/// an internal one or one of the tree, with arguments it takes.
pub fn takes(args: &[String]) -> bool {
  let Some((name, rest)) = args.split_first() else {
    return false;
  };
  internal(name)
    .or_else(|| find(std::slice::from_ref(name)).map(|c| c.no_binary_name(true)))
    .is_some_and(|c| c.try_get_matches_from(rest).is_ok())
}

/// A string argument's value, if given.
pub fn text(m: &ArgMatches, id: &str) -> Option<String> {
  m.try_get_one::<String>(id).ok().flatten().cloned()
}

/// The values of a repeated or many-valued string argument.
pub fn texts(m: &ArgMatches, id: &str) -> Vec<String> {
  m.try_get_many::<String>(id)
    .ok()
    .flatten()
    .map(|v| v.cloned().collect())
    .unwrap_or_default()
}

pub fn tree() -> Command {
  let commands = [
    test(),
    bootstrap(),
    capabilities(),
    changelog(),
    Command::new("check").about(
      "The quick look at what changed: formatting, the rules, the command reference, the \
       unit tests of each package changed (flow.toml stage `check`)",
    ),
    Command::new("checks")
      .about(
        "Every check by its one id, what it checks, and the test group and flow stages it \
         runs in",
      )
      .arg(flag("json", "As JSON (schema mtindo.xtask.checks/1)")),
    compare(),
    corpus(),
    docs(),
    fix_up(),
    flow(),
    fuzz(),
    gaps(),
    gates_command(),
    incidents(),
    memst(),
    Command::new("mcp").about(
      "The tooling as an MCP server over stdio: next, gaps, checks, store, probe and why, \
       for an agent to call",
    ),
    metrics(),
    next(),
    probe(),
    Command::new("prove").about(
      "What a release needs, reusing what is proven for this tree: the rules, the whole \
       suite, the compatibility record, the dependency audit (flow.toml stage `prove`)",
    ),
    runs(),
    Command::new("site").about(
      "The public docs as one site, built by mdBook into target/site/: docs/'s pages, its \
       README's contents in order, links out of docs/ pointed at GitHub",
    ),
    speed(),
    status(),
    store(),
    Command::new("sync")
      .about(
        "Every file written from upstream brought up to date in order: rustfmt's releases \
         (capabilities, the settings table, their docs), Unicode, the changelog, the docs; \
         --check only compares, and says whether a newer rustfmt is out",
      )
      .arg(
        Arg::new("part")
          .value_name("PART")
          .value_parser(["rustfmt", "unicode", "changelog", "docs"])
          .help("One part (default: all)"),
      )
      .arg(flag(
        "check",
        "Only compare; fails when something is out of date or a newer rustfmt is out",
      )),
    toml_test(),
    upstream(),
    Command::new("help")
      .about("Show help for xtask or for one command")
      .arg(
        Arg::new("command")
          .value_name("COMMAND")
          .help("The command"),
      )
      .arg(flag("json", "Print every command and argument as JSON")),
  ];
  let root = Command::new("cargo xtask")
    .bin_name("cargo xtask")
    .about("Development tasks for mtindo. Never published; not part of the mto binary.")
    .subcommands(commands)
    .arg(
      Arg::new("detach")
        .long("detach")
        .global(true)
        .action(ArgAction::SetTrue)
        .help("Run the command in the background; prints its run id (cargo xtask runs)"),
    );
  with_help(root)
}

/// A one-line usage of `command`, called as `prefix`: its arguments and options, or its
/// subcommands.
pub fn usage(command: &Command, prefix: &str) -> String {
  if prefix == "cargo xtask" {
    return "cargo xtask [--detach] <COMMAND> [ARGS]".to_string();
  }
  let mut words = vec![prefix.to_string()];
  for arg in command.get_arguments() {
    if arg.get_id() == "help" || arg.get_id() == "detach" {
      continue;
    }
    words.push(arg_usage(arg));
  }
  let subs: Vec<&str> = command
    .get_subcommands()
    .map(|c| c.get_name())
    .filter(|n| *n != "help" || prefix == "cargo xtask")
    .collect();
  if !subs.is_empty() {
    let joined = subs.join("|");
    words.push(if command.is_subcommand_required_set() {
      joined
    } else {
      format!("[{}]", joined)
    });
  }
  words.join(" ")
}

/// An argument's help with its choices and default.
fn arg_help(arg: &Arg) -> String {
  let mut text = arg.get_help().map(|h| h.to_string()).unwrap_or_default();
  let possible = arg.get_possible_values();
  if !possible.is_empty() && possible.len() > 4 || (!possible.is_empty() && arg.is_positional()) {
    let names: Vec<&str> = possible.iter().map(|p| p.get_name()).collect();
    text.push_str(&format!(" (one of: {})", names.join(", ")));
  }
  let defaults: Vec<String> = arg
    .get_default_values()
    .iter()
    .map(|v| v.to_string_lossy().into_owned())
    .collect();
  // A flag's default (off) goes without saying.
  if !defaults.is_empty() && arg.get_action().takes_values() && !text.contains("Default:") {
    text.push_str(&format!(" [default: {}]", defaults.join(",")));
  }
  text
}

fn arg_usage(arg: &Arg) -> String {
  let takes = arg.get_action().takes_values();
  let text = match (arg.get_short(), arg.get_long()) {
    (_, Some(long)) if takes => format!("--{} {}", long, value_word(arg)),
    (_, Some(long)) => format!("--{}", long),
    (Some(short), None) => format!("-{}", short),
    (None, None) => {
      let many = matches!(arg.get_action(), ArgAction::Append)
        || arg.get_num_args().is_some_and(|n| n.max_values() > 1);
      format!("{}{}", value_word(arg), if many { "..." } else { "" })
    }
  };
  if arg.is_required_set() {
    text
  } else {
    format!("[{}]", text)
  }
}

fn bootstrap() -> Command {
  Command::new("bootstrap")
    .about("Install or update the tools flow.toml pins (toolchains, cargo tools), and turn the git hooks on; shows the plan unless --go")
    .arg(flag("go", "Install and update, rather than only show what would be done"))
    .arg(
      Arg::new("for")
        .long("for")
        .value_name("NAME")
        .help("Only the tools a stage or command needs (flow.toml needed_by)"),
    )
}

fn capabilities() -> Command {
  Command::new("capabilities")
    .about(
      "Track rustfmt's options and flags against mto (records/); `cargo xtask sync rustfmt` \
       re-reads them",
    )
    .subcommand_required(true)
    .subcommand(Command::new("check").about("Every rustfmt option, value and flag has a status"))
    .subcommand(
      Command::new("report")
        .about("How much of rustfmt mto supports, and how rustfmt's own tests fare")
        .arg(flag("markdown", "As a Markdown table")),
    )
}

/// [`capabilities`] with the steps `sync rustfmt` runs: `refresh` and `sync`.
fn capabilities_all() -> Command {
  capabilities()
    .subcommand(Command::new("refresh").about(
      "Re-read rustfmt's options and flags (local binaries; latest stable and nightly when online)",
    ))
    .subcommand(
      Command::new("sync")
        .about("Add new items to records/policy/capabilities.txt as missing; drop stale ones"),
    )
}

fn changelog() -> Command {
  Command::new("changelog")
    .about("Render CHANGELOG.md's newest section from the commits after the last release tag with git-cliff (its settings written from flow.toml's [kinds]), wrapped to 80 columns")
    .arg(
      Arg::new("tag")
        .long("tag")
        .value_name("TAG")
        .help("Name the section for this release (v0.2.0), dated today, in place of Unreleased"),
    )
    .arg(flag("check", "Only compare; fails when the section is out of date"))
    .arg(flag(
      "preview",
      "Print the section a release made now would write, named for the next version (or --tag); write nothing",
    ))
}

/// The commands named at the start of `args`: `["fuzz", "run"]` for `fuzz run -h`.
fn command_path(args: &[String]) -> Vec<String> {
  let mut path = Vec::new();
  let mut command = tree();
  for arg in args {
    match command.find_subcommand(arg) {
      Some(sub) => {
        path.push(arg.clone());
        command = sub.clone();
      }
      None if arg.starts_with('-') => continue,
      None => break,
    }
  }
  path
}

fn compare() -> Command {
  let sources: Vec<PossibleValue> = crate::compare::SOURCES
    .iter()
    .map(|(name, _, help)| PossibleValue::new(*name).help(*help))
    .collect();
  Command::new("compare")
    .about(
      "mto held against an answer key, by source: the recorded sets (the default, as \
       `diff`), scrambled pairs (`pairs`), rustc's lexer (`lexdiff`), the parser alone \
       (`parsecheck`); ARGS after `--` go to that command",
    )
    .arg(
      Arg::new("sources")
        .value_name("SOURCE")
        .num_args(0..)
        .value_parser(sources),
    )
    .arg(
      Arg::new("args")
        .value_name("ARGS")
        .num_args(0..)
        .last(true)
        .help("Passed on: --update, --markdown; --seed, --files, --crates; --edition, PATHS"),
    )
}

fn corpus() -> Command {
  let with_rust = || flag("with-rust", "Also the opt-in repositories (rust-lang/rust)");
  Command::new("corpus")
    .about("The pinned real-world corpus (records/pins/corpus.txt)")
    .subcommand_required(true)
    .subcommand(Command::new("update").about("Move every pin to its repository's current HEAD"))
    .subcommand(
      Command::new("fetch")
        .about("Check out the pinned commits")
        .arg(with_rust()),
    )
    .subcommand(
      Command::new("check")
        .about("Every file round-trips, parses and lexes like rustc; the verifier and speed")
        .arg(with_rust())
        .arg(flag(
          "baseline",
          "Record parsing speed in records/results/baseline.txt",
        )),
    )
}

/// [`corpus`] with `bench`, which `speed throughput` runs.
fn corpus_all() -> Command {
  corpus().subcommand(
    Command::new("bench")
      .about("Speed against rustfmt stable and nightly, as a table; fails below 2x")
      .arg(flag("markdown", "As a Markdown table")),
  )
}

fn diff() -> Command {
  Command::new("diff")
    .about("How much of the Rust style mto reproduces, against records/results/compatibility.txt")
    .arg(flag(
      "update",
      "Record the numbers in records/results/compatibility.txt",
    ))
    .arg(flag("markdown", "As Markdown tables"))
    .arg(
      option(
        "sources",
        "SET,...",
        "Score only these recorded sets (tests, tests-with-settings, corpus, \
         corpus-unformatted): reported, not recorded, logged or filed",
      )
      .value_delimiter(',')
      .value_parser(crate::diff::SETS.to_vec()),
    )
}

fn docs() -> Command {
  Command::new("docs")
    .about("Regenerate everything under docs/generated/: the settings reference and mto.toml's JSON Schema from the settings table, the manual from mto --man, and the metrics papers from the recorded data (records/, its history, .dev/ when present); runs no suite")
    .arg(flag("check", "Only compare; fails when a page is out of date"))
    .arg(flag(
      "tracked",
      "From the tracked records only: each page's machine-local regions are left as written",
    ))
}

fn doctor() -> Command {
  Command::new("doctor")
    .about("Measure mto doctor, the build guard")
    .subcommand_required(true)
    .subcommand(
      Command::new("bench")
        .about("What the guard costs and how few checks a narrowing takes: in mto on the corpus, and on real crates with cargo check; kept as doctor.* metrics")
        .arg(
          option("trials", "N", "Narrowings per size, each with its own culprits")
            .value_parser(value_parser!(u32))
            .default_value("10"),
        ),
    )
}

fn fix_up() -> Command {
  Command::new("fix-up").about(
    "Before a commit: mto's layout and formatting on the Rust files changed since HEAD, \
     `cargo fmt`, then the generated docs from the tracked records (pages that only local \
     data would move are left alone)",
  )
}

fn flag(name: &'static str, help: &'static str) -> Arg {
  Arg::new(name)
    .long(name)
    .action(ArgAction::SetTrue)
    .help(help)
}

fn flow() -> Command {
  Command::new("flow")
    .about("How changes reach the history and releases, from flow.toml: the git hooks, and the checks CI runs")
    .subcommand_required(true)
    .subcommand(Command::new("setup").about("Turn the hooks in .githooks/ on for this clone"))
    .subcommand(
      Command::new("hook")
        .about("What a git hook runs (.githooks/ calls it)")
        .arg(Arg::new("name").value_name("NAME").required(true).help("The hook: prepare-commit-msg, commit-msg, pre-commit, pre-push"))
        .arg(
          Arg::new("args")
            .value_name("ARGS")
            .num_args(0..)
            .trailing_var_arg(true)
            .allow_hyphen_values(true)
            .help("What git passed the hook"),
        ),
    )
    .subcommand(
      Command::new("check")
        .about("Run a stage's checks (flow.toml [stages]) on the whole tree, as CI does")
        .arg(Arg::new("stage").value_name("STAGE").required(true).help("The stage: ci, pre-commit, pre-push"))
        .arg(Arg::new("range").long("range").value_name("RANGE").help("Commits whose messages are checked: A..B")),
    )
    .subcommand(
      Command::new("release")
        .about("Cut a release after flow.toml's release checks: version, changelog section, commit and tag, marked for the hold; prints the push (a plan unless --go)")
        .arg(Arg::new("version").value_name("VERSION").required(true).help("MAJOR.MINOR.PATCH"))
        .arg(flag("go", "Make the release, rather than only show what would be done")),
    )
    .subcommand(
      Command::new("yank")
        .about("Print how to take a release back; runs nothing")
        .arg(Arg::new("version").value_name("VERSION").required(true).help("MAJOR.MINOR.PATCH")),
    )
    .subcommand(
      Command::new("package")
        .about("A release target's archive in dist/, from its release build (the release workflow's step)")
        .arg(Arg::new("target").value_name("TARGET").required(true).help("A target flow.toml release.targets lists")),
    )
    .subcommand(
      Command::new("labels")
        .about("The repository's labels on GitHub made what .github/labels.txt says: missing ones created, others corrected, none deleted (a plan unless --go)")
        .arg(flag("go", "Change GitHub's labels, rather than only show what would change")),
    )
    .subcommand(
      Command::new("try-archive")
        .about("A release target's archive in dist/ unpacked: both binaries and the files beside them there, and where this machine runs the target, its mto naming the version (a release build, from a tag) (the release workflow's step)")
        .arg(Arg::new("target").value_name("TARGET").required(true).help("A target flow.toml release.targets lists")),
    )
    .subcommand(
      Command::new("prepare")
        .about("The publish stage's checks, then dist/mto-VERSION.spdx.json, dist/SHA256SUMS and dist/notes.md, and whether the GitHub release is left a draft (the release workflow's step)")
        .arg(Arg::new("tag").value_name("TAG").required_unless_present("dry-run").help("The release tag, vX.Y.Z"))
        .arg(flag("dry-run", "No tag: only that the crate packages, for Cargo.toml's version (a dry-run/ branch)")),
    )
    .subcommand(
      Command::new("messages")
        .about("Check every commit message in a range, as pushing and CI do")
        .arg(Arg::new("range").value_name("RANGE").required(true).help("Commits A..B")),
    )
}

fn fuzz() -> Command {
  Command::new("fuzz")
    .about("Fuzz a target; what it finds is filed as incidents (cargo xtask incidents)")
    .subcommand_required(true)
    .subcommand(
      Command::new("run")
        .about("Fuzz one target, then file what it found")
        .arg(
          Arg::new("target")
            .value_name("TARGET")
            .required(true)
            .help("The fuzz target, from fuzz/fuzz_targets/"),
        )
        .arg(
          option("time", "SECONDS", "Seconds to fuzz")
            .value_parser(value_parser!(u32))
            .default_value("60"),
        )
        .arg(
          option(
            "sanitizer",
            "SANITIZER",
            "address, or none to tell the sanitizer's memory from the target's",
          )
          .value_parser(["address", "none"])
          .default_value("address"),
        ),
    )
    .subcommand(
      Command::new("soak")
        .about("Fuzz every target side by side for 24 hours, kept awake and drawn live on a terminal, then file what they found")
        .arg(
          option("time", "SECONDS", "Seconds to fuzz each target")
            .value_parser(value_parser!(u32))
            .default_value("86400"),
        )
        .arg(
          option("targets", "A,B", "Only these targets, from fuzz/fuzz_targets/ (default: all)")
            .value_delimiter(','),
        )
        .arg(flag(
          "restart",
          "Stop a target at each find and start it again (at most 10 times) instead of fuzzing on past its finds",
        )),
    )
    .subcommand(Command::new("collect").about("File whatever is waiting as incidents"))
    .subcommand(
      Command::new("diff")
        .about("Differential fuzzing against the standard formatter: scrambled corpus files and generated source, each way they disagree shrunk and filed once as a mismatch incident")
        .arg(
          option("time", "SECONDS", "Seconds to fuzz")
            .value_parser(value_parser!(u32))
            .default_value("120"),
        )
        .arg(
          option("seed", "N", "Start from this seed, to repeat a run (default: a new one)")
            .value_parser(value_parser!(u64)),
        )
        .arg(option("replay", "FILE", "Show how mto and the standard formatter part on one input"))
        .arg(option("edition", "YEAR", "With --replay: the input's edition (default: 2021)")),
    )
    .subcommand(
      Command::new("coverage")
        .about("What the fuzz corpora reach: lines and functions per module of src/, kept as coverage.fuzz.* metrics, with an HTML report in fuzz/coverage/html/")
        .arg(
          Arg::new("targets")
            .value_name("TARGET")
            .num_args(0..)
            .help("Fuzz targets (default: all)"),
        ),
    )
}

fn gaps() -> Command {
  Command::new("gaps")
    .about(
      "The standard formatter's test pairs mto does not match yet, by setting and by where \
       they first differ, from the last diff",
    )
    .arg(option(
      "setting",
      "NAME",
      "Only the pairs that set this setting",
    ))
    .arg(flag("all", "Every setting and kind, not the top ones"))
    .arg(flag("json", "As JSON (schema mtindo.xtask.gaps/1)"))
}

fn gates_command() -> Command {
  let mut names = vec![PossibleValue::new("all")];
  names.extend(
    gates::GATES
      .iter()
      .map(|(name, help, _)| PossibleValue::new(*name).help(*help)),
  );
  Command::new("gates").about("Check repository rules").arg(
    Arg::new("gate")
      .value_name("GATE")
      .value_parser(names)
      .default_value("all")
      .help("One gate, or all"),
  )
}

fn incidents() -> Command {
  let id = || {
    Arg::new("id")
      .value_name("ID")
      .required(true)
      .help("An incident's short id (its signature's start)")
  };
  let kinds: Vec<PossibleValue> = crate::incidents::KINDS
    .iter()
    .map(|(k, help, _)| PossibleValue::new(*k).help(*help))
    .collect();
  let statuses: Vec<PossibleValue> = crate::incidents::STATUSES
    .iter()
    .map(|s| PossibleValue::new(*s))
    .collect();
  Command::new("incidents")
    .about("Every failure kept with its story, filed once: the board, one incident, replay, recheck, minimize, promote")
    .subcommand(
      Command::new("list")
        .about("The incidents, open first (default)")
        .arg(
          option("kind", "KIND", "Only this kind; repeatable")
            .action(ArgAction::Append)
            .value_parser(kinds),
        )
        .arg(
          option("status", "STATUS", "Only this status; repeatable")
            .action(ArgAction::Append)
            .value_parser(statuses.clone()),
        ),
    )
    .subcommand(Command::new("show").about("One incident's page").arg(id()))
    .subcommand(
      Command::new("mark")
        .about("Set an incident's status")
        .arg(id())
        .arg(
          Arg::new("status")
            .value_name("STATUS")
            .required(true)
            .value_parser(statuses)
            .help("The new status"),
        )
        .arg(option("note", "TEXT", "Why, kept in its history")),
    )
    .subcommand(Command::new("replay").about("Run a fuzz incident's input again").arg(id()))
    .subcommand(
      Command::new("recheck")
        .about("Replay every fuzz incident at this tree; a fixed one that fails reopens; exits 1 while any fails or cannot replay")
        .arg(Arg::new("id").value_name("ID").help("Only this incident")),
    )
    .subcommand(
      Command::new("minimize")
        .about("Shrink a fuzz incident's input, keeping it failing")
        .arg(id()),
    )
    .subcommand(
      Command::new("promote")
        .about("Keep a fixed fuzz incident's input as a regression test (tests/fuzz_regressions/)")
        .arg(id()),
    )
    .subcommand(Command::new("board").about("Write .dev/incidents/INCIDENTS.md and every incident's page"))
}

fn io() -> Command {
  Command::new("io")
    .about("Measure what writing files costs")
    .subcommand_required(true)
    .subcommand(
      Command::new("bench")
        .about("Write generated files with each way of syncing, history on and off; kept as io.* metrics")
        .arg(
          option("files", "N", "Files in the generated project")
            .value_parser(value_parser!(u32))
            .default_value("600"),
        ),
    )
}

fn json_command(command: &Command, out: &mut String) {
  let opt = |v: Option<String>| v.map_or("null".to_string(), |s| json_string(&s));
  out.push_str(&format!(
    "{{\"name\": {}, \"about\": {}, \"subcommand_required\": {}, \"args\": [",
    json_string(command.get_name()),
    opt(command.get_about().map(|a| a.to_string())),
    command.is_subcommand_required_set()
  ));
  let args: Vec<&Arg> = command
    .get_arguments()
    .filter(|a| a.get_id() != "help" && a.get_id() != "detach")
    .collect();
  for (i, arg) in args.iter().enumerate() {
    if i > 0 {
      out.push_str(", ");
    }
    let choices: Vec<String> = arg
      .get_possible_values()
      .iter()
      .map(|p| json_string(p.get_name()))
      .collect();
    let default = arg
      .get_default_values()
      .first()
      .map(|v| v.to_string_lossy().into_owned())
      .filter(|_| arg.get_action().takes_values());
    let multiple = matches!(arg.get_action(), ArgAction::Append)
      || arg.get_num_args().is_some_and(|n| n.max_values() > 1);
    out.push_str(&format!(
      "{{\"id\": {}, \"long\": {}, \"short\": {}, \"value\": {}, \"help\": {}, \"required\": {}, \"multiple\": {}, \"takes_value\": {}, \"choices\": [{}], \"default\": {}}}",
      json_string(arg.get_id().as_str()),
      opt(arg.get_long().map(str::to_string)),
      opt(arg.get_short().map(|c| c.to_string())),
      opt(arg.get_value_names().and_then(|v| v.first()).map(|v| v.to_string())),
      opt(arg.get_help().map(|h| h.to_string())),
      arg.is_required_set(),
      multiple,
      arg.get_action().takes_values(),
      choices.join(", "),
      opt(default)
    ));
  }
  out.push_str("], \"commands\": [");
  let subs: Vec<&Command> = command
    .get_subcommands()
    .filter(|c| c.get_name() != "help")
    .collect();
  for (i, sub) in subs.iter().enumerate() {
    if i > 0 {
      out.push_str(", ");
    }
    json_command(sub, out);
  }
  out.push_str("]}");
}

fn json_string(text: &str) -> String {
  crate::common::json::string(text)
}

fn lexdiff() -> Command {
  Command::new("lexdiff")
    .about("Compare mto's lexer with rustc's (default: every .rs file of this repository)")
    .arg(option("edition", "YEAR", "The edition to lex with").default_value("2021"))
    .arg(paths())
}

fn memst() -> Command {
  let report = |command: Command| {
    command
      .arg(
        option("view", "VIEW,...", "Which views: summary, functions, lines, probes, callers, modules, hot, tree, sites, or all")
          .value_delimiter(','),
      )
      .arg(
        option("by", "MEASURE", "What functions and the tree rank by")
          .value_parser(["time", "bytes", "allocs"]),
      )
      .arg(option("top", "N", "Rows per table (default 20)").value_parser(value_parser!(u32)))
      .arg(
        option("function", "PATH,...", "Functions (a path or its end: roles, file::roles) to time exactly with probes, and keep the views to")
          .value_delimiter(','),
      )
      .arg(
        option("file", "FILE,...", "Source files whose every function is probed, and the views keep to")
          .value_delimiter(','),
      )
  };
  report(
    Command::new("memst")
      .about("Where a Rust program's time and memory go, per function and line: wall and CPU time, peak memory, sampled CPU time, every allocation, exact timers for the functions asked about; mto on the corpus's serde by default")
      .long_about(
        "Builds a shadow of the workspace (its sources untouched) with memst's recorder in it, \
         runs the binary: plain runs (wall time, CPU time, peak memory), a sampling run (CPU \
         time by stack), a memory run (every allocation) and, with --function or --file, a \
         probe run (exact calls and times); then reports. Runs are kept in \
         .dev/runs/<UTC>-memst/.\n\nWithout PROJECT it profiles mto on the corpus's serde \
         (or --input PATH...), layout and formatting on, and keeps the totals as \
         memst.<input>.* series for the ratchet.",
      )
      .args_conflicts_with_subcommands(true)
      .arg(
        Arg::new("project")
          .value_name("PROJECT")
          .help("A Cargo workspace whose binary to profile, run with ARGS (default: mto, on --input)"),
      )
      .arg(
        option("input", "PATH", "Without PROJECT: what mto runs on (default: the corpus's serde)")
          .num_args(1..),
      )
      .arg(option("bin", "NAME", "The binary, when the workspace has several"))
      .arg(option("package", "NAME", "The package holding the binary"))
      .arg(
        option("build", "PROFILE", "How the binary is built (default: release)")
          .value_parser(["release", "debug"]),
      )
      .arg(
        option("repeat", "N", "Plain runs, for wall time, CPU time and peak memory (default 3)")
          .value_parser(value_parser!(u32)),
      )
      .arg(
        option("interval", "US", "How often the sampling run samples, in microseconds of CPU time (default 1000)")
          .value_parser(value_parser!(u32)),
      )
      .arg(option(
        "compare",
        "REV",
        "Profile the tree as of REV (a git worktree under target/memst/rev) and the working tree, and compare them",
      ))
      .arg(flag("no-time", "No sampling run"))
      .arg(flag("no-memory", "No memory run"))
      .arg(
        Arg::new("args")
          .value_name("ARGS")
          .num_args(0..)
          .last(true)
          .help("With PROJECT: the program's arguments, after --"),
      ),
  )
  .subcommand(report(
    Command::new("report").about("A kept run's report again").arg(
      Arg::new("run")
        .value_name("RUN")
        .required(true)
        .help("A run's directory (.dev/runs/<UTC>-memst)"),
    ),
  ))
}

fn metrics() -> Command {
  Command::new("metrics")
    .about("Every number runs measure, kept as series (.dev/metrics/), and the ratchet some may not fall behind (records/results/ratchet.txt)")
    .subcommand(
      Command::new("list")
        .about("Every series with its latest value, change and trend (default)")
        .arg(Arg::new("filter").value_name("TEXT").help("Only series whose name contains TEXT")),
    )
    .subcommand(
      Command::new("show")
        .about("One series' points")
        .arg(
          Arg::new("name")
            .value_name("NAME")
            .required(true)
            .help("The series, as list shows it"),
        )
        .arg(
          option("last", "N", "How many points")
            .value_parser(value_parser!(u32))
            .default_value("20"),
        ),
    )
    .subcommand(Command::new("compare").about(
      "Every figure before the optimization rounds (records/archive/2026-10-optimization.txt) beside its latest value",
    ))
    .subcommand(
      Command::new("ratchet")
        .about("The ratchet against the latest values; --update records them as the new limits")
        .arg(flag("update", "Record the latest values (this machine) in records/results/ratchet.txt")),
    )
}

fn next() -> Command {
  Command::new("next")
    .about(
      "What to do next, most urgent first: failing checks, open incidents, an unproven \
       record, test pairs not matching, the settings with the most gaps, fuzzed \
       disagreements, and what is left to triage",
    )
    .arg(flag("all", "The whole queue, not the first ten"))
    .arg(flag("json", "As JSON (schema mtindo.xtask.next/1)"))
}

fn option(name: &'static str, value: &'static str, help: &'static str) -> Arg {
  Arg::new(name).long(name).value_name(value).help(help)
}

fn pairs() -> Command {
  Command::new("pairs")
    .about("The scrambled-pair suite: corpus files drawn by a fresh seed, items shuffled and whitespace scrambled (checked token for token), formatted by mto and by the standard formatter; scored by files and lines matching, and the crates that compile offline built scrambled, after mto and after the standard formatter; kept as pairs.* metrics; exits 1 on an mto compile break, an mto panic or a scrambler bug")
    .arg(
      option("seed", "N", "Draw and scramble from this seed, to replay a run (default: a new one, printed)")
        .value_parser(value_parser!(u64)),
    )
    .arg(
      option("crates", "NAME,...", "The corpus crates (default: six drawn by the seed, three of them crates that compile offline)")
        .value_delimiter(','),
    )
    .arg(
      option("files", "N", "Files drawn per crate; 0 for every file")
        .value_parser(value_parser!(u32))
        .default_value("20"),
    )
    .arg(flag("no-compile", "Score likeness only; build no copies"))
    .arg(option("keep", "DIR", "Keep the scrambled files and the built copies in DIR (outside this repository) instead of a temporary folder that is deleted"))
}

fn parsecheck() -> Command {
  Command::new("parsecheck")
    .about("Parse with each crate's edition; report syntax errors (default: this repository)")
    .arg(paths())
}

fn paths() -> Arg {
  Arg::new("paths")
    .value_name("PATH")
    .num_args(0..)
    .help("Files or folders")
}

fn perf() -> Command {
  Command::new("perf")
    .about(
      "Measure one-file latency, thread scaling and growth with size; all three with no subcommand",
    )
    .subcommand(
      Command::new("latency")
        .about("One small file through the release mto, many times: middle and p99"),
    )
    .subcommand(
      Command::new("scaling").about("Every 8th corpus file laid out by 1, 2, 4 and 8 threads"),
    )
    .subcommand(
      Command::new("growth").about("Generated files of doubling size: how time grows with each"),
    )
    .subcommand(
      Command::new("instructions")
        .about(
          "Instructions the release mto runs on a fixed workload, counted by Cachegrind \
           (Linux): fails when one grows over 3% past records/results/instructions.txt",
        )
        .arg(flag("update", "Record the counts")),
    )
}

fn probe() -> Command {
  Command::new("probe")
    .about(
      "One of the standard formatter's test pairs against every answer key (its suite at \
       stable and nightly, each toolchain's binary), outside the repository; --scan checks \
       every pair for where the keys disagree (records/policy/disagreements.txt)",
    )
    .arg(
      Arg::new("case").value_name("CASE").help(
        "A test pair as the diff names it (tests/configs/wrap_comments/true.rs), or its path",
      ),
    )
    .arg(option(
      "show",
      "KEY",
      "Show mto's output beside this key's, where they first differ",
    ))
    .arg(option(
      "toolchain",
      "NAME",
      "The nightly toolchain for run@nightly (default: nightly)",
    ))
    .arg(flag(
      "scan",
      "Check every pair; fails on a disagreement not recorded",
    ))
    .arg(flag("record", "Record the disagreements found, to triage"))
}

fn race() -> Command {
  Command::new("race")
    .about("Time rustfmt, rustfmt nightly and mto feature by feature (net of start-up), drawn live")
    .arg(
      option("rounds", "N", "Rounds per input")
        .value_parser(value_parser!(usize))
        .default_value("3"),
    )
    .arg(option(
      "filter",
      "TEXT",
      "Only rows whose name contains TEXT",
    ))
    .arg(
      option("real", "N", "Real-world files to time")
        .value_parser(value_parser!(usize))
        .default_value("100"),
    )
    .arg(flag("gross", "Show times with start-up included"))
    .arg(flag("markdown", "As a Markdown table"))
    .arg(flag("live", "Draw live even when not on a terminal"))
    .arg(option(
      "size",
      "COLSxROWS",
      "Draw at these sizes (at least 20x8), comma-separated",
    ))
    .arg(option("frames", "DIR", "Write each drawn frame into DIR"))
}

fn runs() -> Command {
  let id = || {
    Arg::new("id")
      .value_name("ID")
      .required(true)
      .help("A run id, or its end")
  };
  Command::new("runs")
    .about(
      "Run records under .dev/runs/: every command leaves one; --detach runs one in the background",
    )
    .subcommand(
      Command::new("list")
        .about("The latest runs (default)")
        .arg(flag("all", "Every run")),
    )
    .subcommand(Command::new("show").about("One run's record").arg(id()))
    .subcommand(
      Command::new("tail")
        .about("A detached run's output, until it ends")
        .arg(id()),
    )
    .subcommand(
      Command::new("cancel")
        .about("Stop a run and its processes")
        .arg(id()),
    )
    .subcommand(
      Command::new("prune")
        .about(
          "Remove run folders past retention: the newest 50 of the last 30 days are kept, \
           with those still going and those a finding names (each run does this when it ends)",
        )
        .arg(flag("dry-run", "Only say what would go")),
    )
    .subcommand(
      Command::new("why")
        .about(
          "What differs between what produced two runs (tree, tools, machine, records); \
           without OTHER, the run before ID of the same command",
        )
        .arg(id())
        .arg(
          Arg::new("other")
            .value_name("OTHER")
            .help("The run to compare with"),
        ),
    )
}

fn settings() -> Command {
  Command::new("settings")
    .about("Regenerate src/options/rustfmt.rs from the pinned rustfmt releases")
    .arg(flag("check", "Only compare"))
}

fn speed() -> Command {
  let measures: Vec<PossibleValue> = crate::speed::MEASURES
    .iter()
    .map(|(name, _, help)| PossibleValue::new(*name).help(*help))
    .collect();
  Command::new("speed")
    .about(
      "Every speed measurement behind one command: throughput, latency, scaling, growth, \
       io, features (live), doctor; ARGS go to the command that makes it",
    )
    .arg(
      Arg::new("measure")
        .value_name("MEASURE")
        .required(true)
        .value_parser(measures),
    )
    .arg(
      Arg::new("args")
        .value_name("ARGS")
        .num_args(0..)
        .trailing_var_arg(true)
        .allow_hyphen_values(true)
        .help("Passed to the measure's command (e.g. features --live)"),
    )
}

fn status() -> Command {
  Command::new("status").about(
    "Where the project stands: tree, last green, v0.1.0 gates, running jobs, latest runs, fuzz crashes",
  )
}

fn store() -> Command {
  Command::new("store")
    .about(
      "What .dev/ holds, area by area, with what each keeps, and what no command writes any more",
    )
    .arg(flag("json", "As JSON (schema mtindo.xtask.store/1)"))
}

fn test() -> Command {
  let groups: Vec<PossibleValue> = GROUPS.iter().map(|(g, _)| PossibleValue::new(*g)).collect();
  Command::new("test")
    .about("Run the gates and test suites; report each as passed, failed or skipped with a reason")
    .arg(
      option(
        "group",
        "GROUP",
        "Run only this group; repeatable. Default: gates and tests.",
      )
      .action(ArgAction::Append)
      .value_parser(groups),
    )
    .arg(flag(
      "full",
      "Run every group, including heavy (network), fuzz (nightly) and bench.",
    ))
    .arg(flag(
      "failed",
      "Run only the tasks that failed when they last ran (.dev/logs/last-test.tsv).",
    ))
    .arg(option(
      "target",
      "ID",
      "Run one task by id, whatever its group (ids are listed below).",
    ))
    .arg(
      option(
        "fuzz-time",
        "SECONDS",
        "Seconds to fuzz each target. Default: 10.",
      )
      .value_parser(value_parser!(u32))
      .default_value("10"),
    )
    .arg(flag(
      "bless",
      "Regenerate the regression fixtures' expected outputs, then list what changed.",
    ))
    .arg(flag(
      "list",
      "Show which tasks would run, without running them.",
    ))
}

fn toml_test() -> Command {
  Command::new("toml-test")
    .about("Run the TOML conformance suite (records/pins/toml-test.txt) against mto's parser")
}

fn unicode() -> Command {
  Command::new("unicode")
    .about("Regenerate src/text/unicode tables from the Unicode Character Database")
    .arg(option(
      "version",
      "X.Y.Z",
      "The Unicode version (default: the toolchain's)",
    ))
    .arg(flag("check", "Only compare"))
}

fn upstream() -> Command {
  Command::new("upstream")
    .about(
      "The standard formatter's bugs kept as cases in .dev/upstream/rustfmt/: listed, \
       rechecked against each toolchain, drafted as issue text",
    )
    .subcommand(
      Command::new("list").about("Every case, its upstream status and what goes wrong (default)"),
    )
    .subcommand(
      Command::new("recheck")
        .about(
          "Run every case through each toolchain's rustfmt and say whether the bug still \
           shows (a meaning check, or a second run)",
        )
        .arg(
          Arg::new("toolchains")
            .value_name("TOOLCHAIN")
            .num_args(0..)
            .help("The toolchains (default: the pinned one and nightly)"),
        ),
    )
    .subcommand(
      Command::new("draft")
        .about("Write one case's issue text to .dev/upstream/drafts/, for filing by hand")
        .arg(
          Arg::new("case")
            .value_name("CASE")
            .required(true)
            .help("A case's folder, or its number"),
        ),
    )
}

fn value_word(arg: &Arg) -> String {
  let possible = arg.get_possible_values();
  if !possible.is_empty() && possible.len() <= 4 {
    return possible
      .iter()
      .map(|p| p.get_name().to_string())
      .collect::<Vec<_>>()
      .join("|");
  }
  arg
    .get_value_names()
    .and_then(|v| v.first())
    .map(|v| v.to_string())
    .unwrap_or_else(|| arg.get_id().to_string().to_uppercase())
}

fn walk(command: &Command, prefix: &str, out: &mut String) {
  for sub in command.get_subcommands() {
    if sub.get_name() == "help" {
      continue;
    }
    let name = format!("{} {}", prefix, sub.get_name());
    out.push_str(&format!("\n## `{}`\n\n", name));
    if let Some(about) = sub.get_about() {
      out.push_str(&crate::papers::wrap(&about.to_string(), 80, "", ""));
      out.push('\n');
    }
    out.push_str("```text\n");
    out.push_str(&crate::papers::wrap(&usage(sub, &name), 80, "", "    "));
    out.push_str("```\n");
    let args: Vec<&Arg> = sub
      .get_arguments()
      .filter(|a| a.get_id() != "help" && a.get_id() != "detach")
      .collect();
    if !args.is_empty() {
      out.push('\n');
    }
    for arg in args {
      let line = format!(
        "`{}`: {}",
        arg_usage(arg).trim_matches(|c| c == '[' || c == ']'),
        arg_help(arg)
      );
      out.push_str(&crate::papers::wrap(&line, 80, "- ", "  "));
    }
    walk(sub, &name, out);
  }
}

/// Gives every command `-h, --help` as an ordinary flag, read by [`parse`], in place of
/// clap's own help.
fn with_help(command: Command) -> Command {
  let command = command
    .disable_help_flag(true)
    .disable_help_subcommand(true)
    .disable_version_flag(true)
    .arg(
      Arg::new("help")
        .short('h')
        .long("help")
        .action(ArgAction::SetTrue)
        .help("Show this help"),
    );
  let subs: Vec<Command> = command.get_subcommands().cloned().collect();
  let mut command = command;
  for sub in subs {
    let name = sub.get_name().to_string();
    command = command.mut_subcommand(name, |_| with_help(sub));
  }
  command
}

#[cfg(test)]
mod tests {
  use super::*;

  fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
  }

  #[test]
  fn test_help_is_identical_apart_from_color_everywhere() {
    let mut paths = vec![Vec::new()];
    for command in tree().get_subcommands() {
      paths.push(vec![command.get_name().to_string()]);
      for sub in command.get_subcommands() {
        paths.push(vec![
          command.get_name().to_string(),
          sub.get_name().to_string(),
        ]);
      }
    }
    for path in paths {
      let colored = help(&path, true).unwrap();
      let mut stripped = String::new();
      let mut rest = colored.as_str();
      while let Some(idx) = rest.find('\x1b') {
        stripped.push_str(&rest[..idx]);
        rest = &rest[idx..];
        rest = &rest[rest.find('m').unwrap() + 1..];
      }
      stripped.push_str(rest);
      assert_eq!(stripped, help(&path, false).unwrap(), "{:?}", path);
    }
  }

  #[test]
  fn test_help_shows_every_argument_and_command() {
    for command in tree().get_subcommands() {
      let path = vec![command.get_name().to_string()];
      let text = help(&path, false).unwrap();
      for arg in command.get_arguments() {
        if let Some(long) = arg.get_long() {
          assert!(text.contains(&format!("--{}", long)), "{:?} {}", path, long);
        }
      }
      for sub in command.get_subcommands() {
        assert!(
          text.contains(&format!("  {}", sub.get_name())),
          "{:?}",
          path
        );
      }
    }
  }

  #[test]
  fn test_parsing_runs_helps_and_refuses() {
    match parse(&args(&["fuzz", "run", "format", "--time", "5"])) {
      Parsed::Run(name, m) => {
        assert_eq!(name, "fuzz");
        let (sub, run) = m.subcommand().unwrap();
        assert_eq!(sub, "run");
        assert_eq!(run.get_one::<u32>("time"), Some(&5));
        assert_eq!(
          run.get_one::<String>("sanitizer").map(String::as_str),
          Some("address")
        );
      }
      _ => panic!("not a run"),
    }
    assert!(matches!(parse(&args(&[])), Parsed::Help(p) if p.is_empty()));
    assert!(
      matches!(parse(&args(&["fuzz", "run", "-h"])), Parsed::Help(p) if p == ["fuzz", "run"])
    );
    assert!(matches!(parse(&args(&["help", "corpus"])), Parsed::Help(p) if p == ["corpus"]));
    assert!(matches!(parse(&args(&["fuzz", "run"])), Parsed::Error(_)));
    assert!(matches!(parse(&args(&["gates", "nope"])), Parsed::Error(_)));
    assert!(matches!(
      parse(&args(&["fuzz", "run", "x", "--detach"])),
      Parsed::Run(..)
    ));
  }

  #[test]
  fn test_the_tree_is_valid() {
    tree().debug_assert();
  }
}
