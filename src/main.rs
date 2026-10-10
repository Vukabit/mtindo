use std::env;
use std::path::{Path, PathBuf};

use mtindo::cli::history::{self, PruneArgs};
use mtindo::config::{self, Config, Environment, FileKind, Overrides, Resolver, Source};
use mtindo::options::File;
use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::tree::debug_dump;
use mtindo::syntax::{Edition, parse};
use mtindo::{
    Done, Emit, HelpPalette, MtoError, Outcome, RunOptions, Writes, c_green, c_red, c_reset,
    c_yellow, collect_jobs, print_warnings, process_init, process_stdin, run_jobs,
};

/// mto's commands, in the order help lists them. Each has its own help page (`mto history
/// -h`), built from this table and [`FLAGS`].
const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "init",
        args: "[DIR]",
        summary: "Write a default mto.toml",
        about: "Writes an mto.toml with layout turned on and every mto setting at its default, \
                each with its summary, into DIR (default: the current directory). An existing \
                file is never overwritten.",
        usage: &["mto init [DIR]"],
        options: &[],
        examples: &["mto init", "mto init crates/core"],
    },
    CommandSpec {
        name: "config",
        args: "path|print [DIR]",
        summary: "Show the configuration that applies in a folder",
        about: "`path` lists the configuration files that apply in DIR (default: the current \
                directory), and the history folder. `print` shows every setting in effect with \
                where it came from, in sections: layout, then formatting.",
        usage: &["mto config path [DIR]", "mto config print [OPTIONS] [DIR]"],
        options: &[
            (
                "views",
                "mto config print",
                &[Flag::Current, Flag::Default, Flag::Minimal, Flag::User],
            ),
            (
                "configuration",
                "mto config path|print",
                &[
                    Flag::Config,
                    Flag::ConfigPath,
                    Flag::Edition,
                    Flag::StyleEdition,
                ],
            ),
        ],
        examples: &[
            "mto config path",
            "mto config print --minimal",
            "mto config print --user",
        ],
    },
    CommandSpec {
        name: "history",
        args: "[show <RUN> | prune]",
        summary: "List, show or prune the runs that wrote files",
        about: "Every run that writes files is recorded. With no argument, lists the runs, \
                newest first. `show` lists one run's files and where each stands now. `prune` \
                removes old runs, by the user config's limits unless told otherwise. RUN is \
                `last`, `last~N`, or a handle from the list.",
        usage: &[
            "mto history",
            "mto history show <RUN>",
            "mto history prune [--older-than DAYS] [--keep N]",
        ],
        options: &[(
            "options",
            "mto history prune",
            &[Flag::Keep, Flag::OlderThan],
        )],
        examples: &[
            "mto history",
            "mto history show last~1",
            "mto history prune --keep 10",
        ],
    },
    CommandSpec {
        name: "restore",
        args: "<RUN> [FILE...]",
        summary: "Undo a run",
        about: "Puts back what RUN replaced, in every file it wrote or only the FILEs named. A \
                file changed or deleted since is skipped (exit code 1) unless --force. The \
                restore is recorded as a run of its own, so it can be undone too. RUN is \
                `last`, `last~N`, or a handle from `mto history`.",
        usage: &["mto restore <RUN> [FILE...] [--force]"],
        options: &[("options", "mto restore <RUN> [FILE...]", &[Flag::Force])],
        examples: &["mto restore last", "mto restore 3f9a src/lib.rs"],
    },
    CommandSpec {
        name: "debug",
        args: "tokens|tree <FILE>",
        summary: "Print the tokens or syntax tree mto sees, for bug reports",
        about: "Prints the tokens or the syntax tree of FILE as mto reads it. The output is \
                not stable; it is meant for bug reports.",
        usage: &["mto debug tokens|tree <FILE> [--edition YEAR]"],
        options: &[("options", "mto debug tokens|tree <FILE>", &[Flag::Edition])],
        examples: &["mto debug tree src/lib.rs"],
    },
];

/// The settings that decide which defaults apply.
const EDITION_KEYS: [&str; 3] = ["edition", "style_edition", "version"];

const EXAMPLES: &[&str] = &[
    "mto src/main.rs",
    "mto -R --check",
    "mto --check -l src/lib.rs src/main.rs",
    "mto --emit json --check src/",
    "cat src/main.rs | mto -",
    "mto --config max_width=80,hard_tabs=true src/",
    "mto config print --minimal",
    "mto restore last",
];

const EXIT_CODES: &[(&str, &str)] = &[
    ("0", "success"),
    (
        "1",
        "--check found files that would change; restore skipped files changed since",
    ),
    ("2", "bad arguments or configuration"),
    (
        "3",
        "a file could not be read, written, or parsed (it has syntax errors), or the git \
         guard kept it",
    ),
    (
        "4",
        "the verifier stopped a write (a bug in mto); the file is unchanged",
    ),
];

const FLAGS: &[FlagSpec] = &[
    FlagSpec {
        flag: Flag::Check,
        group: Group::Layout,
        short: None,
        long: "--check",
        value: "",
        help: "Print a diff for every file that would change; exit 1 if any",
    },
    FlagSpec {
        flag: Flag::DryRun,
        group: Group::Layout,
        short: Some("-d"),
        long: "--dry-run",
        value: "",
        help: "List the files that would change, without writing",
    },
    FlagSpec {
        flag: Flag::Emit,
        group: Group::Layout,
        short: None,
        long: "--emit",
        value: "<files|stdout|json>",
        help: "Write the files (default), print them, or report changes as JSON",
    },
    FlagSpec {
        flag: Flag::FilesWithDiff,
        group: Group::Layout,
        short: Some("-l"),
        long: "--files-with-diff",
        value: "",
        help: "Name the files that change (or would, with --check) instead of showing how",
    },
    FlagSpec {
        flag: Flag::FollowLinks,
        group: Group::Files,
        short: None,
        long: "--follow-links",
        value: "",
        help: "Also follow symlinked files and directories",
    },
    FlagSpec {
        flag: Flag::Recursive,
        group: Group::Files,
        short: Some("-R"),
        long: "--recursive",
        value: "",
        help: "Lay out every .rs file under the current directory",
    },
    FlagSpec {
        flag: Flag::SkipChildren,
        group: Group::Files,
        short: None,
        long: "--skip-children",
        value: "",
        help: "A file argument is that file alone, not the modules it declares",
    },
    FlagSpec {
        flag: Flag::StdinFilepath,
        group: Group::Stdin,
        short: None,
        long: "--stdin-filepath",
        value: "<PATH>",
        help: "With -: where the source belongs, for its settings and in messages",
    },
    FlagSpec {
        flag: Flag::Config,
        group: Group::Configuration,
        short: None,
        long: "--config",
        value: "<KEY=VALUE,...>",
        help: "Set settings for this run, over every file",
    },
    FlagSpec {
        flag: Flag::ConfigPath,
        group: Group::Configuration,
        short: None,
        long: "--config-path",
        value: "<FILE>",
        help: "Use only this configuration file; search for no other",
    },
    FlagSpec {
        flag: Flag::Edition,
        group: Group::Configuration,
        short: None,
        long: "--edition",
        value: "<YEAR>",
        help: "The Rust edition, over Cargo.toml and configuration files",
    },
    FlagSpec {
        flag: Flag::PrintConfig,
        group: Group::Compatibility,
        short: None,
        long: "--print-config",
        value: "<default|minimal|current> [PATH]",
        help: "Print rustfmt's settings, as rustfmt does, and exit",
    },
    FlagSpec {
        flag: Flag::StyleEdition,
        group: Group::Configuration,
        short: None,
        long: "--style-edition",
        value: "<YEAR>",
        help: "The style edition, which decides the defaults",
    },
    FlagSpec {
        flag: Flag::UnstableFeatures,
        group: Group::Configuration,
        short: None,
        long: "--unstable-features",
        value: "",
        help: "Accepted for rustfmt compatibility; mto applies every setting on every channel",
    },
    FlagSpec {
        flag: Flag::AllowDirty,
        group: Group::History,
        short: None,
        long: "--allow-dirty",
        value: "",
        help: "With history off, rewrite files that have uncommitted changes",
    },
    FlagSpec {
        flag: Flag::NoHistory,
        group: Group::History,
        short: None,
        long: "--no-history",
        value: "",
        help: "Do not record this run in history (it cannot be restored)",
    },
    FlagSpec {
        flag: Flag::Color,
        group: Group::Output,
        short: None,
        long: "--color",
        value: "<always|never|auto>",
        help: "Color the output (default: auto, only on a terminal)",
    },
    FlagSpec {
        flag: Flag::Dbg,
        group: Group::Output,
        short: None,
        long: "--dbg",
        value: "",
        help: "Print how each file was split into items",
    },
    FlagSpec {
        flag: Flag::Quiet,
        group: Group::Output,
        short: Some("-q"),
        long: "--quiet",
        value: "",
        help: "Print only errors and what was asked for",
    },
    FlagSpec {
        flag: Flag::Verbose,
        group: Group::Output,
        short: Some("-v"),
        long: "--verbose",
        value: "",
        help: "Name each file as it is laid out",
    },
    FlagSpec {
        flag: Flag::Help,
        group: Group::General,
        short: Some("-h"),
        long: "--help",
        value: "",
        help: "Show this help and exit",
    },
    FlagSpec {
        flag: Flag::Man,
        group: Group::General,
        short: None,
        long: "--man",
        value: "",
        help: "Print the full manual",
    },
    FlagSpec {
        flag: Flag::Version,
        group: Group::General,
        short: Some("-V"),
        long: "--version",
        value: "",
        help: "Print the version and exit",
    },
    FlagSpec {
        flag: Flag::Current,
        group: Group::Command,
        short: None,
        long: "--current",
        value: "",
        help: "Every setting in effect, with its source (the default)",
    },
    FlagSpec {
        flag: Flag::Default,
        group: Group::Command,
        short: None,
        long: "--default",
        value: "",
        help: "Every setting at its default",
    },
    FlagSpec {
        flag: Flag::Minimal,
        group: Group::Command,
        short: None,
        long: "--minimal",
        value: "",
        help: "Only the settings that differ from their defaults",
    },
    FlagSpec {
        flag: Flag::User,
        group: Group::Command,
        short: None,
        long: "--user",
        value: "",
        help: "The user config's settings",
    },
    FlagSpec {
        flag: Flag::Force,
        group: Group::Command,
        short: None,
        long: "--force",
        value: "",
        help: "Also restore files changed since the run, and missing ones",
    },
    FlagSpec {
        flag: Flag::Keep,
        group: Group::Command,
        short: None,
        long: "--keep",
        value: "<N>",
        help: "Keep only the newest N runs",
    },
    FlagSpec {
        flag: Flag::OlderThan,
        group: Group::Command,
        short: None,
        long: "--older-than",
        value: "<DAYS>",
        help: "Remove runs this old: 30 (days), 2w, 12h",
    },
];

/// The column where help descriptions start.
const HELP_COLUMN: usize = 30;

/// The manual's opening: what mto is and how it works.
const MAN_INTRO: &str = r#"# mtindo (mto)
`mtindo` is a dependency-free layout tool for Rust source files. Its command
is `mto`. While `rustfmt` handles spacing and line wrapping inside items, `mto`
reorders the top-level items of a file (modules, imports, macros, constants,
types with their impls, functions) into a predictable top-down layout.
It never changes the inside of an item.
Layout is opt-in: until an `mto.toml` with `layout = true` applies, mto
leaves every file alone and says so. `mto init` writes one.
## How it works
*   **Parsed, not guessed:** each file is parsed into a syntax tree that keeps
every byte. A file with syntax errors is left untouched and reported as
skipped.
*   **Verified before every write:** the new text must hold exactly the
original items, byte for byte with their comments, in a new order and with
new blank lines between them. Anything else stops the write (exit code 4).
*   **Every level:** the same rules apply inside inline `mod name { ... }`
blocks, at every level; impl blocks are ordered only when asked
(`reorder_impl_items`, `impl_method_order`).
*   **Agrees with rustfmt:** imports and `mod` declarations sort exactly as
rustfmt sorts them for the style edition, and blank lines stay within rustfmt's
bounds, so rustfmt leaves the layout alone.
*   **Sticky Impl Grouping:** `impl Type` and `impl Trait for Type` blocks are
kept directly beneath the type they implement, its own impls first.
*   **Visibility Hierarchy:** Items of the same kind are ordered by visibility
(`visibility_order`), then by name (`name_sort`); modules are ordered by name
alone, as rustfmt does.
*   **Order-Sensitive Items Stay Put:** The file header (shebang, `//!`,
`#![...]`), `macro_rules!` order, `#[macro_use]` modules, `extern crate`, and
item-position macro calls are never moved past each other; a macro never ends
up below an item that originally followed it. `#[rustfmt::skip]` items and items
under `// mto: pin` keep their place, and `#[cfg]` variants of one item move
together.
*   **Atomic Writes:** Files are replaced by an OS-level rename of a synced
sibling temp file. Permissions are kept and symlinked files stay symlinks.
*   **History:** Every run that writes files is recorded, with each file's
contents before and after; `mto restore last` undoes the last one.
*   **CI/CD Check Mode:** Prints a diff for every file that would change and
exits with status 1.
*   **Stdin/Stdout Filtering:** `mto -` reads source from stdin and writes the
result to stdout.
## Installation
Run the following from the root of the repository:
```bash
cargo install --path .
```
"#;

/// The manual's closing: how writes are kept safe and how to keep text in place.
const MAN_SAFETY: &str = r#"## Safety and History
1. The new text is verified against the original before anything is written.
2. With history on (the default), the file's contents before and after are
stored, and synced, in the history folder (`mto config path` shows it), each
content once however many runs share it.
3. The new text is written to a synced sibling temp file, and an atomic
rename replaces the original file.
Each run that writes files is recorded under an id, shown by a short handle:
```bash
mto history            # the runs, newest first
mto history show last  # one run's files, and where each stands now
mto restore last       # undo the last run
mto restore 3f9a src/lib.rs
```
A restore puts back only files that still hold what the run wrote; `--force`
restores changed and missing files too. A restore is a run of its own, so it
can be undone. Runs are pruned after each run by the user config's `[history]`
settings, or with `mto history prune`.
With history off (`--no-history`, or `enabled = false` under `[history]`),
there is no undo, so mto leaves alone any file git sees uncommitted changes in,
unless `--allow-dirty` (or `allow_dirty = true` under `[safety]`). git is used
only if it is installed.
Text between `// mto: off` and `// mto: on` is never moved or changed. An item
directly under `// mto: on` stays inside the region; a region with no
`// mto: on` runs to the end of the file.
"#;

const USAGE_LINES: &[&str] = &[
    "mto [OPTIONS] <PATH>...",
    "mto [OPTIONS] - [--stdin-filepath PATH]",
    "mto <COMMAND> [ARGS]",
    "cargo mto [OPTIONS] [-- <MTO OPTIONS>...]",
];

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
    History {
        view: HistoryView,
    },
    Restore {
        run: String,
        files: Vec<PathBuf>,
        force: bool,
    },
    Init {
        target_path: Option<PathBuf>,
    },
    Debug {
        view: DebugView,
        path: PathBuf,
        edition: Edition,
    },
    Man,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigView {
    Path,
    Default,
    Current,
    Minimal,
    User,
}

#[derive(Clone, Copy)]
enum DebugView {
    Tokens,
    Tree,
}

/// Every flag mto accepts. The parser and the help text both read this table, so a flag
/// cannot be accepted without being documented.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flag {
    Check,
    Color,
    Emit,
    FilesWithDiff,
    PrintConfig,
    Quiet,
    SkipChildren,
    StdinFilepath,
    UnstableFeatures,
    Verbose,
    Config,
    ConfigPath,
    Current,
    Dbg,
    Default,
    DryRun,
    Edition,
    FollowLinks,
    Help,
    Man,
    Minimal,
    Recursive,
    NoHistory,
    AllowDirty,
    Force,
    OlderThan,
    Keep,
    StyleEdition,
    User,
    Version,
}

/// The groups `mto -h` lists flags in, in this order. Within a group, flags are listed by
/// name. `Command` flags belong to one command and are listed on its own help page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Layout,
    Files,
    Stdin,
    Configuration,
    History,
    Output,
    Compatibility,
    General,
    Command,
}

impl Group {
    const SHOWN: [Group; 8] = [
        Group::Layout,
        Group::Files,
        Group::Stdin,
        Group::Configuration,
        Group::History,
        Group::Output,
        Group::Compatibility,
        Group::General,
    ];

    /// The command the group's flags follow, shown beside its heading.
    fn command(self) -> &'static str {
        match self {
            Group::Layout | Group::Files | Group::History => "mto <PATH>...",
            Group::Stdin => "mto -",
            Group::Configuration | Group::Output => "mto <PATH>... | mto -",
            Group::Compatibility | Group::General | Group::Command => "mto",
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Group::Layout => "layout",
            Group::Files => "files",
            Group::Stdin => "stdin",
            Group::Configuration => "configuration",
            Group::History => "history and safety",
            Group::Output => "output",
            Group::Compatibility => "rustfmt compatibility",
            Group::General => "general",
            Group::Command => "command options",
        }
    }
}

enum HistoryView {
    List,
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

/// One of mto's commands: its help page, and its line in `mto -h`.
struct CommandSpec {
    name: &'static str,
    /// What follows the name, for `mto -h` and the manual.
    args: &'static str,
    /// One line, for `mto -h`.
    summary: &'static str,
    /// The command's help page's description.
    about: &'static str,
    usage: &'static [&'static str],
    /// The flags it takes, in sections: (heading, the command they follow, flags in the
    /// order its help lists them).
    options: &'static [(&'static str, &'static str, &'static [Flag])],
    examples: &'static [&'static str],
}

struct FlagSpec {
    flag: Flag,
    group: Group,
    short: Option<&'static str>,
    long: &'static str,
    value: &'static str,
    help: &'static str,
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
            "{}{} file(s) left alone: layout is off. Run `mto init` to write an mto.toml \
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
            "{}{} file(s) left unchanged by the verifier: a bug in mto, reported above.{}",
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

/// A flag as the manual names it: `-q, --quiet`, `--emit <files|stdout|json>`.
fn flag_name(spec: &FlagSpec) -> String {
    let mut name = match spec.short {
        Some(short) => format!("{}, {}", short, spec.long),
        None => spec.long.to_string(),
    };
    if !spec.value.is_empty() {
        name.push(' ');
        name.push_str(spec.value);
    }
    name
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

/// A group's flags, by name.
fn flags_of(group: Group) -> Vec<&'static FlagSpec> {
    let mut flags: Vec<&FlagSpec> = FLAGS.iter().filter(|spec| spec.group == group).collect();
    flags.sort_by_key(|spec| spec.long);
    flags
}

/// The user's history settings, and the history folder (which is read even with history
/// off, so earlier runs can still be restored).
fn history_store() -> Result<(history::Settings, mtindo::history::History), i32> {
    let resolver = resolver(&Overrides::default()).map_err(|e| report_error(&e))?;
    let settings =
        history::Settings::read(&resolver, false).map_err(|e| report_error(&MtoError::Usage(e)))?;
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
        FileKind::Rustfmt => "rustfmt config",
        FileKind::Mto => "mto config",
        FileKind::Explicit => "config (--config-path)",
        FileKind::User => "user config",
        FileKind::Cargo => "Cargo.toml (edition)",
    }
}

fn main() {
    let action = match parse_args() {
        Ok(Some(a)) => a,
        Ok(None) => return,
        Err(()) => std::process::exit(2),
    };
    let code = match action {
        CliAction::Man => {
            print_man_page();
            0
        }
        CliAction::Debug {
            view,
            path,
            edition,
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
        CliAction::Restore { run, files, force } => run_restore(&run, &files, force),
        CliAction::Format {
            paths,
            overrides,
            opts,
        } => run_format(&paths, &overrides, &opts),
    };
    std::process::exit(code);
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
        "restore" => return parse_restore_args(&args[1..]).map(Some),
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
        // `--flag=value` is read as `--flag value`, as rustfmt allows.
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
        if arg == "--backup" {
            return usage_error(
                "--backup is not needed: every run that writes files is recorded, and \
                 `mto restore last` undoes it",
            );
        }
        if arg == "--restore" || arg == "-r" {
            return usage_error(
                "--restore was removed: `mto restore last` undoes the last run, and \
                 `mto history` lists the others",
            );
        }
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
                    println!("mto {}", env!("CARGO_PKG_VERSION"));
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
                    return usage_error(&format!(
                        "{} only goes with `mto config print`",
                        spec.long
                    ));
                }
                Flag::NoHistory => opts.no_history = true,
                Flag::AllowDirty => opts.allow_dirty = true,
                Flag::Force | Flag::OlderThan | Flag::Keep => {
                    return usage_error(&format!(
                        "{} only goes with `mto restore` or `mto history prune`",
                        spec.long
                    ));
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
    let usage = || usage_error("usage: mto config path|print [OPTIONS] [DIR]");
    let mut view = match args.first().map(String::as_str) {
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
                Flag::Default | Flag::Current | Flag::Minimal | Flag::User
                    if view != ConfigView::Path =>
                {
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
            "{}Error: usage: mto debug tokens|tree <FILE> [--edition YEAR]{}",
            c_red(),
            c_reset(true)
        );
    };
    let view = match args.first().map(String::as_str) {
        Some("tokens") => DebugView::Tokens,
        Some("tree") => DebugView::Tree,
        _ => {
            usage();
            return Err(());
        }
    };
    let mut path = None;
    let mut edition = Edition::DEFAULT;
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
        }),
        None => {
            usage();
            Err(())
        }
    }
}

fn parse_history_args(args: &[String]) -> Result<CliAction, ()> {
    let usage =
        || usage_error("usage: mto history [show <RUN> | prune [--older-than DAYS] [--keep N]]");
    let view = match args.first().map(String::as_str) {
        None => HistoryView::List,
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
                            return usage_error(&format!(
                                "--keep takes a number of runs, not `{}`",
                                text
                            ));
                        }
                    },
                    _ => return usage(),
                }
                i += 1;
            }
            HistoryView::Prune(prune)
        }
        Some("list") if args.len() == 1 => HistoryView::List,
        _ => return usage(),
    };
    Ok(CliAction::History { view })
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
    let manual = render_man();
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
                let colored = line
                    .replace("mto", "\x1b[32mmto\x1b[0m")
                    .replace("cargo install", "\x1b[32mcargo install\x1b[0m")
                    .replace("[OPTIONS]", "\x1b[36m[OPTIONS]\x1b[0m")
                    .replace("[TARGET_PATH]", "\x1b[36m[TARGET_PATH]\x1b[0m")
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

/// The full manual: the fixed prose, then the command line, exit codes and configuration,
/// generated from the same tables the parser and the settings checks use.
fn render_man() -> String {
    let wrap = mtindo::cli::manual::wrap;
    let mut out = String::from(MAN_INTRO);
    out.push_str("## Command Line Interface\n```text\n");
    for line in USAGE_LINES {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("```\n");
    out.push_str(&wrap(
        "`<PATH>`: a single `.rs` file or a directory. Directories are traversed \
         recursively, skipping `ignore_dirs` and symlinked entries. Running `mto` with \
         no arguments prints the short help.",
        "*   ",
    ));
    out.push_str(&wrap(
        "`mto -`: Read source from stdin and write the result to stdout; \
         `--stdin-filepath` says where it belongs.",
        "*   ",
    ));
    for group in Group::SHOWN {
        let heading = group.heading();
        out.push_str(&format!(
            "### {}{} (`{}`)\n",
            heading[..1].to_uppercase(),
            &heading[1..],
            group.command()
        ));
        for spec in flags_of(group) {
            out.push_str(&wrap(
                &format!("`{}`: {}.", flag_name(spec), spec.help),
                "*   ",
            ));
        }
    }
    out.push_str("### Commands\n");
    for command in COMMANDS {
        let usage = format!("mto {} {}", command.name, command.args);
        out.push_str(&wrap(
            &format!("`{}`: {}", usage.trim_end(), command.about),
            "*   ",
        ));
        for (_, after, flags) in command.options {
            for flag in *flags {
                let spec = spec_of(*flag);
                out.push_str(&wrap(
                    &format!("`{}` (after `{}`): {}.", flag_name(spec), after, spec.help),
                    "    *   ",
                ));
            }
        }
    }
    out.push_str("## Cargo\n");
    out.push_str(&wrap(
        "`cargo mto` lays out the targets of the current package or workspace, as `cargo fmt` \
         formats them: every member when run from the workspace root, else the current \
         package. Each target's root file (library, binaries, examples, tests, benches, build \
         script, as Cargo discovers them) brings the modules it declares. The manifests are \
         read directly; Cargo is not run. Options after `--` go to `mto`.",
        "",
    ));
    for (_, flags, value, help) in mtindo::cli::CARGO_MTO_OPTIONS {
        let name = if value.is_empty() {
            flags.to_string()
        } else {
            format!("{} {}", flags, value)
        };
        out.push_str(&wrap(&format!("`cargo mto {}`: {}.", name, help), "*   "));
    }
    out.push_str("## Exit Codes\n");
    for (code, meaning) in EXIT_CODES {
        out.push_str(&wrap(&format!("`{}`: {}.", code, meaning), "*   "));
    }
    out.push_str(&mtindo::cli::manual::configuration());
    out.push_str(MAN_SAFETY);
    spaced(&out)
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
    let (jobs, problems) = match collect_jobs(paths, &mut resolver, opts) {
        Ok(found) => found,
        Err(e) => return report_error(&e),
    };
    for problem in problems {
        eprintln!("{}warning: {}{}", c_yellow(true), problem, c_reset(true));
    }
    let settings = match history::Settings::read(&resolver, opts.no_history) {
        Ok(settings) => settings,
        Err(e) => return report_error(&MtoError::Usage(e)),
    };
    let mut writes = Writes::default();
    let writing = opts.emit == Emit::Files && !opts.check && !opts.dry_run;
    if writing {
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
    let done = run_jobs(&jobs, opts, &mut writes);
    if let Some(recorder) = writes.recorder.as_mut() {
        history::finish_record(recorder, settings.limits, opts.quiet);
    }
    finish(&done, opts)
}

/// `mto history`: lists, shows or prunes the runs.
fn run_history(view: HistoryView) -> i32 {
    let (settings, store) = match history_store() {
        Ok(found) => found,
        Err(code) => return code,
    };
    let result = match view {
        HistoryView::List => history::list(&store),
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

/// `--print-config`, as rustfmt prints it: rustfmt's settings as TOML, ready for a
/// `rustfmt.toml`. `default` and `minimal` go to PATH when given, else stdout; `current`
/// prints the settings for PATH (a file or folder; the current folder by default).
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

/// The manual with a blank line before every heading, so sections and entries stand apart
/// in a terminal. Code blocks are left as they are.
fn spaced(manual: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in manual.lines() {
        if line.starts_with("```") {
            in_code = !in_code;
        }
        let heading =
            !in_code && line.starts_with('#') && line.trim_start_matches('#').starts_with(' ');
        if heading && !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn spec_of(flag: Flag) -> &'static FlagSpec {
    FLAGS
        .iter()
        .find(|spec| spec.flag == flag)
        .expect("every flag has a spec")
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
