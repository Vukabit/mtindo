pub mod cli;
pub mod config;
pub mod history;
pub mod layout;
pub mod options;
pub mod syntax;
pub mod text;
pub mod verify;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

pub use layout::import_group_of_text as classify_import;
pub use options::layout::{ImportGroup, ItemKind, LayoutConfig, Visibility};
pub use text::color::{HelpPalette, color_enabled};

static UNIQUE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A file found by the walk, with the settings of its folder; `None` when layout is off
/// there.
pub type Job = (PathBuf, Option<Arc<LayoutConfig>>);

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

enum MismatchLine {
    Context(String),
    /// A line of the new text (`+`).
    Expected(String),
    /// A line of the original (`-`).
    Resulting(String),
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
    /// Left untouched because the verifier found the new text unsafe: a bug in mto.
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
}

/// How one job ended: its outcome, and its `--emit json` entry.
#[derive(Debug)]
pub struct Done {
    pub outcome: Outcome,
    pub json: Option<String>,
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
    /// The file as written, for the run's history record.
    pub written: Option<history::Entry>,
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
struct Mismatch {
    /// The first line shown, in the new text and in the original.
    line_number: u32,
    line_number_orig: u32,
    lines: Vec<MismatchLine>,
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
    /// `-v`: name each file as it is laid out.
    pub verbose: bool,
    /// `--skip-children`: a file argument is that file alone, not its modules.
    pub skip_children: bool,
    /// `--no-history`: do not record this run.
    pub no_history: bool,
    /// `--allow-dirty`: with history off, rewrite files with uncommitted changes.
    pub allow_dirty: bool,
}

/// The state of a directory walk.
struct Walk<'a> {
    visited: &'a mut HashSet<PathBuf>,
    jobs: &'a mut Vec<Job>,
    nested: &'a mut Vec<PathBuf>,
}

/// What [`run_file`] needs to write a file: where its contents are recorded, and which files
/// the git guard keeps.
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteContext<'a> {
    pub history: Option<&'a history::History>,
    pub dirty: Option<&'a HashSet<PathBuf>>,
}

/// What a run that writes files records and guards.
#[derive(Debug, Default)]
pub struct Writes {
    /// Records the run in history; `None` with history off.
    pub recorder: Option<history::Recorder>,
    /// With history off: the files (absolute, symlinks resolved) git sees uncommitted
    /// changes in, which are not rewritten. `None` when the guard is off.
    pub dirty: Option<HashSet<PathBuf>>,
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
    let end = if verbose { "⏎" } else { "" };
    let mut out = String::new();
    for mismatch in mismatches(old, new, 3) {
        out.push_str(&format!(
            "Diff in {}:{}:\n",
            label, mismatch.line_number_orig
        ));
        for line in mismatch.lines {
            match line {
                MismatchLine::Context(text) => out.push_str(&format!(" {}{}\n", text, end)),
                MismatchLine::Expected(text) => out.push_str(&format!(
                    "{}+{}{}{}\n",
                    c_green(),
                    text,
                    end,
                    c_reset(false)
                )),
                MismatchLine::Resulting(text) => out.push_str(&format!(
                    "{}-{}{}{}\n",
                    if color_enabled(false) {
                        "\x1b[1;31m"
                    } else {
                        ""
                    },
                    text,
                    end,
                    c_reset(false)
                )),
            }
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
    let mut visited = HashSet::new();
    let mut jobs = Vec::new();
    let mut problems = Vec::new();
    let mut nested = Vec::new();
    for path in paths {
        if path.is_dir() {
            let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let mut walk = Walk {
                visited: &mut visited,
                jobs: &mut jobs,
                nested: &mut nested,
            };
            walk_inner(path, real, resolver, opts, &mut walk)?;
            continue;
        }
        let files = if opts.skip_children {
            vec![path.clone()]
        } else {
            let (files, missing) = cli::modules::module_files(path);
            problems.extend(missing);
            files
        };
        for file in files {
            let real = fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
            if !visited.insert(real.clone()) {
                continue;
            }
            let dir = real.parent().unwrap_or(Path::new(".")).to_path_buf();
            let resolved = resolver.config_for_absolute(dir)?;
            print_warnings(resolver);
            let config = resolved.layout_on().then(|| Arc::new(resolved.layout()));
            jobs.push((file, config));
        }
    }
    Ok(Collected {
        jobs,
        problems,
        nested,
    })
}

/// The `mto.toml` that `mto init` writes: every mto setting at its default, with its
/// summary.
pub fn init_template() -> String {
    let mut out = String::from(
        "# mto configuration file; `mto --man` explains every setting.\n\n\
         # Rearrange the items of each file.\nlayout = true\n",
    );
    let others = options::all().filter(|s| s.file == options::File::Mto && s.name != "layout");
    for setting in others {
        out.push_str(&format!(
            "\n# {}\n{} = {}\n",
            setting.summary, setting.name, setting.default
        ));
    }
    out
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
        fs::create_dir_all(&dir).map_err(|e| {
            MtoError::Io(format!("cannot create directory {}: {}", dir.display(), e))
        })?;
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
    let config = resolved.layout();
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
    if !resolved.layout_on() {
        return Ok(unchanged(report, Outcome::LayoutOff));
    }
    let (reorganized, edition, _) = match arrange(&source, &config, None) {
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
    if let Err(bug_report) = verified(&source, &reorganized, edition, &config, &shown) {
        report.stderr = bug_report;
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
    config: &LayoutConfig,
    opts: &RunOptions,
    ctx: WriteContext,
) -> FileReport {
    let mut report = FileReport::default();
    let shown = file_path.display().to_string();
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
    let (reorganized, edition) = match arrange(&source, config, debug) {
        Ok((text, edition, skipped)) => {
            // Named as the standard formatter names them: not the files that tell it to skip
            // them.
            if opts.verbose && !skipped {
                report
                    .stdout
                    .insert_str(0, &format!("Formatting {}\n", shown));
            }
            (text, edition)
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
    if let Err(bug_report) = verified(&source, &reorganized, edition, config, &shown) {
        report.stderr = bug_report;
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
        let real = fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
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
    // The contents are stored, and synced, before the file is replaced: what history
    // records, it can give back.
    let mut written = None;
    if let Some(history) = ctx.history {
        let stored = history.store(source.as_bytes()).and_then(|before| {
            let after = history.store(reorganized.as_bytes())?;
            Ok((before, after))
        });
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
    report.written = written;
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
/// Each file written is added to the run's history record as its report arrives.
pub fn run_jobs(jobs: &[Job], opts: &RunOptions, writes: &mut Writes) -> Vec<Done> {
    let workers = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(jobs.len().max(1));
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel::<(usize, FileReport)>();
    let mut outcomes = Vec::with_capacity(jobs.len());
    let history = writes.recorder.as_ref().map(|r| r.history().clone());
    let ctx = WriteContext {
        history: history.as_ref(),
        dirty: writes.dirty.as_ref(),
    };
    let recorder = &mut writes.recorder;
    thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || {
                loop {
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    let Some((path, config)) = jobs.get(idx) else {
                        break;
                    };
                    let report = match config {
                        Some(config) => run_file(path, config, opts, ctx),
                        None => FileReport::default().with(Outcome::LayoutOff),
                    };
                    if sender.send((idx, report)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        let mut waiting = BTreeMap::new();
        for (idx, report) in receiver {
            waiting.insert(idx, report);
            while let Some(report) = waiting.remove(&outcomes.len()) {
                report.print();
                if let (Some(recorder), Some(entry)) = (recorder.as_mut(), &report.written)
                    && let Err(e) = recorder.record(entry)
                {
                    eprintln!(
                        "{}warning: cannot add {} to the history record: {}{}",
                        c_yellow(true),
                        entry.path.display(),
                        e,
                        c_reset(true)
                    );
                }
                outcomes.push(Done {
                    outcome: report.outcome,
                    json: report.json,
                });
            }
        }
    });
    outcomes
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
    Ok(run_jobs(&jobs, opts, &mut Writes::default())
        .into_iter()
        .map(|d| d.outcome)
        .collect())
}

/// Replaces `target` atomically: a sibling temp file is written, synced and renamed over it.
/// A symlinked target keeps its link and its pointee is replaced; permissions are kept.
pub fn write_atomic(target: &Path, content: &str) -> Result<(), std::io::Error> {
    let real_target = match fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(target)?,
        _ => target.to_path_buf(),
    };
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
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, &real_target)?;
        sync_dir(&parent);
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
    if source.trim_start_matches('\u{feff}').trim().is_empty() {
        return Ok((source.to_string(), syntax::Edition::E2021, false));
    }
    let arranged = layout::layout(source, config)?;
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
    // engine has no known way to produce one.
    if cfg!(debug_assertions) && env::var_os("MTINDO_INJECT_FAULT").is_some() {
        text.push_str("// injected by MTINDO_INJECT_FAULT\n");
    }
    Ok((text, arranged.edition, arranged.skipped))
}

/// Where the forward and reverse searches of Myers' algorithm meet: a point on a shortest
/// edit path, to split the problem at. `None` if the texts share nothing.
fn bisect(a: &[u32], b: &[u32]) -> Option<(usize, usize)> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max_d = (n + m + 1) / 2;
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

/// The edit script turning `a` into `b`, line by line: Myers' algorithm in linear space (the
/// bisection of diff-match-patch), on line ids so that lines compare as integers.
fn diff_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<DiffOp> {
    let mut ids: HashMap<&'a str, u32> = HashMap::new();
    let mut id = |line: &'a str| {
        let next = ids.len() as u32;
        *ids.entry(line).or_insert(next)
    };
    let a_ids: Vec<u32> = a.iter().map(|l| id(l)).collect();
    let b_ids: Vec<u32> = b.iter().map(|l| id(l)).collect();
    let mut ops = Vec::with_capacity(a.len().max(b.len()));
    diff_range(&a_ids, 0, &b_ids, 0, &mut ops);
    ops
}

/// Appends the edits turning `a` into `b`, whose lines start at `a_at` and `b_at`.
fn diff_range(a: &[u32], a_at: usize, b: &[u32], b_at: usize, ops: &mut Vec<DiffOp>) {
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
    } else if let Some(anchors) = patience_anchors(a_mid, b_mid) {
        // Lines found once on each side, in the same order on both, are kept; the gaps
        // between them are diffed on their own.
        let (mut i, mut j) = (0, 0);
        for (x, y) in anchors {
            diff_range(&a_mid[i..x], a_at + i, &b_mid[j..y], b_at + j, ops);
            ops.push(DiffOp::Equal);
            (i, j) = (x + 1, y + 1);
        }
        diff_range(&a_mid[i..], a_at + i, &b_mid[j..], b_at + j, ops);
    } else {
        match bisect(a_mid, b_mid) {
            Some((x, y)) => {
                diff_range(&a_mid[..x], a_at, &b_mid[..y], b_at, ops);
                diff_range(&a_mid[x..], a_at + x, &b_mid[y..], b_at + y, ops);
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
                    expected.push_str(&text);
                    expected.push('\n');
                }
                MismatchLine::Resulting(text) => {
                    original_end = original_begin + original_count;
                    original_count += 1;
                    original.push_str(&text);
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
fn mismatches(old: &str, new: &str, context: usize) -> Vec<Mismatch> {
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
            DiffOp::Delete(at) => Some(MismatchLine::Resulting(a[at].to_string())),
            DiffOp::Insert(at) => Some(MismatchLine::Expected(b[at].to_string())),
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
                    current.lines.push(MismatchLine::Context(line.to_string()));
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
                    current.lines.push(MismatchLine::Context(line.to_string()));
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
fn patience_anchors(a: &[u32], b: &[u32]) -> Option<Vec<(usize, usize)>> {
    // line -> (count in a, index in a, count in b, index in b)
    let mut seen: HashMap<u32, (u32, usize, u32, usize)> = HashMap::new();
    for (i, line) in a.iter().enumerate() {
        let entry = seen.entry(*line).or_insert((0, i, 0, 0));
        entry.0 += 1;
    }
    for (j, line) in b.iter().enumerate() {
        if let Some(entry) = seen.get_mut(line) {
            entry.2 += 1;
            entry.3 = j;
        }
    }
    let mut unique: Vec<(usize, usize)> = seen
        .values()
        .filter(|(ca, _, cb, _)| *ca == 1 && *cb == 1)
        .map(|(_, i, _, j)| (*i, *j))
        .collect();
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

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// Checks the new text with the verifier before anything is written or shown. On a
/// violation, the error is the bug report, with the smallest part of the input that still
/// fails.
fn verified(
    source: &str,
    output: &str,
    edition: syntax::Edition,
    config: &LayoutConfig,
    path: &str,
) -> Result<(), String> {
    let violations = verify::verify_layout(source, output, edition);
    if violations.is_empty() {
        return Ok(());
    }
    let fails = |candidate: &str| match arrange(candidate, config, None) {
        Ok((out, _, _)) => !verify::verify_layout(candidate, &out, edition).is_empty(),
        Err(_) => false,
    };
    let minimal = verify::report::minimize(source, edition, &fails);
    Err(format!(
        "{}{}{}",
        c_red(),
        verify::report::bug_report(path, &violations, Some(&minimal)),
        c_reset(true)
    ))
}

/// Collects the jobs under `dir`, shown as given; `real` is the same folder, absolute with
/// symlinks resolved, for looking up its settings.
fn walk_inner(
    dir: &Path,
    real: PathBuf,
    resolver: &mut config::Resolver,
    opts: &RunOptions,
    walk: &mut Walk,
) -> Result<(), MtoError> {
    // Without --follow-links, symlinks are skipped, so every path is visited once anyway.
    if opts.follow_links && !walk.visited.insert(real.clone()) {
        return Ok(());
    }
    let resolved = resolver.config_for_absolute(real.clone())?;
    print_warnings(resolver);
    let config = Arc::new(resolved.layout());
    let layout_on = resolved.layout_on();
    let dir_name = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    if config.ignore_dirs.contains(&dir_name) {
        return Ok(());
    }
    let mut entries: Vec<fs::DirEntry> = match fs::read_dir(dir) {
        Ok(entries) => entries.flatten().collect(),
        Err(_) => return Ok(()),
    };
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
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
            if path.join(".git").exists() {
                walk.nested.push(path);
                continue;
            }
            let child_real = if opts.follow_links {
                match fs::canonicalize(&path) {
                    Ok(real) => real,
                    Err(_) => continue,
                }
            } else {
                real.join(entry.file_name())
            };
            walk_inner(&path, child_real, resolver, opts, walk)?;
        } else if is_file && path.extension().unwrap_or_default() == "rs" {
            if opts.follow_links
                && let Ok(real_file) = fs::canonicalize(&path)
                && !walk.visited.insert(real_file)
            {
                continue;
            }
            walk.jobs
                .push((path, layout_on.then(|| Arc::clone(&config))));
        }
    }
    Ok(())
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
}
