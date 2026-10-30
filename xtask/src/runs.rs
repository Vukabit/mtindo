//! Run records: every `cargo xtask` command leaves one, and any of them can run detached.
//!
//! A command runs under a supervisor: this same binary, which writes the run's record,
//! starts the command as its child (marked with [`RUN_DIR`] so it does not supervise
//! itself), samples it, keeps a heartbeat, and writes how it ended. In the foreground the
//! child has the terminal, so live displays work; detached (`--detach`), the supervisor
//! runs in a process group of its own with the output in `output.log`, keeps the machine
//! awake, and the command line gets the run id back at once.
//!
//! ```text
//! .dev/runs/<UTC>-<command>-<4 hex>/
//!     run.toml      the record (below)
//!     samples.tsv   memory and CPU of the command's process tree, once a second
//!     output.log    what a detached run printed
//!     cancel        written by `cargo xtask runs cancel`; the supervisor stops the run
//! ```
//!
//! `run.toml` holds `id`, `command`, `tree`, `started`, `ended`, `pid` (the supervisor),
//! `child`, `heartbeat`, `state`, `exit`, `detached` and `peak_rss_kb`. The state is
//! `running`, then `passed`, `failed`, `timed-out`, `cancelled` or `crashed` (killed by a
//! signal nobody here sent). A record still `running` whose heartbeat has stopped and whose
//! supervisor is gone reads as `stale`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::common::awake::KeepAwake;
use crate::common::log::timestamp;
use crate::common::{process, sample, tree};

/// Set for a supervised command: its run's folder.
pub const RUN_DIR: &str = "XTASK_RUN_DIR";

/// How often the supervisor rewrites the heartbeat.
const HEARTBEAT: Duration = Duration::from_secs(5);

/// The longest any supervised command may run; each has its own, shorter limits within.
const RUN_LIMIT: Duration = Duration::from_secs(48 * 60 * 60);

/// A heartbeat older than this, with the supervisor gone, makes a run stale.
const STALE_AFTER: Duration = Duration::from_secs(30);

/// Set for a detached supervisor: the run's folder it supervises.
const SUPERVISE: &str = "XTASK_SUPERVISE";

/// Commands that run without a record: they only read records or print help, or (`flow`)
/// run as git hooks, which read what git gives them on stdin and run at every commit.
const UNRECORDED: &[&str] = &["help", "-h", "--help", "status", "runs", "metrics", "flow"];

/// Set by the SIGINT handler: the terminal's Ctrl-C, which the child gets too.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// One run's record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
  pub id: String,
  pub command: Vec<String>,
  pub tree: String,
  pub started: String,
  pub ended: String,
  pub pid: u32,
  pub child: u32,
  pub heartbeat: String,
  pub state: String,
  pub exit: Option<i32>,
  pub detached: bool,
  pub peak_rss_kb: u64,
}

impl Record {
  pub fn to_toml(&self) -> String {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut out = String::new();
    out.push_str(&format!("id = {}\n", quote(&self.id)));
    let words: Vec<String> = self.command.iter().map(|w| quote(w)).collect();
    out.push_str(&format!("command = [{}]\n", words.join(", ")));
    out.push_str(&format!("tree = {}\n", quote(&self.tree)));
    out.push_str(&format!("started = {}\n", quote(&self.started)));
    out.push_str(&format!("ended = {}\n", quote(&self.ended)));
    out.push_str(&format!("pid = {}\n", self.pid));
    out.push_str(&format!("child = {}\n", self.child));
    out.push_str(&format!("heartbeat = {}\n", quote(&self.heartbeat)));
    out.push_str(&format!("state = {}\n", quote(&self.state)));
    if let Some(exit) = self.exit {
      out.push_str(&format!("exit = {}\n", exit));
    }
    out.push_str(&format!("detached = {}\n", self.detached));
    out.push_str(&format!("peak_rss_kb = {}\n", self.peak_rss_kb));
    out
  }

  /// Reads what [`Record::to_toml`] writes.
  pub fn from_toml(text: &str) -> Option<Record> {
    let mut record = Record::default();
    for line in text.lines() {
      let (key, value) = line.split_once(" = ")?;
      let value = value.trim();
      match key.trim() {
        "id" => record.id = unquote(value)?,
        "command" => record.command = strings(value)?,
        "tree" => record.tree = unquote(value)?,
        "started" => record.started = unquote(value)?,
        "ended" => record.ended = unquote(value)?,
        "pid" => record.pid = value.parse().ok()?,
        "child" => record.child = value.parse().ok()?,
        "heartbeat" => record.heartbeat = unquote(value)?,
        "state" => record.state = unquote(value)?,
        "exit" => record.exit = Some(value.parse().ok()?),
        "detached" => record.detached = value == "true",
        "peak_rss_kb" => record.peak_rss_kb = value.parse().ok()?,
        _ => {}
      }
    }
    Some(record)
  }

  /// The state as it stands: a `running` record whose heartbeat stopped and whose
  /// supervisor is gone is `stale`.
  pub fn current_state(&self) -> String {
    if self.state == "running"
      && age(&self.heartbeat).is_none_or(|a| a > STALE_AFTER)
      && !process::alive(self.pid)
    {
      return "stale".to_string();
    }
    self.state.clone()
  }
}

/// Every run with a record, oldest first: (folder, record).
pub fn list(root: &Path) -> Vec<(PathBuf, Record)> {
  let mut runs: Vec<(PathBuf, Record)> = fs::read_dir(runs_dir(root))
    .into_iter()
    .flatten()
    .flatten()
    .filter_map(|e| read(&e.path()).map(|r| (e.path(), r)))
    .collect();
  runs.sort_by(|a, b| a.1.started.cmp(&b.1.started).then(a.1.id.cmp(&b.1.id)));
  runs
}

/// `2026-10-11T15:17:30.123Z` back into a time.
pub fn parse_time(stamp: &str) -> Option<SystemTime> {
  let digits = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
  let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
  let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
  let millis = digits(20..23).unwrap_or(0);
  // Days from the civil date (Howard Hinnant's algorithm).
  let y = if month <= 2 { year - 1 } else { year };
  let era = y.div_euclid(400);
  let yoe = y - era * 400;
  let mp = (month + 9) % 12;
  let doy = (153 * mp + 2) / 5 + day - 1;
  let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  let days = era * 146097 + doe - 719468;
  let seconds = days * 86400 + hour * 3600 + minute * 60 + second;
  Some(UNIX_EPOCH + Duration::from_millis((seconds * 1000 + millis) as u64))
}

pub fn read(dir: &Path) -> Option<Record> {
  Record::from_toml(&fs::read_to_string(record_path(dir)).ok()?)
}

/// `cargo xtask runs list|show ID|tail ID|cancel ID`.
pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let id = |sub: &clap::ArgMatches| crate::cli::text(sub, "id").unwrap_or_default();
  match m.subcommand() {
    None => {
      print!("{}", list_text(&list(root), 30));
      ExitCode::SUCCESS
    }
    Some(("list", sub)) => {
      let limit = if crate::cli::on(sub, "all") {
        usize::MAX
      } else {
        30
      };
      print!("{}", list_text(&list(root), limit));
      ExitCode::SUCCESS
    }
    Some((action, sub)) => {
      let id = id(sub);
      let Some((dir, record)) = find(root, &id) else {
        return unknown(&id);
      };
      match action {
        "show" => {
          print!("{}", show_text(&dir, &record));
          ExitCode::SUCCESS
        }
        "tail" => tail(&dir, &record),
        _ => cancel(&dir, &record),
      }
    }
  }
}

pub fn runs_dir(root: &Path) -> PathBuf {
  root.join(".dev").join("runs")
}

/// Runs `args` under a supervisor: in the foreground (the exit code is the command's) or,
/// with `--detach` among them, in the background (the run id is printed at once).
pub fn start(root: &Path, args: &[String]) -> ExitCode {
  let detach = args.iter().any(|a| a == "--detach");
  let args: Vec<String> = args.iter().filter(|a| *a != "--detach").cloned().collect();
  if let Some(dir) = std::env::var_os(SUPERVISE) {
    return supervise(root, Path::new(&dir), &args);
  }
  let id = new_id(&args[0]);
  let dir = runs_dir(root).join(&id);
  if let Err(e) = fs::create_dir_all(&dir) {
    eprintln!("cannot create {}: {}", dir.display(), e);
    return ExitCode::from(1);
  }
  let record = Record {
    id: id.clone(),
    command: args.clone(),
    tree: tree::identity(root),
    started: timestamp(SystemTime::now()),
    pid: std::process::id(),
    heartbeat: timestamp(SystemTime::now()),
    state: "running".to_string(),
    detached: detach,
    ..Record::default()
  };
  write(&dir, &record);
  if !detach {
    return supervise(root, &dir, &args);
  }
  let me = std::env::current_exe()
    .map(|p| p.display().to_string())
    .unwrap_or_else(|_| "xtask".to_string());
  let env = [(SUPERVISE.to_string(), dir.display().to_string())];
  match process::detach(root, &me, &args, &env, &dir.join("output.log")) {
    Ok(pid) => {
      let mut record = record;
      record.pid = pid;
      write(&dir, &record);
      println!("{}", id);
      eprintln!(
        "started in the background; follow it with `cargo xtask runs tail {}`",
        id
      );
      ExitCode::SUCCESS
    }
    Err(e) => {
      let mut record = record;
      record.state = "failed".to_string();
      record.ended = timestamp(SystemTime::now());
      write(&dir, &record);
      eprintln!("cannot start the run: {}", e);
      ExitCode::from(1)
    }
  }
}

/// Whether `args` (after `cargo xtask`) run under a supervisor: everything but the
/// commands that only read records or print help, and the supervised command itself.
pub fn supervised(args: &[String]) -> bool {
  std::env::var_os(RUN_DIR).is_none()
    && args
      .first()
      .is_some_and(|c| !UNRECORDED.contains(&c.as_str()))
}

/// How long ago a UTC timestamp as [`timestamp`] writes it was.
fn age(stamp: &str) -> Option<Duration> {
  let then = parse_time(stamp)?;
  SystemTime::now().duration_since(then).ok()
}

/// Asks a run's supervisor to stop it; with the supervisor gone, stops the command itself.
fn cancel(dir: &Path, record: &Record) -> ExitCode {
  let state = record.current_state();
  if state != "running" && state != "stale" {
    println!("run {} is not running ({})", record.id, state);
    return ExitCode::SUCCESS;
  }
  let _ = fs::write(dir.join("cancel"), timestamp(SystemTime::now()));
  if state == "stale" && record.child != 0 && process::alive(record.child) {
    #[cfg(unix)]
    process::stop_pid(record.child);
    let mut record = record.clone();
    record.state = "cancelled".to_string();
    record.ended = timestamp(SystemTime::now());
    write(dir, &record);
  }
  println!("asked run {} to stop", record.id);
  ExitCode::SUCCESS
}

/// How a watched command ended, as a record's state.
fn ending(watched: &process::Watched, interrupted: bool) -> &'static str {
  if watched.stopped || interrupted || watched.code == 130 {
    "cancelled"
  } else if watched.timed_out {
    "timed-out"
  } else if watched.code == 0 {
    "passed"
  } else if watched.code > 128 {
    "crashed"
  } else {
    "failed"
  }
}

/// The run whose id is `id` or ends with it (`a1b2` is enough), the newest if several.
fn find(root: &Path, id: &str) -> Option<(PathBuf, Record)> {
  list(root)
    .into_iter()
    .rev()
    .find(|(_, r)| r.id == id || r.id.ends_with(id))
}

fn list_text(runs: &[(PathBuf, Record)], limit: usize) -> String {
  let mut out = format!(
    "{:<10} {:<36} {:<8} {:>9} {:>9}  {}\n",
    "STATE", "RUN", "STARTED", "TOOK", "PEAK RSS", "COMMAND"
  );
  let shown = runs.len().saturating_sub(limit);
  for (_, r) in &runs[shown..] {
    let took = match (parse_time(&r.started), parse_time(&r.ended)) {
      (Some(a), Some(b)) => b.duration_since(a).ok(),
      (Some(a), None) => SystemTime::now().duration_since(a).ok(),
      _ => None,
    };
    out.push_str(&format!(
      "{:<10} {:<36} {:<8} {:>9} {:>9}  {}\n",
      r.current_state(),
      r.id,
      r.started.get(11..19).unwrap_or(""),
      took.map_or("-".to_string(), process::duration),
      if r.peak_rss_kb > 0 {
        sample::kilobytes(r.peak_rss_kb)
      } else {
        "-".to_string()
      },
      r.command.join(" ")
    ));
  }
  out
}

/// A new run's id: `<UTC>-<command>-<4 hex>`.
fn new_id(command: &str) -> String {
  let now = SystemTime::now();
  let stamp: String = timestamp(now)
    .chars()
    .take(19)
    .filter(|c| c.is_ascii_digit() || *c == 'T')
    .map(|c| if c == 'T' { '-' } else { c })
    .collect();
  let nanos = now
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| d.subsec_nanos());
  let salt = (nanos ^ std::process::id().wrapping_mul(2654435761)) & 0xffff;
  let name: String = command
    .chars()
    .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
    .collect();
  format!("{}-{}-{:04x}", stamp, name, salt)
}

extern "C" fn on_interrupt(_: libc::c_int) {
  INTERRUPTED.store(true, Ordering::SeqCst);
}

fn record_path(dir: &Path) -> PathBuf {
  dir.join("run.toml")
}

fn show_text(dir: &Path, record: &Record) -> String {
  let mut out = record.to_toml();
  out.push_str(&format!("state_now = \"{}\"\n", record.current_state()));
  let samples = dir.join("samples.tsv");
  let curve = sample::curve(&samples, 60);
  if !curve.is_empty() {
    out.push_str(&format!("memory {}\n", curve));
  }
  out.push_str(&format!("folder {}\n", dir.display()));
  out
}

/// A TOML array of quoted strings, as [`Record::to_toml`] writes it.
fn strings(value: &str) -> Option<Vec<String>> {
  let inner = value.strip_prefix('[')?.strip_suffix(']')?.trim();
  if inner.is_empty() {
    return Some(Vec::new());
  }
  let mut out = Vec::new();
  let mut rest = inner;
  while !rest.is_empty() {
    rest = rest.trim_start_matches([',', ' ']);
    let body = rest.strip_prefix('"')?;
    let mut end = None;
    let mut escaped = false;
    for (idx, c) in body.char_indices() {
      match c {
        '\\' if !escaped => escaped = true,
        '"' if !escaped => {
          end = Some(idx);
          break;
        }
        _ => escaped = false,
      }
    }
    let end = end?;
    out.push(unquote(&format!("\"{}\"", &body[..end]))?);
    rest = &body[end + 1..];
  }
  Some(out)
}

/// Supervises the run recorded in `dir`: starts the command, keeps the record, and returns
/// the command's exit code.
fn supervise(root: &Path, dir: &Path, args: &[String]) -> ExitCode {
  let Some(mut record) = read(dir) else {
    eprintln!("no run record in {}", dir.display());
    return ExitCode::from(1);
  };
  record.pid = std::process::id();
  write(dir, &record);
  // Ctrl-C reaches the child, which ends; the supervisor stays to record that. A handler,
  // not an ignored signal: an ignored SIGINT would be inherited by the child.
  // SAFETY: installs a handler that only stores to an atomic.
  unsafe {
    libc::signal(
      libc::SIGINT,
      on_interrupt as *const () as libc::sighandler_t,
    );
  }
  let _awake = record.detached.then(KeepAwake::start);
  let me = std::env::current_exe()
    .map(|p| p.display().to_string())
    .unwrap_or_else(|_| "xtask".to_string());
  let env = [(RUN_DIR.to_string(), dir.display().to_string())];
  let child = Arc::new(AtomicU32::new(0));
  let beating = Arc::new(AtomicBool::new(true));
  let heart = {
    let dir = dir.to_path_buf();
    let child = Arc::clone(&child);
    let beating = Arc::clone(&beating);
    let mut record = record.clone();
    thread::spawn(move || {
      let mut last = std::time::Instant::now() - HEARTBEAT;
      while beating.load(Ordering::SeqCst) {
        if last.elapsed() >= HEARTBEAT {
          last = std::time::Instant::now();
          record.child = child.load(Ordering::SeqCst);
          record.heartbeat = timestamp(SystemTime::now());
          write(&dir, &record);
        }
        thread::sleep(Duration::from_millis(100));
      }
    })
  };
  let cancel = dir.join("cancel");
  let output = dir.join("output.log");
  let samples = dir.join("samples.tsv");
  let stop = || cancel.exists();
  let started = |pid: u32| child.store(pid, Ordering::SeqCst);
  let watched = process::watch(
    root,
    &me,
    args,
    &env,
    process::Watch {
      output: record.detached.then_some(output.as_path()),
      samples: Some(&samples),
      limit: RUN_LIMIT,
      stop: &stop,
      started: &started,
    },
  );
  beating.store(false, Ordering::SeqCst);
  let _ = heart.join();
  let mut record = read(dir).unwrap_or(record);
  record.child = child.load(Ordering::SeqCst);
  record.ended = timestamp(SystemTime::now());
  record.heartbeat = record.ended.clone();
  let code = match watched {
    Ok((watched, used)) => {
      record.peak_rss_kb = used.map_or(0, |u| u.peak_rss_kb);
      record.exit = Some(watched.code);
      record.state = ending(&watched, INTERRUPTED.load(Ordering::SeqCst)).to_string();
      watched.code
    }
    Err(e) => {
      eprintln!("cannot run the command: {}", e);
      record.state = "failed".to_string();
      1
    }
  };
  write(dir, &record);
  ExitCode::from(code.clamp(0, 255) as u8)
}

/// Prints a run's output as it grows, until the run ends.
fn tail(dir: &Path, record: &Record) -> ExitCode {
  if !record.detached {
    println!(
      "run {} ran in the foreground; its output went to its terminal and its log",
      record.id
    );
    return ExitCode::SUCCESS;
  }
  let path = dir.join("output.log");
  let mut shown = 0;
  loop {
    if let Ok(text) = fs::read(&path)
      && text.len() > shown
    {
      print!("{}", String::from_utf8_lossy(&text[shown..]));
      shown = text.len();
    }
    let now = read(dir).unwrap_or_else(|| record.clone());
    let state = now.current_state();
    if state != "running" {
      println!("\n[run {} {}]", now.id, state);
      return ExitCode::SUCCESS;
    }
    thread::sleep(Duration::from_millis(500));
  }
}

fn unknown(id: &str) -> ExitCode {
  eprintln!("no run matches `{}` (cargo xtask runs list)", id);
  ExitCode::from(2)
}

/// A quoted TOML string's content.
fn unquote(value: &str) -> Option<String> {
  let inner = value.strip_prefix('"')?.strip_suffix('"')?;
  let mut out = String::new();
  let mut chars = inner.chars();
  while let Some(c) = chars.next() {
    if c == '\\' {
      out.push(chars.next()?);
    } else {
      out.push(c);
    }
  }
  Some(out)
}

fn write(dir: &Path, record: &Record) {
  let path = record_path(dir);
  let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
  if fs::write(&temporary, record.to_toml()).is_ok() {
    let _ = fs::rename(&temporary, &path);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_dead_running_record_is_stale() {
    let mut record = Record {
      state: "running".to_string(),
      heartbeat: "2020-01-01T00:00:00.000Z".to_string(),
      pid: 9_999_999,
      ..Record::default()
    };
    assert_eq!(record.current_state(), "stale");
    record.heartbeat = timestamp(SystemTime::now());
    assert_eq!(record.current_state(), "running");
    record.state = "passed".to_string();
    assert_eq!(record.current_state(), "passed");
  }

  #[test]
  fn test_a_record_round_trips() {
    let record = Record {
      id: "20261011-194500-fuzz-a1b2".to_string(),
      command: vec![
        "fuzz".into(),
        "run".into(),
        "say \"hi\"".into(),
        "a\\b".into(),
      ],
      tree: "7bb39cb-dirty.1234abcd".to_string(),
      started: "2026-10-11T19:45:00.123Z".to_string(),
      ended: String::new(),
      pid: 42,
      child: 43,
      heartbeat: "2026-10-11T19:45:05.000Z".to_string(),
      state: "running".to_string(),
      exit: Some(3),
      detached: true,
      peak_rss_kb: 1024,
    };
    assert_eq!(Record::from_toml(&record.to_toml()), Some(record));
  }

  #[test]
  fn test_endings() {
    let w = |code, timed_out, stopped| process::Watched {
      code,
      timed_out,
      stopped,
    };
    assert_eq!(ending(&w(0, false, false), false), "passed");
    assert_eq!(ending(&w(1, false, false), false), "failed");
    assert_eq!(ending(&w(124, true, false), false), "timed-out");
    assert_eq!(ending(&w(143, false, true), false), "cancelled");
    assert_eq!(ending(&w(130, false, false), false), "cancelled");
    assert_eq!(ending(&w(1, false, false), true), "cancelled");
    assert_eq!(ending(&w(137, false, false), false), "crashed");
  }

  #[test]
  fn test_times_parse_back() {
    let now = SystemTime::now();
    let back = parse_time(&timestamp(now)).unwrap();
    let diff = now.duration_since(back).unwrap_or_else(|e| e.duration());
    assert!(diff < Duration::from_millis(2), "{:?}", diff);
    assert_eq!(
      parse_time("1970-01-01T00:00:01.500Z"),
      Some(UNIX_EPOCH + Duration::from_millis(1500))
    );
  }

  #[test]
  fn test_which_commands_are_supervised() {
    let args = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    if std::env::var_os(RUN_DIR).is_none() {
      assert!(supervised(&args(&["fuzz", "run", "format"])));
      assert!(!supervised(&args(&["runs", "list"])));
      assert!(!supervised(&args(&["help"])));
      assert!(!supervised(&[]));
    }
  }
}
