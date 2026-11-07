//! Run statistics for the command line: whether a run is counted, its line once it ends,
//! and `mto stats`, which shows what the ledger holds ([`crate::stats`]).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use crate::config::{Environment, Resolver, user};
use crate::stats::{CONTEXTS, Context, Counts, Ledger, Mode, Read, Run, Total};
use crate::text::color::{paint, tone};
use crate::text::json::Json;
use crate::{Done, Outcome, RunOptions};

static STARTED: Mutex<Option<Started>> = Mutex::new(None);

/// What `mto stats` shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
  /// The totals, in a few lines.
  Summary,
  Day,
  Month,
  Year,
  Version,
  Mode,
}

impl By {
  pub fn parse(text: &str) -> Option<By> {
    Some(match text {
      "day" => By::Day,
      "month" => By::Month,
      "year" => By::Year,
      "version" => By::Version,
      "mode" => By::Mode,
      _ => return None,
    })
  }
}

/// What a run counted in the ledger needs from its start, kept until it ends.
struct Started {
  ledger: Ledger,
  project: Option<String>,
  at: Instant,
  stdin: bool,
  editor: bool,
}

/// The folder the ledger is kept in: `stats` beside history.
pub fn dir(env: &Environment) -> Option<PathBuf> {
  user::history_dir(env).and_then(|history| history.parent().map(|p| p.join("stats")))
}

/// Whether runs are counted, and if not why: `stats.enabled`, which by default (`auto`)
/// follows `history.enabled` and is off on CI; `MTINDO_NO_STATS`; a home folder to keep it.
pub fn enabled(resolver: &Resolver) -> Result<PathBuf, String> {
  let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
  if set(crate::config::env::NO_STATS) {
    return Err(format!("{} is set", crate::config::env::NO_STATS));
  }
  let choice = resolver
    .user_value("stats.enabled")
    .and_then(|v| v.as_text().map(str::to_string))
    .unwrap_or_else(|| "auto".to_string());
  match choice.as_str() {
    "off" => return Err("`stats.enabled = \"off\"` in the user config".to_string()),
    "on" => {}
    _ => {
      let history = resolver
        .user_value("history.enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
      if !history {
        return Err(
          "history is off (`history.enabled = false`), and `stats.enabled` follows it".to_string(),
        );
      }
      if set("CI") {
        return Err("this is a CI job (`CI` is set)".to_string());
      }
    }
  }
  dir(resolver.env()).ok_or_else(|| "there is no home folder to keep them in".to_string())
}

/// Counts the run readied by [`start`], once its files are `done` and it exits `exit`.
pub fn finish(done: &[Done], opts: &RunOptions, exit: i32) {
  let Some(started) = STARTED.lock().ok().and_then(|mut s| s.take()) else {
    return;
  };
  let count = |outcome: Outcome| done.iter().filter(|d| d.outcome == outcome).count() as u64;
  let sum = |f: fn(&crate::Counts) -> usize| done.iter().map(|d| f(&d.counts) as u64).sum();
  let mode = if started.stdin {
    Mode::Stdin
  } else if opts.check {
    Mode::Check
  } else if opts.dry_run {
    Mode::DryRun
  } else {
    Mode::Write
  };
  let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
  let context = if set("CI") {
    Context::Ci
  } else if started.editor {
    Context::Editor
  } else if set(crate::config::env::PATHS_FILE) || set(crate::config::env::COMMAND) {
    Context::Cargo
  } else if crate::text::terminal::Stream::Out.is_terminal()
    || crate::text::terminal::Stream::Err.is_terminal()
  {
    Context::Terminal
  } else {
    Context::Pipe
  };
  let workers = std::thread::available_parallelism()
    .map(|n| n.get())
    .unwrap_or(1)
    .min(done.len().max(1)) as u64;
  let run = Run {
    millis: crate::stats::now_millis(),
    // The version alone, `-dev` for a build not released: never the commit it came from.
    version: if crate::version::CHANNEL == "rel" {
      crate::version::VERSION.to_string()
    } else {
      format!("{}-dev", crate::version::VERSION)
    },
    system: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
    mode,
    context,
    edition: done
      .iter()
      .find_map(|d| d.edition)
      .map_or("-", |e| e.year())
      .to_string(),
    exit,
    project: started.project,
    counts: Counts {
      files: done.len() as u64,
      changed: count(Outcome::Changed),
      unchanged: count(Outcome::Unchanged),
      skipped: count(Outcome::Skipped)
        + count(Outcome::LayoutOff)
        + count(Outcome::Held)
        + count(Outcome::Dirty),
      syntax_errors: count(Outcome::SyntaxError),
      failed: count(Outcome::Failed),
      refused: count(Outcome::VerifierFailed),
      bytes: sum(|c| c.bytes),
      lines: sum(|c| c.lines),
      inserted: sum(|c| c.inserted),
      removed: sum(|c| c.removed),
      moved: sum(|c| c.moved),
    },
    wall_ms: started.at.elapsed().as_millis() as u64,
    threads: workers,
    layout: done.iter().any(|d| d.layout),
  };
  started.ledger.record(&run);
}

/// `mto stats`: the ledger in `dir`, from day `since` on if given, as `by` asks, or as JSON.
/// `off` says why runs are not counted, when they are not.
pub fn show(dir: &Path, by: By, since: Option<i64>, json: bool, off: Option<&str>) -> String {
  let read = Ledger::new(dir.to_path_buf()).read();
  let totals = within(&read, since);
  if json {
    return show_json(&totals, &read, by, off);
  }
  let on = crate::color_enabled(false);
  let mut out = String::new();
  if totals.is_empty() {
    out.push_str("No runs counted");
    if since.is_some() {
      out.push_str(" in that time");
    }
    out.push('.');
    match off {
      Some(why) => out.push_str(&format!(
        " Runs are not counted: {}.\nTurn counting on with `stats.enabled = \"on\"` in the \
         user config; only counts are kept, never a name (`mto stats -h`).\n",
        why
      )),
      None => out.push_str(" Each run that formats files adds to them.\n"),
    }
    return out;
  }
  if let Some(why) = off {
    out.push_str(&paint(
      on,
      tone::WARN,
      &format!("Runs are not counted now: {}.\n", why),
    ));
  }
  match by {
    By::Summary => out.push_str(&summary(&totals, &read, on)),
    _ => out.push_str(&table(&totals, by, on)),
  }
  out
}

/// Readies the count of a run that formats, when runs are counted: `stdin` for one reading
/// stdin, `editor` when an editor names the file it holds (`--stdin-filepath`).
pub fn start(resolver: &Resolver, opts: &RunOptions, stdin: bool, editor: bool) {
  if opts.no_stats {
    return;
  }
  let Ok(dir) = enabled(resolver) else {
    return;
  };
  let ledger = Ledger::new(dir);
  let project = std::env::current_dir()
    .ok()
    .and_then(|cwd| ledger.project(&super::history::project_root(&cwd)));
  if let Ok(mut started) = STARTED.lock() {
    *started = Some(Started {
      ledger,
      project,
      at: Instant::now(),
      stdin,
      editor,
    });
  }
}

/// Every total added into one.
fn all(totals: &[Total]) -> Total {
  let mut sum = totals[0].clone();
  for total in &totals[1..] {
    sum.add(total);
  }
  sum
}

/// Milliseconds for people: `38 ms`, `4.1 s`, `9 m 41 s`, `2 h 5 m`.
fn duration(ms: u64) -> String {
  match ms {
    ms if ms < 1000 => format!("{} ms", ms),
    ms if ms < 60_000 => format!("{:.1} s", ms as f64 / 1000.0),
    ms if ms < 3_600_000 => format!("{} m {} s", ms / 60_000, ms % 60_000 / 1000),
    ms => format!("{} h {} m", ms / 3_600_000, ms % 3_600_000 / 60_000),
  }
}

/// `part` of `all`, as a whole percentage.
fn percent(part: u64, all: u64) -> String {
  if all == 0 {
    return "0%".to_string();
  }
  format!("{}%", (part * 100 + all / 2) / all)
}

/// The key a total is listed under in a table by `by`.
fn row_key(total: &Total, by: By) -> Option<String> {
  Some(match by {
    By::Day => (total.period.len() == 10).then(|| total.period.clone())?,
    By::Month => (total.period.len() >= 7).then(|| total.period[..7].to_string())?,
    By::Year => total.period[..4].to_string(),
    By::Version => total.version.clone(),
    By::Mode => total.mode.id().to_string(),
    By::Summary => String::new(),
  })
}

/// A large count shortened: `61.2 M`, `48.3 K`, `912`.
fn short(n: u64) -> String {
  match n {
    n if n >= 1_000_000_000 => format!("{:.1} G", n as f64 / 1e9),
    n if n >= 1_000_000 => format!("{:.1} M", n as f64 / 1e6),
    n if n >= 10_000 => format!("{:.1} K", n as f64 / 1e3),
    n => n.to_string(),
  }
}

fn show_json(totals: &[Total], read: &Read, by: By, off: Option<&str>) -> String {
  let mut out = Json::output("stats")
    .with("counting", off.is_none())
    .with("why_not", off.map_or(Json::Null, Json::from))
    .with(
      "since",
      read.since.as_deref().map_or(Json::Null, Json::from),
    );
  if totals.is_empty() {
    return format!("{}\n", out.with("runs", 0u64).text());
  }
  if by != By::Summary {
    let mut rows: std::collections::BTreeMap<String, Total> = std::collections::BTreeMap::new();
    for total in totals {
      if let Some(key) = row_key(total, by) {
        rows
          .entry(key)
          .and_modify(|t| t.add(total))
          .or_insert_with(|| total.clone());
      }
    }
    let rows: Vec<Json> = rows
      .iter()
      .map(|(key, t)| total_json(t).with("key", key.as_str()))
      .collect();
    return format!("{}\n", out.with("rows", Json::Array(rows)).text());
  }
  let sum = all(totals);
  let mut modes = Json::object();
  for mode in [Mode::Check, Mode::Write, Mode::Stdin, Mode::DryRun] {
    let runs: u64 = totals
      .iter()
      .filter(|t| t.mode == mode)
      .map(|t| t.runs)
      .sum();
    modes = modes.with(mode.id(), runs);
  }
  out = out.with("all", total_json(&sum)).with("modes", modes);
  format!("{}\n", out.text())
}

/// Bytes for people: `1.9 GB`, `301.4 MB`, `12.0 KB`.
fn size(bytes: u64) -> String {
  let b = bytes as f64;
  match bytes {
    n if n >= 1 << 30 => format!("{:.1} GB", b / (1u64 << 30) as f64),
    n if n >= 1 << 20 => format!("{:.1} MB", b / (1u64 << 20) as f64),
    n if n >= 1 << 10 => format!("{:.1} KB", b / 1024.0),
    n => format!("{} bytes", n),
  }
}

fn summary(totals: &[Total], read: &Read, on: bool) -> String {
  let sum = all(totals);
  let c = sum.counts;
  let key = |text: &str| paint(on, tone::KEY, &format!("{:<11}", text));
  let dot = paint(on, tone::QUIET, " · ");
  let mut versions: Vec<(&str, &str)> = totals
    .iter()
    .map(|t| (t.period.as_str(), t.version.as_str()))
    .collect();
  versions.sort();
  let first = versions.first().map(|v| v.1).unwrap_or("");
  let last = versions.last().map(|v| v.1).unwrap_or("");
  let mut out = String::new();
  out.push_str(&format!(
    "since {}{}{} runs{}mto {}\n\n",
    read.since.as_deref().unwrap_or("-"),
    dot,
    thousands(sum.runs),
    dot,
    if first == last {
      first.to_string()
    } else {
      format!("{} → {}", first, last)
    }
  ));
  out.push_str(&format!(
    "{} {}{}{} files{}{} lines{}{} files changed\n",
    key("formatted"),
    size(c.bytes),
    dot,
    thousands(c.files),
    dot,
    short(c.lines),
    dot,
    thousands(c.changed)
  ));
  out.push_str(&format!(
    "{} {} lines added{}{} removed{}{} items moved\n",
    key("changes"),
    thousands(c.inserted),
    dot,
    thousands(c.removed),
    dot,
    thousands(c.moved)
  ));
  out.push_str(&format!(
    "{} {} in all{}median run {}{}p95 {}{}longest {}\n",
    key("time"),
    duration(sum.wall_ms),
    dot,
    duration(sum.percentile_ms(0.5)),
    dot,
    duration(sum.percentile_ms(0.95)),
    dot,
    duration(sum.wall_max_ms)
  ));
  let editor: Vec<Total> = totals
    .iter()
    .filter(|t| t.mode == Mode::Stdin)
    .cloned()
    .collect();
  if !editor.is_empty() {
    let e = all(&editor);
    out.push_str(&format!(
      "{} {} formats{}median {}{}p95 {}\n",
      key("stdin"),
      thousands(e.runs),
      dot,
      duration(e.percentile_ms(0.5)),
      dot,
      duration(e.percentile_ms(0.95))
    ));
  }
  let shares = |parts: Vec<(&str, u64)>| -> String {
    parts
      .iter()
      .filter(|(_, n)| *n > 0)
      .map(|(name, n)| format!("{} {}", name, percent(*n, sum.runs)))
      .collect::<Vec<_>>()
      .join(&dot)
  };
  let mode_runs = |mode: Mode| {
    totals
      .iter()
      .filter(|t| t.mode == mode)
      .map(|t| t.runs)
      .sum()
  };
  out.push_str(&format!(
    "{} {}\n",
    key("by mode"),
    shares(vec![
      ("check", mode_runs(Mode::Check)),
      ("write", mode_runs(Mode::Write)),
      ("stdin", mode_runs(Mode::Stdin)),
      ("dry run", mode_runs(Mode::DryRun)),
    ])
  ));
  out.push_str(&format!(
    "{} {}\n",
    key("from"),
    shares(
      CONTEXTS
        .iter()
        .enumerate()
        .map(|(at, c)| (c.id(), sum.contexts[at]))
        .collect()
    )
  ));
  out.push_str(&format!(
    "{} {}{}layout on in {} of runs\n",
    key("projects"),
    thousands(sum.projects.len() as u64),
    dot,
    percent(sum.layout_runs, sum.runs)
  ));
  let problems = c.refused + c.syntax_errors + c.failed;
  let problem_text = format!(
    "{} writes refused{}{} files with syntax errors{}{} unreadable",
    thousands(c.refused),
    dot,
    thousands(c.syntax_errors),
    dot,
    thousands(c.failed)
  );
  out.push_str(&format!(
    "{} {}\n",
    key("problems"),
    if c.refused > 0 {
      paint(on, tone::WARN, &problem_text)
    } else if problems == 0 {
      paint(on, tone::GOOD, &problem_text)
    } else {
      problem_text
    }
  ));
  out
}

fn table(totals: &[Total], by: By, on: bool) -> String {
  let mut rows: std::collections::BTreeMap<String, Total> = std::collections::BTreeMap::new();
  for total in totals {
    let Some(key) = row_key(total, by) else {
      continue;
    };
    rows
      .entry(key)
      .and_modify(|t| t.add(total))
      .or_insert_with(|| total.clone());
  }
  let head = match by {
    By::Day => "day",
    By::Month => "month",
    By::Year => "year",
    By::Version => "version",
    By::Mode => "mode",
    By::Summary => "",
  };
  let width = rows
    .keys()
    .map(|k| k.chars().count())
    .chain([head.len(), 5])
    .max()
    .unwrap_or(5);
  let mut out = paint(
    on,
    tone::HEADING,
    &format!(
      "{:<w$}  {:>7}  {:>9}  {:>8}  {:>9}  {:>9}  {:>8}  {:>8}",
      head,
      "runs",
      "files",
      "changed",
      "lines",
      "time",
      "median",
      "p95",
      w = width
    ),
  );
  out.push('\n');
  let line = |key: &str, t: &Total| {
    format!(
      "{:<w$}  {:>7}  {:>9}  {:>8}  {:>9}  {:>9}  {:>8}  {:>8}\n",
      key,
      thousands(t.runs),
      thousands(t.counts.files),
      thousands(t.counts.changed),
      short(t.counts.lines),
      duration(t.wall_ms),
      duration(t.percentile_ms(0.5)),
      duration(t.percentile_ms(0.95)),
      w = width
    )
  };
  for (key, total) in &rows {
    out.push_str(&line(key, total));
  }
  if rows.len() > 1 {
    let sum = all(&rows.values().cloned().collect::<Vec<_>>());
    out.push_str(&paint(on, tone::QUIET, &line("all", &sum)));
  }
  if by == By::Day && totals.iter().any(|t| t.period.len() < 10) {
    out.push_str(&paint(
      on,
      tone::QUIET,
      "Older runs are kept as months and years, which `--by month` and `--by year` show.\n",
    ));
  }
  out
}

/// `1234567` as `1,234,567`.
fn thousands(n: u64) -> String {
  let digits = n.to_string();
  let mut out = String::new();
  for (at, c) in digits.chars().enumerate() {
    if at > 0 && (digits.len() - at).is_multiple_of(3) {
      out.push(',');
    }
    out.push(c);
  }
  out
}

/// A total's counts as JSON.
fn total_json(t: &Total) -> Json {
  let c = t.counts;
  let mut contexts = Json::object();
  for (at, context) in CONTEXTS.iter().enumerate() {
    contexts = contexts.with(context.id(), t.contexts[at]);
  }
  Json::object()
    .with("runs", t.runs)
    .with(
      "files",
      Json::object()
        .with("all", c.files)
        .with("changed", c.changed)
        .with("unchanged", c.unchanged)
        .with("skipped", c.skipped)
        .with("syntax_errors", c.syntax_errors)
        .with("failed", c.failed)
        .with("refused", c.refused),
    )
    .with("bytes", c.bytes)
    .with(
      "lines",
      Json::object()
        .with("read", c.lines)
        .with("inserted", c.inserted)
        .with("removed", c.removed),
    )
    .with("moved", c.moved)
    .with(
      "time",
      Json::object()
        .with("all_ms", t.wall_ms)
        .with("median_ms", t.percentile_ms(0.5))
        .with("p95_ms", t.percentile_ms(0.95))
        .with("longest_ms", t.wall_max_ms),
    )
    .with("from", contexts)
    .with("layout_runs", t.layout_runs)
    .with("projects", t.projects.len() as u64)
}

/// The totals from day `since` on: days by their date, months and years when they start
/// on or after it (a month or year begun before it is left out whole).
fn within(read: &Read, since: Option<i64>) -> Vec<Total> {
  let start = |period: &str| -> Option<i64> {
    match period.len() {
      10 => crate::stats::parse_date(period),
      7 => crate::stats::parse_date(&format!("{}-01", period)),
      4 => crate::stats::parse_date(&format!("{}-01-01", period)),
      _ => None,
    }
  };
  read
    .years
    .iter()
    .chain(&read.months)
    .chain(&read.days)
    .filter(|t| since.is_none_or(|s| start(&t.period).is_some_and(|p| p >= s)))
    .cloned()
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_numbers_and_times_read_as_people_write_them() {
    assert_eq!(thousands(0), "0");
    assert_eq!(thousands(1_234_567), "1,234,567");
    assert_eq!(short(61_204_417), "61.2 M");
    assert_eq!(short(912), "912");
    assert_eq!(size(2_040_109_465), "1.9 GB");
    assert_eq!(duration(38), "38 ms");
    assert_eq!(duration(4_100), "4.1 s");
    assert_eq!(duration(581_204), "9 m 41 s");
    assert_eq!(percent(1, 3), "33%");
    assert_eq!(percent(0, 0), "0%");
  }
}
