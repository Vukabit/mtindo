//! `upstream`: the standard formatter's bugs mto's work turned up, kept as cases under
//! `.dev/upstream/rustfmt/` for reporting upstream. A case is a folder:
//!
//! ```text
//! input.rs      the input
//! edition       the edition it runs with
//! rustfmt.toml  its settings, when it has any
//! twice         present when the bug is that a second run changes the output
//! notes.md      what goes wrong, how it was found, its status upstream (`- Upstream: ...`)
//! outputs/      each toolchain's output from the last recheck, as TOOLCHAIN.rs
//! drafts        (beside the cases, in ../drafts/) the issue text `draft` writes
//! ```
//!
//! - `upstream list`: every case, its title and upstream status, and the last recheck.
//! - `upstream recheck [TOOLCHAIN...]`: runs each case through each toolchain's rustfmt
//!   (the pinned one and nightly by default) and says whether the bug still shows: the
//!   output fails mto's meaning check, or with `twice`, a second run changes it.
//! - `upstream draft CASE`: writes the issue text for a case, for filing by hand.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::syntax::Edition;
use mtindo::verify::format::verify_format;

use crate::compare::{Answer, mto_output, reference_toolchain, run_rustfmt};

/// What a toolchain's rustfmt made of a case.
#[derive(Debug, PartialEq)]
enum Verdict {
  /// The bug shows: why.
  Shows(String),
  /// The output keeps the meaning (and settles, for `twice`).
  Gone,
  /// rustfmt gave no output: why.
  NoAnswer(String),
}

/// One case.
struct Case {
  name: String,
  dir: PathBuf,
  input: String,
  edition: Edition,
  config: String,
  twice: bool,
  notes: String,
}

pub fn run(args: &[String]) -> ExitCode {
  let (Some(flag), Some(dir)) = (args.first(), args.get(1)) else {
    return usage();
  };
  if flag != "--dir" {
    return usage();
  }
  let dir = PathBuf::from(dir);
  let rest = &args[2..];
  match rest.first().map(String::as_str) {
    None | Some("list") => list(&dir),
    Some("recheck") => {
      let mut toolchains: Vec<String> = rest[1..].to_vec();
      if toolchains.is_empty() {
        toolchains = vec![reference_toolchain(), "nightly".to_string()];
      }
      recheck(&dir, &toolchains)
    }
    Some("draft") => match rest.get(1) {
      Some(name) => draft(&dir, name),
      None => usage(),
    },
    _ => usage(),
  }
}

fn capitalized(text: &str) -> String {
  let mut chars = text.chars();
  match chars.next() {
    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    None => String::new(),
  }
}

fn cases(dir: &Path) -> Vec<Case> {
  let mut names: Vec<String> = fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .filter(|e| e.path().join("input.rs").is_file())
    .map(|e| e.file_name().to_string_lossy().into_owned())
    .collect();
  names.sort();
  names
    .into_iter()
    .map(|name| {
      let case = dir.join(&name);
      let read = |file: &str| fs::read_to_string(case.join(file)).unwrap_or_default();
      Case {
        edition: Edition::from_year(read("edition").trim()).unwrap_or(Edition::E2021),
        input: read("input.rs"),
        config: read("rustfmt.toml"),
        twice: case.join("twice").exists(),
        notes: read("notes.md"),
        dir: case,
        name,
      }
    })
    .collect()
}

/// Writes the issue text for the case `name` to `drafts/NAME.md` beside the cases.
fn draft(dir: &Path, name: &str) -> ExitCode {
  let Some(case) = cases(dir).into_iter().find(|c| {
    c.name == name || c.name.ends_with(name) || c.name.starts_with(&format!("{}-", name))
  }) else {
    eprintln!("no case `{}` in {}", name, dir.display());
    return ExitCode::from(1);
  };
  let home = std::env::temp_dir().join(format!("mtindo-upstream-home-{}", std::process::id()));
  let _ = fs::create_dir_all(&home);
  let stable = reference_toolchain();
  let mut results = Vec::new();
  for toolchain in [stable.clone(), "nightly".to_string()] {
    let (verdict, output) = verdict(&case, &toolchain, &home);
    results.push((toolchain.clone(), version(&toolchain), verdict, output));
  }
  let _ = fs::remove_dir_all(&home);
  let text = draft_text(&case, &results, &kept(&case));
  let drafts = dir.parent().unwrap_or(dir).join("drafts");
  let _ = fs::create_dir_all(&drafts);
  let path = drafts.join(format!("{}.md", case.name));
  match fs::write(&path, &text) {
    Ok(()) => {
      print!("{}", text);
      println!(
        "\n(written to {}; filing it is yours to do)",
        path.display()
      );
      ExitCode::SUCCESS
    }
    Err(e) => {
      eprintln!("cannot write {}: {}", path.display(), e);
      ExitCode::from(1)
    }
  }
}

/// The issue text: the title, what goes wrong, the input and settings, each version's
/// output, and an output that keeps the meaning.
fn draft_text(
  case: &Case,
  results: &[(String, String, Verdict, Option<String>)],
  kept: &str,
) -> String {
  let mut out = format!("# {}\n\n", title(&case.notes));
  let status = upstream_status(&case.notes);
  if status.starts_with('#') {
    out.push_str(&format!(
      "> Already reported upstream as rust-lang/rustfmt{} ; add to it rather than filing \
       anew.\n\n",
      status
    ));
  }
  // The notes, but for the lines that are about mto or the upstream status.
  let body: Vec<&str> = case
    .notes
    .lines()
    .skip_while(|l| l.starts_with("# ") || l.trim().is_empty())
    .filter(|l| !l.starts_with("- Upstream:") && !l.starts_with("- mto:"))
    .collect();
  if !body.is_empty() {
    out.push_str(body.join("\n").trim());
    out.push_str("\n\n");
  }
  out.push_str("## Input\n\n```rust\n");
  out.push_str(&case.input);
  if !case.input.ends_with('\n') {
    out.push('\n');
  }
  out.push_str("```\n\n");
  let year = crate::compare::edition_year(case.edition);
  if case.config.trim().is_empty() {
    out.push_str(&format!("Run with `rustfmt --edition {}`.\n\n", year));
  } else {
    out.push_str(&format!(
      "With this `rustfmt.toml`, run with `rustfmt --edition {}`:\n\n```toml\n{}```\n\n",
      year,
      if case.config.ends_with('\n') {
        case.config.clone()
      } else {
        format!("{}\n", case.config)
      }
    ));
  }
  for (toolchain, version, verdict, output) in results {
    out.push_str(&format!("## Output of {} ({})\n\n", version, toolchain));
    match (verdict, output) {
      (Verdict::NoAnswer(why), _) => out.push_str(&format!("No output: {}\n\n", why)),
      (_, Some(text)) => {
        out.push_str("```rust\n");
        out.push_str(text);
        if !text.ends_with('\n') {
          out.push('\n');
        }
        out.push_str("```\n\n");
        match verdict {
          Verdict::Shows(why) => out.push_str(&format!("{}.\n\n", capitalized(why))),
          _ => out.push_str("This version keeps the meaning.\n\n"),
        }
      }
      _ => {}
    }
  }
  out.push_str("## Expected\n\nThe input's meaning kept, for example:\n\n```rust\n");
  out.push_str(kept);
  if !kept.ends_with('\n') {
    out.push('\n');
  }
  out.push_str("```\n");
  out
}

/// mto's output for the case, which keeps the meaning (the meaning check holds mto's
/// writes to it).
fn kept(case: &Case) -> String {
  let overrides = Overrides {
    config_path: (!case.config.trim().is_empty()).then(|| case.dir.join("rustfmt.toml")),
    ..Overrides::default()
  };
  let empty = std::env::temp_dir().join("mtindo-upstream-empty");
  let _ = fs::create_dir_all(&empty);
  let format = Resolver::new(&overrides, Environment::default())
    .ok()
    .and_then(|mut r| r.config_for(&empty).ok())
    .map(|c| c.format())
    .unwrap_or_default();
  mto_output(&case.input, case.edition, &format)
}

fn list(dir: &Path) -> ExitCode {
  let cases = cases(dir);
  println!("{:<38} {:<26} WHAT GOES WRONG", "CASE", "UPSTREAM");
  for case in &cases {
    println!(
      "{:<38} {:<26} {}",
      case.name,
      upstream_status(&case.notes),
      title(&case.notes)
    );
  }
  println!(
    "\n{} case(s) in {}; recheck them: cargo xtask upstream recheck",
    cases.len(),
    dir.display()
  );
  ExitCode::SUCCESS
}

fn recheck(dir: &Path, toolchains: &[String]) -> ExitCode {
  let home = std::env::temp_dir().join(format!("mtindo-upstream-home-{}", std::process::id()));
  let _ = fs::create_dir_all(&home);
  let mut gone = Vec::new();
  for case in cases(dir) {
    println!("{}", case.name);
    let mut shown = false;
    for toolchain in toolchains {
      let (verdict, output) = verdict(&case, toolchain, &home);
      if let Some(text) = &output {
        let _ = fs::create_dir_all(case.dir.join("outputs"));
        let _ = fs::write(
          case.dir.join("outputs").join(format!("{}.rs", toolchain)),
          text,
        );
      }
      let said = match &verdict {
        Verdict::Shows(why) => {
          shown = true;
          format!("shows: {}", why)
        }
        Verdict::Gone => "keeps the meaning".to_string(),
        Verdict::NoAnswer(why) => format!("no answer: {}", why),
      };
      println!("  {:<24} {}", toolchain, said);
    }
    if !shown {
      gone.push(case.name.clone());
    }
  }
  let _ = fs::remove_dir_all(&home);
  if !gone.is_empty() {
    println!(
      "\nno longer shown by a meaning check or second run (fixed, or a bug the checks do \
       not see; read its notes): {}",
      gone.join(", ")
    );
  }
  ExitCode::SUCCESS
}

/// The first `# ` line of the notes.
fn title(notes: &str) -> String {
  notes
    .lines()
    .find_map(|l| l.strip_prefix("# "))
    .unwrap_or("(no notes.md)")
    .trim()
    .to_string()
}

/// What `- Upstream: ...` says, up to its first comma.
fn upstream_status(notes: &str) -> String {
  notes
    .lines()
    .find_map(|l| l.strip_prefix("- Upstream:"))
    .map(|s| {
      s.split(['(', ',', ';'])
        .next()
        .unwrap_or(s)
        .trim()
        .to_string()
    })
    .unwrap_or_else(|| "-".to_string())
}

fn usage() -> ExitCode {
  eprintln!("upstream --dir DIR list|recheck [TOOLCHAIN...]|draft CASE");
  ExitCode::from(2)
}

/// Runs `case` through `toolchain`'s rustfmt: the verdict, and the output when it gave one.
fn verdict(case: &Case, toolchain: &str, home: &Path) -> (Verdict, Option<String>) {
  let dir = std::env::temp_dir().join(format!(
    "mtindo-upstream-{}-{}",
    std::process::id(),
    case.name
  ));
  let _ = fs::create_dir_all(&dir);
  let _ = fs::remove_file(dir.join("rustfmt.toml"));
  if !case.config.trim().is_empty() {
    let _ = fs::write(dir.join("rustfmt.toml"), &case.config);
  }
  let year = crate::compare::edition_year(case.edition);
  let run = |text: &str| run_rustfmt(text, year, &dir, home, toolchain).0;
  let out = match run(&case.input) {
    Answer::Text(text) => text,
    Answer::Refused(why) | Answer::NotRun(why) => {
      let _ = fs::remove_dir_all(&dir);
      return (
        Verdict::NoAnswer(why.lines().nth(1).unwrap_or(why.trim()).to_string()),
        None,
      );
    }
  };
  let broken = verify_format(&case.input, &out, case.edition);
  let verdict = if let Some(first) = broken.first() {
    Verdict::Shows(format!("the output changes the meaning: {}", first.detail))
  } else if case.twice {
    match run(&out) {
      Answer::Text(again) if again != out => {
        Verdict::Shows("a second run changes the output".to_string())
      }
      _ => Verdict::Gone,
    }
  } else {
    Verdict::Gone
  };
  let _ = fs::remove_dir_all(&dir);
  (verdict, Some(out))
}

fn version(toolchain: &str) -> String {
  crate::probe::version(toolchain).unwrap_or_else(|| format!("rustfmt of {}", toolchain))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn case(notes: &str) -> Case {
    Case {
      name: "01-x".to_string(),
      dir: PathBuf::from("/nowhere"),
      input: "extern \"C\" {\n    static X: i32 = 1;\n}\n".to_string(),
      edition: Edition::E2021,
      config: String::new(),
      twice: false,
      notes: notes.to_string(),
    }
  }

  #[test]
  fn test_a_draft_holds_the_input_outputs_and_an_answer_without_mto() {
    let c = case(
      "# A static loses its initializer.\n\n- Upstream: #6267 (open)\n- Found by: corpus\n- mto: keeps it\n",
    );
    let results = vec![(
      "1.99.1".to_string(),
      "rustfmt 1.10.0-stable".to_string(),
      Verdict::Shows("the output changes the meaning: a token is gone".to_string()),
      Some("extern \"C\" {\n    static X: i32;\n}\n".to_string()),
    )];
    let text = draft_text(&c, &results, &c.input);
    assert!(
      text.starts_with("# A static loses its initializer.\n"),
      "{}",
      text
    );
    assert!(text.contains("rust-lang/rustfmt#6267"), "{}", text);
    assert!(text.contains("- Found by: corpus"), "{}", text);
    assert!(
      !text.contains("- mto:") && !text.contains("- Upstream:"),
      "{}",
      text
    );
    assert!(
      text.contains("## Output of rustfmt 1.10.0-stable (1.99.1)"),
      "{}",
      text
    );
    assert!(
      text.contains("The output changes the meaning: a token is gone."),
      "{}",
      text
    );
    assert!(
      text.contains("Run with `rustfmt --edition 2021`."),
      "{}",
      text
    );
  }

  #[test]
  fn test_the_notes_give_the_title_and_status() {
    let notes = "# A static loses its initializer.\n\n- Upstream: #6267 (open, filed 2024-08)\n- mto: keeps it\n";
    assert_eq!(title(notes), "A static loses its initializer.");
    assert_eq!(upstream_status(notes), "#6267");
    assert_eq!(upstream_status("# t\n- Upstream: not found\n"), "not found");
    assert_eq!(upstream_status("# t\n"), "-");
  }
}
