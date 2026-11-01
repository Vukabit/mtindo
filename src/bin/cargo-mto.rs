//! `cargo mto`: lays out the targets of the current package or workspace, as `cargo fmt`
//! formats them. The packages and targets come from the manifests (`config::workspace`);
//! the sibling `mto` binary then lays out each target's root file and the modules it
//! declares. Options after `--` go to `mto`, as `cargo fmt` passes them to the
//! formatter.
#![deny(unsafe_code)]

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use mtindo::cli::spec::{
  CARGO_FLAGS, CargoFlag, CargoFlagSpec, Forward, cargo_flag, cargo_spec_of, listed, spec_of,
};
use mtindo::config::env::{COMMAND, PATHS_FILE};
use mtindo::config::workspace::{self, Selection};
use mtindo::{HelpPalette, color_enabled};

/// The column where help descriptions start.
const HELP_COLUMN: usize = 30;

/// The longest list of targets given to mto as arguments, in characters.
const LONGEST_LINE: usize = 30_000;

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

/// What `spec`, given `value`, becomes on mto's command line.
fn forwarded(spec: &CargoFlagSpec, value: Option<&str>) -> Vec<String> {
  match spec.forwards {
    Forward::Own => Vec::new(),
    Forward::Flag(flag) => vec![spec_of(flag).long.to_string()],
    Forward::ByValue(pairs) => pairs
      .iter()
      .find(|(given, _)| Some(*given) == value)
      .map(|(_, words)| words.iter().map(|w| w.to_string()).collect())
      .unwrap_or_default(),
  }
}

fn help(color: bool) -> String {
  let p = HelpPalette::new(color, "cargo mto");
  let mut out = format!(
    "{} {}\n\n",
    p.heading("usage:"),
    p.usage_line("cargo mto [OPTIONS] [-- <MTO OPTIONS>...]")
  );
  let about = "Formats the targets of the current package or workspace, as cargo fmt \
                 formats them. Options after -- go to mto (see mto --help).";
  for line in mtindo::text::color::wrap(about, mtindo::text::color::help_width()) {
    out.push_str(&line);
    out.push('\n');
  }
  let mut groups: Vec<&str> = Vec::new();
  for spec in CARGO_FLAGS {
    if !groups.contains(&spec.group) {
      groups.push(spec.group);
    }
  }
  for group in groups {
    // Every group's flags follow `cargo mto` itself, before any `--`.
    out.push_str(&format!(
      "\n{} ({})\n",
      p.heading(&format!("{}:", group)),
      p.usage_line("cargo mto")
    ));
    let mut rows: Vec<_> = CARGO_FLAGS.iter().filter(|s| s.group == group).collect();
    rows.sort_by_key(|spec| spec.long);
    for spec in rows {
      out.push_str(&row(&p, &spec.names(), &spec.value_text(), spec.help));
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
    COMMAND,
    ["cargo mto".to_string()]
      .into_iter()
      .chain(typed.iter().cloned())
      .collect::<Vec<_>>()
      .join(" "),
  );
  command.args(&args.mto);
  let check = forwarded(cargo_spec_of(CargoFlag::Check), None);
  if args.check && !args.mto.iter().any(|a| check.contains(a)) {
    command.args(&check);
  }
  if args.quiet {
    command.args(forwarded(cargo_spec_of(CargoFlag::Quiet), None));
  }
  if args.verbose {
    command.args(forwarded(cargo_spec_of(CargoFlag::Verbose), None));
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
    if arg == "--" {
      args.mto.extend(raw[i + 1..].iter().cloned());
      break;
    }
    let (arg, inline) = match arg.split_once('=') {
      Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
      _ => (arg, None),
    };
    let Some(spec) = cargo_flag(arg) else {
      return Err(format!("unexpected argument '{}'", arg));
    };
    let value = |i: &mut usize| -> Result<String, String> {
      if let Some(v) = &inline {
        return Ok(v.clone());
      }
      *i += 1;
      raw
        .get(*i)
        .cloned()
        .ok_or_else(|| format!("{} needs a value", spec.long))
    };
    match spec.flag {
      CargoFlag::Help => {
        print!("{}", help(color_enabled(false)));
        return Ok(Parsed::Exit(0));
      }
      CargoFlag::Version => {
        println!("mto {}", mtindo::version::short());
        return Ok(Parsed::Exit(0));
      }
      CargoFlag::Check => args.check = true,
      CargoFlag::NoHistory | CargoFlag::AllowDirty => args.mto.extend(forwarded(spec, None)),
      CargoFlag::All => args.selection_all = true,
      CargoFlag::Quiet => args.quiet = true,
      CargoFlag::Verbose => args.verbose = true,
      CargoFlag::Package => {
        args.packages.push(value(&mut i)?);
        // `-p a b` names several, as cargo fmt allows.
        while inline.is_none() && raw.get(i + 1).is_some_and(|next| !next.starts_with('-')) {
          i += 1;
          args.packages.push(raw[i].clone());
        }
      }
      CargoFlag::ManifestPath => {
        let path = value(&mut i)?;
        if !path.ends_with("Cargo.toml") {
          return Err("the manifest-path must be a path to a Cargo.toml file".into());
        }
        args.manifest = Some(PathBuf::from(path));
      }
      CargoFlag::MessageFormat => {
        let given = value(&mut i)?;
        if !spec.values().contains(&given.as_str()) {
          return Err(format!(
            "{} takes {}, not `{}`",
            spec.long,
            listed(&spec.values()),
            given
          ));
        }
        args.mto.extend(forwarded(spec, Some(&given)));
      }
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
    for spec in CARGO_FLAGS {
      assert!(text.contains(&spec.names()), "{} missing", spec.long);
    }
  }

  #[test]
  fn test_the_table_covers_the_parser() {
    for spec in CARGO_FLAGS {
      // Help and the version print and exit; every other flag runs.
      let given: Vec<Vec<&str>> = match spec.flag {
        CargoFlag::Help | CargoFlag::Version => {
          for name in [Some(spec.long), spec.short].into_iter().flatten() {
            assert!(matches!(parse(&[name.into()]), Ok(Parsed::Exit(0))));
          }
          continue;
        }
        CargoFlag::Package => vec![vec![spec.long, "a"]],
        CargoFlag::ManifestPath => vec![vec![spec.long, "Cargo.toml"]],
        _ if spec.values().is_empty() => vec![vec![spec.long]],
        _ => spec.values().iter().map(|v| vec![spec.long, *v]).collect(),
      };
      for words in given {
        let args = run(&words);
        let value = words.get(1).copied();
        // What is forwarded at once, or at the end of the parse for check, quiet and
        // verbose, is what the table says.
        let mut sent = args.mto.clone();
        for (on, flag) in [
          (args.check, CargoFlag::Check),
          (args.quiet, CargoFlag::Quiet),
          (args.verbose, CargoFlag::Verbose),
        ] {
          if on {
            sent.extend(forwarded(cargo_spec_of(flag), None));
          }
        }
        assert_eq!(sent, forwarded(spec, value), "{:?}", words);
        if let Some(short) = spec.short {
          let mut words = words.clone();
          words[0] = short;
          assert_eq!(run(&words), args, "{:?}", words);
        }
      }
      if !spec.values().is_empty() {
        assert!(parse(&[spec.long.into(), "bogus".into()]).is_err());
      }
    }
    // Every word forwarded is an mto flag or one of its values.
    for spec in CARGO_FLAGS {
      if let Forward::ByValue(pairs) = spec.forwards {
        for word in pairs.iter().flat_map(|(_, words)| words.iter()) {
          assert!(
            mtindo::cli::spec::FLAGS
              .iter()
              .any(|f| f.long == *word || f.short == Some(*word) || f.values.contains(word)),
            "{} forwards {}, which mto does not take",
            spec.long,
            word
          );
        }
      }
    }
    for unknown in ["--bogus", "-x", "--emit", "package"] {
      assert!(parse(&[unknown.into()]).is_err(), "{}", unknown);
    }
  }
}
