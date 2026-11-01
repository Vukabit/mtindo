//! `cargo xtask next`: what to do next, as one ranked queue drawn from what the commands
//! already keep, each with the command that acts on it. Most urgent first:
//!
//! 1. checks that failed when they last ran (`.dev/logs/last-test.tsv`);
//! 2. open incidents that fail the suite;
//! 3. a compatibility record not proven for this tree (`common::proof`);
//! 4. default-settings test pairs mto does not match and need not be excused;
//! 5. the settings with the most test pairs mto can match but does not (`crate::gaps`);
//! 6. ways mto and the standard formatter disagree on fuzzed inputs, by syntax kind, and
//!    the other open incidents (those that do not fail the suite), by kind;
//! 7. answer-key disagreements and upstream cases still to triage or draft.
//!
//! Nothing is run: a queue drawn from stale records says so where it can.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::common::json::Json;
use crate::common::log::Logger;
use crate::common::{proof, store, tree};
use crate::incidents;

/// One thing to do: its tier (lower first), what it is, and the command that acts on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
  pub tier: u8,
  pub what: String,
  pub then: String,
}

/// The whole queue as `mtindo.xtask.next/1`: each item's tier, what it is and the command.
pub fn json(items: &[Item]) -> Json {
  Json::document(
    "mtindo.xtask.next/1",
    vec![(
      "items",
      Json::List(
        items
          .iter()
          .map(|i| {
            Json::object(vec![
              ("tier", (i.tier as usize).into()),
              ("what", (&i.what).into()),
              ("then", (&i.then).into()),
            ])
          })
          .collect(),
      ),
    )],
  )
}

/// The queue, most urgent first.
pub fn queue(root: &Path, log: &Logger) -> Vec<Item> {
  let mut items = Vec::new();
  items.extend(failed_checks(root));
  items.extend(blocking_incidents(root));
  items.extend(unproven(root));
  items.extend(gaps(root, log));
  items.extend(mismatches(root));
  items.extend(other_incidents(root));
  items.extend(to_triage(root));
  items.sort_by_key(|i| i.tier);
  items
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "next");
  let items = queue(root, &log);
  if crate::cli::on(m, "json") {
    println!("{}", json(&items).render());
    return ExitCode::SUCCESS;
  }
  let all = crate::cli::on(m, "all");
  print!("{}", text(&items, if all { usize::MAX } else { 10 }));
  ExitCode::SUCCESS
}

pub fn text(items: &[Item], limit: usize) -> String {
  if items.is_empty() {
    return "Nothing is waiting: every check passed, no incident is open, the record is \
            proven and every test pair matches.\n"
      .to_string();
  }
  let mut out = String::new();
  for (n, item) in items.iter().take(limit).enumerate() {
    out.push_str(&format!(
      "{:>2}. {}\n      {}\n",
      n + 1,
      item.what,
      item.then
    ));
  }
  if items.len() > limit {
    out.push_str(&format!("\n... {} more (--all)\n", items.len() - limit));
  }
  out
}

fn blocking_incidents(root: &Path) -> Vec<Item> {
  let open: Vec<incidents::Incident> = incidents::all(root)
    .into_iter()
    .map(|(_, i)| i)
    .filter(|i| i.status == "open" && incidents::blocking(&i.kind))
    .collect();
  open
    .iter()
    .map(|i| Item {
      tier: 2,
      what: format!("open {} incident: {}", i.kind, first_line(&i.title, 70)),
      then: format!("cargo xtask incidents show {}", short(&i.id)),
    })
    .collect()
}

fn failed_checks(root: &Path) -> Vec<Item> {
  let text =
    fs::read_to_string(store::dir(root, store::LOGS).join("last-test.tsv")).unwrap_or_default();
  let here = tree::identity(root);
  let mut out = Vec::new();
  let mut ran_on = None;
  for line in text.lines().skip(1) {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 7 {
      continue;
    }
    ran_on.get_or_insert_with(|| f[5].to_string());
    if f[2] == "failed" {
      out.push(Item {
        tier: 1,
        what: format!("failing check: {}", f[6]),
        then: format!("cargo xtask test --target {}", f[0]),
      });
    }
  }
  if let Some(tree) = ran_on
    && tree != here
    && !out.is_empty()
  {
    for item in &mut out {
      item
        .what
        .push_str(&format!(" (as of {}; the tree has changed)", tree));
    }
  }
  out
}

fn first_line(text: &str, width: usize) -> String {
  let line = text.lines().next().unwrap_or("");
  if line.chars().count() > width {
    format!("{}...", line.chars().take(width).collect::<String>())
  } else {
    line.to_string()
  }
}

fn gaps(root: &Path, log: &Logger) -> Vec<Item> {
  let Ok(gaps) = crate::gaps::collect(root, log) else {
    return vec![Item {
      tier: 3,
      what: "no diff has been run here, so the gaps are unknown".to_string(),
      then: "cargo xtask compare".to_string(),
    }];
  };
  let mut out: Vec<Item> = gaps
    .gaps
    .iter()
    .filter(|g| g.set == "tests" && g.excused.is_none())
    .map(|g| Item {
      tier: 4,
      what: format!(
        "default-settings test pair not matching: {} ({}/{} lines)",
        g.name, g.matched, g.total
      ),
      then: format!("cargo xtask probe {} --show tests@stable", g.name),
    })
    .collect();
  // The settings with the most pairs mto can match now (not excused, needing nothing mto
  // lacks, and no answer key disagreeing on them).
  let mut by_setting: BTreeMap<&str, usize> = BTreeMap::new();
  for g in gaps
    .gaps
    .iter()
    .filter(|g| g.excused.is_none() && g.unhonored.is_empty() && g.disagreements.is_empty())
  {
    for s in &g.settings {
      if s != "edition" && s != "style_edition" && s != "max_width" {
        *by_setting.entry(s.as_str()).or_default() += 1;
      }
    }
  }
  let mut ranked: Vec<(&str, usize)> = by_setting.into_iter().collect();
  ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
  for (setting, n) in ranked.into_iter().take(5) {
    out.push(Item {
      tier: 5,
      what: format!(
        "{}: {} test pair(s) mto could match and does not",
        setting, n
      ),
      then: format!("cargo xtask gaps --setting {}", setting),
    });
  }
  out
}

fn mismatches(root: &Path) -> Vec<Item> {
  let mut by_kind: BTreeMap<String, Vec<String>> = BTreeMap::new();
  for (_, i) in incidents::all(root) {
    if i.kind != "mismatch" || i.status != "open" {
      continue;
    }
    let kind = i
      .title
      .split_once("disagree: ")
      .map(|(_, rest)| rest.split(" / ").next().unwrap_or(rest).to_string())
      .unwrap_or_else(|| "other".to_string());
    by_kind
      .entry(kind)
      .or_default()
      .push(short(&i.id).to_string());
  }
  let total: usize = by_kind.values().map(Vec::len).sum();
  if total == 0 {
    return Vec::new();
  }
  let mut ranked: Vec<(String, Vec<String>)> = by_kind.into_iter().collect();
  ranked.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
  let top: Vec<String> = ranked
    .iter()
    .take(4)
    .map(|(k, ids)| format!("{} {}", k, ids.len()))
    .collect();
  vec![Item {
    tier: 6,
    what: format!(
      "{} open way(s) mto and the standard formatter disagree on fuzzed input; most at {}",
      total,
      top.join(", ")
    ),
    then: format!(
      "cargo xtask incidents show {}, then `cargo xtask fuzz diff` rechecks them all",
      ranked[0].1[0]
    ),
  }]
}

/// The open incidents that neither fail the suite nor are fuzzed disagreements, a line per
/// kind.
fn other_incidents(root: &Path) -> Vec<Item> {
  let mut by_kind: BTreeMap<String, Vec<String>> = BTreeMap::new();
  for (_, i) in incidents::all(root) {
    if i.status == "open" && i.kind != "mismatch" && !incidents::blocking(&i.kind) {
      by_kind
        .entry(i.kind.clone())
        .or_default()
        .push(short(&i.id).to_string());
    }
  }
  by_kind
    .into_iter()
    .map(|(kind, ids)| Item {
      tier: 6,
      what: format!("{} open {} incident(s)", ids.len(), kind),
      then: format!(
        "cargo xtask incidents list --kind {} --status open; then show {}",
        kind, ids[0]
      ),
    })
    .collect()
}

fn short(id: &str) -> &str {
  id.rsplit('-').next().unwrap_or(id)
}

fn to_triage(root: &Path) -> Vec<Item> {
  let mut out = Vec::new();
  let record = fs::read_to_string(root.join(crate::probe::RECORD)).unwrap_or_default();
  let auto: Vec<&str> = record
    .lines()
    .filter(|l| !l.starts_with('#') && l.contains("auto-recorded, to triage"))
    .collect();
  if let Some(first) = auto.first() {
    out.push(Item {
      tier: 7,
      what: format!(
        "{} answer-key disagreement(s) to triage ({})",
        auto.len(),
        crate::probe::RECORD
      ),
      then: format!(
        "cargo xtask probe {} --show tests@nightly",
        first.split_whitespace().next().unwrap_or("CASE")
      ),
    });
  }
  let cases = store::dir(root, store::UPSTREAM).join("rustfmt");
  let drafts = store::dir(root, store::UPSTREAM).join("drafts");
  let mut undrafted: Vec<String> = fs::read_dir(&cases)
    .into_iter()
    .flatten()
    .flatten()
    .filter(|e| e.path().join("input.rs").is_file())
    .filter(|e| {
      fs::read_to_string(e.path().join("notes.md"))
        .is_ok_and(|n| n.lines().any(|l| l.starts_with("- Upstream: not found")))
    })
    .map(|e| e.file_name().to_string_lossy().into_owned())
    .filter(|name| !drafts.join(format!("{}.md", name)).is_file())
    .collect();
  undrafted.sort();
  if let Some(first) = undrafted.first() {
    out.push(Item {
      tier: 7,
      what: format!(
        "{} standard-formatter bug(s) not reported upstream and not drafted",
        undrafted.len()
      ),
      then: format!("cargo xtask upstream draft {}", first),
    });
  }
  out
}

fn unproven(root: &Path) -> Vec<Item> {
  let Some(hash) = proof::inputs(root, &proof::COMPAT) else {
    return Vec::new();
  };
  if proof::holds(root, &proof::COMPAT, &hash).is_some() {
    return Vec::new();
  }
  vec![Item {
    tier: 3,
    what: "the compatibility record is not proven for this tree (what the diff reads changed \
           since it last passed)"
      .to_string(),
    then: "cargo xtask compare".to_string(),
  }]
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_an_incident_id_is_shown_by_its_signature() {
    assert_eq!(short("20261030-223428-fe7ebcf4"), "fe7ebcf4");
    assert_eq!(first_line("a long title\nmore", 6), "a long...");
  }

  #[test]
  fn test_the_queue_is_numbered_most_urgent_first_and_cut() {
    let item = |tier, what: &str| Item {
      tier,
      what: what.to_string(),
      then: format!("do {}", what),
    };
    let mut items = vec![item(5, "gap"), item(1, "check"), item(3, "proof")];
    items.sort_by_key(|i| i.tier);
    let shown = text(&items, 2);
    assert!(
      shown.starts_with(" 1. check\n      do check\n 2. proof\n"),
      "{}",
      shown
    );
    assert!(shown.ends_with("... 1 more (--all)\n"), "{}", shown);
    assert!(text(&[], 10).starts_with("Nothing is waiting"));
  }
}
