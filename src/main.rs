use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;

use mtindo::{
    c_cyan, c_green, c_red, c_reset, c_yellow, parse_toml_config, process_file,
        process_init,
    process_stdin, resolve_config_path, restore_backup, walk_directory,
        LayoutConfig,
};

const MAN_PAGE: &str = r#"# mtindo (mto)
`mtindo` is a dependency-free structural reorganizer for Rust source files.
While `rustfmt` handles internal spacing and line wrapping, `mtindo` operates at
the block level. It physically reorders your source code—moving structs,
enums, impls, and functions into a predictable, strictly organized top-down
layout.
## Core Features
*   **AST-Aware Block Sorting:** Parses brace depth, raw strings, and block
attributes utilizing only the standard library.
*   **Sticky Impl Grouping:** Automatically anchors `impl` and `impl Trait for`
blocks directly beneath their parent data structures.
*   **Visibility Hierarchy:** Sorts identical item types internally by
visibility (`pub` -> `pub(super)` -> `pub(crate)` -> private).
*   **Atomic Writes:** Executes file updates using OS-level atomic renames to
prevent partial writes.
*   **Failsafe Backups:** Automatically snapshots your source code to the
system temporary directory before modification.
*   **CI/CD Check Mode:** Non-destructive verification returning exit code 1
upon formatting drift.
*   **Stdin/Stdout Filtering:** Seamless Unix pipeline integration for editor
tooling.
## Installation
Run the following from the root of the repository:
```bash
cargo install --path .
```
## Command Line Interface
```text
mto [OPTIONS] [TARGET_PATH]
```
*   `[TARGET_PATH]`: Can be a single `.rs` file or a directory. Directories are
traversed recursively (skipping `target/`, `.git/`, and `node_modules/`).
*   `-R, --recursive`: Recursively format all `.rs` files starting from the
current directory.
*   `-c, --config <FILE>`: Use a specific configuration file.
*   `-d, --dry-run`: Evaluate the target path and output the intended actions
without writing to disk.
*   `--check`: Verify formatting without writing to disk; exits with status 1
if drift is detected.
*   `-r, --restore <BACKUP> <TARGET>`: Atomically replace the target file with
the specified backup payload.
*   `--man`: Print this manual.
*   `-h, --help`: Print the short help menu.
## Configuration (.mto.toml)
You control the sorting logic via a `.mto.toml` file. `mtindo` resolves
configuration paths in the following priority order:
1. Explicit path passed via `--config`.
2. `.mto.toml` in the current working directory.
3. `~/.config/mtindo/.mto.toml` (Global XDG-style).
4. `~/.mto.toml` (Global fallback).
5. Internal defaults.
```toml
# Enable or disable alphabetical sorting within matching categories and
    visibilities.
sort_alphabetically = true
# Keep `impl` blocks directly below the struct/enum they implement.
group_impls = true
# Define the absolute top-down ordering of your source files.
# Valid tokens: ModDecl, UseDecl, MacroDecl, ConstDecl, StaticDecl, TraitDecl,
    EnumDecl, StructDecl, ImplBlock, FnDecl, Unknown
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
## Safety and Backups
Data loss is impossible by design.
1. Before any file is modified, an exact copy is saved to `/tmp/mtindo_backups/`
(or your OS equivalent) using a millisecond-precision UNIX timestamp.
2. The reordered source is written to a temporary sidecar file.
3. An atomic file system rename overwrites the original file.
4. `cargo +nightly fmt` is executed to align interior spacing.
5. If `rustfmt` fails (indicating a generated syntax error), `mtindo`
automatically restores the backup snapshot.
If you need to manually revert a file, run:
```bash
mto --restore /tmp/mtindo_backups/170000000_src_main.bak src/main.rs
```
"#;

enum CliAction {
    Format {
        target_path: PathBuf,
        config_path: Option<PathBuf>,
        dry_run: bool,
        dbg: bool,
        check: bool,
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
        Some(a) => a,
        None => std::process::exit(1),
    };
    match action {
        CliAction::Man => {
            print_man_page();
        }
        CliAction::Stdin { config_path, check } => {
            process_stdin(config_path, check);
        }
        CliAction::Init { target_path } => {
            process_init(target_path);
        }
        CliAction::Restore {
            backup_path,
            target_path,
        } => {
            if !backup_path.exists() {
                eprintln!(
                    "{}Error: Backup file {} does not exist.{}",
                    c_red(),
                    backup_path.display(),
                    c_reset(true)
                );
                std::process::exit(1);
            }
            match restore_backup(&backup_path, &target_path) {
                Ok(_) => println!(
                    "{}Successfully restored {} from backup.{}",
                    c_green(),
                    target_path.display(),
                    c_reset(false)
                ),
                Err(e) => {
                    eprintln!("{}Failed to restore file: {}{}", c_red(), e, c_reset(true));
                    std::process::exit(1);
                }
            }
        }
        CliAction::Format {
            target_path,
            config_path,
            dry_run,
            dbg,
            check,
        } => {
            let resolved_config =
                resolve_config_path(config_path, Some(&target_path));
            let config = match resolved_config {
                Some(path) => {
                    println!(
                        "{}Using configuration:{} {}",
                        c_cyan(),
                        c_reset(false),
                        path.display()
                    );
                    parse_toml_config(&path)
                }
                None => {
                    println!(
                        "{}Using configuration:{} Internal Defaults",
                        c_cyan(),
                        c_reset(false)
                    );
                    LayoutConfig::default()
                }
            };
            if !target_path.exists() {
                eprintln!(
                    "{}Error: Target path '{}' does not exist.{}",
                    c_red(),
                    target_path.display(),
                    c_reset(true)
                );
                std::process::exit(1);
            }
            if target_path.is_file() {
                process_file(&target_path, &config, dry_run, dbg, check);
            } else if target_path.is_dir() {
                walk_directory(&target_path, &config, dry_run, dbg, check);
            }
            if check {
                println!(
                    "{}Check passed successfully: no formatting drift detected.{}",
                    c_green(),
                    c_reset(false)
                );
            }
        }
    }
}

fn parse_args() -> Option<CliAction> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        if !std::io::stdin().is_terminal() {
            return Some(
    CliAction::Stdin {
                config_path: None,
                check: false,
            },
);
        }
        print_help();
        return None;
    }
    if args[1] == "init" {
        let target_path = args.get(2).map(PathBuf::from);
        return Some(CliAction::Init { target_path });
    }
    let mut i = 1;
    let mut target_path = None;
    let mut config_path = None;
    let mut dry_run = false;
    let mut recursive = false;
    let mut dbg = false;
    let mut check = false;
    while i < args.len() {
        match args[i].as_str() {
            "--man" => return Some(CliAction::Man),
            "-h" | "--help" => {
                print_help();
                return None;
            }
            "-d" | "--dry-run" => {
                dry_run = true;
            }
            "--dbg" => {
                dbg = true;
            }
            "--check" => {
                check = true;
            }
            "-R" | "--recursive" => {
                recursive = true;
            }
            "-c" | "--config" => {
                if i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                } else {
                    eprintln!("{}Error: Missing path for --config{}", c_red(), c_reset(true));
                    return None;
                }
            }
            "-r" | "--restore" => {
                if i + 2 < args.len() {
                    return Some(
    CliAction::Restore {
                        backup_path: PathBuf::from(&args[i + 1]),
                        target_path: PathBuf::from(&args[i + 2]),
                    },
);
                } else {
                    eprintln!(
                        "{}Error: --restore requires <BACKUP_PATH> <TARGET_PATH>{}",
                        c_red(),
                        c_reset(true)
                    );
                    return None;
                }
            }
            path if target_path.is_none() => {
                target_path = Some(PathBuf::from(path));
            }
            _ => {
                eprintln!("{}Error: Unknown argument '{}'{}", c_red(), args[i], c_reset(true));
                return None;
            }
        }
        i += 1;
    }
    if recursive && target_path.is_none() {
        target_path = Some(PathBuf::from("."));
    }
    match target_path {
        Some(path) => Some(
    CliAction::Format {
            target_path: path,
            config_path,
            dry_run,
            dbg,
            check,
        },
),
        None => {
            if !std::io::stdin().is_terminal() {
                Some(CliAction::Stdin { config_path, check })
            } else {
                eprintln!(
                    "{}Error: No target path provided. Use -R to format current directory recursively, or pipe via stdin.{}",
                    c_red(),
                    c_reset(true)
                );
                None
            }
        }
    }
}

fn print_help() {
    println!("{}mtindo (mto) - Rust Source Reorganizer{}", c_cyan(), c_reset(false));
    println!("Safely and atomically restructures Rust code blocks.\n");
    println!("{}USAGE:{}", c_yellow(false), c_reset(false));
    println!("    mto [OPTIONS] [TARGET_PATH]");
    println!("    mto --restore <BACKUP_FILE> <TARGET_PATH>");
    println!("    cat file.rs | mto\n");
    println!("{}OPTIONS:{}", c_yellow(false), c_reset(false));
    println!("    -c, --config <FILE>     Specify custom .toml config path");
    println!("    -d, --dry-run           Process files without saving changes");
    println!("    -R, --recursive         Format all .rs files from current directory");
    println!("    --check                 Verify formatting without writing changes (exit 1 on drift)");
    println!("    -r, --restore           Restore a file from a backup safely");
    println!("        --man               Print the full manual page");
    println!("    -h, --help              Print this help information\n");
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
        if line.starts_with("```") {
            if in_code_block {
                println!("\x1b[90m```\x1b[0m");
                in_code_block = false;
            } else {
                in_code_block = true;
                lang = line[3..].trim().to_string();
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
