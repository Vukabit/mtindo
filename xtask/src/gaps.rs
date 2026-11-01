//! `cargo xtask gaps`: the standard formatter's test pairs mto does not match yet, grouped
//! by the settings they set and by where they first differ, from what the last
//! `cargo xtask diff` wrote (nothing is run again):
//!
//! - `.dev/logs/diff-scores.txt`: each pair's matched lines, `[SET] NAME MATCHED/TOTAL`;
//! - `.dev/logs/diff-first.txt`: each mismatch's first differing line, `KIND [SET] NAME:LINE`;
//! - the suite at stable (`crate::sources`): each pair's header settings;
//! - `records/policy/exceptions.txt` (excused pairs) and the disagreements between answer
//!   keys (`records/policy/disagreements.txt`), so a pair mto need not match, or whose
//!   target the standard formatter itself does not produce, is marked as such.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::common::harness;
use crate::common::json::Json;
use crate::common::log::Logger;
use crate::common::store;

/// One test pair mto does not match.
#[derive(Clone, Debug, PartialEq)]
pub struct Gap {
  pub name: String,
  pub set: String,
  pub matched: usize,
  pub total: usize,
  /// The syntax kind and line where mto's output first differs.
  pub first: Option<(String, usize)>,
  /// The settings its header sets (with a `config` file's), by name.
  pub settings: Vec<String>,
  /// Those of its settings mto does not honor yet (`records/policy/capabilities.txt` has
  /// the option or its value `missing` or `not-planned`), as `NAME` or `NAME=VALUE`.
  pub unhonored: Vec<String>,
  /// Why mto need not match it, when excused.
  pub excused: Option<String>,
  /// The answer keys that disagree with the suite at stable on it.
  pub disagreements: Vec<String>,
}

/// The gaps, with how many pairs set each setting (failing or not), and when the diff that
/// measured them ran.
pub struct Gaps {
  pub gaps: Vec<Gap>,
  pub pairs_setting: BTreeMap<String, usize>,
  pub measured: Option<String>,
}

/// The gaps the last diff measured.
pub fn collect(root: &Path, log: &Logger) -> Result<Gaps, String> {
  let logs = store::dir(root, store::LOGS);
  let scores_path = logs.join("diff-scores.txt");
  let scores = fs::read_to_string(&scores_path).map_err(|_| {
    "no diff has been run here (.dev/logs/diff-scores.txt): run `cargo xtask diff`".to_string()
  })?;
  let measured = fs::metadata(&scores_path)
    .and_then(|m| m.modified())
    .ok()
    .map(|t| crate::common::log::timestamp(t)[..16].replace('T', " "));
  let first = parse_first(&fs::read_to_string(logs.join("diff-first.txt")).unwrap_or_default());
  let excused = parse_excused(
    &fs::read_to_string(root.join("records/policy/exceptions.txt")).unwrap_or_default(),
  );
  let capabilities = parse_capabilities(
    &fs::read_to_string(root.join("records/policy/capabilities.txt")).unwrap_or_default(),
  );
  let disagreements =
    parse_disagreements(&fs::read_to_string(root.join(crate::probe::RECORD)).unwrap_or_default());
  // The suite's headers; without it, the gaps are listed without their settings.
  let suite = crate::sources::tests_dir(root, "stable", log).ok();
  let settings_of = |name: &str| -> Vec<(String, String)> {
    let Some(suite) = &suite else {
      return Vec::new();
    };
    let relative = name.strip_prefix("tests/").unwrap_or(name);
    let text = fs::read_to_string(suite.join("source").join(relative))
      .or_else(|_| fs::read_to_string(suite.join("target").join(relative)))
      .unwrap_or_default();
    harness::settings(&text, &suite.join("config")).unwrap_or_default()
  };
  let mut gaps = Vec::new();
  let mut pairs_setting: BTreeMap<String, usize> = BTreeMap::new();
  for (set, name, matched, total) in parse_scores(&scores) {
    if !set.starts_with("tests") {
      continue;
    }
    let pairs = settings_of(&name);
    let unhonored = unhonored(&pairs, &capabilities);
    let settings: Vec<String> = pairs.into_iter().map(|(k, _)| k).collect();
    for s in &settings {
      *pairs_setting.entry(s.clone()).or_default() += 1;
    }
    if matched == total && !first.contains_key(&(set.clone(), name.clone())) {
      continue;
    }
    gaps.push(Gap {
      first: first.get(&(set.clone(), name.clone())).cloned(),
      excused: excused.get(&name).cloned(),
      disagreements: disagreements.get(&name).cloned().unwrap_or_default(),
      name,
      set,
      matched,
      total,
      settings,
      unhonored,
    });
  }
  Ok(Gaps {
    gaps,
    pairs_setting,
    measured,
  })
}

/// The gaps as `mtindo.xtask.gaps/1` (one setting's, when named): each pair with its
/// lines, where it first differs, its settings, those mto lacks, and why it is excused or
/// which keys disagree on it.
pub fn json(gaps: &Gaps, setting: Option<&str>) -> Json {
  let pairs: Vec<Json> = gaps
    .gaps
    .iter()
    .filter(|g| setting.is_none_or(|s| g.settings.iter().any(|x| x == s)))
    .map(|g| {
      Json::object(vec![
        ("name", (&g.name).into()),
        ("set", (&g.set).into()),
        ("matched", g.matched.into()),
        ("lines", g.total.into()),
        (
          "first",
          g.first.as_ref().map_or(Json::Null, |(kind, line)| {
            Json::object(vec![("kind", kind.into()), ("line", (*line).into())])
          }),
        ),
        ("settings", g.settings.clone().into()),
        ("unhonored", g.unhonored.clone().into()),
        ("excused", g.excused.clone().into()),
        ("disagreements", g.disagreements.clone().into()),
      ])
    })
    .collect();
  Json::document(
    "mtindo.xtask.gaps/1",
    vec![
      ("measured", gaps.measured.clone().into()),
      ("setting", setting.map(str::to_string).into()),
      ("pairs", Json::List(pairs)),
    ],
  )
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "gaps");
  let gaps = match collect(root, &log) {
    Ok(gaps) => gaps,
    Err(e) => {
      println!("{}", e);
      return ExitCode::from(1);
    }
  };
  let setting = crate::cli::text(m, "setting");
  if crate::cli::on(m, "json") {
    println!("{}", json(&gaps, setting.as_deref()).render());
    return ExitCode::SUCCESS;
  }
  let all = crate::cli::on(m, "all");
  print!("{}", text(&gaps, setting.as_deref(), all));
  ExitCode::SUCCESS
}

/// The report: by setting, then the default-settings pairs; or one setting's pairs.
pub fn text(gaps: &Gaps, setting: Option<&str>, all: bool) -> String {
  let mut out = format!(
    "test pairs mto does not match: {} ({} with default settings), from the diff of {}\n",
    gaps.gaps.len(),
    gaps.gaps.iter().filter(|g| g.set == "tests").count(),
    gaps.measured.as_deref().unwrap_or("an unknown time")
  );
  if let Some(setting) = setting {
    let mine: Vec<&Gap> = gaps
      .gaps
      .iter()
      .filter(|g| g.settings.iter().any(|s| s == setting))
      .collect();
    out.push_str(&format!(
      "\n{}: {} of the {} pairs setting it\n",
      setting,
      mine.len(),
      gaps.pairs_setting.get(setting).copied().unwrap_or(0)
    ));
    for g in mine {
      out.push_str(&pair_line(g));
    }
    return out;
  }
  // A pair that needs a setting mto does not honor yet is counted apart: it waits on that
  // setting, not on the setting being looked at.
  let workable = |g: &&Gap| g.excused.is_none() && g.unhonored.is_empty();
  let mut by_setting: Vec<(&String, usize, usize)> = gaps
    .pairs_setting
    .keys()
    .map(|s| {
      let mine = gaps.gaps.iter().filter(|g| g.settings.contains(s));
      let failing = mine.clone().filter(workable).count();
      let waiting = mine
        .filter(|g| g.excused.is_none() && !g.unhonored.is_empty())
        .count();
      (s, failing, waiting)
    })
    .filter(|(_, n, w)| n + w > 0)
    .collect();
  by_setting.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(b.0)));
  let shown = if all { by_setting.len() } else { 15 };
  out.push_str(
    "\nby setting: failing of the pairs setting it, and failing that need a setting mto \
     does not honor yet:\n",
  );
  for (s, failing, waiting) in by_setting.iter().take(shown) {
    out.push_str(&format!(
      "  {:<36} {:>4} of {:<4} {}\n",
      s,
      failing,
      gaps.pairs_setting.get(*s).copied().unwrap_or(0),
      if *waiting > 0 {
        format!("+{} waiting", waiting)
      } else {
        String::new()
      }
    ));
  }
  if by_setting.len() > shown {
    out.push_str(&format!(
      "  ... {} more (--all)\n",
      by_setting.len() - shown
    ));
  }
  let mut waits: BTreeMap<&str, usize> = BTreeMap::new();
  for g in gaps.gaps.iter().filter(|g| g.excused.is_none()) {
    for u in &g.unhonored {
      *waits.entry(u.as_str()).or_default() += 1;
    }
  }
  if !waits.is_empty() {
    let mut waits: Vec<(&str, usize)> = waits.into_iter().collect();
    waits.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    out.push_str("\nnot honored yet, by the failing pairs that need it:\n");
    for (u, n) in waits.iter().take(if all { waits.len() } else { 10 }) {
      out.push_str(&format!("  {:<36} {:>4}\n", u, n));
    }
  }
  let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
  for g in gaps.gaps.iter().filter(workable) {
    let kind = g
      .first
      .as_ref()
      .map_or("(no first line)", |(k, _)| k.as_str());
    *by_kind.entry(kind).or_default() += 1;
  }
  let mut kinds: Vec<(&str, usize)> = by_kind.into_iter().collect();
  kinds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
  out.push_str("\nby where mto first differs (pairs mto can match now):\n");
  for (kind, n) in kinds.iter().take(if all { kinds.len() } else { 10 }) {
    out.push_str(&format!("  {:<40} {:>4}\n", kind, n));
  }
  out.push_str("\nwith default settings:\n");
  for g in gaps.gaps.iter().filter(|g| g.set == "tests") {
    out.push_str(&pair_line(g));
  }
  out.push_str(
    "\none setting's pairs: cargo xtask gaps --setting NAME; one pair: cargo xtask probe NAME\n",
  );
  out
}

fn pair_line(g: &Gap) -> String {
  let mut line = format!(
    "  {:<52} {:>5}/{:<5} {}",
    g.name,
    g.matched,
    g.total,
    g.first
      .as_ref()
      .map_or(String::new(), |(kind, at)| format!("{} at {}", kind, at))
  );
  if g.excused.is_some() {
    line.push_str("  [excused]");
  }
  if !g.unhonored.is_empty() {
    line.push_str(&format!("  [needs {}]", g.unhonored.join(", ")));
  }
  if !g.disagreements.is_empty() {
    line.push_str(&format!(
      "  [keys disagree: {}]",
      g.disagreements.join(", ")
    ));
  }
  line.push('\n');
  line
}

/// The items of the capabilities record mto does not honor (`missing`, `not-planned`):
/// options by name and values as `NAME=VALUE`.
fn parse_capabilities(text: &str) -> BTreeMap<String, String> {
  text
    .lines()
    .filter_map(|line| {
      let words: Vec<&str> = line.split_whitespace().collect();
      match words.as_slice() {
        ["option" | "value", name, status, ..]
          if *status == "missing" || *status == "not-planned" =>
        {
          Some((name.to_string(), status.to_string()))
        }
        _ => None,
      }
    })
    .collect()
}

/// The disagreements record's keys, by pair.
fn parse_disagreements(text: &str) -> BTreeMap<String, Vec<String>> {
  let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
  for line in text.lines().filter(|l| !l.starts_with('#')) {
    let words: Vec<&str> = line.split_whitespace().collect();
    if let [case, _, "!=", key, ..] = words.as_slice() {
      out
        .entry(case.to_string())
        .or_default()
        .push(key.to_string());
    }
  }
  out
}

/// The `excused PATH [SET]  REASON` lines of the exceptions record, by path.
fn parse_excused(text: &str) -> BTreeMap<String, String> {
  text
    .lines()
    .filter_map(|line| line.strip_prefix("excused "))
    .filter_map(|rest| {
      let (path, rest) = rest.split_once(' ')?;
      let reason = rest.split_once("  ").map_or(rest, |(_, r)| r);
      Some((path.to_string(), reason.trim().to_string()))
    })
    .collect()
}

/// `KIND [SET] NAME:LINE` lines, by (set, name).
fn parse_first(text: &str) -> BTreeMap<(String, String), (String, usize)> {
  text
    .lines()
    .filter_map(|line| {
      let (kind, rest) = line.split_once(" [")?;
      let (set, place) = rest.split_once("] ")?;
      let (name, at) = place.rsplit_once(':')?;
      Some((
        (set.to_string(), name.to_string()),
        (kind.to_string(), at.parse().ok()?),
      ))
    })
    .collect()
}

/// `[SET] NAME MATCHED/TOTAL` lines.
fn parse_scores(text: &str) -> Vec<(String, String, usize, usize)> {
  text
    .lines()
    .filter_map(|line| {
      let (set, rest) = line.strip_prefix('[')?.split_once("] ")?;
      let (name, score) = rest.rsplit_once(' ')?;
      let (matched, total) = score.split_once('/')?;
      Some((
        set.to_string(),
        name.to_string(),
        matched.parse().ok()?,
        total.parse().ok()?,
      ))
    })
    .collect()
}

/// The settings of `pairs` mto does not honor: the value when it is one mto lacks, else
/// the option when mto lacks it whole.
fn unhonored(pairs: &[(String, String)], lacking: &BTreeMap<String, String>) -> Vec<String> {
  pairs
    .iter()
    .filter_map(|(k, v)| {
      let value = format!("{}={}", k, v.trim_matches('"'));
      if lacking.contains_key(&value) {
        Some(value)
      } else {
        lacking.contains_key(k).then(|| k.clone())
      }
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_gaps_group_by_setting_with_excused_pairs_aside() {
    let gap = |name: &str, set: &str, settings: &[&str], excused: bool| Gap {
      name: name.to_string(),
      set: set.to_string(),
      matched: 1,
      total: 2,
      first: Some(("CallExpr".to_string(), 3)),
      settings: settings.iter().map(|s| s.to_string()).collect(),
      unhonored: Vec::new(),
      excused: excused.then(|| "why".to_string()),
      disagreements: Vec::new(),
    };
    let gaps = Gaps {
      gaps: vec![
        gap(
          "tests/a.rs",
          "tests-with-settings",
          &["wrap_comments"],
          false,
        ),
        gap(
          "tests/b.rs",
          "tests-with-settings",
          &["wrap_comments", "max_width"],
          false,
        ),
        gap("tests/c.rs", "tests", &[], true),
      ],
      pairs_setting: [
        ("wrap_comments".to_string(), 5),
        ("max_width".to_string(), 9),
      ]
      .into_iter()
      .collect(),
      measured: Some("2026-11-01 12:00".to_string()),
    };
    let shown = text(&gaps, None, false);
    assert!(
      shown.contains("wrap_comments                           2 of 5"),
      "{}",
      shown
    );
    assert!(
      shown.contains("tests/c.rs") && shown.contains("[excused]"),
      "{}",
      shown
    );
    let one = text(&gaps, Some("max_width"), false);
    assert!(
      one.contains("max_width: 1 of the 9 pairs setting it"),
      "{}",
      one
    );
    assert!(!one.contains("tests/a.rs"));
  }

  #[test]
  fn test_the_diff_logs_are_read() {
    assert_eq!(
      parse_scores("[tests-with-settings] tests/a b.rs 12/13\n[corpus] x/y.rs 4/4\n"),
      vec![
        (
          "tests-with-settings".to_string(),
          "tests/a b.rs".to_string(),
          12,
          13
        ),
        ("corpus".to_string(), "x/y.rs".to_string(), 4, 4),
      ]
    );
    let first = parse_first("UseTree [tests-with-settings] tests/5131_one.rs:7\n");
    assert_eq!(
      first.get(&(
        "tests-with-settings".to_string(),
        "tests/5131_one.rs".to_string()
      )),
      Some(&("UseTree".to_string(), 7))
    );
    let excused =
      parse_excused("tests/x.rs [tests]  not this\nexcused tests/y.rs [tests]  the reason\n");
    assert_eq!(excused.len(), 1);
    assert_eq!(excused["tests/y.rs"], "the reason");
    let lacking = parse_capabilities(
      "option  file_lines  not-planned  # a flag\nvalue   indent_style=Visual  missing\nvalue   indent_style=Block  supported\n",
    );
    let pairs = |list: &[(&str, &str)]| -> Vec<(String, String)> {
      list
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    };
    assert_eq!(
      unhonored(
        &pairs(&[
          ("indent_style", "Visual"),
          ("file_lines", "[]"),
          ("max_width", "80")
        ]),
        &lacking
      ),
      vec!["indent_style=Visual", "file_lines"]
    );
    assert!(unhonored(&pairs(&[("indent_style", "Block")]), &lacking).is_empty());
    let keys = parse_disagreements("# c\ntests/u.rs  tests@stable != run@nightly  why\n");
    assert_eq!(keys["tests/u.rs"], vec!["run@nightly"]);
  }
}
