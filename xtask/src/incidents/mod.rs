//! Incidents: every failure worth fixing, kept with its whole story and filed once.
//!
//! An incident is a folder under `.dev/incidents/<kind>/<UTC>-<signature>/`:
//!
//! ```text
//! incident.toml   kind, target, signature, status, when seen, where (`commit` or
//!                 `commit-dirty.<digest>`), occurrences, replay, history, fix
//! said.txt        what the failure printed when first seen
//! input, input-2  the inputs that showed it (one per distinct input seen)
//! minimized       the smallest input that still shows it, once minimized
//! notes.md        yours; never written by any command
//! README.md       the incident as a page, generated
//! ```
//!
//! and `.dev/incidents/INCIDENTS.md` is the board: every incident, open ones first.
//!
//! The signature (kind, target, the panic's location, the rules a check named and the
//! message's first line, with numbers taken out) decides when two failures are one: a
//! failure seen again becomes an occurrence of its incident, and one seen again after being
//! marked fixed reopens it. Statuses: `open`, `flaky` (fails on some replays), `fixed`,
//! `wont-fix`.

pub mod command;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use command::run;

use crate::common::log::timestamp;
use crate::sha256;

/// Every kind: (name, what it is, whether an open one fails the suite).
pub const KINDS: &[(&str, &str, bool)] = &[
  (
    "fuzz-crash",
    "a fuzz target's check failed or it panicked",
    true,
  ),
  ("fuzz-oom", "a fuzz target passed its memory limit", true),
  (
    "fuzz-timeout",
    "a fuzz target took too long on one input",
    true,
  ),
  ("fuzz-slow", "a fuzz target was slow on one input", true),
  ("fuzz-leak", "a fuzz target leaked memory", true),
  ("meaning", "mto's output fails the meaning check", true),
  ("unstable", "formatting mto's output again changes it", true),
  ("panic", "mto panicked", true),
  (
    "compat-break",
    "the standard formatter leaves a file as written and mto changes it",
    false,
  ),
  (
    "compile-break",
    "a real crate compiles before mto and not after",
    false,
  ),
  (
    "speed-regression",
    "a speed figure fell below its record",
    false,
  ),
  (
    "memory-regression",
    "a memory figure rose above its record",
    false,
  ),
  (
    "mismatch",
    "mto and the standard formatter format an input differently (differential fuzzing)",
    false,
  ),
  (
    "doctor-regression",
    "the build guard takes more checks, costs more, or holds back more than its record",
    false,
  ),
];

pub const STATUSES: &[&str] = &["open", "flaky", "fixed", "wont-fix"];

/// The kinds `cargo xtask diff` finds, and so rechecks.
const DIFF_KINDS: &[&str] = &["meaning", "unstable", "panic", "compat-break"];

/// What [`file`] did.
#[derive(Debug, PartialEq)]
pub enum Filed {
  New(String),
  Again(String),
  /// Seen again after being marked fixed: reopened.
  Reopened(String),
}

impl Filed {
  pub fn id(&self) -> &str {
    match self {
      Filed::New(id) | Filed::Again(id) | Filed::Reopened(id) => id,
    }
  }
}

/// One check of whether an incident still fails.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Check {
  pub when: String,
  pub tree: String,
  /// `fails`, `no longer fails`, `replay did not run`, a status change, or a note.
  pub result: String,
}

/// What [`from_diff`] did.
#[derive(Debug, Default, PartialEq)]
pub struct DiffCounts {
  pub new: usize,
  pub again: usize,
  pub reopened: usize,
  pub fixed: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Incident {
  /// `<kind>/<UTC>-<signature 8>`, its folder under `.dev/incidents/`.
  pub id: String,
  pub kind: String,
  /// The fuzz target, input set, crate or figure it concerns.
  pub target: String,
  pub signature: String,
  /// One line: what went wrong.
  pub title: String,
  /// The source location the failure points at, when it names one.
  pub location: String,
  pub status: String,
  pub occurrences: Vec<Occurrence>,
  /// How to see it again, as a command line.
  pub replay: String,
  pub history: Vec<Check>,
  /// The tree in which it was first seen not failing, once fixed.
  pub fixed_in: String,
  /// The test that guards the fix, once promoted.
  pub regression_test: String,
}

impl Incident {
  pub fn to_toml(&self) -> String {
    let mut out = String::new();
    for (key, value) in [
      ("id", &self.id),
      ("kind", &self.kind),
      ("target", &self.target),
      ("signature", &self.signature),
      ("title", &self.title),
      ("location", &self.location),
      ("status", &self.status),
      ("replay", &self.replay),
      ("fixed_in", &self.fixed_in),
      ("regression_test", &self.regression_test),
    ] {
      out.push_str(&format!("{} = {}\n", key, quote(value)));
    }
    for o in &self.occurrences {
      out.push_str(&format!(
        "\n[[occurrence]]\nwhen = {}\ntree = {}\nrun = {}\ninput = {}\n",
        quote(&o.when),
        quote(&o.tree),
        quote(&o.run),
        quote(&o.input)
      ));
    }
    for c in &self.history {
      out.push_str(&format!(
        "\n[[history]]\nwhen = {}\ntree = {}\nresult = {}\n",
        quote(&c.when),
        quote(&c.tree),
        quote(&c.result)
      ));
    }
    out
  }

  /// Reads what [`Incident::to_toml`] writes.
  pub fn from_toml(text: &str) -> Option<Incident> {
    let mut incident = Incident::default();
    let mut table = "";
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
      match line {
        "[[occurrence]]" => {
          table = "occurrence";
          incident.occurrences.push(Occurrence::default());
          continue;
        }
        "[[history]]" => {
          table = "history";
          incident.history.push(Check::default());
          continue;
        }
        _ => {}
      }
      let (key, value) = line.split_once(" = ")?;
      let value = unquote(value)?;
      match (table, key) {
        ("occurrence", key) => {
          let o = incident.occurrences.last_mut()?;
          match key {
            "when" => o.when = value,
            "tree" => o.tree = value,
            "run" => o.run = value,
            "input" => o.input = value,
            _ => {}
          }
        }
        ("history", key) => {
          let c = incident.history.last_mut()?;
          match key {
            "when" => c.when = value,
            "tree" => c.tree = value,
            "result" => c.result = value,
            _ => {}
          }
        }
        (_, "id") => incident.id = value,
        (_, "kind") => incident.kind = value,
        (_, "target") => incident.target = value,
        (_, "signature") => incident.signature = value,
        (_, "title") => incident.title = value,
        (_, "location") => incident.location = value,
        (_, "status") => incident.status = value,
        (_, "replay") => incident.replay = value,
        (_, "fixed_in") => incident.fixed_in = value,
        (_, "regression_test") => incident.regression_test = value,
        _ => {}
      }
    }
    Some(incident)
  }
}

/// One time an incident was seen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Occurrence {
  pub when: String,
  pub tree: String,
  /// The run that saw it, if one was recorded.
  pub run: String,
  /// Which input file of the incident showed it.
  pub input: String,
}

/// A failure to file.
pub struct Report<'a> {
  pub kind: &'a str,
  pub target: &'a str,
  /// What the failure printed.
  pub said: &'a str,
  /// The input that showed it, if there is one.
  pub input: Option<&'a [u8]>,
  pub replay: &'a str,
  pub tree: &'a str,
  pub run: &'a str,
  /// Overrides the signature's text: a stable name for the failure (a file, a crate)
  /// when the output is not stable enough to recognize it by.
  pub key: Option<&'a str>,
}

/// Every incident: (folder, record), oldest first.
pub fn all(root: &Path) -> Vec<(PathBuf, Incident)> {
  let mut out = Vec::new();
  let Ok(kinds) = fs::read_dir(root_dir(root)) else {
    return out;
  };
  for kind in kinds.flatten().filter(|e| e.path().is_dir()) {
    let Ok(entries) = fs::read_dir(kind.path()) else {
      continue;
    };
    for entry in entries.flatten() {
      if let Ok(text) = fs::read_to_string(entry.path().join("incident.toml"))
        && let Some(incident) = Incident::from_toml(&text)
      {
        out.push((entry.path(), incident));
      }
    }
  }
  out.sort_by(|a, b| a.1.id.rsplit('/').next().cmp(&b.1.id.rsplit('/').next()));
  out
}

/// Whether an open incident of `kind` fails the suite.
pub fn blocking(kind: &str) -> bool {
  KINDS.iter().any(|(k, _, fails)| *k == kind && *fails)
}

/// The open incidents that fail the suite: what the `incidents` gate reports.
pub fn check_gate(root: &Path) -> Vec<String> {
  all(root)
    .into_iter()
    .filter(|(_, i)| (i.status == "open" || i.status == "flaky") && blocking(&i.kind))
    .map(|(_, i)| {
      format!(
        "{} {} {}: {} (fix it, or mark it: cargo xtask incidents mark {} wont-fix)",
        i.status,
        i.kind,
        short(&i),
        i.title,
        short(&i)
      )
    })
    .collect()
}

/// Files a failure: a new incident, another occurrence of the one it matches, or that one
/// reopened if it had been marked fixed.
pub fn file(root: &Path, report: &Report) -> std::io::Result<Filed> {
  let (location, recognized) = recognize(report.said);
  let key = report.key.map(str::to_string).unwrap_or(recognized);
  let signature = signature(report.kind, report.target, &key);
  let now = timestamp(SystemTime::now());
  if let Some((dir, mut incident)) = all(root)
    .into_iter()
    .find(|(_, i)| i.signature == signature)
  {
    let input = match report.input {
      Some(bytes) => keep_input(&dir, bytes)?,
      None => String::new(),
    };
    incident.occurrences.push(Occurrence {
      when: now.clone(),
      tree: report.tree.to_string(),
      run: report.run.to_string(),
      input,
    });
    let reopened = incident.status == "fixed";
    if reopened {
      incident.status = "open".to_string();
      incident.history.push(Check {
        when: now,
        tree: report.tree.to_string(),
        result: format!(
          "seen again after being fixed in {}: reopened",
          incident.fixed_in
        ),
      });
      incident.fixed_in.clear();
    }
    save(root, &dir, &incident)?;
    return Ok(if reopened {
      Filed::Reopened(incident.id)
    } else {
      Filed::Again(incident.id)
    });
  }
  let stamp: String = now
    .chars()
    .take(19)
    .filter(|c| c.is_ascii_digit() || *c == 'T')
    .map(|c| if c == 'T' { '-' } else { c })
    .collect();
  let id = format!("{}/{}-{}", report.kind, stamp, &signature[..8]);
  let dir = root_dir(root).join(&id);
  fs::create_dir_all(&dir)?;
  fs::write(dir.join("said.txt"), report.said)?;
  let input = match report.input {
    Some(bytes) => keep_input(&dir, bytes)?,
    None => String::new(),
  };
  let incident = Incident {
    id: id.clone(),
    kind: report.kind.to_string(),
    target: report.target.to_string(),
    signature,
    title: title_of(report.kind, report.said),
    location,
    status: "open".to_string(),
    occurrences: vec![Occurrence {
      when: now,
      tree: report.tree.to_string(),
      run: report.run.to_string(),
      input,
    }],
    replay: report.replay.to_string(),
    ..Incident::default()
  };
  save(root, &dir, &incident)?;
  Ok(Filed::New(id))
}

/// The incident whose id is `id`, ends with it, or whose signature starts with it.
pub fn find(root: &Path, id: &str) -> Option<(PathBuf, Incident)> {
  all(root)
    .into_iter()
    .rev()
    .find(|(_, i)| i.id == id || i.id.ends_with(id) || i.signature.starts_with(id))
}

/// Files a diff run's findings (`diff-findings.tsv`, inputs in `inputs`) as incidents, one
/// per kind and file; after a complete run, marks fixed the diff incidents it no longer
/// found.
pub fn from_diff(root: &Path, findings: &str, inputs: &Path) -> DiffCounts {
  let mut counts = DiffCounts::default();
  let mut lines = findings.lines();
  let complete = lines.next() == Some("complete\ttrue");
  let tree = crate::common::tree::identity(root);
  let run = std::env::var(crate::runs::RUN_DIR)
    .ok()
    .and_then(|d| {
      Path::new(&d)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    })
    .unwrap_or_default();
  let mut seen = Vec::new();
  for line in lines {
    let f: Vec<&str> = line.split('\t').collect();
    let [kind, set, name, saved, message] = f.as_slice() else {
      continue;
    };
    let input = fs::read(inputs.join(saved)).ok();
    let said = format!("{}: {} [{}]\n", name, message, set);
    let report = Report {
      kind,
      target: set,
      said: &said,
      input: input.as_deref(),
      replay: "cargo xtask diff",
      tree: &tree,
      run: &run,
      key: Some(name),
    };
    match file(root, &report) {
      Ok(Filed::New(id)) => {
        counts.new += 1;
        seen.push(id);
      }
      Ok(Filed::Again(id)) => {
        counts.again += 1;
        seen.push(id);
      }
      Ok(Filed::Reopened(id)) => {
        counts.reopened += 1;
        seen.push(id);
      }
      Err(_) => {}
    }
  }
  if complete {
    for (dir, mut incident) in all(root) {
      let open = incident.status == "open" || incident.status == "flaky";
      if open && DIFF_KINDS.contains(&incident.kind.as_str()) && !seen.contains(&incident.id) {
        mark(root, &dir, &mut incident, "fixed", &tree);
        counts.fixed += 1;
      }
    }
  }
  counts
}

/// Files the compile breaks a `domain_13_rw` run reported (`<crate> (<settings>): failed
/// post-format compilation`, or mto exiting with an error), or, when it passed, marks the
/// open ones fixed.
pub fn from_real_world(root: &Path, passed: bool, output: &str) -> usize {
  let tree = crate::common::tree::identity(root);
  if passed {
    let mut fixed = 0;
    for (dir, mut incident) in all(root) {
      if incident.kind == "compile-break" && incident.status == "open" {
        mark(root, &dir, &mut incident, "fixed", &tree);
        fixed += 1;
      }
    }
    return fixed;
  }
  let mut filed = 0;
  for line in output.lines().map(str::trim) {
    let broken =
      line.contains("failed post-format compilation") || line.contains("mto exited with");
    let Some((which, _)) = line.split_once("): ") else {
      continue;
    };
    if !broken {
      continue;
    }
    let crate_name = which.split(" (").next().unwrap_or(which);
    let report = Report {
      kind: "compile-break",
      target: crate_name,
      said: line,
      input: None,
      replay: "cargo xtask test --target domain_13_rw",
      tree: &tree,
      run: "",
      key: Some(which),
    };
    if file(root, &report).is_ok() {
      filed += 1;
    }
  }
  filed
}

/// Sets an incident's status, with the tree it was set at.
pub fn mark(root: &Path, dir: &Path, incident: &mut Incident, status: &str, tree: &str) {
  if status == "fixed" && incident.fixed_in.is_empty() {
    incident.fixed_in = tree.to_string();
  }
  if status == "open" {
    incident.fixed_in.clear();
  }
  let from = std::mem::replace(&mut incident.status, status.to_string());
  note(
    root,
    dir,
    incident,
    tree,
    &format!("status {} -> {}", from, status),
  );
}

/// Adds a line to an incident's history.
pub fn note(root: &Path, dir: &Path, incident: &mut Incident, tree: &str, result: &str) {
  incident.history.push(Check {
    when: timestamp(SystemTime::now()),
    tree: tree.to_string(),
    result: result.to_string(),
  });
  let _ = save(root, dir, incident);
}

/// The location a panic names (`src/format/stmt.rs:312`), the rules a check named, and
/// the message's first line with numbers taken out: what makes two failures one.
pub fn recognize(said: &str) -> (String, String) {
  let lines: Vec<&str> = said.lines().collect();
  let panic = lines.iter().position(|l| l.contains("panicked at "));
  let location = panic
    .and_then(|i| lines[i].split("panicked at ").nth(1))
    .map(|rest| {
      let rest = rest.trim_end_matches(':');
      // `file:line:column` without the column.
      match rest.rsplit_once(':') {
        Some((head, column)) if column.chars().all(|c| c.is_ascii_digit()) => head.to_string(),
        _ => rest.to_string(),
      }
    })
    .unwrap_or_default();
  let message = panic
    .and_then(|i| lines.get(i + 1))
    .or_else(|| lines.iter().find(|l| l.contains("ERROR: libFuzzer")))
    .or_else(|| lines.iter().find(|l| !l.trim().is_empty()))
    .map(|l| l.trim().to_string())
    .unwrap_or_default();
  let mut rules: Vec<&str> = said
    .split("rule: ")
    .skip(1)
    .filter_map(|r| r.split([',', ' ', '}']).next())
    .collect();
  rules.sort();
  rules.dedup();
  let general: String = message
    .chars()
    .map(|c| if c.is_ascii_digit() { '#' } else { c })
    .take(120)
    .collect();
  let key = format!("{}|{}|{}", location, rules.join(","), general);
  (location, key)
}

pub fn root_dir(root: &Path) -> PathBuf {
  root.join(".dev").join("incidents")
}

/// Writes an incident's record and page, and the board.
pub fn save(root: &Path, dir: &Path, incident: &Incident) -> std::io::Result<()> {
  write_atomic(&dir.join("incident.toml"), &incident.to_toml())?;
  write_atomic(&dir.join("README.md"), &page(dir, incident))?;
  write_board(root)
}

/// An incident's short id: its signature's first 8 characters.
pub fn short(incident: &Incident) -> &str {
  &incident.signature[..8.min(incident.signature.len())]
}

/// The board, `.dev/incidents/INCIDENTS.md`: open and flaky first, then the rest, newest
/// first in each.
pub fn write_board(root: &Path) -> std::io::Result<()> {
  let mut incidents = all(root);
  incidents.reverse();
  let rank = |s: &str| match s {
    "open" => 0,
    "flaky" => 1,
    "fixed" => 2,
    _ => 3,
  };
  incidents.sort_by_key(|(_, i)| rank(&i.status));
  let count = |s: &str| incidents.iter().filter(|(_, i)| i.status == s).count();
  let mut out = format!(
    "# Incidents\n\nGenerated by `cargo xtask incidents`; each folder's `notes.md` is yours.\n\n\
     {} open, {} flaky, {} fixed, {} won't fix.\n\n",
    count("open"),
    count("flaky"),
    count("fixed"),
    count("wont-fix")
  );
  out.push_str("| Status | Kind | Target | What it said | Seen | Last seen | Guarded by | Id |\n");
  out.push_str("|---|---|---|---|---|---|---|---|\n");
  for (_, i) in &incidents {
    let last = i
      .occurrences
      .last()
      .map_or("", |o| o.when.get(..16).unwrap_or(""));
    out.push_str(&format!(
      "| {} | {} | {} | {} | {} | {} | {} | [{}]({}/README.md) |\n",
      i.status,
      i.kind,
      cell(&i.target),
      cell(&i.title),
      i.occurrences.len(),
      last,
      cell(&i.regression_test),
      short(i),
      i.id
    ));
  }
  let dir = root_dir(root);
  fs::create_dir_all(&dir)?;
  write_atomic(&dir.join("INCIDENTS.md"), &out)
}

/// Text fit for a table cell.
fn cell(text: &str) -> String {
  text.replace('|', "\\|").replace('\n', " ")
}

/// Keeps `bytes` as one of the incident's inputs, unless an input already holds them; its
/// file name.
fn keep_input(dir: &Path, bytes: &[u8]) -> std::io::Result<String> {
  for n in 1.. {
    let name = if n == 1 {
      "input".to_string()
    } else {
      format!("input-{}", n)
    };
    let path = dir.join(&name);
    match fs::read(&path) {
      Ok(existing) if existing == bytes => return Ok(name),
      Ok(_) => continue,
      Err(_) => {
        fs::write(&path, bytes)?;
        return Ok(name);
      }
    }
  }
  unreachable!()
}

/// An incident as a page.
fn page(dir: &Path, i: &Incident) -> String {
  let said = fs::read_to_string(dir.join("said.txt")).unwrap_or_default();
  let mut out = format!("# {} {}: {}\n\n", i.kind, short(i), i.title);
  out.push_str(&format!("- **Status:** {}\n", i.status));
  out.push_str(&format!("- **Target:** {}\n", i.target));
  if !i.location.is_empty() {
    out.push_str(&format!("- **Where:** `{}`\n", i.location));
  }
  if let Some(first) = i.occurrences.first() {
    out.push_str(&format!(
      "- **First seen:** {} at `{}`\n",
      first.when, first.tree
    ));
  }
  out.push_str(&format!("- **Seen:** {} time(s)\n", i.occurrences.len()));
  if !i.replay.is_empty() {
    out.push_str(&format!("- **Replay:** `{}`\n", i.replay));
  }
  if !i.fixed_in.is_empty() {
    out.push_str(&format!("- **Fixed in:** `{}`\n", i.fixed_in));
  }
  if !i.regression_test.is_empty() {
    out.push_str(&format!("- **Guarded by:** `{}`\n", i.regression_test));
  }
  out.push_str("\n## What it said\n\n```text\n");
  out.push_str(said.trim_end());
  out.push_str("\n```\n\n## Occurrences\n\n| When | Tree | Run | Input |\n|---|---|---|---|\n");
  for o in &i.occurrences {
    out.push_str(&format!(
      "| {} | {} | {} | {} |\n",
      o.when, o.tree, o.run, o.input
    ));
  }
  if !i.history.is_empty() {
    out.push_str("\n## History\n\n| When | Tree | Result |\n|---|---|---|\n");
    for c in &i.history {
      out.push_str(&format!(
        "| {} | {} | {} |\n",
        c.when,
        c.tree,
        cell(&c.result)
      ));
    }
  }
  if let Ok(notes) = fs::read_to_string(dir.join("notes.md")) {
    out.push_str("\n## Notes\n\n");
    out.push_str(notes.trim_end());
    out.push('\n');
  }
  out
}

fn quote(s: &str) -> String {
  let mut out = String::from("\"");
  for c in s.chars() {
    match c {
      '"' => out.push_str("\\\""),
      '\\' => out.push_str("\\\\"),
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
      c => out.push(c),
    }
  }
  out.push('"');
  out
}

fn signature(kind: &str, target: &str, key: &str) -> String {
  sha256::hex_digest(format!("{}\0{}\0{}", kind, target, key).as_bytes())[..16].to_string()
}

/// The incident's title: the panic's message, libFuzzer's error, or the first line that
/// is not a note in parentheses; else what its kind means.
fn title_of(kind: &str, said: &str) -> String {
  let lines: Vec<&str> = said.lines().collect();
  let line = lines
    .iter()
    .position(|l| l.contains("panicked at "))
    .and_then(|i| lines.get(i + 1))
    .or_else(|| lines.iter().find(|l| l.contains("ERROR: libFuzzer")))
    .or_else(|| {
      lines
        .iter()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('('))
    })
    .map(|l| l.trim().to_string())
    .unwrap_or_else(|| {
      KINDS
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map_or(String::new(), |(_, meaning, _)| meaning.to_string())
    });
  line.chars().take(100).collect()
}

/// Reads a string `quote` wrote.
fn unquote(value: &str) -> Option<String> {
  let inner = value.strip_prefix('"')?.strip_suffix('"')?;
  let mut out = String::new();
  let mut chars = inner.chars();
  while let Some(c) = chars.next() {
    if c != '\\' {
      out.push(c);
      continue;
    }
    match chars.next()? {
      'n' => out.push('\n'),
      'r' => out.push('\r'),
      't' => out.push('\t'),
      'u' => {
        let hex: String = chars.by_ref().take(4).collect();
        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
      }
      c => out.push(c),
    }
  }
  Some(out)
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
  let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
  fs::write(&temporary, text)?;
  fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
  use super::*;

  const SAID: &str = "#4 NEW cov: 9\n\
    thread '<unnamed>' panicked at src/fuzzing/format.rs:36:3:\n\
    meaning changed:\nfn f() {}\n[Violation { rule: MeaningChanged, detail: \"x\" }]\n";

  /// A scratch repository root, removed when dropped.
  struct Scratch(PathBuf);

  impl Scratch {
    fn new(name: &str) -> Scratch {
      let dir =
        std::env::temp_dir().join(format!("xtask-incidents-{}-{}", name, std::process::id()));
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

  fn report<'a>(said: &'a str, input: &'a [u8]) -> Report<'a> {
    Report {
      kind: "fuzz-crash",
      target: "format",
      said,
      input: Some(input),
      replay: "cargo xtask incidents replay",
      tree: "abc1234",
      run: "run-1",
      key: None,
    }
  }

  #[test]
  fn test_a_failure_is_recognized_by_where_and_what() {
    let (location, key) = recognize(SAID);
    assert_eq!(location, "src/fuzzing/format.rs:36");
    assert!(key.contains("MeaningChanged"), "{}", key);
    assert!(key.ends_with("meaning changed:"), "{}", key);
    let other = SAID.replace(":36:3", ":36:9").replace("cov: 9", "cov: 11");
    assert_eq!(
      recognize(&other).1,
      key,
      "the column and the run's figures do not count"
    );
  }

  #[test]
  fn test_a_record_round_trips() {
    let incident = Incident {
      id: "fuzz-crash/20261011-183101-e88529f2".to_string(),
      kind: "fuzz-crash".to_string(),
      target: "format".to_string(),
      signature: "e88529f2aabbccdd".to_string(),
      title: "say \"hi\"\tback\\slash\nnext \u{1}".to_string(),
      location: "src/a.rs:1".to_string(),
      status: "open".to_string(),
      occurrences: vec![Occurrence {
        when: "w".into(),
        tree: "t".into(),
        run: "r".into(),
        input: "input".into(),
      }],
      replay: "cargo xtask incidents replay e885".to_string(),
      history: vec![Check {
        when: "w2".into(),
        tree: "t2".into(),
        result: "fails".into(),
      }],
      fixed_in: String::new(),
      regression_test: "tests/fuzz_regressions/format/e88529f2".to_string(),
    };
    assert_eq!(Incident::from_toml(&incident.to_toml()), Some(incident));
  }

  #[test]
  fn test_compile_breaks_are_filed_and_fixed() {
    let scratch = Scratch::new("realworld");
    let root = &scratch.0;
    let output = "2 target(s) failed:\nserde (mix one): failed post-format compilation\n\
                  clap (defaults): mto exited with Some(4): verifier\nthread 'main' panicked\n";
    assert_eq!(from_real_world(root, false, output), 2);
    let breaks = all(root);
    assert_eq!(breaks.len(), 2);
    assert!(breaks.iter().all(|(_, i)| i.kind == "compile-break"));
    assert!(breaks.iter().any(|(_, i)| i.target == "serde"));
    assert_eq!(from_real_world(root, true, ""), 2);
    assert!(all(root).iter().all(|(_, i)| i.status == "fixed"));
  }

  #[test]
  fn test_diff_findings_are_filed_and_fixed_when_gone() {
    let scratch = Scratch::new("diff");
    let root = &scratch.0;
    let inputs = root.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    fs::write(inputs.join("corpus-a.input"), "fn a() {}").unwrap();
    let findings = "complete\ttrue\n\
      meaning\tcorpus\tserde/a.rs\tcorpus-a.input\tcomment-lost: x\n\
      compat-break\ttests\tissue-1.rs\tmissing.input\tleft as written\n";
    let first = from_diff(root, findings, &inputs);
    assert_eq!((first.new, first.fixed), (2, 0));
    let meaning = all(root)
      .into_iter()
      .find(|(_, i)| i.kind == "meaning")
      .unwrap();
    assert_eq!(fs::read(meaning.0.join("input")).unwrap(), b"fn a() {}");
    assert_eq!(meaning.1.target, "corpus");
    // The meaning finding is gone from a complete run: fixed. An incomplete run fixes nothing.
    let later = "complete\ttrue\ncompat-break\ttests\tissue-1.rs\tmissing.input\tleft as written\n";
    let incomplete = from_diff(root, &later.replace("true", "false"), &inputs);
    assert_eq!((incomplete.again, incomplete.fixed), (1, 0));
    let second = from_diff(root, later, &inputs);
    assert_eq!((second.again, second.fixed), (1, 1));
    assert_eq!(find(root, &meaning.1.id).unwrap().1.status, "fixed");
  }

  #[test]
  fn test_filing_dedupes_keeps_inputs_and_reopens() {
    let scratch = Scratch::new("file");
    let root = &scratch.0;
    let first = file(root, &report(SAID, b"one")).unwrap();
    assert!(matches!(first, Filed::New(_)));
    let again = file(root, &report(SAID, b"two")).unwrap();
    assert_eq!(again, Filed::Again(first.id().to_string()));
    let same_input = file(root, &report(SAID, b"one")).unwrap();
    assert_eq!(same_input.id(), first.id());
    let (dir, mut incident) = find(root, first.id()).unwrap();
    assert_eq!(incident.occurrences.len(), 3);
    assert_eq!(
      incident
        .occurrences
        .iter()
        .map(|o| o.input.as_str())
        .collect::<Vec<_>>(),
      ["input", "input-2", "input"]
    );
    assert_eq!(fs::read(dir.join("input-2")).unwrap(), b"two");
    assert_eq!(incident.location, "src/fuzzing/format.rs:36");
    assert_eq!(incident.title, "meaning changed:");

    mark(root, &dir, &mut incident, "fixed", "def5678");
    assert_eq!(find(root, first.id()).unwrap().1.fixed_in, "def5678");
    let reopened = file(root, &report(SAID, b"three")).unwrap();
    assert_eq!(reopened, Filed::Reopened(first.id().to_string()));
    let incident = find(root, first.id()).unwrap().1;
    assert_eq!(incident.status, "open");
    assert!(incident.history.last().unwrap().result.contains("reopened"));

    let other = file(
      root,
      &Report {
        kind: "compat-break",
        key: Some("tests/a.rs"),
        ..report("anything", b"x")
      },
    )
    .unwrap();
    assert_ne!(other.id(), first.id());
    let board = fs::read_to_string(root_dir(root).join("INCIDENTS.md")).unwrap();
    assert!(board.contains("2 open"), "{}", board);
    assert!(
      fs::read_to_string(dir.join("README.md"))
        .unwrap()
        .contains("## Occurrences")
    );
  }

  #[test]
  fn test_the_gate_names_blocking_open_incidents_only() {
    let scratch = Scratch::new("gate");
    let root = &scratch.0;
    let crash = file(root, &report(SAID, b"x")).unwrap();
    file(
      root,
      &Report {
        kind: "compat-break",
        key: Some("tests/a.rs"),
        ..report("changed", b"y")
      },
    )
    .unwrap();
    let problems = check_gate(root);
    assert_eq!(problems.len(), 1, "{:?}", problems);
    assert!(problems[0].starts_with("open fuzz-crash"));
    let (dir, mut incident) = find(root, crash.id()).unwrap();
    mark(root, &dir, &mut incident, "wont-fix", "t");
    assert!(check_gate(root).is_empty());
  }
}
