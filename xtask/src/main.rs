//! Development tasks for mtindo, run with `cargo xtask <command>`.
//!
//! This crate is never published and is not part of the `mto` binary.

mod gates;
mod lexdiff;
mod parsecheck;
mod sha256;
mod unicode;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mtindo::HelpPalette;

const COMMANDS: &[Command] = &[
    (
        "gates",
        "[all|GATE]",
        "Check repository rules (default: all; gates listed below)",
        gates::run,
    ),
    (
        "lexdiff",
        "[--edition YEAR] [PATH...]",
        "Compare mto's lexer with rustc's (default: this repo, edition 2021)",
        lexdiff::run,
    ),
    (
        "parsecheck",
        "[PATH...]",
        "Parse with each crate's edition; report syntax errors",
        parsecheck::run,
    ),
    (
        "unicode",
        "[--version X.Y.Z] [--check]",
        "Regenerate src/text/unicode tables; --check only compares",
        unicode::run,
    ),
];

const EXIT_CODES: &[(&str, &str)] = &[
    ("0", "success"),
    ("1", "a check failed"),
    ("2", "bad arguments"),
];

/// The column where help descriptions start.
const HELP_COLUMN: usize = 34;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let root = project_root();
    match args.first().map(String::as_str) {
        None | Some("help") | Some("-h") | Some("--help") => {
            print!("{}", render_help(mtindo::color_enabled(false)));
            ExitCode::SUCCESS
        }
        Some(name) => match COMMANDS.iter().find(|command| command.0 == name) {
            Some(command) => (command.3)(&root, &args[1..]),
            None => {
                eprintln!("Unknown xtask command `{}`.", name);
                eprintln!("Run 'cargo xtask help' for usage.");
                ExitCode::from(2)
            }
        },
    }
}

fn project_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest_dir)
}

/// The help screen. The text is fixed; only the colors depend on `color`.
fn render_help(color: bool) -> String {
    let p = HelpPalette::new(color, "cargo xtask");
    let mut out = String::new();
    out.push_str(&format!(
        "{} {}\n",
        p.heading("usage:"),
        p.usage_line("cargo xtask <COMMAND> [ARGS]")
    ));
    out.push_str("\nDevelopment tasks for mtindo. Never published; not part of the mto binary.\n");
    out.push_str(&format!("\n{}\n", p.heading("commands:")));
    for (name, args, help, _) in COMMANDS {
        let label = format!("  {} {}", p.long(name), p.value(args));
        let width = 2 + name.len() + 1 + args.len();
        out.push_str(&p.row(&label, width, HELP_COLUMN, help));
    }
    let help_label = format!("  {}", p.long("help"));
    out.push_str(&p.row(
        &help_label,
        6,
        HELP_COLUMN,
        "Show this help (also -h, --help, or no command)",
    ));
    out.push_str(&format!("\n{}\n", p.heading("gates:")));
    for (name, help, _) in gates::GATES {
        let label = format!("  {}", p.value(name));
        out.push_str(&p.row(&label, 2 + name.len(), HELP_COLUMN, help));
    }
    out.push_str(&format!("\n{}\n", p.heading("exit codes:")));
    for (code, meaning) in EXIT_CODES {
        out.push_str(&format!("  {}  {}\n", p.value(code), meaning));
    }
    out
}

/// One command: (name, arguments, what it does, entry point). Dispatch and help both read
/// this table, so a command cannot exist without being documented.
type Command = (
    &'static str,
    &'static str,
    &'static str,
    fn(&Path, &[String]) -> ExitCode,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help_lists_every_command_and_gate() {
        let help = render_help(false);
        for (name, args, help_text, _) in COMMANDS {
            assert!(
                help.contains(&format!("  {} {}", name, args)),
                "{} missing from help",
                name
            );
            assert!(help.contains(help_text));
        }
        for (name, _, _) in gates::GATES {
            assert!(
                help.contains(&format!("\n  {}", name)),
                "gate {} missing from help",
                name
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
}
