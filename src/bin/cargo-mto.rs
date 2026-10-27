//! `cargo mto`: lays out the targets of the current package or workspace, as `cargo fmt`
//! formats them. The packages and targets come from the manifests (`config::workspace`);
//! the sibling `mto` binary then lays out each target's root file and the modules it
//! declares. Options after `--` go to `mto`, as `cargo fmt` passes them to the
//! formatter.
#![deny(unsafe_code)]

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use mtindo::cli::{CARGO_MTO_OPTIONS as OPTIONS, long_flag};
use mtindo::config::workspace::{self, Selection};
use mtindo::{HelpPalette, color_enabled};

/// The column where help descriptions start.
const HELP_COLUMN: usize = 30;

/// The longest list of targets given to mto as arguments, in characters.
const LONGEST_LINE: usize = 30_000;

/// Names the file a longer list is given to mto in, one path a line.
const PATHS_FILE: &str = "MTINDO_PATHS_FILE";

/// Small blocks come from a per-thread cache in front of the system allocator.
#[global_allocator]
static GLOBAL: mtindo::cache_alloc::Cached = mtindo::cache_alloc::Cached;

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
    "{} {}\n\n",
    p.heading("usage:"),
    p.usage_line("cargo mto [OPTIONS] [-- <MTO OPTIONS>...]")
  );
  let about = "Lays out the targets of the current package or workspace, as cargo fmt \
                 formats them. Options after -- go to mto (see mto --help).";
  for line in mtindo::text::color::wrap(about, mtindo::text::color::help_width()) {
    out.push_str(&line);
    out.push('\n');
  }
  let mut groups: Vec<&str> = Vec::new();
  for (group, _, _, _) in OPTIONS {
    if !groups.contains(group) {
      groups.push(group);
    }
  }
  for group in groups {
    // Every group's flags follow `cargo mto` itself, before any `--`.
    out.push_str(&format!(
      "\n{} ({})\n",
      p.heading(&format!("{}:", group)),
      p.usage_line("cargo mto")
    ));
    let mut rows: Vec<_> = OPTIONS.iter().filter(|o| o.0 == group).collect();
    rows.sort_by_key(|(_, flags, _, _)| long_flag(flags));
    for (_, flags, value, text) in rows {
      out.push_str(&row(&p, flags, value, text));
    }
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
  // A Windows command line holds at most 32,767 characters: a longer list of targets goes
  // to mto in a file instead.
  let listed: usize = targets.iter().map(|t| t.path.as_os_str().len() + 3).sum();
  let lines: Option<Vec<&str>> = targets.iter().map(|t| t.path.to_str()).collect();
  let list = match lines {
    Some(lines) if listed > LONGEST_LINE => {
      let file = env::temp_dir().join(format!("mtindo-paths-{}.txt", std::process::id()));
      if let Err(e) = std::fs::write(&file, lines.join("\n")) {
        eprintln!("Error: cannot write {}: {}", file.display(), e);
        return ExitCode::from(2);
      }
      command.env(PATHS_FILE, &file);
      Some(file)
    }
    _ => {
      command.args(targets.iter().map(|t| t.path.as_path()));
      None
    }
  };
  let status = command.status();
  if let Some(file) = list {
    let _ = std::fs::remove_file(file);
  }
  match status {
    // A code that does not fit one byte (a Windows crash's) must not read as another
    // code, or as success: it is a failure.
    Ok(status) => ExitCode::from(
      status
        .code()
        .and_then(|c| u8::try_from(c).ok())
        .unwrap_or(3),
    ),
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
      raw
        .get(*i)
        .cloned()
        .ok_or_else(|| format!("{} needs a value", name))
    };
    match arg {
      "-h" | "--help" => {
        print!("{}", help(color_enabled(false)));
        return Ok(Parsed::Exit(0));
      }
      "--version" => {
        println!("mto {}", mtindo::version::short());
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
        while inline.is_none() && raw.get(i + 1).is_some_and(|next| !next.starts_with('-')) {
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

/// A flag's line in the help, the short form first.
fn row(p: &HelpPalette, flags: &str, value: &str, text: &str) -> String {
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
  p.row(&label, width, HELP_COLUMN, text)
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
    for (_, flags, _, _) in OPTIONS {
      assert!(text.contains(flags), "{} missing", flags);
    }
  }
}
