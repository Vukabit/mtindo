//! `diff-fuzz`: differential fuzzing against the standard formatter. For a while, inputs
//! are made and formatted by both mto (`format = true`) and the standard formatter, and
//! every way they disagree is found, shrunk and reported once.
//!
//! An input is a corpus file with its whitespace scrambled by a fresh seed, or source
//! built from random bytes by the generator (`mtindo::fuzzing::generate`). A disagreement
//! is named by the first line where the outputs part: the syntax kind that line starts in
//! the standard formatter's output, and how the lines differ (`indentation`, `spacing`
//! within the line, or `line breaks`), as `MatchExpr / line breaks`. A name not seen before
//! in the run is shrunk, lines at a time, while the input still parses and the outputs
//! still disagree the same way, and then printed as
//!
//! ```text
//! mismatch<TAB>NAME<TAB>FILE<TAB>EDITION
//! ```
//!
//! with the shrunk input in FILE, for `cargo xtask fuzz diff` to file. The figures end the
//! output as `metric NAME VALUE UNIT` lines.
//!
//! An input whose meaning the standard formatter's output changes (by the meaning check
//! mto applies to its own writes) has no answer to compare against: it is counted, not
//! reported.
//!
//! `diff-fuzz --replay FILE [--edition YEAR]` shows how the two outputs part on one input;
//! `diff-fuzz --recheck YEAR:FILE...` says of each input whether they still disagree, as
//! `agree<TAB>FILE`, `differ<TAB>FILE<TAB>NAME` or, when the standard formatter's output
//! changes its meaning, `broken<TAB>FILE<TAB>WHAT`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::format::file::format_file;
use mtindo::history::sha256;
use mtindo::options::format::FormatConfig;
use mtindo::syntax::{Edition, parse};
use mtindo::verify::format::verify_format;

use crate::common::edition_for;
use crate::diff::{Answer, edition_year, kind_at_line, run_standard};
use crate::unformat::unformat_with;

/// Corpus files larger than this are left out: the standard formatter's time grows with
/// them and small inputs shrink faster.
const LARGEST: usize = 48 * 1024;

/// The most times one shrink runs the standard formatter.
const SHRINK_LIMIT: usize = 150;

/// What it takes to run both formatters.
struct Both {
  /// An empty home folder for the standard formatter.
  home: PathBuf,
  /// An empty folder to run it in, so no configuration file applies.
  empty: PathBuf,
}

impl Both {
  /// How mto and the standard formatter disagree on `input`, if they do.
  fn disagree(&self, input: &str, edition: Edition) -> Option<Disagreement> {
    let want = match run_standard(input, edition_year(edition), &self.empty, &self.home) {
      Answer::Text(text) => text,
      Answer::Refused(_) | Answer::NotRun(_) => {
        return Some(Disagreement {
          name: String::new(),
          got: String::new(),
          want: String::new(),
          refused: true,
          broken: None,
        });
      }
    };
    let got = format_file(input, edition, &config(edition, &self.empty));
    if got == want {
      return None;
    }
    let broken = verify_format(input, &want, edition)
      .first()
      .map(|v| v.detail.clone());
    Some(Disagreement {
      name: name(&got, &want, edition),
      got,
      want,
      refused: false,
      broken,
    })
  }

  /// `input` cut down, lines at a time, while it still parses and the outputs still
  /// disagree as `name` says.
  fn shrink(&self, input: &str, edition: Edition, name: &str) -> String {
    let mut lines: Vec<&str> = input.split_inclusive('\n').collect();
    let mut runs = 0;
    let mut chunk = lines.len() / 2;
    while chunk > 0 && runs < SHRINK_LIMIT {
      let mut at = 0;
      let mut cut_any = false;
      while at < lines.len() && runs < SHRINK_LIMIT {
        let end = (at + chunk).min(lines.len());
        let candidate: String = lines[..at].concat() + &lines[end..].concat();
        let still =
          !candidate.trim().is_empty() && parse(&candidate, edition).errors().is_empty() && {
            runs += 1;
            self
              .disagree(&candidate, edition)
              .is_some_and(|d| !d.refused && d.broken.is_none() && d.name == name)
          };
        if still {
          lines.drain(at..end);
          cut_any = true;
        } else {
          at += chunk;
        }
      }
      if !cut_any {
        chunk /= 2;
      }
    }
    lines.concat()
  }
}

/// How the two outputs part on an input.
struct Disagreement {
  name: String,
  got: String,
  want: String,
  /// The standard formatter refused the input.
  refused: bool,
  /// How the standard formatter's output changes the input's meaning, if it does.
  broken: Option<String>,
}

/// A seeded xorshift, so a run repeats from its seed.
struct Random(u64);

impl Random {
  fn next(&mut self) -> u64 {
    self.0 ^= self.0 << 13;
    self.0 ^= self.0 >> 7;
    self.0 ^= self.0 << 17;
    self.0
  }
}

pub fn run(args: &[String]) -> ExitCode {
  let mut seconds = 60u64;
  let mut seed = 0x5eed_u64;
  let mut out = PathBuf::from(".");
  let mut dirs = Vec::new();
  let mut replay = None;
  let mut recheck: Option<Vec<String>> = None;
  let mut edition = Edition::E2021;
  let mut at = 0;
  while at < args.len() {
    if args[at] == "--recheck" {
      recheck = Some(args[at + 1..].to_vec());
      break;
    }
    let value = args.get(at + 1).cloned().unwrap_or_default();
    match args[at].as_str() {
      "--time" => seconds = value.parse().unwrap_or(60),
      "--seed" => seed = value.parse().unwrap_or(seed),
      "--out" => out = PathBuf::from(value),
      "--replay" => replay = Some(PathBuf::from(value)),
      "--edition" => edition = edition_of(&value),
      other => {
        dirs.push(PathBuf::from(other));
        at += 1;
        continue;
      }
    }
    at += 2;
  }
  let home = std::env::temp_dir().join("mtindo-diff-fuzz-home");
  let empty = std::env::temp_dir().join("mtindo-diff-fuzz-empty");
  let _ = fs::create_dir_all(&home);
  let _ = fs::create_dir_all(&empty);
  let both = Both { home, empty };
  if let Some(inputs) = recheck {
    for input in inputs {
      let (year, file) = input.split_once(':').unwrap_or(("2021", &input));
      let Ok(text) = fs::read_to_string(file) else {
        continue;
      };
      match both.disagree(&text, edition_of(year)) {
        Some(Disagreement {
          broken: Some(what), ..
        }) => println!("broken\t{}\t{}", file, what),
        Some(found) if !found.refused => println!("differ\t{}\t{}", file, found.name),
        Some(_) => {}
        None => println!("agree\t{}", file),
      }
    }
    return ExitCode::SUCCESS;
  }
  if let Some(file) = replay {
    let Ok(input) = fs::read_to_string(&file) else {
      eprintln!("cannot read {}", file.display());
      return ExitCode::from(2);
    };
    return match both.disagree(&input, edition) {
      Some(found) => {
        println!(
          "{}\n\nmto:\n{}\nthe standard formatter:\n{}",
          found.name, found.got, found.want
        );
        if let Some(what) = &found.broken {
          println!(
            "the standard formatter's output changes the meaning: {}",
            what
          );
        }
        ExitCode::from(1)
      }
      None => {
        println!("mto and the standard formatter agree on {}", file.display());
        ExitCode::SUCCESS
      }
    };
  }
  let mut files = Vec::new();
  for dir in &dirs {
    crate::corpus::collect(dir, &mut files);
  }
  files.sort();
  files.retain(|f| fs::metadata(f).is_ok_and(|m| m.len() as usize <= LARGEST));
  let _ = fs::create_dir_all(&out);
  let mut random = Random(seed | 1);
  let started = Instant::now();
  let limit = Duration::from_secs(seconds);
  let (mut tried, mut agreed, mut refused, mut broken) = (0usize, 0usize, 0usize, 0usize);
  let mut seen: HashSet<String> = HashSet::new();
  while started.elapsed() < limit {
    // Every other input from the generator; the rest scrambled corpus files.
    let (input, edition) = if files.is_empty() || random.next().is_multiple_of(2) {
      let length = 64 + (random.next() % 1024) as usize;
      let bytes: Vec<u8> = (0..length).map(|_| random.next() as u8).collect();
      (mtindo::fuzzing::generate::source(&bytes), Edition::E2021)
    } else {
      let file = &files[(random.next() % files.len() as u64) as usize];
      let Ok(text) = fs::read_to_string(file) else {
        continue;
      };
      let edition = edition_for(file, &text);
      match unformat_with(&text, edition, random.next()) {
        Some(messy) => (messy, edition),
        None => continue,
      }
    };
    if !parse(&input, edition).errors().is_empty() {
      continue;
    }
    tried += 1;
    let Some(found) = both.disagree(&input, edition) else {
      agreed += 1;
      continue;
    };
    if found.refused {
      refused += 1;
      continue;
    }
    if found.broken.is_some() {
      broken += 1;
      continue;
    }
    if !seen.insert(found.name.clone()) {
      continue;
    }
    let small = both.shrink(&input, edition, &found.name);
    let file = out.join(format!("{}.rs", &sha256::hex(found.name.as_bytes())[..12]));
    if fs::write(&file, &small).is_ok() {
      println!(
        "mismatch\t{}\t{}\t{}",
        found.name,
        file.display(),
        edition_year(edition)
      );
    }
  }
  let took = started.elapsed().as_secs_f64().max(1e-9);
  let compared = tried - refused - broken;
  eprintln!(
    "diff-fuzz: {} inputs in {:.0} s; mto agreed on {} of {} the standard formatter took \
     ({:.1}%); {} ways of disagreeing; {} where its output changes the meaning",
    tried,
    took,
    agreed,
    compared,
    100.0 * agreed as f64 / compared.max(1) as f64,
    seen.len(),
    broken
  );
  println!(
    "metric fuzz.diff.inputs_per_s {:.2} inputs/s",
    tried as f64 / took
  );
  println!(
    "metric fuzz.diff.agreed.pct {:.2} %",
    100.0 * agreed as f64 / compared.max(1) as f64
  );
  println!("metric fuzz.diff.ways {} ways", seen.len());
  ExitCode::SUCCESS
}

/// The formatting settings for `edition`, as mto finds them in a folder with no
/// configuration file.
fn config(edition: Edition, empty: &Path) -> FormatConfig {
  let overrides = Overrides {
    settings: vec![("edition".to_string(), edition_year(edition).to_string())],
    ..Overrides::default()
  };
  Resolver::new(&overrides, Environment::default())
    .ok()
    .and_then(|mut r| r.config_for(empty).ok())
    .map(|c| c.format())
    .unwrap_or_default()
}

fn edition_of(year: &str) -> Edition {
  match year {
    "2015" => Edition::E2015,
    "2018" => Edition::E2018,
    "2024" => Edition::E2024,
    _ => Edition::E2021,
  }
}

/// The name of a disagreement: the syntax kind the first differing line starts in the
/// standard formatter's output, and how the lines differ.
fn name(got: &str, want: &str, edition: Edition) -> String {
  let got_lines: Vec<&str> = got.lines().collect();
  let want_lines: Vec<&str> = want.lines().collect();
  let at = got_lines
    .iter()
    .zip(&want_lines)
    .position(|(g, w)| g != w)
    .unwrap_or(got_lines.len().min(want_lines.len()));
  let (g, w) = (
    got_lines.get(at).copied().unwrap_or(""),
    want_lines.get(at).copied().unwrap_or(""),
  );
  let solid = |s: &str| s.split_whitespace().collect::<String>();
  let how = if g.trim() == w.trim() {
    "indentation"
  } else if solid(g) == solid(w) {
    "spacing"
  } else if g.trim().is_empty() || w.trim().is_empty() {
    "blank lines"
  } else {
    "line breaks"
  };
  format!("{} / {}", kind_at_line(want, at, edition), how)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_disagreement_is_named_by_its_first_differing_line() {
    let want = "fn f() {\n    let x = 1;\n}\n";
    assert_eq!(
      name("fn f() {\n  let x = 1;\n}\n", want, Edition::E2021),
      "LetStmt / indentation"
    );
    assert_eq!(
      name("fn f() {\n    let x=1;\n}\n", want, Edition::E2021),
      "LetStmt / spacing"
    );
    assert_eq!(
      name(
        "fn f() {\n    let x =\n        1;\n}\n",
        want,
        Edition::E2021
      ),
      "LetStmt / line breaks"
    );
  }
}
