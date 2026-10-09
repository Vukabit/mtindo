use std::env;
use std::path::PathBuf;

use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::tree::debug_dump;
use mtindo::syntax::{Edition, parse};
use mtindo::{
    HelpPalette, LayoutConfig, MtoError, Outcome, RunOptions, c_cyan, c_green, c_red, c_reset,
    c_yellow, parse_toml_config, process_file, process_init, process_stdin, resolve_config_path,
    restore_backup, walk_directory,
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
        "debug",
        "tokens|tree <FILE> [--edition YEAR]",
        "Print the tokens or syntax tree mto sees (unstable output, for bug reports)",
    ),
];

const EXAMPLES: &[&str] = &[
    "mto src/main.rs",
    "mto -R --check",
    "cat src/main.rs | mto -",
    "mto --restore /tmp/mtindo_backups/123.bak src/main.rs",
];

const EXIT_CODES: &[(&str, &str)] = &[
    ("0", "success"),
    ("1", "--check found files that would change"),
    ("2", "bad arguments or configuration"),
    ("3", "a file could not be read or written"),
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
        short: Some("-c"),
        long: "--config",
        value: "<FILE>",
        help: "Use this configuration file instead of searching for one",
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

const MAN_PAGE: &str = r#"# mtindo (mto)
`mtindo` is a dependency-free layout tool for Rust source files. Its command
is `mto`. While `rustfmt` handles spacing and line wrapping inside items, `mto`
reorders the top-level items of a file (modules, imports, macros, constants,
types with their impls, functions) into a predictable top-down layout.
It never changes the inside of an item.
## Core Features
*   **Structure-Aware Splitting:** A standard-library scanner tracks braces,
strings, raw strings, char literals and nested comments to find where each
top-level item starts and ends.
*   **Refuses When Unsure:** If a file cannot be split cleanly (unbalanced
delimiters, an unterminated string or comment, mixed line endings), it is
left untouched and reported as skipped.
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
## Command Line Interface
```text
mto [OPTIONS] <PATH>
mto [OPTIONS] -
```
*   `<PATH>`: A single `.rs` file or a directory. Directories are
traversed recursively, skipping `ignore_dirs` and symlinked entries.
*   `-`: Read source from stdin and write the result to stdout.
*   Running `mto` with no arguments prints the short help.
*   `-R, --recursive`: Format all `.rs` files from the current directory.
*   `-c, --config <FILE>`: Use a specific configuration file.
*   `-d, --dry-run`: List the files that would change without writing.
*   `--check`: Print a diff for every file that would change; exit 1 if any.
*   `--follow-links`: Also follow symlinked files and directories (each real
path is still visited once).
*   `-r, --restore <BACKUP> <TARGET>`: Atomically replace the target file with
the specified backup.
*   `--dbg`: Print how each file was split into items.
*   `--man`: Print this manual.
*   `-h, --help`: Print the short help menu.
*   `-V, --version`: Print the version.
*   `mto debug tokens|tree <FILE> [--edition YEAR]`: Print the tokens or the
syntax tree mto sees in a file, for bug reports. The output format is unstable.
## Exit Codes
*   `0`: success.
*   `1`: `--check` found files that would change.
*   `2`: bad arguments or configuration.
*   `3`: a file could not be read or written.
## Configuration (mto.toml)
`mto` looks for configuration in this order and uses the first file found:
1. The path passed via `--config`.
2. `mto.toml` or `.mto.toml` in the target's directory, then each parent
directory in turn.
3. `~/.config/mtindo/mto.toml` or `~/.config/mtindo/.mto.toml`.
4. `~/mto.toml` or `~/.mto.toml`.
5. Internal defaults.
```toml
# Sort items of the same kind and visibility alphabetically.
sort_alphabetically = true
# Keep `impl` blocks directly below the struct/enum/trait they implement.
group_impls = true
# Sort use declarations, and separate std, external and local groups.
sort_imports = true
import_grouping = true
# Blank lines between items.
blank_lines_between_blocks = 1
# The top-down order of item kinds. Valid tokens: ModDecl, UseDecl,
# MacroDecl, ConstDecl, StaticDecl, TraitDecl, EnumDecl, StructDecl,
# ImplBlock, FnDecl, Unknown
type_order = [
    "ModDecl",
    "UseDecl",
    "MacroDecl",
    "ConstDecl",
    "StaticDecl",
    "TraitDecl",
    "EnumDecl",
    "StructDecl",
    "ImplBlock",
    "FnDecl",
    "Unknown"
]
```
`max_width` and `tab_width` are accepted but have no effect yet; they are
reserved for the formatter.
## Safety and Backups
1. Before a file is modified, an exact copy is saved to
`/tmp/mtindo_backups/` (or your OS equivalent).
2. The reordered source is written to a synced sibling temp file.
3. An atomic rename replaces the original file.
To revert a file, run the undo command printed after formatting:
```bash
mto --restore /tmp/mtindo_backups/1700000000000_123_0_src_main.rs.bak src/main.rs
```
Text between `// mto: off` and `// mto: on` is never moved or changed.
"#;

const USAGE_LINES: &[&str] = &[
    "mto [OPTIONS] <PATH>",
    "mto [OPTIONS] -",
    "mto --restore <BACKUP> <TARGET>",
    "mto init [DIR]",
    "mto debug tokens|tree <FILE> [--edition YEAR]",
];

enum CliAction {
    Format {
        target_path: PathBuf,
        config_path: Option<PathBuf>,
        opts: RunOptions,
    },
    Stdin {
        config_path: Option<PathBuf>,
        check: bool,
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
    Dbg,
    DryRun,
    FollowLinks,
    Help,
    Man,
    Recursive,
    Restore,
    Version,
}

struct FlagSpec {
    flag: Flag,
    short: Option<&'static str>,
    long: &'static str,
    value: &'static str,
    help: &'static str,
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
        CliAction::Stdin { config_path, check } => match process_stdin(config_path, check) {
            Ok(Outcome::Changed) if check => 1,
            Ok(_) => 0,
            Err(e) => report_error(&e),
        },
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
            config_path,
            opts,
        } => run_format(target_path, config_path, &opts),
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
        _ => {}
    }
    let mut target_path = None;
    let mut read_stdin = false;
    let mut config_path = None;
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
                Flag::Config => {
                    let value = match args.get(i + 1) {
                        Some(value) => value,
                        None => return usage_error("--config needs a <FILE>"),
                    };
                    config_path = Some(PathBuf::from(value));
                    i += 1;
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
            config_path,
            check: opts.check,
        }));
    }
    if recursive && target_path.is_none() {
        target_path = Some(PathBuf::from("."));
    }
    match target_path {
        Some(path) => Ok(Some(CliAction::Format {
            target_path: path,
            config_path,
            opts,
        })),
        None => usage_error(
            "no path given: pass a file or directory, -R for the current directory, or - to read stdin",
        ),
    }
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
    if !mtindo::color_enabled(false) {
        println!("{}", MAN_PAGE);
        return;
    }
    let mut in_code_block = false;
    let mut lang = String::new();
    for line in MAN_PAGE.lines() {
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
        if line.starts_with("# ") || line.starts_with("## ") {
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

fn report_error(error: &MtoError) -> i32 {
    eprintln!("{}Error: {}{}", c_red(), error, c_reset(true));
    error.exit_code()
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

fn run_format(target_path: PathBuf, config_path: Option<PathBuf>, opts: &RunOptions) -> i32 {
    let config = match resolve_config_path(config_path, Some(&target_path)) {
        Some(path) => {
            if opts.debug {
                eprintln!(
                    "{}Using configuration:{} {}",
                    c_cyan(),
                    c_reset(true),
                    path.display()
                );
            }
            match parse_toml_config(&path) {
                Ok(config) => config,
                Err(e) => return report_error(&e),
            }
        }
        None => LayoutConfig::default(),
    };
    if !target_path.exists() {
        return report_error(&MtoError::Usage(format!(
            "target path '{}' does not exist",
            target_path.display()
        )));
    }
    let outcomes = if target_path.is_file() {
        vec![process_file(&target_path, &config, opts)]
    } else {
        walk_directory(&target_path, &config, opts)
    };
    let changed = outcomes.iter().filter(|o| **o == Outcome::Changed).count();
    let skipped = outcomes.iter().filter(|o| **o == Outcome::Skipped).count();
    let failed = outcomes.iter().filter(|o| **o == Outcome::Failed).count();
    if skipped > 0 {
        eprintln!(
            "{}{} file(s) skipped because they could not be split safely.{}",
            c_yellow(true),
            skipped,
            c_reset(true)
        );
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
