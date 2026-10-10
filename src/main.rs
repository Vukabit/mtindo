use std::env;
use std::path::PathBuf;

use mtindo::config::{self, Config, Environment, FileKind, Overrides, Resolver, Source};
use mtindo::options::{self, File};
use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::tree::debug_dump;
use mtindo::syntax::{Edition, parse};
use mtindo::{
    HelpPalette, MtoError, Outcome, RunOptions, c_cyan, c_green, c_red, c_reset, c_yellow,
    print_warnings, process_file, process_init, process_stdin, restore_backup, walk_directory,
};

/// Arguments that are not flags: (name, value, help).
const COMMANDS: &[(&str, &str, &str)] = &[
    (
        "-",
        "",
        "Read source from stdin and write the result to stdout",
    ),
    (
        "init",
        "[DIR]",
        "Write a default mto.toml into DIR (default: the current directory)",
    ),
    (
        "config",
        "path|print [DIR]",
        "Show the configuration files, or the settings, that apply in DIR",
    ),
    (
        "debug",
        "tokens|tree <FILE> [--edition YEAR]",
        "Print the tokens or syntax tree mto sees (unstable output, for bug reports)",
    ),
];

const EXAMPLES: &[&str] = &[
    "mto src/main.rs",
    "mto -R --check",
    "cat src/main.rs | mto -",
    "mto --config max_width=80,hard_tabs=true src/",
    "mto config print --minimal",
    "mto --restore /tmp/mtindo_backups/123.bak src/main.rs",
];

const EXIT_CODES: &[(&str, &str)] = &[
    ("0", "success"),
    ("1", "--check found files that would change"),
    ("2", "bad arguments or configuration"),
    ("3", "a file could not be read or written"),
    (
        "4",
        "the verifier stopped a write (a bug in mto); the file is unchanged",
    ),
];

const FLAGS: &[FlagSpec] = &[
    FlagSpec {
        flag: Flag::Help,
        short: Some("-h"),
        long: "--help",
        value: "",
        help: "Show this help and exit",
    },
    FlagSpec {
        flag: Flag::Version,
        short: Some("-V"),
        long: "--version",
        value: "",
        help: "Print the version and exit",
    },
    FlagSpec {
        flag: Flag::Check,
        short: None,
        long: "--check",
        value: "",
        help: "Print a diff for every file that would change; exit 1 if any",
    },
    FlagSpec {
        flag: Flag::DryRun,
        short: Some("-d"),
        long: "--dry-run",
        value: "",
        help: "List the files that would change, without writing",
    },
    FlagSpec {
        flag: Flag::Recursive,
        short: Some("-R"),
        long: "--recursive",
        value: "",
        help: "Format every .rs file under the current directory",
    },
    FlagSpec {
        flag: Flag::Config,
        short: None,
        long: "--config",
        value: "<KEY=VALUE,...>",
        help: "Set settings for this run, over every file",
    },
    FlagSpec {
        flag: Flag::ConfigPath,
        short: None,
        long: "--config-path",
        value: "<FILE>",
        help: "Use only this configuration file; search for no other",
    },
    FlagSpec {
        flag: Flag::Edition,
        short: None,
        long: "--edition",
        value: "<YEAR>",
        help: "The Rust edition, over Cargo.toml and configuration files",
    },
    FlagSpec {
        flag: Flag::StyleEdition,
        short: None,
        long: "--style-edition",
        value: "<YEAR>",
        help: "The style edition, which decides the defaults",
    },
    FlagSpec {
        flag: Flag::Default,
        short: None,
        long: "--default",
        value: "",
        help: "config print: every setting at its default",
    },
    FlagSpec {
        flag: Flag::Current,
        short: None,
        long: "--current",
        value: "",
        help: "config print: every setting in effect, with its source (the default)",
    },
    FlagSpec {
        flag: Flag::Minimal,
        short: None,
        long: "--minimal",
        value: "",
        help: "config print: only the settings that differ from their defaults",
    },
    FlagSpec {
        flag: Flag::User,
        short: None,
        long: "--user",
        value: "",
        help: "config print: the user config's settings",
    },
    FlagSpec {
        flag: Flag::FollowLinks,
        short: None,
        long: "--follow-links",
        value: "",
        help: "Also follow symlinked files and directories",
    },
    FlagSpec {
        flag: Flag::Restore,
        short: Some("-r"),
        long: "--restore",
        value: "<BACKUP> <TARGET>",
        help: "Atomically replace TARGET with the BACKUP copy",
    },
    FlagSpec {
        flag: Flag::Dbg,
        short: None,
        long: "--dbg",
        value: "",
        help: "Print how each file was split into items",
    },
    FlagSpec {
        flag: Flag::Man,
        short: None,
        long: "--man",
        value: "",
        help: "Print the full manual",
    },
];

/// The column where help descriptions start.
const HELP_COLUMN: usize = 36;

/// The manual's opening: what mto is and how it works.
const MAN_INTRO: &str = r#"# mtindo (mto)
`mtindo` is a dependency-free layout tool for Rust source files. Its command
is `mto`. While `rustfmt` handles spacing and line wrapping inside items, `mto`
reorders the top-level items of a file (modules, imports, macros, constants,
types with their impls, functions) into a predictable top-down layout.
It never changes the inside of an item.
## How it works
*   **Parsed, not guessed:** each file is parsed into a syntax tree that keeps
every byte. A file with syntax errors is left untouched and reported as
skipped.
*   **Verified before every write:** the new text must hold exactly the
original items, byte for byte with their comments, in a new order and with
new blank lines between them. Anything else stops the write (exit code 4).
*   **Sticky Impl Grouping:** `impl Type` and `impl Trait for Type` blocks are
kept directly beneath the type they implement.
*   **Visibility Hierarchy:** Items of the same kind are ordered by visibility
(`pub` -> `pub(super)` -> `pub(crate)` -> private); modules are ordered by name
alone, as rustfmt does.
*   **Order-Sensitive Items Stay Put:** The file header (shebang, `//!`,
`#![...]`), `macro_rules!` order, `#[macro_use]` modules, `extern crate`, and
item-position macro calls are never moved past each other; a macro never ends
up below an item that originally followed it; a `#[cfg(test)]` module stays
last.
*   **Atomic Writes:** Files are replaced by an OS-level rename of a synced
sibling temp file. Permissions are kept and symlinked files stay symlinks.
*   **Backups:** Every changed file is first copied to the system temporary
directory, and the undo command is printed.
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
const MAN_SAFETY: &str = r#"## Safety and Backups
1. Before a file is modified, an exact copy is saved to
`/tmp/mtindo_backups/` (or your OS equivalent).
2. The reordered source is written to a synced sibling temp file.
3. An atomic rename replaces the original file.
To revert a file, run the undo command printed after formatting:
```bash
mto --restore /tmp/mtindo_backups/1700000000000_123_0_main.rs.bak main.rs
```
Text between `// mto: off` and `// mto: on` is never moved or changed. An item
directly under `// mto: on` stays inside the region; a region with no
`// mto: on` runs to the end of the file.
"#;

const USAGE_LINES: &[&str] = &[
    "mto [OPTIONS] <PATH>",
    "mto [OPTIONS] -",
    "mto --restore <BACKUP> <TARGET>",
    "mto init [DIR]",
    "mto config path|print [OPTIONS] [DIR]",
    "mto debug tokens|tree <FILE> [--edition YEAR]",
];

enum CliAction {
    Format {
        target_path: PathBuf,
        overrides: Overrides,
        opts: RunOptions,
    },
    Stdin {
        overrides: Overrides,
        check: bool,
    },
    Config {
        view: ConfigView,
        dir: PathBuf,
        overrides: Overrides,
    },
    Restore {
        backup_path: PathBuf,
        target_path: PathBuf,
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
    Restore,
    StyleEdition,
    User,
    Version,
}

struct FlagSpec {
    flag: Flag,
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
        CliAction::Stdin { overrides, check } => match resolver(&overrides) {
            Ok(mut resolver) => match process_stdin(&mut resolver, check) {
                Ok(Outcome::VerifierFailed) => 4,
                Ok(Outcome::Changed) if check => 1,
                Ok(_) => 0,
                Err(e) => report_error(&e),
            },
            Err(e) => report_error(&e),
        },
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
        CliAction::Restore {
            backup_path,
            target_path,
        } => {
            if !backup_path.exists() {
                report_error(&MtoError::Usage(format!(
                    "backup file {} does not exist",
                    backup_path.display()
                )))
            } else {
                match restore_backup(&backup_path, &target_path) {
                    Ok(_) => {
                        println!(
                            "{}Successfully restored {} from backup.{}",
                            c_green(),
                            target_path.display(),
                            c_reset(false)
                        );
                        0
                    }
                    Err(e) => report_error(&MtoError::Io(format!("failed to restore file: {}", e))),
                }
            }
        }
        CliAction::Format {
            target_path,
            overrides,
            opts,
        } => run_format(target_path, &overrides, &opts),
    };
    std::process::exit(code);
}

fn parse_args() -> Result<Option<CliAction>, ()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        return Ok(None);
    }
    match args[0].as_str() {
        "init" => {
            let target_path = args.get(1).map(PathBuf::from);
            return Ok(Some(CliAction::Init { target_path }));
        }
        "debug" => return parse_debug_args(&args[1..]).map(Some),
        "config" => return parse_config_args(&args[1..]).map(Some),
        _ => {}
    }
    let mut target_path = None;
    let mut read_stdin = false;
    let mut overrides = Overrides::default();
    let mut recursive = false;
    let mut opts = RunOptions::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
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
                Flag::Config | Flag::ConfigPath | Flag::Edition | Flag::StyleEdition => {
                    i += 1;
                    config_flag(spec, args.get(i), &mut overrides)?;
                }
                Flag::Default | Flag::Current | Flag::Minimal | Flag::User => {
                    return usage_error(&format!(
                        "{} only goes with `mto config print`",
                        spec.long
                    ));
                }
                Flag::Restore => {
                    if i + 2 >= args.len() {
                        return usage_error("--restore needs <BACKUP> <TARGET>");
                    }
                    return Ok(Some(CliAction::Restore {
                        backup_path: PathBuf::from(&args[i + 1]),
                        target_path: PathBuf::from(&args[i + 2]),
                    }));
                }
            }
        } else if arg == "-" && !read_stdin && target_path.is_none() {
            read_stdin = true;
        } else if !arg.starts_with('-') && !read_stdin && target_path.is_none() {
            target_path = Some(PathBuf::from(arg));
        } else {
            return usage_error(&format!("unexpected argument '{}'", arg));
        }
        i += 1;
    }
    if read_stdin {
        return Ok(Some(CliAction::Stdin {
            overrides,
            check: opts.check,
        }));
    }
    if recursive && target_path.is_none() {
        target_path = Some(PathBuf::from("."));
    }
    match target_path {
        Some(path) => Ok(Some(CliAction::Format {
            target_path: path,
            overrides,
            opts,
        })),
        None => usage_error(
            "no path given: pass a file or directory, -R for the current directory, or - to read stdin",
        ),
    }
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
                    .replace("--path .", "\x1b[33m--path .\x1b[0m")
                    .replace("--restore", "\x1b[33m--restore\x1b[0m");
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
        return out;
    }
    out.push_str(&format!("# style edition {}\n", config.style_edition()));
    for (setting, value, source) in config.values() {
        let shown = match view {
            ConfigView::Default => !setting.hidden && setting.file != File::User,
            ConfigView::Current => {
                setting.file != File::User && (!setting.hidden || *source != Source::Default)
            }
            ConfigView::Minimal => {
                setting.file != File::User
                    && !matches!(source, Source::Default | Source::Derived(_))
            }
            ConfigView::User => setting.file == File::User,
            ConfigView::Path => false,
        };
        if !shown {
            continue;
        }
        if view == ConfigView::Default {
            let default = setting.default_for(config.style_edition());
            out.push_str(&format!("{} = {}\n", setting.name, default));
        } else {
            out.push_str(&format!(
                "{} = {}  # {}\n",
                setting.name,
                value.to_toml(),
                source
            ));
        }
    }
    if view == ConfigView::User && !options::all().any(|s| s.file == File::User) {
        out.push_str("# mto has no user settings yet\n");
    }
    out
}

/// The short help. The text is fixed; only the colors depend on `color`.
fn render_help(color: bool) -> String {
    let p = HelpPalette::new(color, "mto");
    let mut out = String::new();
    for (idx, line) in USAGE_LINES.iter().enumerate() {
        let lead = if idx == 0 {
            p.heading("usage:")
        } else {
            "      ".to_string()
        };
        out.push_str(&format!("{} {}\n", lead, p.usage_line(line)));
    }
    out.push_str("\nReorders the top-level items of Rust files into a predictable layout, safely and atomically.\n");
    out.push_str(&format!("\n{}\n", p.heading("options:")));
    for spec in FLAGS {
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
        out.push_str(&p.row(&label, width, HELP_COLUMN, spec.help));
    }
    out.push_str(&format!("\n{}\n", p.heading("commands:")));
    for (name, value, help) in COMMANDS {
        let painted_name = if *name == "-" {
            p.short(name)
        } else {
            p.long(name)
        };
        let painted_value = if value.is_empty() {
            String::new()
        } else {
            format!(" {}", p.value(value))
        };
        let label = format!("  {}{}", painted_name, painted_value);
        let width = 2 + name.len() + if value.is_empty() { 0 } else { 1 + value.len() };
        out.push_str(&p.row(&label, width, HELP_COLUMN, help));
    }
    out.push_str(&format!("\n{}\n", p.heading("exit codes:")));
    for (code, meaning) in EXIT_CODES {
        out.push_str(&format!("  {}  {}\n", p.value(code), meaning));
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
    for spec in FLAGS {
        let mut name = match spec.short {
            Some(short) => format!("{}, {}", short, spec.long),
            None => spec.long.to_string(),
        };
        if !spec.value.is_empty() {
            name.push(' ');
            name.push_str(spec.value);
        }
        out.push_str(&wrap(&format!("`{}`: {}.", name, spec.help), "*   "));
    }
    for (name, value, help) in COMMANDS {
        let usage = if *name == "-" {
            "mto -".to_string()
        } else {
            format!("mto {} {}", name, value)
        };
        out.push_str(&wrap(&format!("`{}`: {}.", usage.trim_end(), help), "*   "));
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
    Ok(Resolver::new(overrides, env)?)
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

fn run_format(target_path: PathBuf, overrides: &Overrides, opts: &RunOptions) -> i32 {
    let mut resolver = match resolver(overrides) {
        Ok(resolver) => resolver,
        Err(e) => return report_error(&e),
    };
    if !target_path.exists() {
        return report_error(&MtoError::Usage(format!(
            "target path '{}' does not exist",
            target_path.display()
        )));
    }
    let outcomes = if target_path.is_file() {
        let dir = target_path.parent().unwrap_or(std::path::Path::new("."));
        let config = match resolver.config_for(dir) {
            Ok(config) => config,
            Err(e) => return report_error(&e.into()),
        };
        print_warnings(&mut resolver);
        if opts.debug {
            for (kind, path) in config.files() {
                eprintln!(
                    "{}Using {}:{} {}",
                    c_cyan(),
                    kind_label(*kind),
                    c_reset(true),
                    path.display()
                );
            }
        }
        vec![process_file(&target_path, &config.layout(), opts)]
    } else {
        match walk_directory(&target_path, &mut resolver, opts) {
            Ok(outcomes) => outcomes,
            Err(e) => return report_error(&e),
        }
    };
    let changed = outcomes.iter().filter(|o| **o == Outcome::Changed).count();
    let skipped = outcomes.iter().filter(|o| **o == Outcome::Skipped).count();
    let failed = outcomes.iter().filter(|o| **o == Outcome::Failed).count();
    let unsafe_writes = outcomes
        .iter()
        .filter(|o| **o == Outcome::VerifierFailed)
        .count();
    if skipped > 0 {
        eprintln!(
            "{}{} file(s) skipped because they could not be split safely.{}",
            c_yellow(true),
            skipped,
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
    if failed > 0 {
        return 3;
    }
    if opts.check {
        if changed > 0 {
            eprintln!(
                "{}{} file(s) would be reformatted.{}",
                c_red(),
                changed,
                c_reset(true)
            );
            return 1;
        }
        println!(
            "{}Check passed successfully: no formatting drift detected.{}",
            c_green(),
            c_reset(false)
        );
    }
    0
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

/// Prints a usage error and points at the help; returns the error for `?`-style use.
fn usage_error<T>(message: &str) -> Result<T, ()> {
    eprintln!("{}Error: {}{}", c_red(), message, c_reset(true));
    eprintln!("Run 'mto --help' for usage.");
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_help_documents_every_flag_and_command() {
        let help = render_help(false);
        for spec in FLAGS {
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
        for (name, _, _) in COMMANDS {
            assert!(
                help.contains(&format!("\n  {}", name)),
                "{} missing from help",
                name
            );
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
}
