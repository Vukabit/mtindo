//! `cargo xtask status`: where the project stands, in one screen. The text the TUI's
//! dashboard shows, from the same records:
//!
//! - the tree (commit, and a digest of any uncommitted changes) and the last green run;
//! - the v0.1.0 gates against their targets, from `compat/compatibility.txt`;
//! - jobs running now, and the latest runs;
//! - incidents by status, and the open ones that fail the suite.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::common::{sample, tree};
use crate::runs;

/// The v0.1.0 bar for the corpus sets: share of files and of lines matching the standard
/// formatter.
const FILES_TARGET: f64 = 99.0;

const LINES_TARGET: f64 = 99.9;

/// Latest runs shown.
const RECENT: usize = 8;

pub fn run(root: &Path, _: &clap::ArgMatches) -> ExitCode {
  print!("{}", render(root));
  ExitCode::SUCCESS
}

/// Each set of the compatibility record: (name, files %, lines %).
fn gates(record: &str) -> Vec<(String, f64, f64)> {
  record
    .lines()
    .filter_map(|line| {
      let words: Vec<&str> = line.split_whitespace().collect();
      match words.as_slice() {
        ["set", name, "files", mf, f, "lines", ml, l, ..] => {
          let share = |a: &str, b: &str| {
            let (a, b) = (a.parse::<f64>().ok()?, b.parse::<f64>().ok()?);
            Some(100.0 * a / b.max(1.0))
          };
          Some((name.to_string(), share(mf, f)?, share(ml, l)?))
        }
        _ => None,
      }
    })
    .collect()
}

/// The last line of `.dev/logs/last-green.tsv`: when, which tree, which groups.
fn last_green(root: &Path) -> String {
  fs::read_to_string(root.join(".dev/logs/last-green.tsv"))
    .ok()
    .and_then(|text| {
      let line = text.lines().skip(1).last()?.to_string();
      let f: Vec<&str> = line.split('\t').collect();
      Some(format!(
        "{} at {} ({})",
        f.get(1)?,
        f.first()?.get(..19)?,
        f.get(2)?
      ))
    })
    .unwrap_or_else(|| "none recorded".to_string())
}

fn mark(value: f64, target: f64) -> &'static str {
  if value >= target { "ok" } else { "  " }
}

fn render(root: &Path) -> String {
  let mut out = String::new();
  out.push_str(&format!("tree       {}\n", tree::identity(root)));
  out.push_str(&format!("last green {}\n", last_green(root)));
  out.push_str("\nV0.1.0 GATES                   files           lines\n");
  let record = fs::read_to_string(root.join("compat/compatibility.txt")).unwrap_or_default();
  for (set, files, lines) in gates(&record) {
    out.push_str(&format!(
      "  {:<22} {:>7.1}% {} {:>7.2}% {}\n",
      set,
      files,
      mark(files, FILES_TARGET),
      lines,
      mark(lines, LINES_TARGET)
    ));
  }
  out.push_str(&format!(
    "  {:<22} {:>9} {:>15}\n",
    "target (corpus sets)",
    format!("{}%", FILES_TARGET),
    format!("{}%", LINES_TARGET)
  ));
  let runs = runs::list(root);
  let running: Vec<&(std::path::PathBuf, runs::Record)> = runs
    .iter()
    .filter(|(_, r)| r.current_state() == "running")
    .collect();
  out.push_str(&format!("\nRUNNING ({})\n", running.len()));
  for (dir, r) in &running {
    let curve = sample::curve(&dir.join("samples.tsv"), 20);
    out.push_str(&format!("  {}  {}  {}\n", r.id, r.command.join(" "), curve));
  }
  out.push_str("\nRECENT\n");
  for (_, r) in runs.iter().rev().take(RECENT) {
    out.push_str(&format!(
      "  {:<10} {}  {}\n",
      r.current_state(),
      r.started.get(..19).unwrap_or(""),
      r.command.join(" ")
    ));
  }
  out.push_str("\nMETRICS (latest, trend; cargo xtask metrics)\n");
  for name in [
    "speed.speedup.stable",
    "speed.layout.one_thread.mb_per_s",
    "memst.serde.allocs",
    "memst.serde.peak_live",
    "compat.corpus-unformatted.lines",
  ] {
    let points = crate::metrics::series(root, name);
    if let Some(last) = points.last() {
      let values: Vec<f64> = points
        .iter()
        .rev()
        .take(20)
        .rev()
        .map(|p| p.value)
        .collect();
      out.push_str(&format!(
        "  {:<34} {:>14} {}\n",
        name,
        format!("{} {}", crate::metrics::short(last.value), last.unit),
        crate::metrics::sparkline(&values)
      ));
    }
  }
  let incidents = crate::incidents::all(root);
  let count = |status: &str| incidents.iter().filter(|(_, i)| i.status == status).count();
  out.push_str(&format!(
    "\nINCIDENTS  {} open, {} flaky, {} fixed, {} won't fix (cargo xtask incidents)\n",
    count("open"),
    count("flaky"),
    count("fixed"),
    count("wont-fix")
  ));
  for problem in crate::incidents::check_gate(root) {
    out.push_str(&format!("  {}\n", problem));
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_gates_are_read() {
    let record = "reference x\nset corpus files 99 100 lines 999 1000 skipped 2\n";
    assert_eq!(gates(record), vec![("corpus".to_string(), 99.0, 99.9)]);
  }
}
