//! Reading what the suites recorded: the tracked records under records/ and their
//! history, and the machine-local records under `.dev/`. Nothing here measures anything.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::common::log::timestamp;
use crate::common::store;

/// What [`score_bands`] counts, in order.
pub const SCORE_BANDS: [&str; 4] = ["every line", "99% or more", "90% to 99%", "under 90%"];

/// records/results/baseline.txt.
#[derive(Debug, Default, PartialEq)]
pub struct Baseline {
  pub machine: String,
  pub corpus: String,
  pub lex: Option<f64>,
  pub lex_parse: Option<f64>,
}

/// One figure of records/archive/2026-10-optimization.txt, as written.
#[derive(Debug, PartialEq)]
pub struct Before {
  pub name: String,
  pub value: String,
  pub unit: String,
  pub when: String,
}

/// records/results/compatibility.txt: the reference it was made against and each set's numbers.
#[derive(Debug, Default, PartialEq)]
pub struct Compat {
  pub reference: String,
  pub sets: BTreeMap<String, Tally>,
}

/// One entry of records/policy/exceptions.txt.
#[derive(Debug, PartialEq)]
pub struct Exception {
  pub path: String,
  pub set: String,
  pub reason: String,
}

/// One set's numbers in the compatibility record.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tally {
  pub matched_files: u64,
  pub files: u64,
  pub matched_lines: u64,
  pub lines: u64,
  /// Absent from records made before refusals were counted.
  pub skipped: Option<u64>,
}

impl Tally {
  pub fn files_pct(&self) -> f64 {
    percent(self.matched_files, self.files)
  }

  pub fn lines_pct(&self) -> f64 {
    percent(self.matched_lines, self.lines)
  }
}

/// One commit that changed a tracked record: its short hash, its author date and the
/// record's text at it.
#[derive(Debug, PartialEq)]
pub struct Version {
  pub commit: String,
  pub date: String,
  pub text: String,
}

/// records/pins/corpus.txt's fingerprint, as the dictionary defines `corpus_fingerprint`, and
/// the number of repositories it covers.
pub fn corpus_fingerprint(root: &Path) -> Result<(String, usize), String> {
  let pins = crate::corpus::load_pins(root)?;
  let mut list = String::new();
  let mut count = 0;
  for pin in pins.iter().filter(|p| !p.opt_in) {
    list.push_str(&format!("{} {}\n", pin.name, pin.commit));
    count += 1;
  }
  let digest = crate::sha256::hex_digest(list.as_bytes());
  Ok((format!("sha256:{}", &digest[..16]), count))
}

/// The day a machine-local record was last written, `YYYY-MM-DD`.
pub fn day_written(path: &Path) -> Option<String> {
  let modified = fs::metadata(path).ok()?.modified().ok()?;
  Some(timestamp(modified)[..10].to_string())
}

/// The text of a tracked record each time it changed, newest first, from
/// records/results/history.txt (without its comments).
pub fn file_history(root: &Path, path: &str) -> Result<Vec<Version>, String> {
  Ok(
    super::history::of(root, path)
      .into_iter()
      .map(|e| Version {
        commit: e.commit,
        date: e.date,
        text: e.text,
      })
      .collect(),
  )
}

/// The kinds of syntax at the first differing line of every mismatching file of the last
/// diff run (`.dev/logs/diff-first.txt`), with how many files each, most first; `None`
/// without the log.
pub fn first_kinds(root: &Path) -> Option<Vec<(String, u64)>> {
  let text = fs::read_to_string(store::dir(root, store::LOGS).join("diff-first.txt")).ok()?;
  let mut counts: BTreeMap<String, u64> = BTreeMap::new();
  for line in text.lines() {
    if let Some((kind, _)) = line.split_once(" [") {
      *counts.entry(kind.to_string()).or_default() += 1;
    }
  }
  let mut ranked: Vec<(String, u64)> = counts.into_iter().collect();
  ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
  Some(ranked)
}

pub fn parse_baseline(text: &str) -> Baseline {
  let mut baseline = Baseline::default();
  for line in text.lines().filter(|l| !l.starts_with('#')) {
    let Some((key, value)) = line.split_once(' ') else {
      continue;
    };
    let value = value.trim();
    let speed = || value.split_whitespace().next().and_then(|v| v.parse().ok());
    match key {
      "machine" => baseline.machine = value.to_string(),
      "corpus" => baseline.corpus = value.to_string(),
      "lex" => baseline.lex = speed(),
      "lex+parse" => baseline.lex_parse = speed(),
      _ => {}
    }
  }
  baseline
}

/// The figures of records/archive/2026-10-optimization.txt: `NAME VALUE UNIT BETTER WHEN SOURCE`.
pub fn parse_before(text: &str) -> Vec<Before> {
  text
    .lines()
    .filter(|l| !l.starts_with('#') && !l.starts_with("note "))
    .filter_map(|line| {
      let words: Vec<&str> = line.split_whitespace().collect();
      match words.as_slice() {
        [name, value, unit, _, when, ..] => Some(Before {
          name: name.to_string(),
          value: value.to_string(),
          unit: unit.to_string(),
          when: when.to_string(),
        }),
        _ => None,
      }
    })
    .collect()
}

/// The compatibility record's text, read as `cargo xtask compare` reads it.
pub fn parse_compat(text: &str) -> Compat {
  let mut compat = Compat::default();
  for line in text.lines() {
    if let Some(reference) = line.strip_prefix("reference ") {
      compat.reference = reference.trim().to_string();
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let n = |s: &str| s.parse::<u64>().unwrap_or(0);
    let (name, tally) = match words.as_slice() {
      ["set", name, "files", mf, f, "lines", ml, l, rest @ ..] => (
        name,
        Tally {
          matched_files: n(mf),
          files: n(f),
          matched_lines: n(ml),
          lines: n(l),
          skipped: match rest {
            ["skipped", k] => Some(n(k)),
            _ => None,
          },
        },
      ),
      _ => continue,
    };
    compat.sets.insert(name.to_string(), tally);
  }
  compat
}

/// The entries of records/policy/exceptions.txt: `PATH [SET]` and the reason.
pub fn parse_exceptions(text: &str) -> Vec<Exception> {
  text
    .lines()
    .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    .filter_map(|line| {
      let (path, rest) = line.split_once(" [")?;
      let (set, reason) = rest.split_once(']')?;
      Some(Exception {
        path: path.to_string(),
        set: set.to_string(),
        reason: reason.trim().to_string(),
      })
    })
    .collect()
}

pub fn percent(part: u64, whole: u64) -> f64 {
  if whole == 0 {
    0.0
  } else {
    part as f64 * 100.0 / whole as f64
  }
}

/// The program's name taken off a version line: `1.9.0-stable (...)` from the record's
/// `reference` line, so that a page names the version and not the program.
pub fn reference_version(reference: &str) -> String {
  match reference.split_once(' ') {
    Some((_, version)) if version.starts_with(|c: char| c.is_ascii_digit()) => version.to_string(),
    _ => reference.to_string(),
  }
}

/// Each set's per-file scores from the last diff run (`.dev/logs/diff-scores.txt`): for
/// each, the files by the share of their answer's lines they keep, in [`SCORE_BANDS`]; and
/// the day the log was written. `None` without the log.
pub fn score_bands(root: &Path) -> Option<(String, BTreeMap<String, [u64; 4]>)> {
  let path = store::dir(root, store::LOGS).join("diff-scores.txt");
  let text = fs::read_to_string(&path).ok()?;
  let mut bands: BTreeMap<String, [u64; 4]> = BTreeMap::new();
  for line in text.lines() {
    let Some((key, score)) = line.rsplit_once(' ') else {
      continue;
    };
    let Some(set) = key.strip_prefix('[').and_then(|k| k.split_once(']')) else {
      continue;
    };
    let Some((kept, total)) = score.split_once('/') else {
      continue;
    };
    let (Ok(kept), Ok(total)) = (kept.parse::<u64>(), total.parse::<u64>()) else {
      continue;
    };
    let share = percent(kept, total);
    let band = if kept == total {
      0
    } else if share >= 99.0 {
      1
    } else if share >= 90.0 {
      2
    } else {
      3
    };
    bands.entry(set.0.to_string()).or_default()[band] += 1;
  }
  Some((day_written(&path).unwrap_or_default(), bands))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_exceptions_baselines_and_before_figures_parse() {
    let exceptions = parse_exceptions(
      "# header\n\ntests/a.rs [tests]  a reason: not modelled\nb/c d.rs [corpus]  why\n",
    );
    assert_eq!(exceptions.len(), 2);
    assert_eq!(exceptions[1].path, "b/c d.rs");
    assert_eq!(exceptions[1].set, "corpus");
    assert_eq!(exceptions[0].reason, "a reason: not modelled");
    let baseline =
      parse_baseline("# x\nmachine    macos aarch64, 10 cores\nlex        363.5 MB/s\n");
    assert_eq!(baseline.machine, "macos aarch64, 10 cores");
    assert_eq!(baseline.lex, Some(363.5));
    assert_eq!(baseline.lex_parse, None);
    let before = parse_before("# x\nspeed.a 120 MB/s higher 2026-10-11 [polled] bench\nnote n\n");
    assert_eq!(before.len(), 1);
    assert_eq!(
      (before[0].value.as_str(), before[0].when.as_str()),
      ("120", "2026-10-11")
    );
  }

  #[test]
  fn test_the_compatibility_record_parses_old_and_new() {
    let compat = parse_compat(
      "# note\nreference rustfmt 1.9.0-stable (88d9e12ae1 2026-08-18)\n\
       set tests files 3 4 lines 90 100 skipped 0\nset corpus files 5 10 lines 1 2\n",
    );
    assert_eq!(
      reference_version(&compat.reference),
      "1.9.0-stable (88d9e12ae1 2026-08-18)"
    );
    let tests = compat.sets["tests"];
    assert_eq!(
      (tests.matched_files, tests.files, tests.skipped),
      (3, 4, Some(0))
    );
    assert_eq!(tests.files_pct(), 75.0);
    assert_eq!(tests.lines_pct(), 90.0);
    assert_eq!(compat.sets["corpus"].skipped, None);
    assert_eq!(percent(1, 0), 0.0);
  }

  #[test]
  fn test_the_last_diff_logs_are_read_by_set_and_kind() {
    let root = std::env::temp_dir().join(format!("mtindo-papers-{}", std::process::id()));
    let logs = store::dir(&root, store::LOGS);
    assert_eq!(score_bands(&root), None);
    fs::create_dir_all(&logs).unwrap();
    fs::write(
      logs.join("diff-scores.txt"),
      "[tests] a.rs 10/10\n[tests] b c.rs 995/1000\n[corpus] d.rs 95/100\n\
       [corpus] e.rs 1/100\n",
    )
    .unwrap();
    fs::write(
      logs.join("diff-first.txt"),
      "Use [tests] b c.rs:3\nend of file [corpus] d.rs:9\nUse [corpus] e.rs:1\n",
    )
    .unwrap();
    let (day, bands) = score_bands(&root).unwrap();
    assert_eq!(day.len(), 10);
    assert_eq!(bands["tests"], [1, 1, 0, 0]);
    assert_eq!(bands["corpus"], [0, 0, 1, 1]);
    assert_eq!(
      first_kinds(&root).unwrap(),
      vec![("Use".to_string(), 2), ("end of file".to_string(), 1)]
    );
    let _ = fs::remove_dir_all(&root);
  }

  #[test]
  fn test_the_tracked_records_have_history() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let versions = file_history(root, "records/results/compatibility.txt").unwrap();
    assert!(!versions.is_empty());
    assert!(
      versions
        .iter()
        .all(|v| v.commit.len() == 7 && v.date.len() == 10)
    );
    let (fingerprint, repositories) = corpus_fingerprint(root).unwrap();
    assert!(fingerprint.starts_with("sha256:") && fingerprint.len() == 23);
    assert!(repositories > 0);
  }
}
