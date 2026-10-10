//! `cargo mto`: lays out the targets of the current package or workspace, as `cargo fmt`
//! formats them. The packages and targets come from the manifests (`config::workspace`);
//! the sibling `mto` binary then lays out each target's root file and the modules it
//! declares. Options after `--` go to `mto`, as `cargo fmt` passes them to rustfmt.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use mtindo::cli::CARGO_MTO_OPTIONS as OPTIONS;
use mtindo::config::workspace::{self, Selection};
use mtindo::{HelpPalette, color_enabled};

/// The column where help descriptions start.
const HELP_COLUMN: usize = 34;

enum Parsed {
    Run(Args),
    Exit(u8),
}

/// What the command line asked for.
#[derive(Debug, Default, PartialEq)]
struct Args {
    selection_all: bool,
    packages: Vec<String>,
    manifest: Option<PathBuf>,
    check: bool,
    quiet: bool,
    verbose: bool,
    /// What goes to `mto`: the translated options, then those after `--`.
    mto: Vec<String>,
}

fn help(color: bool) -> String {
    let p = HelpPalette::new(color, "cargo mto");
    let mut out = format!(
        "{} {}\n\nLays out the targets of the current package or workspace, as cargo fmt \
         formats them.\nOptions after -- go to mto (see mto --help).\n\n{}\n",
        p.heading("usage:"),
        p.usage_line("cargo mto [OPTIONS] [-- <MTO OPTIONS>...]"),
        p.heading("options:")
    );
    for (flags, value, text) in OPTIONS {
        let painted = flags
            .split(", ")
            .map(|f| {
                if f.starts_with("--") {
                    p.long(f)
                } else {
                    p.short(f)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let indent = if flags.starts_with("--") {
            "      "
        } else {
            "  "
        };
        let value_part = if value.is_empty() {
            String::new()
        } else {
            format!(" {}", p.value(value))
        };
        let label = format!("{}{}{}", indent, painted, value_part);
        let width = indent.len() + flags.len() + if value.is_empty() { 0 } else { 1 + value.len() };
        out.push_str(&p.row(&label, width, HELP_COLUMN, text));
    }
    out
}

fn main() -> ExitCode {
    let raw: Vec<String> = env::args().skip(1).collect();
    let args = match parse(&raw) {
        Ok(Parsed::Run(args)) => args,
        Ok(Parsed::Exit(code)) => return ExitCode::from(code),
        Err(message) => {
            eprintln!("Error: {}", message);
            eprintln!("Run 'cargo mto --help' for usage.");
            return ExitCode::from(2);
        }
    };
    let selection = if args.selection_all {
        Selection::All
    } else if !args.packages.is_empty() {
        Selection::Packages(args.packages.clone())
    } else {
        Selection::Root
    };
    let start = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let targets = match workspace::targets(args.manifest.as_deref(), &start, &selection) {
        Ok(targets) => targets,
        Err(message) => {
            eprintln!("Error: {}", message);
            return ExitCode::from(2);
        }
    };
    if args.verbose {
        for target in &targets {
            println!("[{}] {}", target.kind, target.path.display());
        }
    }
    let mut command = Command::new(mto_binary());
    // History records the command as it was typed, not the file list mto is given.
    let typed: Vec<String> = env::args().skip(1).collect();
    let typed = match typed.first().map(String::as_str) {
        Some("mto") => &typed[1..],
        _ => &typed[..],
    };
    command.env(
        "MTINDO_COMMAND",
        ["cargo mto".to_string()]
            .into_iter()
            .chain(typed.iter().cloned())
            .collect::<Vec<_>>()
            .join(" "),
    );
    command.args(&args.mto);
    if args.check && !args.mto.iter().any(|a| a == "--check") {
        command.arg("--check");
    }
    if args.quiet {
        command.arg("--quiet");
    }
    if args.verbose {
        command.arg("--verbose");
    }
    command.args(targets.iter().map(|t| t.path.as_path()));
    match command.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!(
                "Error: cannot run {}: {}; is mto installed?",
                Path::new(command.get_program()).display(),
                e
            );
            ExitCode::from(2)
        }
    }
}

/// The `mto` binary next to this one, or `$MTO`, or `mto` on the PATH.
fn mto_binary() -> PathBuf {
    if let Some(path) = env::var_os("MTO") {
        return PathBuf::from(path);
    }
    let name = format!("mto{}", env::consts::EXE_SUFFIX);
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn parse(raw: &[String]) -> Result<Parsed, String> {
    let mut args = Args::default();
    // cargo runs `cargo-mto mto ARGS...`.
    let raw = match raw.first().map(String::as_str) {
        Some("mto") => &raw[1..],
        _ => raw,
    };
    let mut i = 0;
    while i < raw.len() {
        let arg = raw[i].as_str();
        let (arg, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg, None),
        };
        let value = |i: &mut usize, name: &str| -> Result<String, String> {
            if let Some(v) = &inline {
                return Ok(v.clone());
            }
            *i += 1;
            raw.get(*i)
                .cloned()
                .ok_or_else(|| format!("{} needs a value", name))
        };
        match arg {
            "-h" | "--help" => {
                print!("{}", help(color_enabled(false)));
                return Ok(Parsed::Exit(0));
            }
            "--version" => {
                println!("mto {}", env!("CARGO_PKG_VERSION"));
                return Ok(Parsed::Exit(0));
            }
            "--check" => args.check = true,
            "--no-history" | "--allow-dirty" => args.mto.push(arg.to_string()),
            "--all" => args.selection_all = true,
            "-q" | "--quiet" => args.quiet = true,
            "-v" | "--verbose" => args.verbose = true,
            "-p" | "--package" => {
                args.packages.push(value(&mut i, "--package")?);
                // `-p a b` names several, as cargo fmt allows.
                while inline.is_none() && raw.get(i + 1).is_some_and(|next| !next.starts_with('-'))
                {
                    i += 1;
                    args.packages.push(raw[i].clone());
                }
            }
            "--manifest-path" => {
                let path = value(&mut i, "--manifest-path")?;
                if !path.ends_with("Cargo.toml") {
                    return Err("the manifest-path must be a path to a Cargo.toml file".into());
                }
                args.manifest = Some(PathBuf::from(path));
            }
            "--message-format" => match value(&mut i, "--message-format")?.as_str() {
                "short" => args.mto.push("-l".into()),
                "json" => args.mto.extend(["--emit".into(), "json".into()]),
                "human" => {}
                other => {
                    return Err(format!(
                        "--message-format takes short, json or human, not `{}`",
                        other
                    ));
                }
            },
            "--" => {
                args.mto.extend(raw[i + 1..].iter().cloned());
                break;
            }
            other => return Err(format!("unexpected argument '{}'", other)),
        }
        i += 1;
    }
    if args.quiet && args.verbose {
        return Err("quiet mode and verbose mode are not compatible".into());
    }
    Ok(Parsed::Run(args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Args {
        let raw: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match parse(&raw) {
            Ok(Parsed::Run(args)) => args,
            _ => panic!("{:?}", args),
        }
    }

    #[test]
    fn test_arguments_as_cargo_fmt_takes_them() {
        let args = run(&[
            "mto",
            "--check",
            "-p",
            "a",
            "b",
            "--message-format=short",
            "--",
            "--config",
            "max_width=80",
        ]);
        assert!(args.check);
        assert_eq!(args.packages, ["a", "b"]);
        assert_eq!(args.mto, ["-l", "--config", "max_width=80"]);
        assert_eq!(run(&["--no-history"]).mto, ["--no-history"]);
        assert!(parse(&["-q".into(), "-v".into()]).is_err());
        assert!(parse(&["--manifest-path".into(), "x.toml".into()]).is_err());
    }

    #[test]
    fn test_help_lists_every_option() {
        let text = help(false);
        for (flags, _, _) in OPTIONS {
            assert!(text.contains(flags), "{} missing", flags);
        }
    }
}
