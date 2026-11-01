//! Metrics: every number a run measures, kept as a series, and the ratchet some of them
//! may not fall behind.
//!
//! A series is `.dev/metrics/<name>.tsv`, one row per measurement:
//!
//! ```text
//! when                       tree                     run                          value  unit
//! 2026-10-11T20:31:02.114Z   7bb39cb-dirty.47bbb074   20261011-203055-corpus-1a2b  19.2   x
//! ```
//!
//! Names are dotted: `compat.<set>.files`, `speed.mto.laid_out`, `memst.serde.allocs`,
//! `fuzz.<target>.peak_rss`, `task.<id>.seconds`. The run is the recorded run that
//! measured it, when there is one.
//!
//! The ratchet, `records/results/ratchet.txt` (committed), holds for some series the value they
//! may not get worse than, with a tolerance for noise and which way is better. A figure
//! past its limit files a `speed-regression` or `memory-regression` incident (keyed by the
//! series), and a figure back within it marks that incident fixed. Speed depends on the
//! machine, so the file names the one it was recorded on and is only applied there.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::time::SystemTime;

use crate::common::conditions;
use crate::common::log::timestamp;
use crate::common::store;
use crate::common::tree;
use crate::incidents;

/// Where every figure stood before the optimization rounds, committed: what `compare` sets
/// the latest values beside.
pub const BASELINE: &str = "records/archive/2026-10-optimization.txt";

/// The series the ratchet holds, with their tolerance and direction. A figure measured for
/// one of these is checked as it is recorded (see [`check`]).
///
/// Tolerances from what the figures do on an unchanged tree: timings move by up to about
/// 14% with other work on the machine, peak live memory by a quarter with how threads
/// interleave, allocation counts and bytes by a few allocations, the time a write takes
/// by half with what else the drive is doing. A growth exponent moves by about a tenth; a
/// fifth over linear still catches one turning quadratic.
pub const RATCHETED: &[(&str, f64, Better)] = &[
  ("speed.speedup.stable", 0.15, Better::Higher),
  ("speed.speedup.nightly", 0.15, Better::Higher),
  ("speed.mto.laid_out.mb_per_s", 0.15, Better::Higher),
  ("speed.mto.as_fetched.mb_per_s", 0.15, Better::Higher),
  ("speed.layout.one_thread.mb_per_s", 0.15, Better::Higher),
  ("memst.serde.allocs", 0.01, Better::Lower),
  ("memst.serde.bytes", 0.01, Better::Lower),
  ("memst.serde.peak_live", 0.25, Better::Lower),
  ("doctor.held.cost.x", 0.25, Better::Lower),
  ("doctor.narrow.n10.k1.checks", 0.1, Better::Lower),
  ("doctor.narrow.n100.k1.checks", 0.1, Better::Lower),
  ("doctor.narrow.n100.k3.checks", 0.1, Better::Lower),
  ("doctor.narrow.found_rate", 0.0, Better::Higher),
  ("io.write.batch.history.ms_per_file", 0.5, Better::Lower),
  ("io.write.batch.plain.ms_per_file", 0.5, Better::Lower),
  ("perf.scaling.one_thread.mb_per_s", 0.15, Better::Higher),
  ("perf.latency.check.median_ms", 0.5, Better::Lower),
  ("perf.growth.items.exponent", 0.2, Better::Lower),
  ("perf.growth.nesting.exponent", 0.2, Better::Lower),
  ("perf.growth.line.exponent", 0.2, Better::Lower),
  ("perf.growth.chain.exponent", 0.2, Better::Lower),
  ("perf.growth.match.exponent", 0.2, Better::Lower),
];

/// The ratchet, committed.
pub const RECORD: &str = "records/results/ratchet.txt";

/// `busy` (the share of the processor other work used, percent) and `state` were added
/// later: rows written before have neither, and count as taken in unknown conditions
/// (judged as before).
const HEADER: &str = "when\ttree\trun\tvalue\tunit\tbusy\tstate\n";

/// Which way a figure is better.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Better {
  Higher,
  Lower,
}

/// One line of the ratchet: series `name` may not get worse than `value` by more than
/// `tolerance` (a fraction).
#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
  pub name: String,
  pub value: f64,
  pub tolerance: f64,
  pub better: Better,
}

impl Limit {
  /// The worst value still within the limit.
  pub fn bound(&self) -> f64 {
    match self.better {
      Better::Higher => self.value * (1.0 - self.tolerance),
      Better::Lower => self.value * (1.0 + self.tolerance),
    }
  }

  pub fn holds(&self, value: f64) -> bool {
    match self.better {
      Better::Higher => value >= self.bound(),
      Better::Lower => value <= self.bound(),
    }
  }

  /// The incident kind a breach files.
  fn kind(&self) -> &'static str {
    if self.name.starts_with("doctor.") {
      "doctor-regression"
    } else if self.name.starts_with("memst.") {
      "memory-regression"
    } else {
      // Speed, write times (`io.`), latency and growth (`perf.`).
      "speed-regression"
    }
  }
}

/// One measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
  pub when: String,
  pub tree: String,
  pub run: String,
  pub value: f64,
  pub unit: String,
  /// The conditions it was taken in (see `common::conditions`): `quiet`, `busy`,
  /// `throttled`, or empty when not known.
  pub state: String,
}

impl Point {
  /// Taken while the machine was busy or throttled: a timing not to be judged.
  pub fn noisy(&self) -> bool {
    matches!(self.state.as_str(), "busy" | "throttled")
  }
}

pub fn dir(root: &Path) -> PathBuf {
  store::dir(root, store::METRICS)
}

/// This machine, as the ratchet names it: system, architecture and core count.
pub fn machine() -> String {
  let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
  format!(
    "{} {}, {} cores",
    std::env::consts::OS,
    std::env::consts::ARCH,
    cores
  )
}

/// Every series' name, sorted.
pub fn names(root: &Path) -> Vec<String> {
  let mut names: Vec<String> = fs::read_dir(dir(root))
    .into_iter()
    .flatten()
    .flatten()
    .filter_map(|e| {
      e.file_name()
        .to_string_lossy()
        .strip_suffix(".tsv")
        .map(str::to_string)
    })
    .collect();
  names.sort();
  names
}

/// [`read_record`] on the ratchet's text, as any commit kept it.
pub fn parse_record(text: &str) -> (String, Vec<Limit>) {
  let mut machine = String::new();
  let mut limits = Vec::new();
  for line in text.lines() {
    if let Some(m) = line.strip_prefix("machine ") {
      machine = m.trim().to_string();
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    if let ["limit", name, value, tolerance, better] = words.as_slice() {
      let (Ok(value), Ok(tolerance)) = (value.parse(), tolerance.parse()) else {
        continue;
      };
      limits.push(Limit {
        name: name.to_string(),
        value,
        tolerance,
        better: if *better == "lower" {
          Better::Lower
        } else {
          Better::Higher
        },
      });
    }
  }
  (machine, limits)
}

/// The ratchet: the machine it was recorded on, and its limits.
pub fn read_record(root: &Path) -> (String, Vec<Limit>) {
  parse_record(&fs::read_to_string(root.join(RECORD)).unwrap_or_default())
}

/// Appends a measurement to series `name`. Never fails the command: a figure that cannot
/// be kept is reported on stderr.
pub fn record(root: &Path, name: &str, value: f64, unit: &str) {
  if !value.is_finite() {
    return;
  }
  let path = dir(root).join(format!("{}.tsv", name));
  let conditions = conditions::settled();
  let line = format!(
    "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
    timestamp(SystemTime::now()),
    this_tree(root),
    this_run(),
    value,
    unit,
    conditions
      .and_then(|c| c.busy)
      .map_or(String::new(), |b| format!("{:.1}", b)),
    conditions.map_or("", |c| c.state())
  );
  let written = fs::create_dir_all(dir(root)).and_then(|_| {
    let fresh = !path.exists();
    let mut file = fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(&path)?;
    if fresh {
      file.write_all(HEADER.as_bytes())?;
    }
    file.write_all(line.as_bytes())
  });
  if let Err(e) = written {
    eprintln!("cannot keep metric {}: {}", name, e);
  }
}

/// Records `value` for `name` and, if the ratchet holds `name` and this is its machine,
/// checks it: a breach files (or adds to) a regression incident, a value within the limit
/// fixes an open one. Returns the breach's message, if any.
pub fn record_checked(root: &Path, name: &str, value: f64, unit: &str) -> Option<String> {
  record(root, name, value, unit);
  if timed(name) && conditions::settled().is_some_and(|c| c.noisy()) {
    eprintln!(
      "{}: taken while the machine was busy or throttled; kept, not judged",
      name
    );
    return None;
  }
  check(root, name, value, unit)
}

/// `cargo xtask metrics [list | show NAME [--last N] | ratchet [--update]]`.
pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  match m.subcommand() {
    None | Some(("list", _)) => {
      let filter = m
        .subcommand()
        .and_then(|(_, sub)| crate::cli::text(sub, "filter"))
        .unwrap_or_default();
      print!("{}", list_text(root, &filter));
      ExitCode::SUCCESS
    }
    Some(("show", sub)) => {
      let name = crate::cli::text(sub, "name").unwrap_or_default();
      let last = sub.get_one::<u32>("last").copied().unwrap_or(20) as usize;
      let points = series(root, &name);
      if points.is_empty() {
        eprintln!("no series `{}` (cargo xtask metrics list)", name);
        return ExitCode::from(2);
      }
      let shown = &points[points.len().saturating_sub(last)..];
      println!(
        "{}  {}",
        name,
        sparkline(&shown.iter().map(|p| p.value).collect::<Vec<_>>())
      );
      for p in shown {
        println!(
          "  {}  {:<26} {:>14} {}  {}",
          &p.when[..19.min(p.when.len())],
          p.tree,
          p.value,
          p.unit,
          p.run
        );
      }
      ExitCode::SUCCESS
    }
    Some(("compare", _)) => match compare_text(root) {
      Ok(text) => {
        print!("{}", text);
        ExitCode::SUCCESS
      }
      Err(e) => {
        eprintln!("{}", e);
        ExitCode::from(2)
      }
    },
    Some(("ratchet", sub)) => {
      if crate::cli::on(sub, "update") {
        return match record_text(root) {
          Ok(text) => match fs::write(root.join(RECORD), text) {
            Ok(()) => {
              println!("recorded in {}", RECORD);
              ExitCode::SUCCESS
            }
            Err(e) => {
              println!("cannot write {}: {}", RECORD, e);
              ExitCode::from(1)
            }
          },
          Err(e) => {
            println!("{}", e);
            ExitCode::from(1)
          }
        };
      }
      print!("{}", ratchet_text(root));
      ExitCode::SUCCESS
    }
    _ => unreachable!("clap knows the subcommands"),
  }
}

/// Every point of series `name`, oldest first.
pub fn series(root: &Path, name: &str) -> Vec<Point> {
  fs::read_to_string(dir(root).join(format!("{}.tsv", name)))
    .unwrap_or_default()
    .lines()
    .skip(1)
    .filter_map(|line| {
      let f: Vec<&str> = line.split('\t').collect();
      Some(Point {
        when: f.first()?.to_string(),
        tree: f.get(1)?.to_string(),
        run: f.get(2)?.to_string(),
        value: f.get(3)?.parse().ok()?,
        unit: f.get(4).unwrap_or(&"").to_string(),
        state: f.get(6).unwrap_or(&"").to_string(),
      })
    })
    .collect()
}

/// A value in few characters: `3.20M`, `45.30`.
pub fn short(value: f64) -> String {
  if value.abs() >= 1e9 {
    format!("{:.2}G", value / 1e9)
  } else if value.abs() >= 1e6 {
    format!("{:.2}M", value / 1e6)
  } else if value.abs() >= 1e3 {
    format!("{:.1}k", value / 1e3)
  } else if value.fract() == 0.0 {
    format!("{}", value)
  } else {
    format!("{:.2}", value)
  }
}

/// A line of bars for a series' values.
pub fn sparkline(values: &[f64]) -> String {
  const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
  let (min, max) = values
    .iter()
    .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
  values
    .iter()
    .map(|v| {
      if (max - min).abs() < f64::EPSILON {
        BARS[3]
      } else {
        BARS[((v - min) / (max - min) * 7.0).round() as usize]
      }
    })
    .collect()
}

fn check(root: &Path, name: &str, value: f64, unit: &str) -> Option<String> {
  let (recorded_on, limits) = read_record(root);
  let limit = limits.iter().find(|l| l.name == name)?;
  if recorded_on != machine() {
    return None;
  }
  let tree = this_tree(root);
  if limit.holds(value) {
    for (dir, mut incident) in incidents::all(root) {
      if incident.kind == limit.kind() && incident.target == name && incident.status == "open" {
        incidents::mark(root, &dir, &mut incident, "fixed", tree);
      }
    }
    return None;
  }
  let message = format!(
    "{} is {} {}, past its limit of {} (recorded {} with {:.0}% tolerance)",
    name,
    rounded(value),
    unit,
    rounded(limit.bound()),
    rounded(limit.value),
    limit.tolerance * 100.0
  );
  let report = incidents::Report {
    kind: limit.kind(),
    target: name,
    said: &message,
    input: None,
    replay: "cargo xtask metrics show NAME, and the command that measures it",
    tree,
    run: &this_run(),
    key: Some(name),
  };
  let _ = incidents::file(root, &report);
  Some(message)
}

/// Each figure of [`BASELINE`] beside its series' latest judged value: the change, and
/// whether it is better (under 5% for a timing and 2% for a count is the same), then the
/// baseline's notes.
fn compare_text(root: &Path) -> Result<String, String> {
  let text = fs::read_to_string(root.join(BASELINE))
    .map_err(|e| format!("cannot read {}: {}", BASELINE, e))?;
  let mut out = format!(
    "{:<38} {:>18} {:>18} {:>8}  {}\n",
    "figure", "before", "now", "change", ""
  );
  let mut notes = Vec::new();
  let (mut better, mut worse) = (0, 0);
  for line in text.lines() {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }
    if let Some(note) = line.strip_prefix("note ") {
      notes.push(note.to_string());
      continue;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let (Some(name), Some(Ok(before)), Some(unit), Some(direction)) = (
      words.first(),
      words.get(1).map(|v| v.parse::<f64>()),
      words.get(2),
      words.get(3),
    ) else {
      return Err(format!("{}: cannot read `{}`", BASELINE, line));
    };
    let unit = if *unit == "-" { "" } else { unit };
    // A wall time rounded up by polling (see the baseline's header): not compared.
    let polled = words.get(5) == Some(&"[polled]");
    let Some(now) = latest_judged(root, name).map(|p| p.value) else {
      out.push_str(&format!(
        "{:<38} {:>18} {:>18}\n",
        name,
        format!("{} {}", rounded(before), unit),
        "not measured"
      ));
      continue;
    };
    let change = if before == 0.0 {
      0.0
    } else {
      (now - before) / before.abs()
    };
    // A timing moves a few percent between runs on a quiet machine; a count does not.
    let same = if timed(name) { 0.05 } else { 0.02 };
    let verdict = if polled {
      "polled, not compared"
    } else if change.abs() < same {
      "same"
    } else if (change > 0.0) == (*direction == "higher") {
      better += 1;
      "better"
    } else {
      worse += 1;
      "worse"
    };
    out.push_str(&format!(
      "{:<38} {:>18} {:>18} {:>+7.1}%  {}\n",
      name,
      format!("{} {}", rounded(before), unit).trim_end(),
      format!("{} {}", rounded(now), unit).trim_end(),
      change * 100.0,
      verdict
    ));
  }
  out.push_str(&format!("{} better, {} worse\n", better, worse));
  for note in notes {
    out.push_str(&format!("note: {}\n", note));
  }
  Ok(out)
}

/// The latest point of series `name` that can be judged: taken in quiet or unknown
/// conditions, or not a timing at all.
fn latest_judged(root: &Path, name: &str) -> Option<Point> {
  let mut points = series(root, name);
  if timed(name) {
    points.retain(|p| !p.noisy());
  }
  points.pop()
}

fn list_text(root: &Path, filter: &str) -> String {
  let mut out = format!(
    "{:<40} {:>16} {:>9}  {:<24} {}\n",
    "SERIES", "LATEST", "CHANGE", "TREND", "POINTS"
  );
  for name in names(root).iter().filter(|n| n.contains(filter)) {
    let points = series(root, name);
    let Some(last) = points.last() else {
      continue;
    };
    let change = points
      .len()
      .checked_sub(2)
      .map(|i| points[i].value)
      .filter(|before| *before != 0.0)
      .map_or("-".to_string(), |before| {
        format!("{:+.1}%", 100.0 * (last.value - before) / before.abs())
      });
    let values: Vec<f64> = points
      .iter()
      .rev()
      .take(24)
      .rev()
      .map(|p| p.value)
      .collect();
    out.push_str(&format!(
      "{:<40} {:>16} {:>9}  {:<24} {}\n",
      name,
      format!("{} {}", short(last.value), last.unit),
      change,
      sparkline(&values),
      points.len()
    ));
  }
  out
}

fn ratchet_text(root: &Path) -> String {
  let (recorded_on, limits) = read_record(root);
  if limits.is_empty() {
    return format!(
      "no ratchet yet in {}; measure, then `cargo xtask metrics ratchet --update`\n",
      RECORD
    );
  }
  let here = machine();
  let mut out = format!("ratchet recorded on {}", recorded_on);
  if recorded_on != here {
    out.push_str(&format!(" (this is {}: not applied here)", here));
  }
  out.push('\n');
  for limit in &limits {
    let latest = series(root, &limit.name).last().map(|p| p.value);
    let verdict = match latest {
      Some(v) if limit.holds(v) => "ok",
      Some(_) => "PAST LIMIT",
      None => "not measured",
    };
    out.push_str(&format!(
      "  {:<36} limit {:>12} latest {:>12}  {}\n",
      limit.name,
      short(limit.bound()),
      latest.map_or("-".to_string(), short),
      verdict
    ));
  }
  out
}

/// The ratchet's text for this machine from each ratcheted series' latest value.
fn record_text(root: &Path) -> Result<String, String> {
  let mut out = String::from(
    "# The performance ratchet (`cargo xtask metrics ratchet --update` writes it): each\n\
     # figure may not get worse than its value by more than the tolerance (a fraction).\n\
     # Speed depends on the machine, so the limits apply only on the one named here.\n",
  );
  out.push_str(&format!("machine {}\n", machine()));
  let mut missing = Vec::new();
  for (name, tolerance, better) in RATCHETED {
    match latest_judged(root, name) {
      Some(point) => out.push_str(&format!(
        "limit {} {} {} {}\n",
        name,
        rounded(point.value),
        tolerance,
        if *better == Better::Lower {
          "lower"
        } else {
          "higher"
        }
      )),
      None => missing.push(*name),
    }
  }
  if !missing.is_empty() {
    return Err(format!(
      "no measurement yet for {}; run `cargo xtask corpus bench`, `cargo xtask memst`, `cargo xtask doctor bench`, `cargo xtask io bench` and `cargo xtask perf` first",
      missing.join(", ")
    ));
  }
  Ok(out)
}

/// A value for the ratchet file: whole numbers as they are, others to four significant
/// digits.
fn rounded(value: f64) -> String {
  if value.fract() == 0.0 {
    return format!("{}", value);
  }
  let digits = (3 - value.abs().log10().floor() as i32).max(0) as usize;
  format!("{:.*}", digits, value)
}

/// The recorded run this process belongs to, if any.
fn this_run() -> String {
  std::env::var(crate::runs::RUN_DIR)
    .ok()
    .and_then(|d| {
      Path::new(&d)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    })
    .unwrap_or_default()
}

/// This process's tree, worked out once: a run records many figures.
fn this_tree(root: &Path) -> &'static str {
  static TREE: OnceLock<String> = OnceLock::new();
  TREE.get_or_init(|| tree::identity(root))
}

/// Whether series `name` is a timing, which other work on the machine moves; counts
/// (allocations, checks, coverage) it does not.
fn timed(name: &str) -> bool {
  ["speed.", "perf.", "io."]
    .iter()
    .any(|p| name.starts_with(p))
    || [
      ".seconds",
      ".time_ms",
      ".mb_per_s",
      ".exec_per_s",
      ".inputs_per_s",
    ]
    .iter()
    .any(|s| name.ends_with(s))
}

#[cfg(test)]
mod tests {
  use super::*;

  struct Scratch(PathBuf);

  impl Scratch {
    fn new(name: &str) -> Scratch {
      let dir = std::env::temp_dir().join(format!("xtask-metrics-{}-{}", name, std::process::id()));
      let _ = fs::remove_dir_all(&dir);
      fs::create_dir_all(&dir).unwrap();
      Scratch(dir)
    }
  }

  impl Drop for Scratch {
    fn drop(&mut self) {
      let _ = fs::remove_dir_all(&self.0);
    }
  }

  #[test]
  fn test_a_breach_files_an_incident_and_recovery_fixes_it() {
    let scratch = Scratch::new("breach");
    let root = &scratch.0;
    fs::create_dir_all(root.join(RECORD).parent().unwrap()).unwrap();
    fs::write(
      root.join(RECORD),
      format!(
        "machine {}\nlimit memst.serde.allocs 1000 0.01 lower\n",
        machine()
      ),
    )
    .unwrap();
    assert_eq!(
      record_checked(root, "memst.serde.allocs", 1005.0, "allocations"),
      None
    );
    let breach = record_checked(root, "memst.serde.allocs", 1200.0, "allocations").unwrap();
    assert!(breach.contains("past its limit"), "{}", breach);
    let filed = incidents::all(root);
    assert_eq!(filed.len(), 1);
    assert_eq!(
      (filed[0].1.kind.as_str(), filed[0].1.status.as_str()),
      ("memory-regression", "open")
    );
    record_checked(root, "memst.serde.allocs", 990.0, "allocations");
    assert_eq!(incidents::all(root)[0].1.status, "fixed");
    // Not this machine: recorded, not checked.
    fs::write(
      root.join(RECORD),
      "machine elsewhere\nlimit memst.serde.allocs 1 0 lower\n",
    )
    .unwrap();
    assert_eq!(
      record_checked(root, "memst.serde.allocs", 5000.0, "allocations"),
      None
    );
  }

  #[test]
  fn test_a_series_keeps_every_point() {
    let scratch = Scratch::new("series");
    let root = &scratch.0;
    record(root, "speed.x", 10.0, "MB/s");
    record(root, "speed.x", 12.5, "MB/s");
    record(root, "speed.x", f64::NAN, "MB/s");
    let points = series(root, "speed.x");
    assert_eq!(
      points.iter().map(|p| p.value).collect::<Vec<_>>(),
      [10.0, 12.5]
    );
    assert_eq!(points[0].unit, "MB/s");
    assert_eq!(names(root), ["speed.x"]);
    assert!(list_text(root, "").contains("+25.0%"));
  }

  #[test]
  fn test_limits_hold_either_way() {
    let higher = Limit {
      name: "speed.x".into(),
      value: 100.0,
      tolerance: 0.1,
      better: Better::Higher,
    };
    assert!(higher.holds(91.0) && !higher.holds(89.0));
    let lower = Limit {
      better: Better::Lower,
      ..higher.clone()
    };
    assert!(lower.holds(109.0) && !lower.holds(111.0));
    assert_eq!(higher.kind(), "speed-regression");
    assert_eq!(
      Limit {
        name: "memst.serde.allocs".into(),
        ..lower.clone()
      }
      .kind(),
      "memory-regression"
    );
    // A slower write is a matter of speed, though less of it is better.
    assert_eq!(
      Limit {
        name: "io.write.batch.plain.ms_per_file".into(),
        ..higher.clone()
      }
      .kind(),
      "speed-regression"
    );
  }

  #[test]
  fn test_ratchet_values_are_rounded() {
    assert_eq!(rounded(19.48148619272352), "19.48");
    assert_eq!(rounded(119.99944748931084), "120.0");
    assert_eq!(rounded(3200302.0), "3200302");
    assert_eq!(rounded(0.012345), "0.01235");
  }

  #[test]
  fn test_sparkline() {
    assert_eq!(sparkline(&[1.0, 2.0, 3.0]), "▁▅█");
    assert_eq!(sparkline(&[5.0, 5.0]), "▄▄");
  }
}
