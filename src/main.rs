use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;

use mtindo::{
    LayoutConfig, MtoError, Outcome, RunOptions, c_cyan, c_green, c_red, c_reset, c_yellow,
    parse_toml_config, process_file, process_init, process_stdin, resolve_config_path,
    restore_backup, walk_directory,
};

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
(`pub` -> `pub(super)` -> `pub(crate)` -> private).
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
*   **Stdin/Stdout Filtering:** Reads source from stdin when no path is given.
## Installation
Run the following from the root of the repository:
```bash
cargo install --path .
```
## Command Line Interface
```text
mto [OPTIONS] [TARGET_PATH]
```
*   `[TARGET_PATH]`: A single `.rs` file or a directory. Directories are
traversed recursively, skipping `ignore_dirs` and symlinked entries.
*   `-R, --recursive`: Format all `.rs` files from the current directory.
*   `-c, --config <FILE>`: Use a specific configuration file.
*   `-d, --dry-run`: List the files that would change without writing.
*   `--check`: Print a diff for every file that would change; exit 1 if any.
*   `--follow-links`: Also follow symlinked files and directories (each real
path is still visited once).
*   `-r, --restore <BACKUP> <TARGET>`: Atomically replace the target file with
the specified backup.
*   `--man`: Print this manual.
*   `-h, --help`: Print the short help menu.
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
    Man,
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
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        if !std::io::stdin().is_terminal() {
            return Ok(Some(CliAction::Stdin {
                config_path: None,
                check: false,
            }));
        }
        print_help();
        return Ok(None);
    }
    if args[1] == "init" {
        let target_path = args.get(2).map(PathBuf::from);
        return Ok(Some(CliAction::Init { target_path }));
    }
    let mut i = 1;
    let mut target_path = None;
    let mut config_path = None;
    let mut recursive = false;
    let mut opts = RunOptions::default();
    while i < args.len() {
        match args[i].as_str() {
            "--man" => return Ok(Some(CliAction::Man)),
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "-d" | "--dry-run" => {
                opts.dry_run = true;
            }
            "--dbg" => {
                opts.debug = true;
            }
            "--check" => {
                opts.check = true;
            }
            "--follow-links" => {
                opts.follow_links = true;
            }
            "-R" | "--recursive" => {
                recursive = true;
            }
            "-c" | "--config" => {
                if i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                } else {
                    eprintln!(
                        "{}Error: Missing path for --config{}",
                        c_red(),
                        c_reset(true)
                    );
                    return Err(());
                }
            }
            "-r" | "--restore" => {
                if i + 2 < args.len() {
                    return Ok(Some(CliAction::Restore {
                        backup_path: PathBuf::from(&args[i + 1]),
                        target_path: PathBuf::from(&args[i + 2]),
                    }));
                } else {
                    eprintln!(
                        "{}Error: --restore requires <BACKUP_PATH> <TARGET_PATH>{}",
                        c_red(),
                        c_reset(true)
                    );
                    return Err(());
                }
            }
            path if target_path.is_none() && !path.starts_with('-') => {
                target_path = Some(PathBuf::from(path));
            }
            _ => {
                eprintln!(
                    "{}Error: Unknown argument '{}'{}",
                    c_red(),
                    args[i],
                    c_reset(true)
                );
                return Err(());
            }
        }
        i += 1;
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
        None => {
            if !std::io::stdin().is_terminal() {
                Ok(Some(CliAction::Stdin {
                    config_path,
                    check: opts.check,
                }))
            } else {
                eprintln!(
                    "{}Error: No target path provided. Use -R to format current directory recursively, or pipe via stdin.{}",
                    c_red(),
                    c_reset(true)
                );
                Err(())
            }
        }
    }
}

fn print_help() {
    println!(
        "{}mtindo (mto) - Rust Source Reorganizer{}",
        c_cyan(),
        c_reset(false)
    );
    println!("Safely and atomically reorders the top-level items of Rust files.\n");
    println!("{}USAGE:{}", c_yellow(false), c_reset(false));
    println!("    mto [OPTIONS] [TARGET_PATH]");
    println!("    mto --restore <BACKUP_FILE> <TARGET_PATH>");
    println!("    mto init [DIR]");
    println!("    cat file.rs | mto\n");
    println!("{}OPTIONS:{}", c_yellow(false), c_reset(false));
    println!("    -c, --config <FILE>     Specify custom .toml config path");
    println!("    -d, --dry-run           List files that would change without saving");
    println!("    -R, --recursive         Format all .rs files from current directory");
    println!(
        "    --check                 Print a diff for files that would change (exit 1 if any)"
    );
    println!("    --follow-links          Follow symlinked files and directories");
    println!("    -r, --restore           Restore a file from a backup safely");
    println!("        --man               Print the full manual page");
    println!("    -h, --help              Print this help information\n");
    println!("{}EXIT CODES:{}", c_yellow(false), c_reset(false));
    println!(
        "    0 success, 1 check found changes, 2 bad arguments or config, 3 read/write failure\n"
    );
    println!("{}EXAMPLES:{}", c_yellow(false), c_reset(false));
    println!("    mto src/main.rs");
    println!("    mto -R --check");
    println!("    cat src/main.rs | mto");
    println!("    mto --restore /tmp/mtindo_backups/123.bak src/main.rs");
}

fn print_man_page() {
    if !std::io::stdout().is_terminal() {
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

fn report_error(error: &MtoError) -> i32 {
    eprintln!("{}Error: {}{}", c_red(), error, c_reset(true));
    error.exit_code()
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
