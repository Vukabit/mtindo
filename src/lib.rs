#![deny(unsafe_code)]

pub mod cache_alloc;
pub mod cli;
pub mod config;
pub mod doctor;
pub mod durable;
pub mod format;
#[doc(hidden)]
pub mod fuzzing;
pub mod history;
pub mod layout;
pub mod options;
pub mod order;
pub mod report;
pub mod syntax;
pub mod text;
pub mod verify;
pub mod version;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

pub use layout::import_group_of_text as classify_import;
pub use options::layout::{ImportGroup, ItemKind, LayoutConfig, Visibility};
pub use text::color::{HelpPalette, color_enabled};

/// The stack every thread that lays out or formats a file runs with. Those stages walk the
/// tree recursively, a few hundred bytes a level up to [`syntax::MAX_DEPTH`] levels, so the
/// stack must not be left to the platform: a thread gets 2 MB by default, and the main
/// thread 8 MB on Linux and macOS but 1 MB on Windows. Only what is used is taken from
/// memory.
pub const STACK: usize = 16 << 20;

/// How far [`bisect`] searches, in edits from each end. Its cost grows with the square of
/// that, so two long texts that share little would take minutes; past it, the range is
/// shown removed and added whole, a correct diff if not the shortest.
const MAX_EDITS: isize = 1024;

static UNIQUE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A file found by the walk, with the settings of its folder; `None` when neither layout nor
/// formatting is on there.
pub type Job = (PathBuf, Option<Arc<Settings>>);

enum DiffOp {
  Equal,
  Delete(usize),
  Insert(usize),
}

/// What a run produces, as the standard formatter's `--emit` says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Emit {
  /// Write each changed file.
  #[default]
  Files,
  /// Print each file, laid out, to stdout; write nothing.
  Stdout,
  /// Print the JSON report of the files that would change; write nothing.
  Json,
}

enum MismatchLine<'a> {
  Context(&'a str),
  /// A line of the new text (`+`).
  Expected(&'a str),
  /// A line of the original (`-`).
  Resulting(&'a str),
}

/// Errors that end a run. Each maps to a process exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MtoError {
  /// Bad arguments or configuration (exit code 2).
  Usage(String),
  /// A read or write failed (exit code 3).
  Io(String),
}

impl MtoError {
  pub fn exit_code(&self) -> i32 {
    match self {
      MtoError::Usage(_) => 2,
      MtoError::Io(_) => 3,
    }
  }
}

impl fmt::Display for MtoError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      MtoError::Usage(msg) | MtoError::Io(msg) => write!(f, "{}", msg),
    }
  }
}

impl From<config::Error> for MtoError {
  fn from(error: config::Error) -> MtoError {
    MtoError::Usage(error.message)
  }
}

/// What happened to one file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Outcome {
  #[default]
  Unchanged,
  Changed,
  /// Left untouched because it could not be split into items safely.
  Skipped,
  Failed,
  /// Left untouched because of a bug in mto: the verifier found the new text unsafe, or
  /// laying the file out failed.
  VerifierFailed,
  /// Left untouched because no `layout = true` applies to it.
  LayoutOff,
  /// Left untouched because it has syntax errors (exit code 3).
  SyntaxError,
  /// Left untouched by the git guard: it has uncommitted changes and history is off.
  Dirty,
}

/// What collecting the files of a run found.
#[derive(Debug, Default)]
pub struct Collected {
  /// The files to lay out, in order, each once.
  pub jobs: Vec<Job>,
  /// Modules whose files could not be found.
  pub problems: Vec<String>,
  /// Folders a walk passed over because they hold a repository of their own (`.git`).
  pub nested: Vec<PathBuf>,
  /// Temporary files a run cut short left behind in the folders walked ([`is_stale_temp`]),
  /// for a run that writes to remove.
  pub stale: Vec<PathBuf>,
}

/// How one job ended: its outcome, and its `--emit json` entry.
#[derive(Debug, Default)]
pub struct Done {
  pub outcome: Outcome,
  pub json: Option<String>,
  /// With the build guard checking the run: the file as read and as it is to be written.
  pub planned: Option<(String, String)>,
  /// The remembered holds found in the file, when it had any.
  pub holds: Option<Vec<doctor::holds::Kind>>,
}

/// What processing one file produced: its outcome, and the text it prints to stdout and
/// stderr.
#[derive(Debug, Default)]
pub struct FileReport {
  pub outcome: Outcome,
  pub stdout: String,
  pub stderr: String,
  /// The file's entry in `--emit json`'s report, when it would change.
  pub json: Option<String>,
  /// With the build guard checking the run: the file as read and as it is to be written.
  pub planned: Option<(String, String)>,
  /// The remembered holds found in the file, when it had any.
  pub holds: Option<Vec<doctor::holds::Kind>>,
}

impl FileReport {
  pub fn print(&self) {
    if !self.stdout.is_empty() {
      let _ = io::stdout().lock().write_all(self.stdout.as_bytes());
    }
    if !self.stderr.is_empty() {
      let _ = io::stderr().lock().write_all(self.stderr.as_bytes());
    }
  }

  fn with(mut self, outcome: Outcome) -> FileReport {
    self.outcome = outcome;
    self
  }
}

/// One changed region, as the standard formatter's diff groups them.
struct Mismatch<'a> {
  /// The first line shown, in the new text and in the original.
  line_number: u32,
  line_number_orig: u32,
  lines: Vec<MismatchLine<'a>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions {
  pub dry_run: bool,
  pub check: bool,
  pub debug: bool,
  pub follow_links: bool,
  /// `--emit`: write the files (default), print them, or report as JSON.
  pub emit: Emit,
  /// `-l`: name the files that change (or would) instead of showing how.
  pub files_with_diff: bool,
  /// `-q`: only errors and what was asked for.
  pub quiet: bool,
  /// `-v`: name each file as it is formatted.
  pub verbose: bool,
  /// `--skip-children`: a file argument is that file alone, not its modules.
  pub skip_children: bool,
  /// `--no-history`: do not record this run.
  pub no_history: bool,
  /// `--allow-dirty`: with history off, rewrite files with uncommitted changes.
  pub allow_dirty: bool,
  /// `--unguarded`: no build guard for this run.
  pub unguarded: bool,
}

/// How often a line id occurs in each side of a range, and where: patience diff's table,
/// indexed by line id and left all zero between uses.
#[derive(Clone, Copy, Default)]
struct Seen {
  in_a: u32,
  at_a: usize,
  in_b: u32,
  at_b: usize,
}

/// What mto does to a file: layout when `layout = true`, formatting when `format = true`.
#[derive(Clone, Debug, Default)]
pub struct Settings {
  pub layout: Option<LayoutConfig>,
  pub format: Option<options::format::FormatConfig>,
  /// The configured edition: files are parsed with it where they can be.
  pub edition: syntax::Edition,
}

impl Settings {
  /// The settings of a folder, or `None` when neither is on there.
  pub fn of(resolved: &config::Config) -> Option<Settings> {
    let settings = Settings {
      layout: resolved.layout_on().then(|| resolved.layout()),
      format: resolved.format_on().then(|| resolved.format()),
      edition: resolved.edition(),
    };
    (settings.layout.is_some() || settings.format.is_some()).then_some(settings)
  }
}

/// What mto makes of one file: the text layout gives, the text after formatting, and the
/// edits that went into them.
#[derive(Debug)]
pub struct Shaped {
  pub laid_out: String,
  pub text: String,
  pub edition: syntax::Edition,
  /// The file skips itself (`#![rustfmt::skip]`).
  pub skipped: bool,
  /// The units layout moved; those held back are not among them.
  pub moves: Vec<layout::Move>,
  /// The remembered holds found in the file, which the rest have lapsed against.
  pub found: Vec<doctor::holds::Kind>,
}

/// The state of a directory walk.
struct Walk<'a> {
  visited: &'a mut HashSet<PathBuf>,
  /// Takes each file as it is found.
  found: &'a mut dyn FnMut(Job),
  nested: &'a mut Vec<PathBuf>,
  stale: &'a mut Vec<PathBuf>,
}

/// What [`run_file`] needs to write a file: where its contents are recorded, and which files
/// the git guard keeps.
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteContext<'a> {
  pub history: Option<&'a history::History>,
  /// The run's record: a file is added to it before it is replaced, so a run cut short
  /// anywhere can be undone.
  pub recorder: Option<&'a Mutex<history::Recorder>>,
  pub dirty: Option<&'a HashSet<PathBuf>>,
  /// The edits the build guard holds back, remembered from earlier runs.
  pub holds: Option<&'a doctor::holds::Holds>,
  /// The build guard checks this run: the new text is kept in the report, not written.
  pub defer: bool,
  /// Where a bug found in mto is kept as a report; `None`: it is printed whole.
  pub reports: Option<&'a report::Store>,
}

/// What a run that writes files records and guards.
#[derive(Debug, Default)]
pub struct Writes {
  /// Records the run in history; `None` with history off.
  pub recorder: Option<history::Recorder>,
  /// With history off: the files (absolute, symlinks resolved) git sees uncommitted
  /// changes in, which are not rewritten. `None` when the guard is off.
  pub dirty: Option<HashSet<PathBuf>>,
  /// The edits the build guard holds back, remembered from earlier runs.
  pub holds: Option<doctor::holds::Holds>,
  /// The build guard checks this run: nothing is written yet, and each report keeps the
  /// text to write.
  pub defer: bool,
  /// Where a bug found in mto is kept as a report (`mto report`).
  pub reports: Option<report::Store>,
}

pub fn c_cyan() -> &'static str {
  if color_enabled(false) {
    "\x1b[1;36m"
  } else {
    ""
  }
}

pub fn c_green() -> &'static str {
  if color_enabled(false) {
    "\x1b[1;32m"
  } else {
    ""
  }
}

pub fn c_red() -> &'static str {
  if color_enabled(true) {
    "\x1b[1;31m"
  } else {
    ""
  }
}

pub fn c_reset(is_err: bool) -> &'static str {
  if color_enabled(is_err) { "\x1b[0m" } else { "" }
}

pub fn c_yellow(is_err: bool) -> &'static str {
  if color_enabled(is_err) {
    "\x1b[1;33m"
  } else {
    ""
  }
}

/// What `--check` prints for a file, exactly as the standard formatter prints it: `Diff in
/// FILE:LINE:` for each changed region, then its lines, with three lines of context.
/// `verbose` marks line ends with `⏎`, as the standard formatter's `-v` does.
pub fn check_diff(old: &str, new: &str, label: &str, verbose: bool) -> String {
  use std::fmt::Write as _;
  let end = if verbose { "⏎" } else { "" };
  let (green, reset) = (c_green(), c_reset(false));
  let red = if color_enabled(false) {
    "\x1b[1;31m"
  } else {
    ""
  };
  let mut out = String::new();
  for mismatch in mismatches(old, new, 3) {
    let _ = writeln!(out, "Diff in {}:{}:", label, mismatch.line_number_orig);
    for line in mismatch.lines {
      // Context lines take no colour, so no reset either.
      let (start, sign, text, stop) = match line {
        MismatchLine::Context(text) => ("", ' ', text, ""),
        MismatchLine::Expected(text) => (green, '+', text, reset),
        MismatchLine::Resulting(text) => (red, '-', text, reset),
      };
      out.push_str(start);
      out.push(sign);
      out.push_str(text);
      out.push_str(end);
      out.push_str(stop);
      out.push('\n');
    }
  }
  out
}

/// The files to lay out for `paths`, in order, each once: a folder is walked; a file brings
/// the modules it declares, as the standard formatter follows them, unless
/// `--skip-children`. A walk passes over folders below its start that hold a repository of
/// their own: another project, laid out only when named.
pub fn collect_jobs(
  paths: &[PathBuf],
  resolver: &mut config::Resolver,
  opts: &RunOptions,
) -> Result<Collected, MtoError> {
  let mut jobs = Vec::new();
  let mut collected = collect_jobs_into(paths, resolver, opts, &mut |job| jobs.push(job))?;
  collected.jobs = jobs;
  Ok(collected)
}

/// [`collect_jobs`], handing each file to `found` as it is found (the [`Collected`] it
/// gives holds no jobs): so that [`run_jobs_walking`] can lay files out while the walk
/// goes on.
pub fn collect_jobs_into(
  paths: &[PathBuf],
  resolver: &mut config::Resolver,
  opts: &RunOptions,
  found: &mut dyn FnMut(Job),
) -> Result<Collected, MtoError> {
  let mut visited = HashSet::new();
  let mut problems = Vec::new();
  let mut nested = Vec::new();
  let mut stale = Vec::new();
  for path in paths {
    if path.is_dir() {
      let real = real_path(path).unwrap_or_else(|_| path.to_path_buf());
      let mut walk = Walk {
        visited: &mut visited,
        found: &mut *found,
        nested: &mut nested,
        stale: &mut stale,
      };
      walk_inner(path, real, resolver, opts, &mut walk, true)?;
      continue;
    }
    let real = real_path(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = real.parent().unwrap_or(Path::new(".")).to_path_buf();
    let skip_children = opts.skip_children || resolver.config_for_absolute(dir)?.skip_children();
    let (files, skips) = if skip_children {
      (vec![path.clone()], Default::default())
    } else {
      let (files, missing) = cli::modules::module_files(path);
      problems.extend(missing);
      let text = fs::read_to_string(path).unwrap_or_default();
      (files, cli::modules::crate_skips(&text))
    };
    for file in files {
      let real = real_path(&file).unwrap_or_else(|_| file.clone());
      if !visited.insert(real.clone()) {
        continue;
      }
      let dir = real.parent().unwrap_or(Path::new(".")).to_path_buf();
      let resolved = resolver.config_for_absolute(dir)?;
      print_warnings(resolver);
      let mut settings = Settings::of(&resolved);
      if let Some(format) = settings.as_mut().and_then(|s| s.format.as_mut()) {
        format.crate_skipped_macros = skips.0.clone();
        format.crate_skipped_attributes = skips.1.clone();
      }
      found((file, settings.map(Arc::new)));
    }
  }
  Ok(Collected {
    jobs: Vec::new(),
    problems,
    nested,
    stale,
  })
}

/// `mto debug layout`: the file at `path` read and shaped with the settings that apply to
/// it, as a run would, and the places it is left as written, with why
/// ([`format::explain`]), and whether formatting is on there; nothing is written. `None`
/// when neither layout nor formatting is on there.
pub fn explain_file(
  resolver: &mut config::Resolver,
  path: &Path,
) -> Result<Option<(String, Vec<format::explain::Note>, bool)>, MtoError> {
  let source = fs::read_to_string(path)
    .map_err(|e| MtoError::Io(format!("cannot read {}: {}", path.display(), e)))?;
  let dir = match path.parent() {
    Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
    _ => env::current_dir()
      .map_err(|e| MtoError::Io(format!("cannot read the current directory: {}", e)))?,
  };
  let resolved = resolver.config_for(&dir)?;
  print_warnings(resolver);
  let Some(settings) = Settings::of(&resolved) else {
    return Ok(None);
  };
  let formatting = settings.format.is_some();
  let (shaped, notes) = format::explain::collect(|| {
    shape(&source, &settings, &doctor::Held::default(), None).map(|_| ())
  });
  if let Err(reason) = shaped {
    return Err(MtoError::Io(format!("{}: {}", path.display(), reason)));
  }
  Ok(Some((source, notes, formatting)))
}

/// The `mto.toml` that `mto init` writes: every mto setting at its default, with its
/// summary.
pub fn init_template() -> String {
  let mut out = String::from(
    "# mto configuration file; `mto --man` explains every setting.\n\n\
         # Rearrange the items of each file.\nlayout = true\n",
  );
  // `format` beside `layout`, the other switch; then the rest in table order.
  let switch = options::all().filter(|s| s.name == "format");
  let others = options::all()
    .filter(|s| s.file == options::File::Mto && !matches!(s.name, "layout" | "format"));
  for setting in switch.chain(others) {
    out.push_str(&format!(
      "\n# {}\n{} = {}\n",
      setting.summary, setting.name, setting.default
    ));
  }
  out
}

/// Whether `path` is a temporary file [`write_atomic`] made (`.NAME.PID.N.tmp_mtindo`) and a
/// run cut short left behind: one older than history's grace period, so that no run going on
/// at the same time is still writing it.
pub fn is_stale_temp(path: &Path) -> bool {
  let name = path.file_name().unwrap_or_default().to_string_lossy();
  name.starts_with('.')
    && name.ends_with(".tmp_mtindo")
    && fs::symlink_metadata(path)
      .ok()
      .filter(|m| m.is_file())
      .and_then(|m| m.modified().ok())
      .and_then(|m| m.elapsed().ok())
      .is_some_and(|age| age >= history::GRACE)
}

/// Reads the layout settings of one file, as `--config-path` does: no other file is searched
/// for.
pub fn parse_toml_config(path: &Path) -> Result<LayoutConfig, MtoError> {
  let overrides = config::Overrides {
    config_path: Some(path.to_path_buf()),
    ..config::Overrides::default()
  };
  let mut resolver = config::Resolver::new(&overrides, config::Environment::default())?;
  let dir = path.parent().unwrap_or(Path::new("."));
  let config = resolver.config_for(dir)?.layout();
  print_warnings(&mut resolver);
  Ok(config)
}

/// Prints the configuration warnings found so far to stderr.
pub fn print_warnings(resolver: &mut config::Resolver) {
  for warning in resolver.take_warnings() {
    eprintln!("{}warning: {}{}", c_yellow(true), warning, c_reset(true));
  }
}

pub fn process_init(target_dir: Option<PathBuf>) -> Result<PathBuf, MtoError> {
  let dir = target_dir.unwrap_or_else(|| PathBuf::from("."));
  if !dir.exists() {
    fs::create_dir_all(&dir)
      .map_err(|e| MtoError::Io(format!("cannot create directory {}: {}", dir.display(), e)))?;
  }
  let config_file_path = dir.join("mto.toml");
  if config_file_path.exists() {
    return Err(MtoError::Usage(format!(
      "configuration file already exists at {}",
      config_file_path.display()
    )));
  }
  let default_config_template = init_template();
  write_atomic(&config_file_path, &default_config_template)
    .map_err(|e| MtoError::Io(format!("cannot write configuration file: {}", e)))?;
  Ok(config_file_path)
}

/// Lays out the source on stdin and prints the result (or, with `--check`, the diff, or the
/// JSON report). `name` is `--stdin-filepath`: where the source belongs, for its settings
/// and in messages; without it, the current folder's settings apply.
pub fn process_stdin(
  resolver: &mut config::Resolver,
  opts: &RunOptions,
  name: Option<&Path>,
) -> Result<FileReport, MtoError> {
  let dir = match name.and_then(Path::parent) {
    Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
    _ => env::current_dir()
      .map_err(|e| MtoError::Io(format!("cannot read the current directory: {}", e)))?,
  };
  let shown = name.map_or("<stdin>".to_string(), |n| n.display().to_string());
  let resolved = resolver.config_for(&dir)?;
  print_warnings(resolver);
  let settings = Settings::of(&resolved);
  let reports = cli::report::store(resolver);
  let mut source = String::new();
  io::stdin()
    .read_to_string(&mut source)
    .map_err(|e| MtoError::Io(format!("cannot read stdin: {}", e)))?;
  let mut report = FileReport::default();
  let unchanged = |mut report: FileReport, outcome: Outcome| {
    if !opts.check && opts.emit != Emit::Json {
      report.stdout.push_str(&source);
    }
    report.with(outcome)
  };
  let Some(settings) = settings else {
    return Ok(unchanged(report, Outcome::LayoutOff));
  };
  let shaped = match caught(&shown, &source, &settings, || {
    shape(&source, &settings, &doctor::Held::default(), None)
  }) {
    Ok(shaped) => shaped.map(|s| (s.laid_out, s.text, s.edition)),
    Err(bug) => {
      report.stderr = deliver(&bug, reports.as_ref());
      return Ok(unchanged(report, Outcome::VerifierFailed));
    }
  };
  let (laid_out, reorganized, edition) = match shaped {
    Ok(arranged) => arranged,
    Err(reason) => {
      let syntax = reason == layout::SYNTAX_ERRORS;
      report.stderr = format!(
        "{}{} {}: {}{}\n",
        if syntax { c_red() } else { c_yellow(true) },
        if syntax { "Error in" } else { "Skipped" },
        shown,
        reason,
        c_reset(true)
      );
      let outcome = if syntax {
        Outcome::SyntaxError
      } else {
        Outcome::Skipped
      };
      return Ok(unchanged(report, outcome));
    }
  };
  if reorganized == source {
    return Ok(unchanged(report, Outcome::Unchanged));
  }
  if let Err(bug) | Ok(Err(bug)) = caught(&shown, &source, &settings, || {
    verified_all(&source, &laid_out, &reorganized, edition, &settings, &shown)
  }) {
    report.stderr = deliver(&bug, reports.as_ref());
    return Ok(unchanged(report, Outcome::VerifierFailed));
  }
  if opts.emit == Emit::Json {
    report.json = Some(json_report(&shown, &source, &reorganized));
  } else if opts.check {
    if opts.files_with_diff {
      report.stdout.push_str(&format!("{}\n", shown));
    } else {
      report
        .stdout
        .push_str(&check_diff(&source, &reorganized, &shown, opts.verbose));
    }
  } else {
    report.stdout.push_str(&reorganized);
  }
  Ok(report.with(Outcome::Changed))
}

/// `path` absolute with symlinks resolved, as [`fs::canonicalize`] gives it, but on Windows
/// without the `\\?\` prefix when the plain path names the same file: mto shows these paths,
/// and `\\?\C:\src` is not how anyone writes one.
pub fn real_path(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
  let real = fs::canonicalize(path)?;
  Ok(if cfg!(windows) {
    plain_path(&real).unwrap_or(real)
  } else {
    real
  })
}

/// Removes the stale temporary files `paths` names; how many went.
pub fn remove_stale(paths: &[PathBuf]) -> usize {
  paths
    .iter()
    .filter(|p| is_stale_temp(p) && fs::remove_file(p).is_ok())
    .count()
}

pub fn reorganize_file(source: String, config: &LayoutConfig, dbg: bool) -> String {
  match try_reorganize(&source, config, dbg) {
    Ok(text) => text,
    Err(_) => source,
  }
}

/// Lays out one file: reads, arranges, verifies, then shows the diff (`--check`), names the
/// file (`--dry-run`), or records and writes it. Nothing is printed; the report holds the
/// output, so files can be processed in parallel and still print in order.
pub fn run_file(
  file_path: &Path,
  settings: &Settings,
  opts: &RunOptions,
  ctx: WriteContext,
) -> FileReport {
  let mut report = FileReport::default();
  let shown = file_path.display().to_string();
  // Layout, formatting and the verifiers parse the same few texts; each is parsed once.
  let _parses = syntax::ParseCache::open();
  let source = match fs::read_to_string(file_path) {
    Ok(content) => content,
    Err(e) => {
      report.stderr = format!(
        "{}Error reading {}: {}{}\n",
        c_red(),
        shown,
        e,
        c_reset(true)
      );
      return report.with(Outcome::Failed);
    }
  };
  if opts.debug {
    report.stdout.push_str(&format!(
      "{}--- [DBG] Processing file: {} ---\n",
      c_cyan(),
      shown
    ));
  }
  let debug = opts.debug.then_some(&mut report.stdout);
  let held = ctx
    .holds
    .map(|holds| holds.held_for(&std::path::absolute(file_path).unwrap_or_default()))
    .unwrap_or_default();
  let shaped = match caught(&shown, &source, settings, || {
    shape(&source, settings, &held, debug)
  }) {
    Ok(shaped) => shaped,
    Err(bug) => {
      report.stderr = deliver(&bug, ctx.reports);
      return report.with(Outcome::VerifierFailed);
    }
  };
  let (laid_out, reorganized, edition) = match shaped {
    Ok(Shaped {
      laid_out,
      text,
      edition,
      skipped,
      found,
      ..
    }) => {
      report.holds = (!held.is_empty()).then_some(found);
      // Named as the standard formatter names them: not the files that tell it to skip
      // them.
      if opts.verbose && !skipped {
        report
          .stdout
          .insert_str(0, &format!("Formatting {}\n", shown));
      }
      (laid_out, text, edition)
    }
    Err(reason) => {
      if opts.verbose {
        report
          .stdout
          .insert_str(0, &format!("Formatting {}\n", shown));
      }
      let syntax = reason == layout::SYNTAX_ERRORS;
      report.stderr = format!(
        "{}{} {}: {}{}\n",
        if syntax { c_red() } else { c_yellow(true) },
        if syntax { "Error in" } else { "Skipped" },
        shown,
        reason,
        c_reset(true)
      );
      let outcome = if syntax {
        Outcome::SyntaxError
      } else {
        Outcome::Skipped
      };
      if opts.emit == Emit::Stdout {
        emit_stdout(&mut report, &shown, &source, opts);
      }
      return report.with(outcome);
    }
  };
  if reorganized == source {
    if opts.emit == Emit::Stdout {
      emit_stdout(&mut report, &shown, &source, opts);
    }
    return report.with(Outcome::Unchanged);
  }
  if let Err(bug) | Ok(Err(bug)) = caught(&shown, &source, settings, || {
    verified_all(&source, &laid_out, &reorganized, edition, settings, &shown)
  }) {
    report.stderr = deliver(&bug, ctx.reports);
    return report.with(Outcome::VerifierFailed);
  }
  match opts.emit {
    Emit::Stdout => {
      emit_stdout(&mut report, &shown, &reorganized, opts);
      return report.with(Outcome::Changed);
    }
    Emit::Json => {
      report.json = Some(json_report(&shown, &source, &reorganized));
      return report.with(Outcome::Changed);
    }
    Emit::Files => {}
  }
  if opts.check {
    if opts.files_with_diff {
      report.stdout.push_str(&format!("{}\n", shown));
    } else {
      report
        .stdout
        .push_str(&check_diff(&source, &reorganized, &shown, opts.verbose));
    }
    return report.with(Outcome::Changed);
  }
  if opts.dry_run {
    report.stdout.push_str(&format!(
      "{}[DRY RUN]{} Would format: {}\n",
      c_cyan(),
      c_reset(false),
      shown
    ));
    return report.with(Outcome::Changed);
  }
  if let Some(dirty) = ctx.dirty {
    let real = real_path(file_path).unwrap_or_else(|_| file_path.to_path_buf());
    if dirty.contains(&real) {
      report.stderr = format!(
        "{}Left {} alone: it has uncommitted changes and history is off.{}\n",
        c_yellow(true),
        shown,
        c_reset(true)
      );
      return report.with(Outcome::Dirty);
    }
  }
  if ctx.defer {
    report.planned = Some((source, reorganized));
    return report.with(Outcome::Changed);
  }
  // The contents are stored, and synced, before the file is replaced: what history
  // records, it can give back.
  let mut written = None;
  if let Some(history) = ctx.history {
    // Only the contents before are kept: what the run wrote is known by its hash, and
    // nothing is ever put back from it.
    let stored = history
      .store(source.as_bytes())
      .map(|before| (before, history::sha256::hex(reorganized.as_bytes())));
    match stored {
      Ok((before, after)) => {
        written = Some(history::Entry {
          path: std::path::absolute(file_path).unwrap_or_else(|_| file_path.into()),
          before,
          after,
        });
      }
      Err(e) => {
        report.stderr = format!(
          "{}Left {} alone: cannot record it in history ({}): {}; --no-history \
                     writes without a record{}\n",
          c_red(),
          shown,
          history.dir().display(),
          e,
          c_reset(true)
        );
        return report.with(Outcome::Failed);
      }
    }
  }
  if let (Some(recorder), Some(entry)) = (ctx.recorder, &written) {
    let mut recorder = recorder.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = recorder.record(entry).and_then(|_| recorder.order()) {
      report.stderr = format!(
        "{}Left {} alone: cannot add it to the history record: {}{}\n",
        c_red(),
        shown,
        e,
        c_reset(true)
      );
      return report.with(Outcome::Failed);
    }
  }
  if let Err(e) = write_atomic(file_path, &reorganized) {
    report.stderr = format!(
      "{}Cannot write {}: {}; it is unchanged{}\n",
      c_red(),
      shown,
      e,
      c_reset(true)
    );
    return report.with(Outcome::Failed);
  }
  if opts.files_with_diff {
    report.stdout.push_str(&format!("{}\n", shown));
    return report.with(Outcome::Changed);
  }
  if opts.quiet {
    return report.with(Outcome::Changed);
  }
  report.stdout.push_str(&format!(
    "{}Formatted:{} {}\n",
    c_green(),
    c_reset(false),
    shown
  ));
  report.with(Outcome::Changed)
}

/// Runs every job on a pool of threads, printing each report as soon as every report
/// before it has been printed, so the output is the same as one thread's.
/// Each file written is added to the run's history record before it is replaced.
pub fn run_jobs(jobs: &[Job], opts: &RunOptions, writes: &mut Writes) -> Vec<Done> {
  // No more workers than files: a run of one file starts one.
  let (_, _, done) = walking(
    opts,
    writes,
    jobs.len().max(1),
    |found| {
      for job in jobs {
        found(job.clone());
      }
    },
    |_| true,
  );
  done
}

thread_local! {
  /// Set while [`quietly`] runs: a panic is kept here instead of printed, with its
  /// backtrace when one is asked for (taking one costs: only the first failure needs it).
  static QUIET: std::cell::RefCell<Option<(String, Option<String>, bool)>> =
    const { std::cell::RefCell::new(None) };
}

/// [`run_jobs`] while `walk` is still finding the jobs: it hands each to the function it is
/// given, and the workers lay each out as soon as it is found. Once `walk` returns,
/// `before_results` gets what it returned (to print what the walk noted, before any
/// file's report) and says whether the reports are wanted; if so they print in the order
/// the files were found. Gives the jobs, what `walk` returned, and how each job ended
/// (none when the reports were not wanted).
///
/// Files are laid out before the walk is known to succeed, so only a run that writes
/// nothing walks this way.
pub fn run_jobs_walking<T>(
  opts: &RunOptions,
  writes: &mut Writes,
  walk: impl FnOnce(&mut dyn FnMut(Job)) -> T,
  before_results: impl FnOnce(&T) -> bool,
) -> (Vec<Job>, T, Vec<Done>) {
  walking(opts, writes, usize::MAX, walk, before_results)
}

/// Lays out and formats `source` as `settings` say, holding back the edits `held` names
/// (none for an ordinary run). `Err` with the reason when the file is left alone.
pub fn shape(
  source: &str,
  settings: &Settings,
  held: &doctor::Held,
  mut debug: Option<&mut String>,
) -> Result<Shaped, String> {
  let mut found = Vec::new();
  if held.all {
    let (edition, parsed) = layout::parse_any_edition(source, settings.edition);
    if let Some(reason) = layout::refusal(&parsed) {
      return Err(reason);
    }
    return Ok(Shaped {
      laid_out: source.to_string(),
      text: source.to_string(),
      edition,
      skipped: false,
      moves: Vec::new(),
      found,
    });
  }
  let (laid_out, edition, skipped, moves) = match &settings.layout {
    Some(config) => {
      let mut starts = held.moves.clone();
      if !held.move_ids.is_empty() {
        // Remembered holds name units by content: found among the moves of a first pass.
        let (_, _, _, free) = arrange_holding(source, config, &starts, None)?;
        for moved in &free {
          let id = doctor::move_id(source, moved);
          if held.move_ids.contains(&id) {
            starts.insert(moved.span.0);
            found.push(doctor::holds::Kind::Move(id));
          }
        }
      }
      arrange_holding(source, config, &starts, debug.take())?
    }
    None if source.trim_start_matches('\u{feff}').trim().is_empty() => (
      source.to_string(),
      syntax::Edition::E2021,
      false,
      Vec::new(),
    ),
    None => {
      let (edition, parsed) = layout::parse_any_edition(source, settings.edition);
      if let Some(reason) = layout::refusal(&parsed) {
        return Err(reason);
      }
      (source.to_string(), edition, false, Vec::new())
    }
  };
  let mut text = match &settings.format {
    Some(config) => format::file::format_file(&laid_out, edition, config),
    None => laid_out.clone(),
  };
  if !held.lines.is_empty() && text != laid_out {
    let hunks = doctor::hunks(&laid_out, &text);
    for hunk in &hunks {
      if held.lines.contains(&hunk.id) {
        found.push(doctor::holds::Kind::Lines(hunk.id.clone()));
      }
    }
    text = doctor::apply(&laid_out, &text, &hunks, |h| !held.lines.contains(&h.id));
  }
  Ok(Shaped {
    laid_out,
    text,
    edition,
    skipped,
    moves,
    found,
  })
}

/// Reorders the items of one source file, or explains why the file was left alone.
pub fn try_reorganize(source: &str, config: &LayoutConfig, dbg: bool) -> Result<String, String> {
  let mut log = String::new();
  let result = arrange(source, config, dbg.then_some(&mut log)).map(|(text, _, _)| text);
  print!("{}", log);
  result
}

/// Lays out every `.rs` file under `dir`, each with the settings of its own folder. Files
/// are processed in parallel and their output printed in walk order. Nothing is recorded
/// in history.
pub fn walk_directory(
  dir: &Path,
  resolver: &mut config::Resolver,
  opts: &RunOptions,
) -> Result<Vec<Outcome>, MtoError> {
  let jobs = collect_jobs(&[dir.to_path_buf()], resolver, opts)?.jobs;
  Ok(
    run_jobs(&jobs, opts, &mut Writes::default())
      .into_iter()
      .map(|d| d.outcome)
      .collect(),
  )
}

/// Runs `work` on a thread of its own with the stack [`STACK`], and gives what it returns.
/// A panic in `work` goes on in the caller.
pub fn with_stack<T: Send>(work: impl FnOnce() -> T + Send) -> T {
  thread::scope(|scope| {
    let worker = thread::Builder::new()
      .stack_size(STACK)
      .spawn_scoped(scope, work)
      .expect("a thread to run on");
    worker
      .join()
      .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
  })
}

/// Replaces `target` atomically: a sibling temp file is written, synced and renamed over it.
/// A symlinked target keeps its link and its pointee is replaced; permissions are kept. A
/// read-only target is left as it is, on every system (Windows would refuse to replace it,
/// Unix would not).
pub fn write_atomic(target: &Path, content: &str) -> Result<(), std::io::Error> {
  let real_target = match fs::symlink_metadata(target) {
    Ok(meta) if meta.file_type().is_symlink() => real_path(target)?,
    _ => target.to_path_buf(),
  };
  if fs::metadata(&real_target).is_ok_and(|meta| meta.permissions().readonly()) {
    return Err(std::io::Error::new(
      std::io::ErrorKind::PermissionDenied,
      "the file is read-only",
    ));
  }
  let parent = match real_target.parent() {
    Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
    _ => PathBuf::from("."),
  };
  let file_name = real_target
    .file_name()
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_default();
  let unique = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
  let tmp_path = parent.join(format!(
    ".{}.{}.{}.tmp_mtindo",
    file_name,
    std::process::id(),
    unique
  ));
  let result = (|| -> Result<(), std::io::Error> {
    let mut file = File::create(&tmp_path)?;
    file.write_all(content.as_bytes())?;
    if let Ok(meta) = fs::metadata(&real_target) {
      fs::set_permissions(&tmp_path, meta.permissions())?;
    }
    durable::ordered(&file, &parent)?;
    drop(file);
    rename(&tmp_path, &real_target)?;
    // The file is replaced either way; a folder that cannot be synced is no reason to say
    // it is not.
    let _ = durable::renamed(&parent);
    Ok(())
  })();
  if result.is_err() {
    let _ = fs::remove_file(&tmp_path);
  }
  result
}

/// The arranged text and the edition the file was parsed with.
fn arrange(
  source: &str,
  config: &LayoutConfig,
  debug: Option<&mut String>,
) -> Result<(String, syntax::Edition, bool), String> {
  arrange_holding(source, config, &HashSet::new(), debug)
    .map(|(text, edition, skipped, _)| (text, edition, skipped))
}

/// [`arrange`], keeping the units that start at `held` where they are; also gives the
/// moves made.
fn arrange_holding(
  source: &str,
  config: &LayoutConfig,
  held: &HashSet<usize>,
  debug: Option<&mut String>,
) -> Result<(String, syntax::Edition, bool, Vec<layout::Move>), String> {
  if source.trim_start_matches('\u{feff}').trim().is_empty() {
    return Ok((
      source.to_string(),
      syntax::Edition::E2021,
      false,
      Vec::new(),
    ));
  }
  let arranged = layout::layout_holding(source, config, held)?;
  if let Some(out) = debug {
    out.push_str(&format!(
      "{}--- [DBG] Extracted {} blocks from source ---\n",
      c_cyan(),
      arranged.before.len()
    ));
    for (idx, unit) in arranged.before.iter().enumerate() {
      out.push_str(&format!(
        "  [Block #{}] Kind: {:?}, Visibility: {:?}, Identifier: '{}', Parent Target: '{}'\n",
        idx, unit.kind, unit.visibility, unit.identifier, unit.parent_target
      ));
    }
    out.push_str(&format!("{}--- [DBG] Blocks after sorting ---\n", c_cyan()));
    for (idx, unit) in arranged.after.iter().enumerate() {
      out.push_str(&format!(
        "  [Sorted #{}] Kind: {:?}, Identifier: '{}'\n",
        idx, unit.kind, unit.identifier
      ));
    }
  }
  let mut text = arranged.text;
  // Debug builds only: lets the tests prove the verifier stops a bad write, now that the
  // engine has no known way to produce one, and that a bug failing on one file stops only
  // that file.
  // With `panic`, a file holding `inject_panic` fails instead, as a bug in mto would.
  if cfg!(debug_assertions)
    && let Some(fault) = env::var_os("MTINDO_INJECT_FAULT")
  {
    if fault != "panic" {
      text.push_str("// injected by MTINDO_INJECT_FAULT\n");
    } else if source.contains("inject_panic") {
      panic!("injected by MTINDO_INJECT_FAULT");
    }
  }
  Ok((text, arranged.edition, arranged.skipped, arranged.moves))
}

/// Where the forward and reverse searches of Myers' algorithm meet: a point on a shortest
/// edit path, to split the problem at. `None` if the texts share nothing, or too little to
/// find within [`MAX_EDITS`].
fn bisect(a: &[u32], b: &[u32]) -> Option<(usize, usize)> {
  let (n, m) = (a.len() as isize, b.len() as isize);
  let max_d = ((n + m + 1) / 2).min(MAX_EDITS);
  let offset = max_d;
  let width = (2 * max_d) as usize + 2;
  let mut forward = vec![-1isize; width];
  let mut reverse = vec![-1isize; width];
  forward[(offset + 1) as usize] = 0;
  reverse[(offset + 1) as usize] = 0;
  let delta = n - m;
  let front = delta % 2 != 0;
  let (mut k1_start, mut k1_end, mut k2_start, mut k2_end) = (0, 0, 0, 0);
  for d in 0..max_d {
    let mut k1 = -d + k1_start;
    while k1 <= d - k1_end {
      let at = (offset + k1) as usize;
      let mut x1 = if k1 == -d || (k1 != d && forward[at - 1] < forward[at + 1]) {
        forward[at + 1]
      } else {
        forward[at - 1] + 1
      };
      let mut y1 = x1 - k1;
      while x1 < n && y1 < m && a[x1 as usize] == b[y1 as usize] {
        x1 += 1;
        y1 += 1;
      }
      forward[at] = x1;
      if x1 > n {
        k1_end += 2;
      } else if y1 > m {
        k1_start += 2;
      } else if front {
        let other = offset + delta - k1;
        if other >= 0 && (other as usize) < width && reverse[other as usize] != -1 {
          let x2 = n - reverse[other as usize];
          if x1 >= x2 {
            return Some((x1 as usize, y1 as usize));
          }
        }
      }
      k1 += 2;
    }
    let mut k2 = -d + k2_start;
    while k2 <= d - k2_end {
      let at = (offset + k2) as usize;
      let mut x2 = if k2 == -d || (k2 != d && reverse[at - 1] < reverse[at + 1]) {
        reverse[at + 1]
      } else {
        reverse[at - 1] + 1
      };
      let mut y2 = x2 - k2;
      while x2 < n && y2 < m && a[(n - x2 - 1) as usize] == b[(m - y2 - 1) as usize] {
        x2 += 1;
        y2 += 1;
      }
      reverse[at] = x2;
      if x2 > n {
        k2_end += 2;
      } else if y2 > m {
        k2_start += 2;
      } else if !front {
        let other = offset + delta - k2;
        if other >= 0 && (other as usize) < width && forward[other as usize] != -1 {
          let x1 = forward[other as usize];
          let y1 = offset + x1 - other;
          if x1 >= n - x2 {
            return Some((x1 as usize, y1 as usize));
          }
        }
      }
      k2 += 2;
    }
  }
  None
}

/// A bug found on `shown` (read as `source`, laid out with `settings`).
fn bug(
  shown: &str,
  source: &str,
  settings: &Settings,
  failure: report::Failure,
  minimal: Option<String>,
  backtrace: Option<String>,
) -> report::Bug {
  // The file named as shown, from the current folder unless it is stdin.
  let path = (shown != "<stdin>")
    .then(|| std::path::absolute(shown).ok())
    .flatten();
  report::Bug {
    failure,
    file: shown.to_string(),
    path,
    minimal,
    backtrace,
    source_hash: history::sha256::hex(source.as_bytes()),
    settings: settings_text(settings),
    command: cli::history::command_line(),
  }
}

/// Runs one step of laying out `shown` (shaping it, verifying it), turning a panic into a
/// bug report, as the verifier makes one: nothing has been written yet, so the file is left
/// as it was and the rest of the run goes on. The report holds the smallest part of
/// `source` that still fails with `settings`.
fn caught<T>(
  shown: &str,
  source: &str,
  settings: &Settings,
  work: impl FnOnce() -> T,
) -> Result<T, Box<report::Bug>> {
  let (said, backtrace) = match quietly(true, work) {
    Ok(done) => return Ok(done),
    Err(failed) => failed,
  };
  let fails = |candidate: &str| {
    quietly(false, || {
      let _parses = syntax::ParseCache::open();
      if let Ok(shaped) = shape(candidate, settings, &doctor::Held::default(), None) {
        let _ = verified_all(
          candidate,
          &shaped.laid_out,
          &shaped.text,
          shaped.edition,
          settings,
          shown,
        );
      }
    })
    .is_err()
  };
  let minimal = fails(source).then(|| {
    let (edition, _) = layout::parse_any_edition(source, settings.edition);
    verify::report::minimize(source, edition, &fails)
  });
  Err(Box::new(bug(
    shown,
    source,
    settings,
    report::Failure::Panicked { said },
    minimal,
    backtrace,
  )))
}

/// What the terminal shows for `bug`: one line naming the report it is kept as, or the
/// whole report when reports are not kept (or keeping it failed).
fn deliver(bug: &report::Bug, reports: Option<&report::Store>) -> String {
  let mut note = String::new();
  if let Some(store) = reports {
    match store.save(bug) {
      Ok(id) => {
        let id = &id[..6];
        return format!(
          "{}mto hit a bug on {}; saved as report {}: run `mto report show {}`{}\n",
          c_red(),
          bug.file,
          id,
          id,
          c_reset(true)
        );
      }
      Err(e) => {
        note = format!(
          "(cannot keep this report in {}: {})\n",
          store.dir().display(),
          e
        )
      }
    }
  }
  format!("{}{}{}{}", c_red(), bug.text(), note, c_reset(true))
}

/// The edit script turning `a` into `b`, line by line: Myers' algorithm in linear space (the
/// bisection of diff-match-patch), on line ids so that lines compare as integers.
fn diff_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<DiffOp> {
  // The lines the texts share at either end are kept without being hashed; only the
  // middle gets ids.
  let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
  let suffix = a[prefix..]
    .iter()
    .rev()
    .zip(b[prefix..].iter().rev())
    .take_while(|(x, y)| x == y)
    .count();
  let (a_mid, b_mid) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
  let mut ids: HashMap<&'a str, u32> = HashMap::with_capacity(a_mid.len() + b_mid.len());
  let mut id = |line: &'a str| {
    let next = ids.len() as u32;
    *ids.entry(line).or_insert(next)
  };
  let a_ids: Vec<u32> = a_mid.iter().map(|l| id(l)).collect();
  let b_ids: Vec<u32> = b_mid.iter().map(|l| id(l)).collect();
  let mut seen = vec![Seen::default(); ids.len()];
  let mut ops = Vec::with_capacity(a.len().max(b.len()));
  ops.extend((0..prefix).map(|_| DiffOp::Equal));
  diff_range(&a_ids, prefix, &b_ids, prefix, &mut seen, &mut ops);
  ops.extend((0..suffix).map(|_| DiffOp::Equal));
  ops
}

/// Appends the edits turning `a` into `b`, whose lines start at `a_at` and `b_at`.
fn diff_range(
  a: &[u32],
  a_at: usize,
  b: &[u32],
  b_at: usize,
  seen: &mut [Seen],
  ops: &mut Vec<DiffOp>,
) {
  let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
  let suffix = a[prefix..]
    .iter()
    .rev()
    .zip(b[prefix..].iter().rev())
    .take_while(|(x, y)| x == y)
    .count();
  ops.extend((0..prefix).map(|_| DiffOp::Equal));
  let (a_mid, b_mid) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
  let (a_at, b_at) = (a_at + prefix, b_at + prefix);
  if a_mid.is_empty() {
    ops.extend((0..b_mid.len()).map(|j| DiffOp::Insert(b_at + j)));
  } else if b_mid.is_empty() {
    ops.extend((0..a_mid.len()).map(|i| DiffOp::Delete(a_at + i)));
  } else if let Some(anchors) = patience_anchors(a_mid, b_mid, seen) {
    // Lines found once on each side, in the same order on both, are kept; the gaps
    // between them are diffed on their own.
    let (mut i, mut j) = (0, 0);
    for (x, y) in anchors {
      diff_range(&a_mid[i..x], a_at + i, &b_mid[j..y], b_at + j, seen, ops);
      ops.push(DiffOp::Equal);
      (i, j) = (x + 1, y + 1);
    }
    diff_range(&a_mid[i..], a_at + i, &b_mid[j..], b_at + j, seen, ops);
  } else {
    match bisect(a_mid, b_mid) {
      Some((x, y)) => {
        diff_range(&a_mid[..x], a_at, &b_mid[..y], b_at, seen, ops);
        diff_range(&a_mid[x..], a_at + x, &b_mid[y..], b_at + y, seen, ops);
      }
      None => {
        ops.extend((0..a_mid.len()).map(|i| DiffOp::Delete(a_at + i)));
        ops.extend((0..b_mid.len()).map(|j| DiffOp::Insert(b_at + j)));
      }
    }
  }
  ops.extend((0..suffix).map(|_| DiffOp::Equal));
}

/// `--emit stdout`: the file's name, as the standard formatter prints it, then its text.
fn emit_stdout(report: &mut FileReport, shown: &str, text: &str, opts: &RunOptions) {
  if !opts.quiet {
    report.stdout.push_str(&format!("{}:\n\n", shown));
  }
  report.stdout.push_str(text);
}

fn is_symlink(entry: &fs::DirEntry) -> bool {
  entry.file_type().is_ok_and(|t| t.is_symlink())
}

/// A file's entry in `--emit json`, as the standard formatter writes it: its name and each
/// changed block, with line ranges in the original and the new text.
fn json_report(name: &str, old: &str, new: &str) -> String {
  let mut blocks = Vec::new();
  for mismatch in mismatches(old, new, 0) {
    let (original_begin, expected_begin) = (mismatch.line_number_orig, mismatch.line_number);
    let (mut original_end, mut expected_end) = (original_begin, expected_begin);
    let (mut original_count, mut expected_count) = (0, 0);
    let (mut original, mut expected) = (String::new(), String::new());
    for line in mismatch.lines {
      match line {
        MismatchLine::Expected(text) => {
          expected_end = expected_begin + expected_count;
          expected_count += 1;
          expected.push_str(text);
          expected.push('\n');
        }
        MismatchLine::Resulting(text) => {
          original_end = original_begin + original_count;
          original_count += 1;
          original.push_str(text);
          original.push('\n');
        }
        MismatchLine::Context(_) => {}
      }
    }
    blocks.push(format!(
            "{{\"original_begin_line\":{},\"original_end_line\":{},\"expected_begin_line\":{},\"expected_end_line\":{},\"original\":{},\"expected\":{}}}",
            original_begin,
            original_end,
            expected_begin,
            expected_end,
            json_string(&original),
            json_string(&expected)
        ));
  }
  format!(
    "{{\"name\":{},\"mismatches\":[{}]}}",
    json_string(name),
    blocks.join(",")
  )
}

/// A JSON string, escaped as serde_json escapes it.
fn json_string(text: &str) -> String {
  let mut out = String::from("\"");
  for ch in text.chars() {
    match ch {
      '"' => out.push_str("\\\""),
      '\\' => out.push_str("\\\\"),
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      '\u{8}' => out.push_str("\\b"),
      '\u{c}' => out.push_str("\\f"),
      c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
      c => out.push(c),
    }
  }
  out.push('"');
  out
}

/// The changed regions between `old` and `new` with `context` lines around each: a port of
/// The standard formatter's diff regions, over mto's own line diff. Lines are split as its
/// diff splits them, so a final line break leaves an empty last line.
fn mismatches<'a>(old: &'a str, new: &'a str, context: usize) -> Vec<Mismatch<'a>> {
  let a: Vec<&str> = old.split('\n').collect();
  let b: Vec<&str> = new.split('\n').collect();
  let (mut line_number, mut line_number_orig) = (1u32, 1u32);
  let mut queue: std::collections::VecDeque<&str> = std::collections::VecDeque::new();
  let mut since = context + 1;
  let mut results = Vec::new();
  let mut current = Mismatch {
    line_number: 0,
    line_number_orig: 0,
    lines: Vec::new(),
  };
  let (mut i, mut j) = (0usize, 0usize);
  for op in diff_ops(&a, &b) {
    let change = match op {
      DiffOp::Equal => None,
      DiffOp::Delete(at) => Some(MismatchLine::Resulting(a[at])),
      DiffOp::Insert(at) => Some(MismatchLine::Expected(b[at])),
    };
    match change {
      Some(line) => {
        if since >= context && since > 0 {
          results.push(std::mem::replace(
            &mut current,
            Mismatch {
              line_number: line_number - queue.len() as u32,
              line_number_orig: line_number_orig - queue.len() as u32,
              lines: Vec::new(),
            },
          ));
        }
        while let Some(line) = queue.pop_front() {
          current.lines.push(MismatchLine::Context(line));
        }
        match line {
          MismatchLine::Resulting(_) => {
            line_number_orig += 1;
            i += 1;
          }
          _ => {
            line_number += 1;
            j += 1;
          }
        }
        current.lines.push(line);
        since = 0;
      }
      None => {
        let line = a[i];
        if queue.len() >= context {
          queue.pop_front();
        }
        if since < context {
          current.lines.push(MismatchLine::Context(line));
        } else if context > 0 {
          queue.push_back(line);
        }
        line_number += 1;
        line_number_orig += 1;
        since += 1;
        i += 1;
        j += 1;
      }
    }
  }
  let _ = j;
  results.push(current);
  results.remove(0);
  results
}

/// Patience diff's anchors: the lines that occur exactly once in `a` and once in `b`, as
/// (index in a, index in b), keeping the longest run that is in order on both sides.
/// `None` when there are none. A text that was only reordered is mostly such lines, which
/// Myers' algorithm would take quadratic time over.
fn patience_anchors(a: &[u32], b: &[u32], seen: &mut [Seen]) -> Option<Vec<(usize, usize)>> {
  for (i, &line) in a.iter().enumerate() {
    let entry = &mut seen[line as usize];
    if entry.in_a == 0 {
      entry.at_a = i;
    }
    entry.in_a += 1;
  }
  for (j, &line) in b.iter().enumerate() {
    let entry = &mut seen[line as usize];
    if entry.in_a > 0 {
      entry.in_b += 1;
      entry.at_b = j;
    }
  }
  let mut unique: Vec<(usize, usize)> = Vec::new();
  for &line in a {
    let entry = seen[line as usize];
    if entry.in_a == 1 && entry.in_b == 1 {
      unique.push((entry.at_a, entry.at_b));
    }
    seen[line as usize] = Seen::default();
  }
  if unique.is_empty() {
    return None;
  }
  unique.sort_unstable();
  // The longest increasing run of b indexes (patience sorting).
  let mut tails: Vec<usize> = Vec::new();
  let mut before: Vec<Option<usize>> = vec![None; unique.len()];
  for (k, &(_, j)) in unique.iter().enumerate() {
    let pile = tails.partition_point(|&t| unique[t].1 < j);
    before[k] = pile.checked_sub(1).map(|p| tails[p]);
    if pile == tails.len() {
      tails.push(k);
    } else {
      tails[pile] = k;
    }
  }
  let mut run = Vec::with_capacity(tails.len());
  let mut at = tails.last().copied();
  while let Some(k) = at {
    run.push(unique[k]);
    at = before[k];
  }
  run.reverse();
  Some(run)
}

/// A verbatim Windows path (`\\?\C:\a`, `\\?\UNC\server\share\a`) as a plain one, when
/// it reads the same plain: short enough for the old limit, and no part a device name or
/// ending in a dot or space, which the plain form would drop or read otherwise.
fn plain_path(path: &Path) -> Option<PathBuf> {
  let text = path.to_str()?;
  let rest = text.strip_prefix(r"\\?\")?;
  let plain = if let Some(unc) = rest.strip_prefix(r"UNC\") {
    format!(r"\\{}", unc)
  } else {
    let bytes = rest.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":\\" {
      return None;
    }
    rest.to_string()
  };
  const DEVICES: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
  let safe = |part: &str| {
    let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
    let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
      && stem.len() == 4
      && stem.as_bytes()[3].is_ascii_digit();
    !part.ends_with('.') && !part.ends_with(' ') && !DEVICES.contains(&stem.as_str()) && !numbered
  };
  (plain.len() < 260 && plain.split('\\').skip(1).all(|p| p.is_empty() || safe(p)))
    .then(|| PathBuf::from(plain))
}

/// Runs `work`; a panic in it is caught and given as what it said and where (and, with
/// `backtrace`, the backtrace), without the standard message on stderr.
fn quietly<T>(backtrace: bool, work: impl FnOnce() -> T) -> Result<T, (String, Option<String>)> {
  static HOOK: std::sync::Once = std::sync::Once::new();
  HOOK.call_once(|| {
    let standard = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
      let kept = QUIET.with(|quiet| match quiet.borrow_mut().as_mut() {
        Some((said, trace, wanted)) => {
          let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
          *said = match info.location() {
            Some(at) => format!("{} (at {}:{})", message, at.file(), at.line()),
            None => message,
          };
          if *wanted {
            *trace = Some(std::backtrace::Backtrace::force_capture().to_string());
          }
          true
        }
        None => false,
      });
      if !kept {
        standard(info);
      }
    }));
  });
  let outer = QUIET.with(|quiet| quiet.replace(Some((String::new(), None, backtrace))));
  let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
  let (said, trace, _) = QUIET.with(|quiet| quiet.replace(outer)).unwrap_or_default();
  result.map_err(|_| (said, trace))
}

/// [`fs::rename`], tried again for a while on Windows when another program has the file
/// open: a virus scanner or the search indexer often does, for a moment, after a write.
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
  // ERROR_ACCESS_DENIED and ERROR_SHARING_VIOLATION.
  let busy = |e: &std::io::Error| cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32));
  let mut wait = std::time::Duration::from_millis(10);
  for _ in 0..7 {
    match fs::rename(from, to) {
      Err(e) if busy(&e) => std::thread::sleep(wait),
      done => return done,
    }
    wait *= 2;
  }
  fs::rename(from, to)
}

/// The settings that differ from the defaults, one per line, for a bug report.
fn settings_text(settings: &Settings) -> String {
  let differing = |name: &str, value: String, default: String| -> String {
    let defaults: HashSet<&str> = default.lines().collect();
    value
      .lines()
      .filter(|line| !defaults.contains(line))
      .map(|line| format!("{}: {}\n", name, line.trim().trim_end_matches(',')))
      .collect()
  };
  let mut out = String::new();
  match &settings.layout {
    None => out.push_str("layout: off\n"),
    Some(config) => out.push_str(&differing(
      "layout",
      format!("{:#?}", config),
      format!("{:#?}", LayoutConfig::default()),
    )),
  }
  match &settings.format {
    None => out.push_str("format: off\n"),
    Some(config) => out.push_str(&differing(
      "format",
      format!("{:#?}", config),
      format!("{:#?}", options::format::FormatConfig::default()),
    )),
  }
  out
}

/// Checks layout's new text with the verifier before anything is written or shown. On a
/// violation, the rules broken and the smallest part of the input that still breaks them.
fn verified(
  source: &str,
  output: &str,
  edition: syntax::Edition,
  config: &LayoutConfig,
) -> Result<(), (Vec<verify::Violation>, String)> {
  let violations = verify::verify_layout(source, output, edition);
  if violations.is_empty() {
    return Ok(());
  }
  let fails = |candidate: &str| match arrange(candidate, config, None) {
    Ok((out, _, _)) => !verify::verify_layout(candidate, &out, edition).is_empty(),
    Err(_) => false,
  };
  Err((
    violations,
    verify::report::minimize(source, edition, &fails),
  ))
}

/// Both checks before anything is written or shown: layout's output against the source,
/// then formatting's output against layout's. A violation is a bug report.
fn verified_all(
  source: &str,
  laid_out: &str,
  formatted: &str,
  edition: syntax::Edition,
  settings: &Settings,
  path: &str,
) -> Result<(), Box<report::Bug>> {
  if let Some(config) = &settings.layout
    && laid_out != source
    && let Err((violations, minimal)) = verified(source, laid_out, edition, config)
  {
    return Err(Box::new(bug(
      path,
      source,
      settings,
      report::Failure::Verifier { violations },
      Some(minimal),
      None,
    )));
  }
  if settings.format.is_some() && formatted != laid_out {
    let violations = verify::format::verify_format(laid_out, formatted, edition);
    if !violations.is_empty() {
      return Err(Box::new(bug(
        path,
        source,
        settings,
        report::Failure::Verifier { violations },
        None,
        None,
      )));
    }
  }
  Ok(())
}

/// Collects the jobs under `dir`, shown as given; `real` is the same folder, absolute with
/// symlinks resolved, for looking up its settings.
fn walk_inner(
  dir: &Path,
  real: PathBuf,
  resolver: &mut config::Resolver,
  opts: &RunOptions,
  walk: &mut Walk,
  top: bool,
) -> Result<(), MtoError> {
  // Without --follow-links, symlinks are skipped, so every path is visited once anyway.
  if opts.follow_links && !walk.visited.insert(real.clone()) {
    return Ok(());
  }
  // The listing first: it says whether a folder below the one named is a repository of its
  // own, and which configuration files the folder holds, without asking the file system
  // about each.
  let entries: Option<Vec<(fs::DirEntry, std::ffi::OsString)>> =
    fs::read_dir(dir).ok().map(|read| {
      read
        .flatten()
        .map(|entry| {
          let name = entry.file_name();
          (entry, name)
        })
        .collect()
    });
  if let Some(entries) = &entries {
    // As `.git` existing: a file (a worktree's) or a folder, or a link to either.
    let repository = entries
      .iter()
      .any(|(entry, name)| name == ".git" && (!is_symlink(entry) || entry.path().exists()));
    if !top && repository {
      walk.nested.push(dir.to_path_buf());
      return Ok(());
    }
    resolver.note_folder(&real, |wanted| {
      entries.iter().any(|(entry, name)| {
        if name == wanted {
          if is_symlink(entry) {
            entry.path().is_file()
          } else {
            entry.file_type().is_ok_and(|t| t.is_file())
          }
        } else {
          // `Mto.toml`: the file system says whether it is the file asked for, as it does
          // when the standard formatter looks for its own (case-insensitive on Windows,
          // and on macOS by default).
          name
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
            && real.join(wanted).is_file()
        }
      })
    });
  }
  let resolved = resolver.config_for_absolute(real.clone())?;
  print_warnings(resolver);
  let config = resolved.layout();
  let settings = Settings::of(&resolved).map(Arc::new);
  let dir_name = dir
    .file_name()
    .unwrap_or_default()
    .to_string_lossy()
    .to_string();
  if config.ignore_dirs.contains(&dir_name) {
    return Ok(());
  }
  let Some(mut entries) = entries else {
    return Ok(());
  };
  // By name: within one folder that is the order of their paths.
  entries.sort_by(|a, b| a.1.cmp(&b.1));
  for (entry, _) in entries {
    let path = entry.path();
    let file_type = match entry.file_type() {
      Ok(file_type) => file_type,
      Err(_) => continue,
    };
    let (is_dir, is_file) = if file_type.is_symlink() {
      if !opts.follow_links {
        continue;
      }
      match fs::metadata(&path) {
        Ok(meta) => (meta.is_dir(), meta.is_file()),
        Err(_) => continue,
      }
    } else {
      (file_type.is_dir(), file_type.is_file())
    };
    if is_dir {
      let child_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
      if config.ignore_dirs.contains(&child_name) {
        continue;
      }
      let child_real = if opts.follow_links {
        match real_path(&path) {
          Ok(real) => real,
          Err(_) => continue,
        }
      } else {
        real.join(entry.file_name())
      };
      walk_inner(&path, child_real, resolver, opts, walk, false)?;
    } else if is_file && is_stale_temp(&path) {
      walk.stale.push(path);
    } else if is_file && path.extension().unwrap_or_default() == "rs" {
      if opts.follow_links
        && let Ok(real_file) = real_path(&path)
        && !walk.visited.insert(real_file)
      {
        continue;
      }
      (walk.found)((path, settings.clone()));
    }
  }
  Ok(())
}

/// [`run_jobs_walking`] with at most `most` workers.
fn walking<T>(
  opts: &RunOptions,
  writes: &mut Writes,
  most: usize,
  walk: impl FnOnce(&mut dyn FnMut(Job)) -> T,
  before_results: impl FnOnce(&T) -> bool,
) -> (Vec<Job>, T, Vec<Done>) {
  /// The jobs found so far, and whether the walk is over.
  struct Queue {
    jobs: Vec<Job>,
    over: bool,
  }
  fn lock(queue: &Mutex<Queue>) -> std::sync::MutexGuard<'_, Queue> {
    queue.lock().unwrap_or_else(|e| e.into_inner())
  }
  /// Ends the walk for the workers however it ends: a walk that panicked would otherwise
  /// leave them waiting for files that never come.
  struct Over<'a>(&'a Mutex<Queue>, &'a std::sync::Condvar);
  impl Drop for Over<'_> {
    fn drop(&mut self) {
      lock(self.0).over = true;
      self.1.notify_all();
    }
  }
  let workers = thread::available_parallelism()
    .map(|n| n.get())
    .unwrap_or(1)
    .min(most);
  let queue = Mutex::new(Queue {
    jobs: Vec::new(),
    over: false,
  });
  let found_one = std::sync::Condvar::new();
  let next = AtomicUsize::new(0);
  let (sender, receiver) = mpsc::channel::<(usize, FileReport)>();
  let history = writes.recorder.as_ref().map(|r| r.history().clone());
  // Shared by the workers, each adding a file before it replaces it.
  let recorder = writes.recorder.take().map(Mutex::new);
  let ctx = WriteContext {
    history: history.as_ref(),
    recorder: recorder.as_ref(),
    dirty: writes.dirty.as_ref(),
    holds: writes.holds.as_ref(),
    defer: writes.defer,
    reports: writes.reports.as_ref(),
  };
  let (jobs, walked, outcomes) = thread::scope(|scope| {
    for _ in 0..workers {
      let sender = sender.clone();
      let (queue, found_one, next) = (&queue, &found_one, &next);
      let worker = move || {
        loop {
          // The next job not taken, waiting for the walk to find it if need be.
          let job = {
            let mut state = lock(queue);
            loop {
              let idx = next.load(Ordering::Relaxed);
              if let Some(job) = state.jobs.get(idx) {
                next.store(idx + 1, Ordering::Relaxed);
                break Some((idx, job.clone()));
              }
              if state.over {
                break None;
              }
              state = found_one.wait(state).unwrap_or_else(|e| e.into_inner());
            }
          };
          let Some((idx, (path, config))) = job else {
            break;
          };
          let report = match config {
            Some(settings) => run_file(&path, &settings, opts, ctx),
            None => FileReport::default().with(Outcome::LayoutOff),
          };
          if sender.send((idx, report)).is_err() {
            break;
          }
        }
      };
      thread::Builder::new()
        .stack_size(STACK)
        .spawn_scoped(scope, worker)
        .expect("a thread to lay out files on");
    }
    drop(sender);
    let over = Over(&queue, &found_one);
    let walked = walk(&mut |job| {
      lock(&queue).jobs.push(job);
      found_one.notify_one();
    });
    drop(over);
    let wanted = before_results(&walked);
    let mut outcomes = Vec::new();
    let mut waiting = BTreeMap::new();
    for (idx, report) in receiver {
      if !wanted {
        continue;
      }
      waiting.insert(idx, report);
      while let Some(report) = waiting.remove(&outcomes.len()) {
        report.print();
        outcomes.push(Done {
          outcome: report.outcome,
          json: report.json,
          planned: report.planned,
          holds: report.holds,
        });
      }
    }
    let jobs = std::mem::take(&mut lock(&queue).jobs);
    (jobs, walked, outcomes)
  });
  writes.recorder = recorder.map(|r| r.into_inner().unwrap_or_else(|e| e.into_inner()));
  (jobs, walked, outcomes)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// (line_number, line_number_orig, lines as " x", "-x", "+x").
  fn shape(old: &str, new: &str, context: usize) -> Vec<(u32, u32, Vec<String>)> {
    mismatches(old, new, context)
      .into_iter()
      .map(|m| {
        let lines = m
          .lines
          .into_iter()
          .map(|l| match l {
            MismatchLine::Context(t) => format!(" {}", t),
            MismatchLine::Resulting(t) => format!("-{}", t),
            MismatchLine::Expected(t) => format!("+{}", t),
          })
          .collect();
        (m.line_number, m.line_number_orig, lines)
      })
      .collect()
  }

  /// Two long texts with no line in common diff quickly (the search stops at
  /// `MAX_EDITS`), and the edits still turn one into the other.
  #[test]
  fn test_a_diff_of_texts_sharing_little_is_quick_and_whole() {
    let old: Vec<String> = (0..20_000)
      .map(|i| format!("{} => {{ x+{} }}", i, i))
      .collect();
    let new: Vec<String> = (0..20_000)
      .map(|i| format!("{} => x + {},", i, i))
      .collect();
    let (a, b): (Vec<&str>, Vec<&str>) = (
      old.iter().map(String::as_str).collect(),
      new.iter().map(String::as_str).collect(),
    );
    let started = std::time::Instant::now();
    let ops = diff_ops(&a, &b);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    let (mut i, mut rebuilt) = (0, Vec::new());
    for op in ops {
      match op {
        DiffOp::Equal => {
          rebuilt.push(a[i]);
          i += 1;
        }
        DiffOp::Delete(_) => i += 1,
        DiffOp::Insert(j) => rebuilt.push(b[j]),
      }
    }
    assert_eq!(i, a.len());
    assert_eq!(rebuilt, b);
  }

  /// A file is read in the configured edition where it parses: in 2015 a path starting
  /// with `::` is an ordinary path, which sorts with the others.
  #[test]
  fn test_files_are_read_in_the_configured_edition() {
    let source = "use ::c;\nuse b;\nuse ::a::{z, y};\n";
    let text = |edition| {
      let settings = Settings {
        layout: None,
        format: Some(options::format::FormatConfig::default()),
        edition,
      };
      let shaped = super::shape(source, &settings, &doctor::Held::default(), None).unwrap();
      (shaped.edition, shaped.text)
    };
    assert_eq!(
      text(syntax::Edition::E2015),
      (
        syntax::Edition::E2015,
        "use a::{y, z};\nuse b;\nuse c;\n".to_string()
      )
    );
    assert_eq!(
      text(syntax::Edition::E2018),
      (
        syntax::Edition::E2018,
        "use ::a::{y, z};\nuse ::c;\nuse b;\n".to_string()
      )
    );
    // Code the configured edition does not fit is read in one it does.
    let settings = Settings {
      layout: None,
      format: Some(options::format::FormatConfig::default()),
      edition: syntax::Edition::E2021,
    };
    let shaped =
      super::shape("fn async() {}\n", &settings, &doctor::Held::default(), None).unwrap();
    assert_eq!(shaped.edition, syntax::Edition::E2015);
  }

  // The standard formatter's own tests of its diff, ported.
  #[test]
  fn test_mismatches_match_the_standard_diff() {
    let lines = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
      shape(
        "one\ntwo\nthree\nfour\nfive\n",
        "one\ntwo\ntrois\nfour\nfive\n",
        1
      ),
      vec![(2, 2, lines(&[" two", "-three", "+trois", " four"]))]
    );
    assert_eq!(
      shape(
        "one\ntwo\nthree\nfour\nfive\nsix\nseven\n",
        "one\ntwo\ntrois\nfour\ncinq\nsix\nseven\n",
        1
      ),
      vec![
        (2, 2, lines(&[" two", "-three", "+trois", " four"])),
        (5, 5, lines(&["-five", "+cinq", " six"])),
      ]
    );
    assert_eq!(
      shape(
        "one\ntwo\nthree\nfour\nfive\n",
        "one\ntwo\ntrois\nfour\nfive\n",
        0
      ),
      vec![(3, 3, lines(&["-three", "+trois"]))]
    );
    assert_eq!(
      shape(
        "one\ntwo\nthree\nfour\nfive",
        "one\ntwo\nthree\nfour\nfive\n",
        1
      ),
      vec![(5, 5, lines(&[" five", "+"]))]
    );
  }

  /// Windows' verbatim paths are shown plain where the plain form names the same file.
  #[test]
  fn test_verbatim_paths_are_shown_plain_where_they_read_the_same() {
    let plain = |text: &str| plain_path(Path::new(text)).map(|p| p.display().to_string());
    assert_eq!(
      plain(r"\\?\C:\src\lib.rs"),
      Some(r"C:\src\lib.rs".to_string())
    );
    assert_eq!(
      plain(r"\\?\UNC\server\share\a.rs"),
      Some(r"\\server\share\a.rs".to_string())
    );
    // A device name, a trailing dot or space, or a path past the old limit reads
    // otherwise without the prefix.
    assert_eq!(plain(r"\\?\C:\src\con.rs"), None);
    assert_eq!(plain(r"\\?\C:\src\COM1"), None);
    assert_eq!(plain(r"\\?\C:\src\a."), None);
    assert_eq!(plain(r"\\?\C:\src\a "), None);
    assert_eq!(plain(&format!(r"\\?\C:\{}", "a".repeat(300))), None);
    // Not verbatim, or not a drive.
    assert_eq!(plain(r"C:\src"), None);
    assert_eq!(plain(r"\\?\Volume{1234}\src"), None);
    // A name merely starting like a device is fine.
    assert_eq!(
      plain(r"\\?\C:\console\com10.rs"),
      Some(r"C:\console\com10.rs".to_string())
    );
  }
}
