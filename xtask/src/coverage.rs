//! `cargo xtask fuzz coverage [TARGET...]`: what the fuzz corpora reach. Each target's
//! corpus is run through a coverage build (`cargo fuzz coverage`), the profiles are merged,
//! and the lines and functions reached are counted per module of `src/`, for each target
//! and for all of them together. The figures are kept as series,
//! `coverage.fuzz.target.<target>.lines`, `coverage.fuzz.module.<module>.lines` and
//! `coverage.fuzz.lines`; an HTML report of the whole is written to `fuzz/coverage/html/`
//! (gitignored).
//!
//! It needs a nightly toolchain with the LLVM tools (`llvm-tools-preview`): plain `nightly`
//! when it has them, else the newest dated nightly installed that does.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::common::log::Logger;
use crate::common::process;
use crate::metrics;

/// Files the counts leave out: dependencies, the standard library, the targets themselves.
const IGNORED: &str = r"(\.cargo/registry|\.cargo/git|/rustc/|fuzz_targets|/library/)";

/// Lines and functions, reached and all.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Count {
  lines: u64,
  missed_lines: u64,
  functions: u64,
  missed_functions: u64,
}

impl Count {
  fn add(&mut self, other: &Count) {
    self.lines += other.lines;
    self.missed_lines += other.missed_lines;
    self.functions += other.functions;
    self.missed_functions += other.missed_functions;
  }

  fn lines_percent(&self) -> f64 {
    percent(self.lines - self.missed_lines, self.lines)
  }

  fn functions_percent(&self) -> f64 {
    percent(self.functions - self.missed_functions, self.functions)
  }
}

pub fn run(root: &Path, sub: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "coverage");
  let mut targets = crate::cli::texts(sub, "targets");
  if targets.is_empty() {
    targets = all_targets(root);
  }
  let Some((toolchain, tools)) = toolchain() else {
    println!(
      "no nightly toolchain with the LLVM tools: run `rustup component add \
       llvm-tools-preview --toolchain nightly`"
    );
    return ExitCode::from(2);
  };
  println!(
    "coverage with {}, over {} target(s)",
    toolchain,
    targets.len()
  );
  let fuzz = root.join("fuzz");
  let triple = host_triple(&toolchain);
  // Run from the root, `cargo fuzz` builds into the root's target folder.
  let binary = |target: &str| {
    root
      .join("target")
      .join(&triple)
      .join("coverage")
      .join(&triple)
      .join("release")
      .join(target)
  };
  let mut covered: Vec<String> = Vec::new();
  let mut failed = false;
  for target in &targets {
    if !fuzz.join("corpus").join(target).is_dir() {
      println!(
        "  {:<10} no corpus yet (cargo xtask fuzz run {})",
        target, target
      );
      continue;
    }
    log.info(&format!("cargo fuzz coverage {}", target));
    let (code, output) = process::capture(
      root,
      "cargo",
      &[
        format!("+{}", toolchain),
        "fuzz".into(),
        "coverage".into(),
        target.clone(),
      ],
      &[],
      process::LONG,
    );
    let profile = fuzz.join("coverage").join(target).join("coverage.profdata");
    if code != 0 || !profile.is_file() {
      let last = output.lines().rev().find(|l| !l.trim().is_empty());
      println!("  {:<10} failed: {}", target, last.unwrap_or(""));
      failed = true;
      continue;
    }
    // The raw profiles are merged into coverage.profdata; only it is kept.
    let _ = std::fs::remove_dir_all(fuzz.join("coverage").join(target).join("raw"));
    let rows = report(root, &tools, &profile, &[binary(target)]);
    // No rows: the build or the profile was not found, which is not 0%.
    if rows.is_empty() {
      println!("  {:<10} failed: llvm-cov found nothing to count", target);
      failed = true;
      continue;
    }
    let total = sum(rows.values());
    println!(
      "  {:<10} {:>5.1}% of lines, {:>5.1}% of functions",
      target,
      total.lines_percent(),
      total.functions_percent()
    );
    metrics::record(
      root,
      &format!("coverage.fuzz.target.{}.lines", target),
      round(total.lines_percent()),
      "%",
    );
    covered.push(target.clone());
  }
  if covered.is_empty() {
    return ExitCode::from(1);
  }
  // All targets together.
  let merged = fuzz.join("coverage").join("all.profdata");
  let mut args = vec![
    "merge".to_string(),
    "-sparse".to_string(),
    "-o".to_string(),
    merged.display().to_string(),
  ];
  args.extend(covered.iter().map(|t| {
    fuzz
      .join("coverage")
      .join(t)
      .join("coverage.profdata")
      .display()
      .to_string()
  }));
  let (code, output) = process::capture(
    root,
    &tools.join("llvm-profdata").display().to_string(),
    &args,
    &[],
    process::LONG,
  );
  if code != 0 {
    println!("cannot merge the profiles: {}", output.trim());
    return ExitCode::from(1);
  }
  let binaries: Vec<PathBuf> = covered.iter().map(|t| binary(t)).collect();
  let rows = report(root, &tools, &merged, &binaries);
  if rows.is_empty() {
    println!("llvm-cov found nothing to count in the merged profile");
    return ExitCode::from(1);
  }
  let mut modules: BTreeMap<String, Count> = BTreeMap::new();
  for (file, count) in &rows {
    let module = match file.split_once('/') {
      Some((module, _)) => module.to_string(),
      None => "crate root".to_string(),
    };
    modules.entry(module).or_default().add(count);
  }
  println!("\nall targets together, by module of src/:");
  println!("  {:<12} {:>8} {:>10}", "MODULE", "LINES", "FUNCTIONS");
  for (module, count) in &modules {
    println!(
      "  {:<12} {:>7.1}% {:>9.1}%",
      module,
      count.lines_percent(),
      count.functions_percent()
    );
    let name = module.replace(' ', "_");
    metrics::record(
      root,
      &format!("coverage.fuzz.module.{}.lines", name),
      round(count.lines_percent()),
      "%",
    );
  }
  let total = sum(rows.values());
  println!(
    "  {:<12} {:>7.1}% {:>9.1}%",
    "all",
    total.lines_percent(),
    total.functions_percent()
  );
  metrics::record(
    root,
    "coverage.fuzz.lines",
    round(total.lines_percent()),
    "%",
  );
  let html = fuzz.join("coverage").join("html");
  let mut args = vec![
    "show".to_string(),
    "-format=html".to_string(),
    format!("-output-dir={}", html.display()),
    format!("-instr-profile={}", merged.display()),
    format!("-ignore-filename-regex={}", IGNORED),
    binaries[0].display().to_string(),
  ];
  for other in &binaries[1..] {
    args.push(format!("-object={}", other.display()));
  }
  let (code, _) = process::capture(
    root,
    &tools.join("llvm-cov").display().to_string(),
    &args,
    &[],
    process::LONG,
  );
  if code == 0 {
    println!("\nreport: {}", html.join("index.html").display());
  }
  if failed {
    ExitCode::from(1)
  } else {
    ExitCode::SUCCESS
  }
}

/// Every fuzz target, by the files in `fuzz/fuzz_targets/`.
fn all_targets(root: &Path) -> Vec<String> {
  let mut targets: Vec<String> = std::fs::read_dir(root.join("fuzz").join("fuzz_targets"))
    .into_iter()
    .flatten()
    .flatten()
    .filter_map(|e| {
      e.file_name()
        .to_string_lossy()
        .strip_suffix(".rs")
        .map(str::to_string)
    })
    .collect();
  targets.sort();
  targets
}

/// The host's target triple, as `rustc -vV` says it.
fn host_triple(toolchain: &str) -> String {
  process::output(
    piped(std::process::Command::new("rustc").args([format!("+{}", toolchain), "-vV".into()])),
    process::QUICK,
  )
  .ok()
  .and_then(|out| {
    String::from_utf8_lossy(&out.stdout)
      .lines()
      .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
  })
  .unwrap_or_default()
}

/// The rows of an `llvm-cov report` table: filename, regions, missed regions, cover,
/// functions, missed functions, executed, lines, missed lines, cover, and more.
fn parse_report(text: &str) -> BTreeMap<String, Count> {
  let mut rows = BTreeMap::new();
  for line in text.lines() {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() < 10 || words[0] == "Filename" || words[0] == "TOTAL" {
      continue;
    }
    let number = |at: usize| words[at].parse::<u64>().ok();
    let (Some(functions), Some(missed_functions), Some(lines), Some(missed_lines)) =
      (number(4), number(5), number(7), number(8))
    else {
      continue;
    };
    // Paths are shown from the folder all files share; the library's are under src/.
    let file = words[0];
    let file = file
      .rsplit_once("src/")
      .map_or(file, |(_, rest)| rest)
      .to_string();
    rows.insert(
      file,
      Count {
        lines,
        missed_lines,
        functions,
        missed_functions,
      },
    );
  }
  rows
}

fn percent(part: u64, whole: u64) -> f64 {
  if whole == 0 {
    return 0.0;
  }
  100.0 * part as f64 / whole as f64
}

/// `command` with its output read, not shown.
fn piped(command: &mut std::process::Command) -> &mut std::process::Command {
  command
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
}

/// Each source file's counts, by its path under `src/`, from `llvm-cov report`.
fn report(
  root: &Path,
  tools: &Path,
  profile: &Path,
  binaries: &[PathBuf],
) -> BTreeMap<String, Count> {
  let mut args = vec![
    "report".to_string(),
    format!("-instr-profile={}", profile.display()),
    format!("-ignore-filename-regex={}", IGNORED),
    binaries[0].display().to_string(),
  ];
  for other in &binaries[1..] {
    args.push(format!("-object={}", other.display()));
  }
  let (_, output) = process::capture(
    root,
    &tools.join("llvm-cov").display().to_string(),
    &args,
    &[],
    process::LONG,
  );
  parse_report(&output)
}

fn round(value: f64) -> f64 {
  (value * 100.0).round() / 100.0
}

fn sum<'a>(counts: impl Iterator<Item = &'a Count>) -> Count {
  let mut total = Count::default();
  for count in counts {
    total.add(count);
  }
  total
}

/// A nightly toolchain with the LLVM tools, and the folder they are in.
fn toolchain() -> Option<(String, PathBuf)> {
  let listed = process::output(
    piped(std::process::Command::new("rustup").args(["toolchain", "list"])),
    process::QUICK,
  )
  .ok()?;
  let mut nightlies: Vec<String> = String::from_utf8_lossy(&listed.stdout)
    .lines()
    .filter_map(|l| l.split_whitespace().next())
    .filter(|name| name.starts_with("nightly"))
    .map(|name| {
      // `nightly-2026-10-05-aarch64-apple-darwin` is used as `nightly-2026-10-05`.
      let parts: Vec<&str> = name.split('-').collect();
      match parts.as_slice() {
        ["nightly", y, m, d, ..] if y.len() == 4 => format!("nightly-{}-{}-{}", y, m, d),
        _ => "nightly".to_string(),
      }
    })
    .collect();
  // Plain nightly first, then the newest dated one.
  nightlies.sort_by(|a, b| (a != "nightly").cmp(&(b != "nightly")).then(b.cmp(a)));
  nightlies.dedup();
  for toolchain in nightlies {
    let Ok(out) = process::output(
      piped(std::process::Command::new("rustc").args([
        format!("+{}", toolchain),
        "--print".into(),
        "sysroot".into(),
      ])),
      process::QUICK,
    ) else {
      continue;
    };
    let sysroot = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let tools = sysroot
      .join("lib")
      .join("rustlib")
      .join(host_triple(&toolchain))
      .join("bin");
    if tools.join("llvm-cov").is_file() && tools.join("llvm-profdata").is_file() {
      return Some((toolchain, tools));
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_report_rows_are_read_by_file() {
    let text = "Filename  Regions  Missed Regions  Cover  Functions  Missed Functions  Executed  \
                Lines  Missed Lines  Cover  Branches  Missed Branches  Cover\n\
                ------\n\
                config/toml/parser.rs  718  9  98.75%  50  1  98.00%  608  8  98.68%  0  0  -\n\
                /work/src/x.rs  10  10  0.00%  2  2  0.00%  10  10  0.00%  0  0  -\n\
                lib.rs  10  5  50.00%  4  2  50.00%  20  10  50.00%  0  0  -\n\
                TOTAL  738  24  1%  56  5  1%  638  28  1%  0  0  -\n";
    let rows = parse_report(text);
    assert_eq!(rows.len(), 3);
    let parser = rows["config/toml/parser.rs"];
    assert_eq!((parser.lines, parser.missed_lines), (608, 8));
    assert_eq!((parser.functions, parser.missed_functions), (50, 1));
    assert!(rows.contains_key("x.rs"));
    assert!((sum(rows.values()).lines_percent() - 100.0 * 610.0 / 638.0).abs() < 1e-9);
  }
}
