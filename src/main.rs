#![deny(unsafe_code)]

use std::env;
use std::path::{Path, PathBuf};

use mtindo::cli::doctor as doctor_cli;
use mtindo::cli::history::{self, PruneArgs};
use mtindo::cli::spec::{
  COMMANDS, CommandSpec, EXAMPLES, EXIT_CODES, FLAGS, Flag, FlagSpec, Group, USAGE_LINES, flags_of,
  spec_of,
};
use mtindo::config::{self, Config, Environment, FileKind, Overrides, Resolver, Source};
use mtindo::doctor::guard::{self, Ended, Planned};
use mtindo::options::File;
use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::tree::debug_dump;
use mtindo::syntax::{Edition, parse};
use mtindo::{
  Done, Emit, HelpPalette, MtoError, Outcome, RunOptions, Writes, c_green, c_red, c_reset,
  c_yellow, collect_jobs, collect_jobs_into, print_warnings, process_init, process_stdin, run_jobs,
  run_jobs_walking,
};

/// The settings that decide which defaults apply.
const EDITION_KEYS: [&str; 3] = ["edition", "style_edition", "version"];

/// The column where help descriptions start.
const HELP_COLUMN: usize = 30;

/// Small blocks come from a per-thread cache in front of the system allocator.
#[global_allocator]
static GLOBAL: mtindo::cache_alloc::Cached = mtindo::cache_alloc::Cached;

enum CliAction {
  Format {
    paths: Vec<PathBuf>,
    overrides: Overrides,
    opts: RunOptions,
  },
  Stdin {
    overrides: Overrides,
    opts: RunOptions,
    filepath: Option<PathBuf>,
  },
  PrintConfig {
    kind: PrintKind,
    path: Option<PathBuf>,
    overrides: Overrides,
  },
  Config {
    view: ConfigView,
    dir: PathBuf,
    overrides: Overrides,
  },
  Report {
    view: ReportView,
  },
  History {
    view: HistoryView,
  },
  Restore {
    run: String,
    files: Vec<PathBuf>,
    force: bool,
  },
  Doctor {
    view: DoctorView,
  },
  Init {
    target_path: Option<PathBuf>,
  },
  Debug {
    view: DebugView,
    path: PathBuf,
    edition: Edition,
    /// `--edition` as given, over the settings for `layout`.
    edition_given: Option<String>,
  },
  Man,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigView {
  Path,
  /// Every setting a file may hold; as a commented file when true.
  Options(bool),
  Default,
  Current,
  Minimal,
  User,
}

#[derive(Clone, Copy)]
enum DebugView {
  Tokens,
  Tree,
  Layout,
}

/// What `mto doctor` was asked for.
enum DoctorView {
  Status,
  /// Turn the guard on, with the command after `--` when given.
  On(Option<Vec<String>>),
  Off,
  Holds,
  Release(String),
  /// Narrow down a run already made.
  Run {
    run: String,
    command: Option<Vec<String>>,
  },
}

enum HistoryView {
  /// Every project's runs when true, else the current folder's project's.
  List(bool),
  Show(String),
  Prune(PruneArgs),
}

/// `--print-config default|minimal|current`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrintKind {
  Default,
  Minimal,
  Current,
}

/// `mto report`'s views.
enum ReportView {
  List,
  Show(String),
  Save {
    id: String,
    file: String,
    with_file: bool,
  },
  Send(String),
  Prune,
}

/// Reads the value of a configuration flag into `overrides`.
fn config_flag(
  spec: &FlagSpec,
  value: Option<&String>,
  overrides: &mut Overrides,
) -> Result<(), ()> {
  let Some(value) = value else {
    return usage_error(&format!("{} needs a {}", spec.long, spec.value));
  };
  match spec.flag {
    Flag::Config => {
      for pair in split_pairs(value) {
        let Some((key, text)) = pair.split_once('=') else {
          return usage_error(&format!(
            "--config takes KEY=VALUE pairs; did you mean `--config-path {}`?",
            value
          ));
        };
        overrides
          .settings
          .push((key.trim().to_string(), text.trim().to_string()));
      }
    }
    Flag::ConfigPath => overrides.config_path = Some(PathBuf::from(value)),
    Flag::Edition => overrides.edition = Some(value.clone()),
    Flag::StyleEdition => overrides.style_edition = Some(value.clone()),
    _ => unreachable!("not a configuration flag"),
  }
  Ok(())
}

/// Prints the JSON report and the summary for a run, and returns its exit code.
fn finish(done: &[Done], opts: &RunOptions) -> i32 {
  let count = |outcome: Outcome| done.iter().filter(|d| d.outcome == outcome).count();
  if opts.emit == Emit::Json {
    let entries: Vec<&str> = done.iter().filter_map(|d| d.json.as_deref()).collect();
    println!("[{}]", entries.join(","));
  }
  let changed = count(Outcome::Changed);
  let off = count(Outcome::LayoutOff);
  let skipped = count(Outcome::Skipped);
  let broken = count(Outcome::SyntaxError);
  let failed = count(Outcome::Failed);
  let unsafe_writes = count(Outcome::VerifierFailed);
  let dirty = count(Outcome::Dirty);
  if dirty > 0 {
    eprintln!(
      "{}{} file(s) with uncommitted changes left alone: history is off, so nothing \
             could undo the change. Commit them, turn history on, or pass --allow-dirty.{}",
      c_yellow(true),
      dirty,
      c_reset(true)
    );
  }
  if off > 0 && !opts.quiet {
    eprintln!(
      "{}{} file(s) left alone: layout and formatting are off (`format = false`). \
             Remove `format = false` to format them, or run `mto init` for an mto.toml \
             with `layout = true`.{}",
      c_yellow(true),
      off,
      c_reset(true)
    );
  }
  if skipped > 0 && !opts.quiet {
    eprintln!(
      "{}{} file(s) skipped because they could not be laid out safely.{}",
      c_yellow(true),
      skipped,
      c_reset(true)
    );
  }
  if broken > 0 {
    eprintln!(
      "{}{} file(s) have syntax errors and were left unchanged.{}",
      c_red(),
      broken,
      c_reset(true)
    );
  }
  if unsafe_writes > 0 {
    eprintln!(
      "{}{} file(s) left unchanged because of a bug in mto, reported above.{}",
      c_red(),
      unsafe_writes,
      c_reset(true)
    );
    return 4;
  }
  if failed > 0 || broken > 0 || dirty > 0 {
    return 3;
  }
  if opts.check {
    if changed > 0 {
      if !opts.quiet {
        eprintln!(
          "{}{} file(s) would be reformatted.{}",
          c_red(),
          changed,
          c_reset(true)
        );
      }
      return 1;
    }
    if !opts.quiet && !opts.files_with_diff && opts.emit == Emit::Files {
      println!(
        "{}Check passed successfully: no formatting drift detected.{}",
        c_green(),
        c_reset(false)
      );
    }
  }
  0
}

/// A flag's line in a help page: `  -q, --quiet   Print ...`, the short form first.
fn flag_row(p: &HelpPalette, spec: &FlagSpec) -> String {
  let short = spec
    .short
    .map(|s| format!("{}, ", p.short(s)))
    .unwrap_or_else(|| "    ".to_string());
  let value = if spec.value.is_empty() {
    String::new()
  } else {
    format!(" {}", p.value(spec.value))
  };
  let label = format!("  {}{}{}", short, p.long(spec.long), value);
  let width = 2
    + 4
    + spec.long.len()
    + if spec.value.is_empty() {
      0
    } else {
      1 + spec.value.len()
    };
  p.row(&label, width, HELP_COLUMN, spec.help)
}

/// The user's history settings, and the history folder (which is read even with history
/// off, so earlier runs can still be restored).
fn history_store() -> Result<(history::Settings, mtindo::history::History), i32> {
  let resolver = resolver(&Overrides::default()).map_err(|e| report_error(&e))?;
  let settings =
    history::Settings::read(&resolver, false).map_err(|e| report_error(&MtoError::Usage(e)))?;
  mtindo::durable::set_mode(settings.sync);
  let dir = config::user::history_dir(resolver.env()).ok_or_else(|| {
    report_error(&MtoError::Usage(
      "there is no history: no home folder to keep it in".into(),
    ))
  })?;
  Ok((settings, mtindo::history::History::new(dir)))
}

/// The name a configuration file kind is shown with.
fn kind_label(kind: FileKind) -> &'static str {
  match kind {
    FileKind::Standard => "formatting config",
    FileKind::Mto => "mto config",
    FileKind::Explicit => "config (--config-path)",
    FileKind::User => "user config",
    FileKind::Cargo => "Cargo.toml (edition)",
  }
}

fn main() {
  quiet_on_closed_pipe();
  let action = match parse_args() {
    Ok(Some(a)) => a,
    Ok(None) => return,
    Err(()) => std::process::exit(2),
  };
  // All of it with the same stack on every platform: see `mtindo::STACK`.
  let code = mtindo::with_stack(move || match action {
    CliAction::Man => {
      print_man_page();
      0
    }
    CliAction::Debug {
      view: DebugView::Layout,
      path,
      edition_given,
      ..
    } => run_debug_layout(&path, edition_given),
    CliAction::Debug {
      view,
      path,
      edition,
      ..
    } => run_debug(view, &path, edition),
    CliAction::Stdin {
      overrides,
      opts,
      filepath,
    } => run_stdin(&overrides, &opts, filepath.as_deref()),
    CliAction::PrintConfig {
      kind,
      path,
      overrides,
    } => run_print_config(kind, path.as_deref(), &overrides),
    CliAction::Config {
      view,
      dir,
      overrides,
    } => run_config(view, &dir, &overrides),
    CliAction::Init { target_path } => match process_init(target_path) {
      Ok(path) => {
        println!(
          "{}Initialized default configuration:{} {}",
          c_green(),
          c_reset(false),
          path.display()
        );
        0
      }
      Err(e) => report_error(&e),
    },
    CliAction::History { view } => run_history(view),
    CliAction::Report { view } => run_report(view),
    CliAction::Restore { run, files, force } => run_restore(&run, &files, force),
    CliAction::Doctor { view } => run_doctor(view),
    CliAction::Format {
      paths,
      overrides,
      opts,
    } => run_format(&paths, &overrides, &opts),
  });
  // Whatever was written is made to last before mto says it is done.
  let code = match mtindo::durable::settle() {
    Ok(()) => code,
    Err(e) => {
      eprintln!(
        "{}error: cannot sync what was written to disk: {}{}",
        c_red(),
        e,
        c_reset(true)
      );
      if code == 0 { 3 } else { code }
    }
  };
  std::process::exit(code);
}

/// The note for folders a walk passed over because each holds a repository of its own: a
/// few by name, or every one with `-v`.
fn nested_note(nested: &[PathBuf], verbose: bool) -> String {
  const SHOWN: usize = 3;
  let names: Vec<String> = nested
    .iter()
    .take(if verbose { nested.len() } else { SHOWN })
    .map(|p| p.display().to_string())
    .collect();
  let more = if nested.len() > names.len() {
    format!(" and {} more (-v lists them)", nested.len() - names.len())
  } else {
    String::new()
  };
  format!(
    "note: passed over {} folder(s) holding a repository of their own: {}{}; name one to \
         lay it out",
    nested.len(),
    names.join(", "),
    more
  )
}

fn parse_args() -> Result<Option<CliAction>, ()> {
  let args: Vec<String> = env::args().skip(1).collect();
  if args.is_empty() {
    print_help();
    return Ok(None);
  }
  if let Some(command) = COMMANDS.iter().find(|c| c.name == args[0])
    && args[1..].iter().any(|a| a == "-h" || a == "--help")
  {
    print!(
      "{}",
      render_command_help(command, mtindo::color_enabled(false))
    );
    return Ok(None);
  }
  match args[0].as_str() {
    "init" => {
      let target_path = args.get(1).map(PathBuf::from);
      return Ok(Some(CliAction::Init { target_path }));
    }
    "debug" => return parse_debug_args(&args[1..]).map(Some),
    "config" => return parse_config_args(&args[1..]).map(Some),
    "history" => return parse_history_args(&args[1..]).map(Some),
    "report" => return parse_report_args(&args[1..]).map(Some),
    "restore" => return parse_restore_args(&args[1..]).map(Some),
    "doctor" => return parse_doctor_args(&args[1..]).map(Some),
    _ => {}
  }
  let mut paths: Vec<PathBuf> = Vec::new();
  let mut read_stdin = false;
  let mut filepath = None;
  let mut print_config = None;
  let mut overrides = Overrides::default();
  let mut recursive = false;
  let mut opts = RunOptions::default();
  let mut i = 0;
  while i < args.len() {
    let arg = args[i].as_str();
    // `--flag=value` is read as `--flag value`, as the standard formatter allows.
    let (arg, inline) = match arg.split_once('=') {
      Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
      _ => (arg, None),
    };
    let value = |i: &mut usize| -> Option<String> {
      if inline.is_some() {
        return inline.clone();
      }
      *i += 1;
      args.get(*i).cloned()
    };
    if let Some(spec) = FLAGS
      .iter()
      .find(|spec| spec.short == Some(arg) || spec.long == arg)
    {
      match spec.flag {
        Flag::Help => {
          print_help();
          return Ok(None);
        }
        Flag::Version => {
          println!("mto {}", mtindo::version::short());
          return Ok(None);
        }
        Flag::Man => return Ok(Some(CliAction::Man)),
        Flag::Check => opts.check = true,
        Flag::DryRun => opts.dry_run = true,
        Flag::Dbg => opts.debug = true,
        Flag::FollowLinks => opts.follow_links = true,
        Flag::Recursive => recursive = true,
        Flag::FilesWithDiff => opts.files_with_diff = true,
        Flag::SkipChildren => opts.skip_children = true,
        Flag::Quiet => opts.quiet = true,
        Flag::Verbose => opts.verbose = true,
        Flag::UnstableFeatures => {}
        Flag::Emit => {
          opts.emit = match value(&mut i).as_deref() {
            Some("files") => Emit::Files,
            Some("stdout") => Emit::Stdout,
            Some("json") => Emit::Json,
            Some(other) => {
              return usage_error(&format!(
                "--emit takes files, stdout or json, not `{}`",
                other
              ));
            }
            None => return usage_error("--emit needs files, stdout or json"),
          }
        }
        Flag::Color => {
          let choice = match value(&mut i).as_deref() {
            Some("always") => Some(true),
            Some("never") => Some(false),
            Some("auto") => None,
            other => {
              return usage_error(&format!(
                "--color takes always, never or auto, not `{}`",
                other.unwrap_or("")
              ));
            }
          };
          mtindo::text::color::set_color(choice);
        }
        Flag::StdinFilepath => match value(&mut i) {
          Some(path) => filepath = Some(PathBuf::from(path)),
          None => return usage_error("--stdin-filepath needs a <PATH>"),
        },
        Flag::PrintConfig => {
          let kind = match value(&mut i).as_deref() {
            Some("default") => PrintKind::Default,
            Some("minimal") => PrintKind::Minimal,
            Some("current") => PrintKind::Current,
            other => {
              return usage_error(&format!(
                "--print-config takes default, minimal or current, not `{}`",
                other.unwrap_or("")
              ));
            }
          };
          print_config = Some(kind);
        }
        Flag::Config | Flag::ConfigPath | Flag::Edition | Flag::StyleEdition => {
          let given = value(&mut i);
          config_flag(spec, given.as_ref(), &mut overrides)?;
        }
        Flag::Default | Flag::Current | Flag::Minimal | Flag::User => {
          return usage_error(&format!("{} only goes with `mto config print`", spec.long));
        }
        Flag::NoHistory => opts.no_history = true,
        Flag::AllowDirty => opts.allow_dirty = true,
        Flag::Unguarded => opts.unguarded = true,
        Flag::Force | Flag::OlderThan | Flag::Keep => {
          return usage_error(&format!(
            "{} only goes with `mto restore` or `mto history prune`",
            spec.long
          ));
        }
        Flag::All => {
          return usage_error("--all only goes with `mto history`");
        }
        Flag::WithFile => {
          return usage_error("--with-file only goes with `mto report save`");
        }
        Flag::Toml => {
          return usage_error("--toml only goes with `mto config options`");
        }
      }
    } else if arg == "-" && !read_stdin {
      read_stdin = true;
    } else if !arg.starts_with('-') {
      paths.push(PathBuf::from(arg));
    } else {
      return usage_error(&format!("unexpected argument '{}'", arg));
    }
    i += 1;
  }
  if let Some(kind) = print_config {
    if paths.len() > 1 {
      return usage_error("--print-config takes at most one PATH");
    }
    return Ok(Some(CliAction::PrintConfig {
      kind,
      path: paths.pop(),
      overrides,
    }));
  }
  if read_stdin {
    if !paths.is_empty() {
      return usage_error("- reads stdin; it cannot be combined with paths");
    }
    return Ok(Some(CliAction::Stdin {
      overrides,
      opts,
      filepath,
    }));
  }
  if filepath.is_some() {
    return usage_error("--stdin-filepath only goes with -");
  }
  if recursive && paths.is_empty() {
    paths.push(PathBuf::from("."));
  }
  if paths.is_empty() {
    return usage_error(
      "no path given: pass files or directories, -R for the current directory, or - to read stdin",
    );
  }
  Ok(Some(CliAction::Format {
    paths,
    overrides,
    opts,
  }))
}

fn parse_config_args(args: &[String]) -> Result<CliAction, ()> {
  let usage = || usage_error("usage: mto config path|print|options [OPTIONS] [DIR]");
  let mut view = match args.first().map(String::as_str) {
    Some("options") => {
      return match &args[1..] {
        [] => Ok(CliAction::Config {
          view: ConfigView::Options(false),
          dir: PathBuf::from("."),
          overrides: Overrides::default(),
        }),
        [flag] if flag == "--toml" => Ok(CliAction::Config {
          view: ConfigView::Options(true),
          dir: PathBuf::from("."),
          overrides: Overrides::default(),
        }),
        _ => usage_error("usage: mto config options [--toml]"),
      };
    }
    Some("path") => ConfigView::Path,
    Some("print") => ConfigView::Current,
    _ => return usage(),
  };
  let mut dir = None;
  let mut overrides = Overrides::default();
  let mut i = 1;
  while i < args.len() {
    let arg = args[i].as_str();
    match FLAGS.iter().find(|spec| spec.long == arg) {
      Some(spec) => match spec.flag {
        Flag::Config | Flag::ConfigPath | Flag::Edition | Flag::StyleEdition => {
          i += 1;
          config_flag(spec, args.get(i), &mut overrides)?;
        }
        Flag::Default | Flag::Current | Flag::Minimal | Flag::User if view != ConfigView::Path => {
          view = match spec.flag {
            Flag::Default => ConfigView::Default,
            Flag::Minimal => ConfigView::Minimal,
            Flag::User => ConfigView::User,
            _ => ConfigView::Current,
          };
        }
        _ => return usage_error(&format!("unexpected argument '{}'", arg)),
      },
      None if dir.is_none() && !arg.starts_with('-') => dir = Some(PathBuf::from(arg)),
      None => return usage_error(&format!("unexpected argument '{}'", arg)),
    }
    i += 1;
  }
  Ok(CliAction::Config {
    view,
    dir: dir.unwrap_or_else(|| PathBuf::from(".")),
    overrides,
  })
}

fn parse_debug_args(args: &[String]) -> Result<CliAction, ()> {
  let usage = || {
    eprintln!(
      "{}Error: usage: mto debug tokens|tree|layout <FILE> [--edition YEAR]{}",
      c_red(),
      c_reset(true)
    );
  };
  let view = match args.first().map(String::as_str) {
    Some("tokens") => DebugView::Tokens,
    Some("tree") => DebugView::Tree,
    Some("layout") => DebugView::Layout,
    _ => {
      usage();
      return Err(());
    }
  };
  let mut path = None;
  let mut edition = Edition::DEFAULT;
  let mut edition_given = None;
  let mut i = 1;
  while i < args.len() {
    match args[i].as_str() {
      "--edition" if i + 1 < args.len() => {
        edition = match Edition::from_year(&args[i + 1]) {
          Some(edition) => edition,
          None => {
            eprintln!(
              "{}Error: unknown edition '{}'{}",
              c_red(),
              args[i + 1],
              c_reset(true)
            );
            return Err(());
          }
        };
        edition_given = Some(args[i + 1].clone());
        i += 1;
      }
      other if path.is_none() && !other.starts_with('-') => path = Some(PathBuf::from(other)),
      _ => {
        usage();
        return Err(());
      }
    }
    i += 1;
  }
  match path {
    Some(path) => Ok(CliAction::Debug {
      view,
      path,
      edition,
      edition_given,
    }),
    None => {
      usage();
      Err(())
    }
  }
}

fn parse_doctor_args(args: &[String]) -> Result<CliAction, ()> {
  let (words, command) = match args.iter().position(|a| a == "--") {
    Some(at) => (&args[..at], Some(args[at + 1..].to_vec())),
    None => (args, None),
  };
  if command.as_ref().is_some_and(|c| c.is_empty()) {
    return usage_error("`--` must be followed by the command, as its words");
  }
  let words: Vec<&str> = words.iter().map(String::as_str).collect();
  let view = match (words.as_slice(), command) {
    ([], None) => DoctorView::Status,
    (["on"], command) => DoctorView::On(command),
    (["off"], None) => DoctorView::Off,
    (["holds"], None) => DoctorView::Holds,
    (["release", which], None) => DoctorView::Release(which.to_string()),
    ([run], command) if !run.starts_with('-') && !matches!(*run, "release") => DoctorView::Run {
      run: run.to_string(),
      command,
    },
    _ => {
      return usage_error(
        "usage: mto doctor [on [-- COMMAND...] | off | holds | release N|all | RUN [-- COMMAND...]]",
      );
    }
  };
  Ok(CliAction::Doctor { view })
}

fn parse_history_args(args: &[String]) -> Result<CliAction, ()> {
  let usage = || {
    usage_error("usage: mto history [--all | show <RUN> | prune [--older-than DAYS] [--keep N]]")
  };
  let view = match args.first().map(String::as_str) {
    None => HistoryView::List(false),
    Some("--all") if args.len() == 1 => HistoryView::List(true),
    Some("show") => match &args[1..] {
      [run] => HistoryView::Show(run.clone()),
      _ => return usage(),
    },
    Some("prune") => {
      let mut prune = PruneArgs::default();
      let mut i = 1;
      while i < args.len() {
        let (arg, inline) = match args[i].split_once('=') {
          Some((flag, value)) => (flag, Some(value.to_string())),
          None => (args[i].as_str(), None),
        };
        let value = inline.or_else(|| {
          i += 1;
          args.get(i).cloned()
        });
        match (arg, value) {
          ("--older-than", Some(text)) => match history::days(&text) {
            Some(days) => prune.older_than_days = Some(days),
            None => {
              return usage_error(&format!(
                "--older-than takes days (30), weeks (2w) or hours (12h), \
                                 not `{}`",
                text
              ));
            }
          },
          ("--keep", Some(text)) => match text.parse() {
            Ok(keep) => prune.keep = Some(keep),
            Err(_) => {
              return usage_error(&format!("--keep takes a number of runs, not `{}`", text));
            }
          },
          _ => return usage(),
        }
        i += 1;
      }
      HistoryView::Prune(prune)
    }
    Some("list") if args.len() == 1 => HistoryView::List(false),
    Some("list") if args.len() == 2 && args[1] == "--all" => HistoryView::List(true),
    _ => return usage(),
  };
  Ok(CliAction::History { view })
}

fn parse_report_args(args: &[String]) -> Result<CliAction, ()> {
  let usage = || {
    usage_error(
      "usage: mto report [show <ID> | save <ID> <FILE> [--with-file] | send <ID> | prune]",
    )
  };
  let words: Vec<&str> = args.iter().map(String::as_str).collect();
  let view = match words.as_slice() {
    [] => ReportView::List,
    ["show", id] => ReportView::Show(id.to_string()),
    ["save", id, file] => ReportView::Save {
      id: id.to_string(),
      file: file.to_string(),
      with_file: false,
    },
    ["save", id, file, "--with-file"] | ["save", "--with-file", id, file] => ReportView::Save {
      id: id.to_string(),
      file: file.to_string(),
      with_file: true,
    },
    ["send", id] => ReportView::Send(id.to_string()),
    ["prune"] => ReportView::Prune,
    _ => return usage(),
  };
  Ok(CliAction::Report { view })
}

fn parse_restore_args(args: &[String]) -> Result<CliAction, ()> {
  let mut run = None;
  let mut files = Vec::new();
  let mut force = false;
  for arg in args {
    match arg.as_str() {
      "--force" => force = true,
      other if other.starts_with('-') => {
        return usage_error(&format!("unexpected argument '{}'", other));
      }
      other if run.is_none() => run = Some(other.to_string()),
      other => files.push(PathBuf::from(other)),
    }
  }
  match run {
    Some(run) => Ok(CliAction::Restore { run, files, force }),
    None => usage_error(
      "usage: mto restore <RUN> [FILE...] [--force]; RUN is last, last~N, or a handle from `mto history`",
    ),
  }
}

fn print_help() {
  print!("{}", render_help(mtindo::color_enabled(false)));
}

fn print_man_page() {
  let manual = mtindo::cli::manual::render();
  if !mtindo::color_enabled(false) {
    print!("{}", manual);
    return;
  }
  let mut in_code_block = false;
  let mut lang = String::new();
  for line in manual.lines() {
    if let Some(fence_lang) = line.strip_prefix("```") {
      if in_code_block {
        println!("\x1b[90m```\x1b[0m");
        in_code_block = false;
      } else {
        in_code_block = true;
        lang = fence_lang.trim().to_string();
        println!("\x1b[90m{}\x1b[0m", line);
      }
      continue;
    }
    if in_code_block {
      let trimmed = line.trim_start();
      if trimmed.starts_with('#') {
        println!("\x1b[90m{}\x1b[0m", line);
      } else if lang == "toml" {
        if let Some(idx) = line.find('=') {
          println!(
            "\x1b[36m{}\x1b[0m=\x1b[32m{}\x1b[0m",
            &line[..idx],
            &line[idx + 1..]
          );
        } else if line.contains('"') {
          println!("\x1b[32m{}\x1b[0m", line);
        } else {
          println!("\x1b[35m{}\x1b[0m", line);
        }
      } else if lang == "bash" || lang == "text" {
        // The command as a word, not inside one (`mto.toml`).
        let colored = line
          .replace("mto ", "\x1b[32mmto\x1b[0m ")
          .replace("cargo install", "\x1b[32mcargo install\x1b[0m")
          .replace("[OPTIONS]", "\x1b[36m[OPTIONS]\x1b[0m")
          .replace("--path .", "\x1b[33m--path .\x1b[0m");
        println!("{}", colored);
      } else {
        println!("{}", line);
      }
      continue;
    }
    if line.starts_with('#') && line.trim_start_matches('#').starts_with(' ') {
      println!("\x1b[1;36m{}\x1b[0m", line);
    } else {
      let mut out = String::new();
      let mut in_backtick = false;
      let mut in_bold = false;
      let mut chars = line.chars().peekable();
      while let Some(c) = chars.next() {
        if c == '`' {
          in_backtick = !in_backtick;
          if in_backtick {
            out.push_str("\x1b[33m");
          } else {
            out.push_str("\x1b[0m");
          }
        } else if c == '*' && chars.peek() == Some(&'*') {
          chars.next();
          in_bold = !in_bold;
          if in_bold {
            out.push_str("\x1b[1m");
          } else {
            out.push_str("\x1b[0m");
          }
        } else {
          out.push(c);
        }
      }
      out.push_str("\x1b[0m");
      println!("{}", out);
    }
  }
}

/// Output piped into a reader that stops early (`mto history | head`) ends the run quietly
/// with 141, the status of a process ended by SIGPIPE, instead of a panic. Rust ignores
/// SIGPIPE, so the closed pipe surfaces as a failed print; other panics are untouched.
fn quiet_on_closed_pipe() {
  let default = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    let message = info
      .payload()
      .downcast_ref::<String>()
      .map(String::as_str)
      .or_else(|| info.payload().downcast_ref::<&str>().copied())
      .unwrap_or("");
    if message.starts_with("failed printing to stdout") && message.contains("Broken pipe") {
      std::process::exit(141);
    }
    default(info);
  }));
}

/// A command's help page: `mto history -h`.
fn render_command_help(command: &CommandSpec, color: bool) -> String {
  let p = HelpPalette::new(color, "mto");
  let mut out = usage_block(&p, command.usage);
  out.push('\n');
  for line in mtindo::text::color::wrap(command.about, mtindo::text::color::help_width()) {
    out.push_str(&line);
    out.push('\n');
  }
  for (heading, after, flags) in command.options {
    out.push_str(&section_heading(&p, heading, after));
    for flag in *flags {
      out.push_str(&flag_row(&p, spec_of(*flag)));
    }
  }
  out.push_str(&format!("\n{}\n", p.heading("examples:")));
  for example in command.examples {
    out.push_str(&format!("  {}\n", p.usage_line(example)));
  }
  out
}

/// What `mto config path|print` shows.
fn render_config(view: ConfigView, config: &Config, env: &Environment) -> String {
  let mut out = String::new();
  if view == ConfigView::Path {
    let user = config::user::path(env).ok().flatten();
    if let Some(user) = user.filter(|p| !p.is_file()) {
      out.push_str(&format!("user config: {} (not created)\n", user.display()));
    }
    for (kind, path) in config.files() {
      out.push_str(&format!("{}: {}\n", kind_label(*kind), path.display()));
    }
    if config.files().is_empty() {
      out.push_str("no configuration files; every setting is at its default\n");
    }
    if let Some(history) = config::user::history_dir(env) {
      out.push_str(&format!("history: {}\n", history.display()));
    }
    return out;
  }
  if view == ConfigView::User {
    let mut section = "";
    for (setting, value, source) in config.values() {
      if setting.file != File::User {
        continue;
      }
      // As the user config holds them: `[history]`, then `enabled = ...`.
      let (heading, key) = setting.name.split_once('.').unwrap_or(("", setting.name));
      if heading != section {
        if !section.is_empty() {
          out.push('\n');
        }
        out.push_str(&format!("[{}]\n", heading));
        section = heading;
      }
      out.push_str(&format!("{} = {}  # {}\n", key, value.to_toml(), source));
    }
    return out;
  }
  if view == ConfigView::Default {
    out.push_str(&format!(
      "# defaults for style edition {}\n",
      config.style_edition()
    ));
  } else {
    out.push_str(&format!("# style edition {}\n", config.style_edition()));
  }
  // Layout first, then formatting; each sorted by name, so the output is the same
  // whatever order the table holds the settings in.
  for (file, heading) in [
    (File::Mto, "layout (mto.toml)"),
    (File::Format, "formatting (mto.toml or rustfmt.toml)"),
  ] {
    let mut lines: Vec<(&str, String)> = Vec::new();
    for (setting, value, source) in config.values() {
      let shown = setting.file == file
        && match view {
          ConfigView::Default => !setting.hidden,
          ConfigView::Current => !setting.hidden || *source != Source::Default,
          _ => !matches!(source, Source::Default | Source::Derived(_)),
        };
      if !shown {
        continue;
      }
      let line = if view != ConfigView::Default {
        format!("{} = {}  # {}", setting.name, value.to_toml(), source)
      } else if EDITION_KEYS.contains(&setting.name) {
        // These decide which defaults apply, so they show what is in effect.
        format!("{} = {}  # in effect", setting.name, value.to_toml())
      } else {
        format!(
          "{} = {}",
          setting.name,
          setting.default_for(config.style_edition())
        )
      };
      lines.push((setting.name, line));
    }
    if lines.is_empty() {
      continue;
    }
    lines.sort();
    out.push_str(&format!("\n# {}\n", heading));
    for (_, line) in lines {
      out.push_str(&line);
      out.push('\n');
    }
  }
  out
}

/// The short help. The text is fixed; only the colors depend on `color`.
fn render_help(color: bool) -> String {
  let p = HelpPalette::new(color, "mto");
  let mut out = usage_block(&p, USAGE_LINES);
  out.push('\n');
  let about = "Reorders the top-level items of Rust files into a predictable layout, safely \
                 and atomically.";
  for line in mtindo::text::color::wrap(about, mtindo::text::color::help_width()) {
    out.push_str(&line);
    out.push('\n');
  }
  for group in Group::SHOWN {
    out.push_str(&section_heading(&p, group.heading(), group.command()));
    for spec in flags_of(group) {
      out.push_str(&flag_row(&p, spec));
    }
  }
  out.push_str(&format!("\n{}\n", p.heading("commands:")));
  for command in COMMANDS {
    let label = format!("  {}", p.long(command.name));
    out.push_str(&p.row(&label, 2 + command.name.len(), HELP_COLUMN, command.summary));
  }
  out.push_str(&format!(
    "\nRun `{}` for a command's options.\n",
    p.usage_line("mto <COMMAND> -h")
  ));
  out.push_str(&format!("\n{}\n", p.heading("exit codes:")));
  for (code, meaning) in EXIT_CODES {
    out.push_str(&p.row(&format!("  {}", p.value(code)), 3, 5, meaning));
  }
  out.push_str(&format!("\n{}\n", p.heading("examples:")));
  for example in EXAMPLES {
    out.push_str(&format!("  {}\n", p.usage_line(example)));
  }
  out
}

fn report_error(error: &MtoError) -> i32 {
  eprintln!("{}Error: {}{}", c_red(), error, c_reset(true));
  error.exit_code()
}

/// A resolver for this run's overrides; creates the user config if it is missing.
fn resolver(overrides: &Overrides) -> Result<Resolver, MtoError> {
  let env = Environment::from_process();
  if let Err(e) = config::user::create_if_missing(&env) {
    eprintln!("{}warning: {}{}", c_yellow(true), e, c_reset(true));
  }
  let resolver = Resolver::new(overrides, env)?;
  let color = resolver.user_value("output.color");
  mtindo::text::color::set_configured_color(match color.as_ref().and_then(|v| v.as_text()) {
    Some("always") => Some(true),
    Some("never") => Some(false),
    _ => None,
  });
  Ok(resolver)
}

fn run_config(view: ConfigView, dir: &std::path::Path, overrides: &Overrides) -> i32 {
  if let ConfigView::Options(toml) = view {
    print!("{}", mtindo::cli::options::render(toml));
    return 0;
  }
  if !dir.is_dir() {
    return report_error(&MtoError::Usage(format!(
      "{} is not a directory",
      dir.display()
    )));
  }
  let mut resolver = match resolver(overrides) {
    Ok(resolver) => resolver,
    Err(e) => return report_error(&e),
  };
  let config = match resolver.config_for(dir) {
    Ok(config) => config,
    Err(e) => return report_error(&e.into()),
  };
  print_warnings(&mut resolver);
  print!(
    "{}",
    render_config(view, &config, &Environment::from_process())
  );
  0
}

fn run_debug(view: DebugView, path: &std::path::Path, edition: Edition) -> i32 {
  let text = match std::fs::read_to_string(path) {
    Ok(text) => text,
    Err(e) => {
      return report_error(&MtoError::Io(format!(
        "cannot read {}: {}",
        path.display(),
        e
      )));
    }
  };
  match view {
    DebugView::Tokens => {
      let mut offset = 0;
      for token in tokenize(&text, edition) {
        let end = offset + token.len as usize;
        let mut line = format!(
          "{}@{}..{} {:?}",
          token.kind.name(),
          offset,
          end,
          &text[offset..end]
        );
        if let Some(error) = token.error {
          line.push_str(&format!("  error: {}", error.message()));
        }
        println!("{}", line);
        offset = end;
      }
    }
    // Run apart (`run_debug_layout`).
    DebugView::Layout => {}
    DebugView::Tree => {
      let parse = parse(&text, edition);
      print!("{}", debug_dump(&parse.syntax_node()));
      for error in parse.errors() {
        println!("error@{}: {}", error.range, error.message);
      }
    }
  }
  0
}

/// `mto debug layout FILE`: each place the formatter leaves code as written in FILE, and
/// why, with the settings that apply to it and formatting on.
fn run_debug_layout(path: &std::path::Path, edition: Option<String>) -> i32 {
  // Formatting on, wherever it is off: it is what leaves code as written.
  let overrides = Overrides {
    edition,
    settings: vec![("format".to_string(), "true".to_string())],
    ..Overrides::default()
  };
  let mut resolver = match resolver(&overrides) {
    Ok(resolver) => resolver,
    Err(e) => return report_error(&e),
  };
  let (source, notes) = match mtindo::explain_file(&mut resolver, path) {
    Ok(Some((source, notes, _))) => (source, notes),
    Ok(None) => {
      println!("{}: formatting is not on here", path.display());
      return 0;
    }
    Err(e) => return report_error(&e),
  };
  if notes.is_empty() {
    println!("{}: nothing left as written", path.display());
    return 0;
  }
  // In the file's order; one not found in the file is about text mto made on the way (a
  // macro body formatted with its `$` names replaced), not the file's.
  let places = mtindo::format::explain::locate(&source, &notes);
  let mut found: Vec<_> = notes
    .iter()
    .zip(places)
    .filter_map(|(note, place)| place.map(|place| (note, place)))
    .collect();
  if found.is_empty() {
    println!("{}: nothing left as written", path.display());
    return 0;
  }
  found.sort_by_key(|(_, place)| *place);
  for (note, (line, column)) in found {
    let at = format!("{}:{}", line, column);
    println!(
      "{}:{}  {}  {}\n    {}",
      path.display(),
      at,
      note.kind,
      note.reason,
      note.shown
    );
  }
  0
}

/// `mto doctor`: the build guard's settings and holds, or a run narrowed down.
fn run_doctor(view: DoctorView) -> i32 {
  let mut resolver = match resolver(&Overrides::default()) {
    Ok(resolver) => resolver,
    Err(e) => return report_error(&e),
  };
  let settings = doctor_cli::Settings::read(&resolver);
  let config_path = config::user::path(resolver.env()).ok().flatten();
  let result = match view {
    DoctorView::Status => Ok(doctor_cli::status(&settings, config_path.as_deref())),
    DoctorView::On(command) => {
      let Some(path) = &config_path else {
        return report_error(&MtoError::Usage(
          "there is no user config to keep the setting in: no home folder".into(),
        ));
      };
      doctor_cli::turn(path, true, command.as_deref()).map(|()| {
        let words = command.unwrap_or_else(|| settings.command.clone());
        format!(
          "The build guard is on. Each run that writes files now runs, in the project's \
           folder,\n\n    {}\n\nbefore and after it, and holds back the fewest edits that \
           break the build. mto runs no other command. `mto doctor off` turns it off \
           ({}).\n",
          words.join(" "),
          path.display()
        )
      })
    }
    DoctorView::Off => match &config_path {
      Some(path) => doctor_cli::turn(path, false, None)
        .map(|()| "The build guard is off: mto runs no command.\n".to_string()),
      None => Ok("The build guard is off.\n".to_string()),
    },
    DoctorView::Holds => Ok(doctor_cli::list(&settings)),
    DoctorView::Release(which) => doctor_cli::release(&settings, &which),
    DoctorView::Run { run, command } => {
      if command.is_none() && !settings.enabled {
        return report_error(&MtoError::Usage(format!(
          "the build guard is off, so there is no command to check with: give it after \
           `--` (mto doctor {} -- cargo check), or turn the guard on with `mto doctor on`",
          run
        )));
      }
      return run_doctor_run(&mut resolver, &settings, &run, command);
    }
  };
  match result {
    Ok(text) => {
      print!("{}", text);
      0
    }
    Err(e) => report_error(&MtoError::Usage(e)),
  }
}

/// `mto doctor RUN`: puts the run's files back as they were, checks, then writes them as
/// the run did, less the fewest edits that break the build.
fn run_doctor_run(
  resolver: &mut Resolver,
  settings: &doctor_cli::Settings,
  spec: &str,
  command: Option<Vec<String>>,
) -> i32 {
  let (history_settings, store) = match history_store() {
    Ok(found) => found,
    Err(code) => return code,
  };
  for line in guard::recover(&store) {
    eprintln!("{}", line);
  }
  let id = match history::resolve_here(&store, spec) {
    Ok(id) => id,
    Err(e) => return report_error(&MtoError::Usage(e)),
  };
  let record = match store.read(&id) {
    Ok(record) => record,
    Err(e) => return report_error(&MtoError::Io(e.to_string())),
  };
  let changed: Vec<String> = record
    .entries
    .iter()
    .filter(|e| mtindo::history::Run::state(e) != mtindo::history::State::AsWritten)
    .map(|e| e.path.display().to_string())
    .collect();
  if !changed.is_empty() {
    return report_error(&MtoError::Usage(format!(
      "these files no longer hold what the run wrote, so it cannot be narrowed down: {}",
      changed.join(", ")
    )));
  }
  let holds = settings.holds();
  let mut planned = Vec::new();
  for entry in &record.entries {
    let dir = entry.path.parent().unwrap_or(Path::new("."));
    let file_settings = resolver
      .config_for(dir)
      .ok()
      .and_then(|config| mtindo::Settings::of(&config));
    let (Some(file_settings), Ok(source), Ok(text)) = (
      file_settings,
      store.load(&entry.before),
      std::fs::read_to_string(&entry.path),
    ) else {
      eprintln!(
        "{}note: {} is left as it is: it is no longer laid out or formatted, or its \
         contents are gone{}",
        c_yellow(true),
        entry.path.display(),
        c_reset(true)
      );
      continue;
    };
    // Shown as a formatting run shows it, from the current folder when under it.
    let shown = match entry
      .path
      .strip_prefix(env::current_dir().unwrap_or_default())
    {
      Ok(relative) => Path::new(".").join(relative),
      Err(_) => entry.path.clone(),
    };
    planned.push(Planned {
      held: holds.held_for(&entry.path),
      path: shown,
      source: String::from_utf8_lossy(&source).into_owned(),
      text,
      settings: std::sync::Arc::new(file_settings),
    });
  }
  let cwd = env::current_dir().unwrap_or_default();
  let guard = settings.guard(&cwd, command, false);
  let mut recorder = Some(store.recorder(&history::command_line(), &cwd, None));
  // As before the run, for the check before.
  for file in &planned {
    if let Err(e) = guard::write(&file.path, &file.text, &file.source, &mut recorder) {
      return report_error(&MtoError::Io(e));
    }
  }
  let handle = mtindo::history::id::handle(&id, &store.ids().unwrap_or_default());
  eprintln!(
    "doctor: checking run {} with `{}`",
    handle,
    guard.check.line()
  );
  let before = guard.check.run();
  let mut holds = holds;
  let ended = guard::finish(
    &guard,
    before,
    planned,
    &mut recorder,
    &mut holds,
    &record.command,
  );
  if let Some(recorder) = recorder.as_mut() {
    history::finish_record(recorder, history_settings.limits, false);
  }
  match ended {
    Ended::Written => 0,
    Ended::Failed => 3,
    Ended::NotMto => 5,
  }
}

fn run_format(paths: &[PathBuf], overrides: &Overrides, opts: &RunOptions) -> i32 {
  let mut resolver = match resolver(overrides) {
    Ok(resolver) => resolver,
    Err(e) => return report_error(&e),
  };
  if let Some(missing) = paths.iter().find(|p| !p.exists()) {
    return report_error(&MtoError::Usage(format!(
      "target path '{}' does not exist",
      missing.display()
    )));
  }
  let writing = opts.emit == Emit::Files && !opts.check && !opts.dry_run;
  // A run that writes nothing lays files out while the walk is still finding them; one
  // that writes walks first, so that a folder's bad configuration stops it before any file
  // changes, and knows every file before the git guard and history see them.
  if !writing && paths.iter().any(|p| p.is_dir()) {
    let settings = match history::Settings::read(&resolver, opts.no_history) {
      Ok(settings) => settings,
      Err(e) => return report_error(&MtoError::Usage(e)),
    };
    mtindo::durable::set_mode(settings.sync);
    let mut writes = Writes {
      reports: mtindo::cli::report::store(&resolver),
      ..Writes::default()
    };
    let (_, collected, done) = run_jobs_walking(
      opts,
      &mut writes,
      |found| collect_jobs_into(paths, &mut resolver, opts, found),
      |collected| match collected {
        Ok(collected) => {
          walk_notes(collected, opts);
          true
        }
        Err(_) => false,
      },
    );
    return match collected {
      Ok(_) => finish(&done, opts),
      Err(e) => report_error(&e),
    };
  }
  let collected = match collect_jobs(paths, &mut resolver, opts) {
    Ok(found) => found,
    Err(e) => return report_error(&e),
  };
  walk_notes(&collected, opts);
  let jobs = collected.jobs;
  let settings = match history::Settings::read(&resolver, opts.no_history) {
    Ok(settings) => settings,
    Err(e) => return report_error(&MtoError::Usage(e)),
  };
  mtindo::durable::set_mode(settings.sync);
  let mut writes = Writes {
    reports: mtindo::cli::report::store(&resolver),
    ..Writes::default()
  };
  if writing {
    let removed = mtindo::remove_stale(&collected.stale);
    if removed > 0 && !opts.quiet {
      eprintln!(
        "{}note: removed {} temporary file(s) a run cut short left behind{}",
        c_yellow(true),
        removed,
        c_reset(true)
      );
    }
    if let Some(note) = &settings.note
      && !opts.quiet
    {
      eprintln!("{}note: {}{}", c_yellow(true), note, c_reset(true));
    }
    match &settings.history {
      Some(store) => {
        let cwd = env::current_dir().unwrap_or_default();
        writes.recorder = Some(store.recorder(&history::command_line(), &cwd, None));
      }
      None if !opts.allow_dirty && !settings.allow_dirty => {
        let files: Vec<PathBuf> = jobs
          .iter()
          .filter(|(_, config)| config.is_some())
          .map(|(path, _)| std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
          .collect();
        let dirty = mtindo::history::git::dirty(&files);
        if let Some(reason) = dirty.skipped
          && !opts.quiet
        {
          eprintln!(
            "{}note: {}; files with uncommitted changes are not protected{}",
            c_yellow(true),
            reason,
            c_reset(true)
          );
        }
        writes.dirty = Some(dirty.files);
      }
      None => {}
    }
  }
  let doctor = doctor_cli::Settings::read(&resolver);
  let cwd = env::current_dir().unwrap_or_default();
  let mut guarded = None;
  if writing {
    if let Some(store) = &settings.history {
      for line in guard::recover(store) {
        eprintln!("{}", line);
      }
    }
    writes.holds = Some(doctor.holds());
    // Editors name a single file; those runs are not guarded.
    let single = paths.len() == 1 && paths[0].is_file();
    if doctor.enabled && !opts.unguarded && !single {
      let guard = doctor.guard(&cwd, None, opts.quiet);
      let check = guard.check.clone();
      // The check before runs while the files are laid out in memory.
      guarded = Some((guard, std::thread::spawn(move || check.run())));
      writes.defer = true;
    }
  }
  let mut done = run_jobs(&jobs, opts, &mut writes);
  let mut code_after = 0;
  if let Some(holds) = writes.holds.as_mut() {
    let mut lapsed = false;
    for ((path, _), finished) in jobs.iter().zip(&done) {
      if let Some(found) = &finished.holds {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        lapsed |= holds.lapse(&path, found);
      }
    }
    if lapsed {
      let _ = holds.save();
    }
  }
  if let Some((guard, before)) = guarded {
    let planned: Vec<Planned> = jobs
      .iter()
      .zip(done.iter_mut())
      .filter_map(|((path, settings), finished)| {
        let (source, text) = finished.planned.take()?;
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        Some(Planned {
          held: writes
            .holds
            .as_ref()
            .map(|h| h.held_for(&absolute))
            .unwrap_or_default(),
          path: path.clone(),
          source,
          text,
          settings: settings.clone()?,
        })
      })
      .collect();
    let before = before.join().unwrap_or_default();
    let mut holds = writes.holds.take().unwrap_or_default();
    match guard::finish(
      &guard,
      before,
      planned,
      &mut writes.recorder,
      &mut holds,
      &history::command_line(),
    ) {
      Ended::Written => {}
      Ended::Failed => code_after = 3,
      Ended::NotMto => code_after = 5,
    }
  }
  if let Some(recorder) = writes.recorder.as_mut() {
    history::finish_record(recorder, settings.limits, opts.quiet);
  }
  finish(&done, opts).max(code_after)
}

fn run_history(view: HistoryView) -> i32 {
  let (settings, store) = match history_store() {
    Ok(found) => found,
    Err(code) => return code,
  };
  let result = match view {
    HistoryView::List(all) => {
      let here = std::env::current_dir().ok().filter(|_| !all);
      history::list(&store, here.as_deref())
    }
    HistoryView::Show(run) => history::show(&store, &run),
    HistoryView::Prune(args) => history::prune(&store, settings.limits, args),
  };
  match result {
    Ok(text) => {
      print!("{}", text);
      0
    }
    Err(e) => report_error(&MtoError::Usage(e)),
  }
}

/// `--print-config`, as the standard formatter prints it: the formatting settings as TOML,
/// ready for a `rustfmt.toml`. `default` and `minimal` go to PATH when given, else stdout;
/// `current` prints the settings for PATH (a file or folder; the current folder by default).
fn run_print_config(kind: PrintKind, path: Option<&Path>, overrides: &Overrides) -> i32 {
  let mut resolver = match resolver(overrides) {
    Ok(resolver) => resolver,
    Err(e) => return report_error(&e),
  };
  let dir = match (kind, path) {
    (PrintKind::Current, Some(p)) if p.is_file() => {
      p.parent().unwrap_or(Path::new(".")).to_path_buf()
    }
    (PrintKind::Current, Some(p)) => p.to_path_buf(),
    _ => PathBuf::from("."),
  };
  let config = match resolver.config_for(&dir) {
    Ok(config) => config,
    Err(e) => return report_error(&e.into()),
  };
  print_warnings(&mut resolver);
  let mut text = String::new();
  for (setting, value, source) in config.values() {
    if setting.file != File::Format || (setting.hidden && *source == Source::Default) {
      continue;
    }
    let shown = match kind {
      PrintKind::Default => setting.default_for(config.style_edition()).to_string(),
      PrintKind::Minimal if matches!(source, Source::Default | Source::Derived(_)) => {
        continue;
      }
      _ => value.to_toml(),
    };
    text.push_str(&format!("{} = {}\n", setting.name, shown));
  }
  match (kind, path) {
    (PrintKind::Default | PrintKind::Minimal, Some(file)) => {
      if let Err(e) = std::fs::write(file, &text) {
        return report_error(&MtoError::Io(format!(
          "cannot write {}: {}",
          file.display(),
          e
        )));
      }
    }
    _ => print!("{}", text),
  }
  0
}

/// `mto history`: lists, shows or prunes the runs.
/// `mto report`: lists, shows, saves, sends or prunes the bug reports.
fn run_report(view: ReportView) -> i32 {
  let resolver = match resolver(&Overrides::default()) {
    Ok(resolver) => resolver,
    Err(e) => return report_error(&e),
  };
  let Some(dir) = mtindo::cli::report::dir(&resolver) else {
    return report_error(&MtoError::Usage(
      "there are no reports: no home folder to keep them in".into(),
    ));
  };
  let store = mtindo::report::Store::new(dir);
  use mtindo::cli::report as reports;
  let result = match view {
    ReportView::List => Ok(reports::list(&store)),
    ReportView::Show(id) => reports::show(&store, &id),
    ReportView::Save {
      id,
      file,
      with_file,
    } => reports::save(&store, &id, &file, with_file),
    ReportView::Send(id) => reports::send(&store, &id),
    ReportView::Prune => reports::prune(&store),
  };
  match result {
    Ok(text) => {
      print!("{}", text);
      0
    }
    Err(e) => report_error(&MtoError::Usage(e)),
  }
}

/// `mto restore`.
fn run_restore(run: &str, files: &[PathBuf], force: bool) -> i32 {
  let (settings, store) = match history_store() {
    Ok(found) => found,
    Err(code) => return code,
  };
  match history::restore(&settings, &store, run, files, force) {
    Ok(code) => code,
    Err(e) => report_error(&MtoError::Usage(e)),
  }
}

fn run_stdin(overrides: &Overrides, opts: &RunOptions, filepath: Option<&Path>) -> i32 {
  let mut resolver = match resolver(overrides) {
    Ok(resolver) => resolver,
    Err(e) => return report_error(&e),
  };
  let report = match process_stdin(&mut resolver, opts, filepath) {
    Ok(report) => report,
    Err(e) => return report_error(&e),
  };
  report.print();
  let done = [Done {
    outcome: report.outcome,
    json: report.json,
    ..Done::default()
  }];
  finish(&done, opts)
}

/// A section heading with the command its flags follow: `layout: (mto <PATH>...)`.
fn section_heading(p: &HelpPalette, heading: &str, command: &str) -> String {
  format!(
    "\n{} ({})\n",
    p.heading(&format!("{}:", heading)),
    p.usage_line(command)
  )
}

/// `--config` pairs split at commas outside brackets and quotes, so
/// `type_order=["FnDecl","UseDecl"]` stays one pair.
fn split_pairs(text: &str) -> Vec<&str> {
  let mut pairs = Vec::new();
  let (mut depth, mut quote, mut start) = (0i32, None, 0);
  for (idx, ch) in text.char_indices() {
    match (quote, ch) {
      (Some(q), c) if c == q => quote = None,
      (Some(_), _) => {}
      (None, '"' | '\'') => quote = Some(ch),
      (None, '[' | '{') => depth += 1,
      (None, ']' | '}') => depth -= 1,
      (None, ',') if depth == 0 => {
        pairs.push(&text[start..idx]);
        start = idx + 1;
      }
      _ => {}
    }
  }
  pairs.push(&text[start..]);
  pairs.into_iter().filter(|p| !p.trim().is_empty()).collect()
}

/// `usage:` and its lines, aligned.
fn usage_block(p: &HelpPalette, lines: &[&str]) -> String {
  let mut out = String::new();
  for (idx, line) in lines.iter().enumerate() {
    let lead = if idx == 0 {
      p.heading("usage:")
    } else {
      "      ".to_string()
    };
    out.push_str(&format!("{} {}\n", lead, p.usage_line(line)));
  }
  out
}

/// Prints a usage error and points at the help; returns the error for `?`-style use.
fn usage_error<T>(message: &str) -> Result<T, ()> {
  eprintln!("{}Error: {}{}", c_red(), message, c_reset(true));
  eprintln!("Run 'mto --help' for usage.");
  Err(())
}

/// What the walk noted, before any file's report: modules whose files are missing, and
/// folders passed over.
fn walk_notes(collected: &mtindo::Collected, opts: &RunOptions) {
  for problem in &collected.problems {
    eprintln!("{}warning: {}{}", c_yellow(true), problem, c_reset(true));
  }
  if !collected.nested.is_empty() && !opts.quiet {
    eprintln!("{}", nested_note(&collected.nested, opts.verbose));
  }
}

#[cfg(test)]
mod tests {
  use std::collections::HashSet;

  use super::*;

  #[test]
  fn test_every_command_has_a_help_page() {
    for command in COMMANDS {
      let page = render_command_help(command, false);
      assert!(page.starts_with("usage: mto "), "{}", page);
      for (_, after, _) in command.options {
        // Each section names the command its flags follow, and it is the command's.
        assert!(
          after.starts_with(&format!("mto {}", command.name)),
          "{}",
          after
        );
        assert!(page.contains(&format!(" ({})\n", after)), "{}", page);
      }
      for flag in command
        .options
        .iter()
        .flat_map(|(_, _, flags)| flags.iter())
      {
        let spec = spec_of(*flag);
        assert!(
          page.contains(spec.long),
          "{} not on {}'s page",
          spec.long,
          command.name
        );
      }
    }
  }

  #[test]
  fn test_flags_appear_once_in_a_fixed_place() {
    let help = render_help(false);
    for spec in FLAGS {
      let shown = help.matches(&format!(" {} ", spec.long)).count()
        + help.matches(&format!(" {}\n", spec.long)).count();
      // Command flags are on their command's page; the others once in `mto -h`.
      if spec.group == Group::Command {
        assert!(
          COMMANDS
            .iter()
            .flat_map(|c| c.options.iter())
            .any(|(_, _, flags)| flags.contains(&spec.flag)),
          "{} belongs to no command",
          spec.long
        );
      } else {
        assert!(shown >= 1, "{} is not in mto -h", spec.long);
      }
    }
    // Groups come in their order, and each lists its flags by name.
    let mut last = 0;
    for group in Group::SHOWN {
      let at = help
        .find(&format!("\n{}: ({})\n", group.heading(), group.command()))
        .expect("every group has a heading");
      assert!(at > last, "{} out of order", group.heading());
      last = at;
      let names: Vec<&str> = flags_of(group).iter().map(|s| s.long).collect();
      let mut sorted = names.clone();
      sorted.sort();
      assert_eq!(names, sorted);
    }
  }

  #[test]
  fn test_flags_are_unique() {
    let mut seen = HashSet::new();
    for spec in FLAGS {
      assert!(seen.insert(spec.long), "duplicate {}", spec.long);
      if let Some(short) = spec.short {
        assert!(seen.insert(short), "duplicate {}", short);
      }
    }
  }

  #[test]
  fn test_help_documents_every_flag_and_command() {
    let help = render_help(false);
    for spec in FLAGS.iter().filter(|spec| spec.group != Group::Command) {
      assert!(help.contains(spec.long), "{} missing from help", spec.long);
      if let Some(short) = spec.short {
        assert!(
          help.contains(&format!("{}, {}", short, spec.long)),
          "{} missing from help",
          short
        );
      }
      assert!(!spec.help.is_empty(), "{} has no description", spec.long);
    }
    for command in COMMANDS {
      assert!(
        help.contains(&format!("\n  {}", command.name)),
        "{} missing from help",
        command.name
      );
    }
  }

  #[test]
  fn test_help_is_identical_apart_from_color() {
    let colored = render_help(true);
    let mut stripped = String::new();
    let mut rest = colored.as_str();
    while let Some(idx) = rest.find('\x1b') {
      stripped.push_str(&rest[..idx]);
      rest = &rest[idx..];
      rest = &rest[rest.find('m').unwrap() + 1..];
    }
    stripped.push_str(rest);
    assert_eq!(stripped, render_help(false));
  }

  #[test]
  fn test_help_lines_fit_the_width() {
    let width = mtindo::text::color::help_width();
    let pages = std::iter::once(render_help(false))
      .chain(COMMANDS.iter().map(|c| render_command_help(c, false)));
    for page in pages {
      for line in page.lines() {
        // A flag label or usage line may be wider; descriptions never are.
        let label = line.starts_with("  -") || line.starts_with("      --");
        let usage = line.starts_with("usage:") || line.starts_with("       ");
        if !label && !usage {
          assert!(line.chars().count() <= width, "too wide: {}", line);
        }
      }
    }
  }
}
