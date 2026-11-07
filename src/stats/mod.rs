//! Layer 2: run statistics, what mto has done to date as counts, never a name.
//!
//! A run that formats appends one line to the day's file once its output is printed: when
//! it ran, the mto version and system, how it was run, and how many files, lines and items
//! it read and changed, and how long it took. Never a path, a file's name or contents, the
//! command's arguments, a user, host, remote or commit. A project is a hash of its folder
//! salted per machine, which tells projects apart without naming one.
//!
//! The folder holds four tiers, each line of a tier the sum of lines below it:
//!
//! - `runs/YYYY-MM-DD.tsv`: a line per run, kept [`KEEP_RUNS_DAYS`] days;
//! - `days/YYYY-MM.tsv`: a line per day, mode and version, kept [`KEEP_DAYS_DAYS`] days;
//! - `months/YYYY.tsv`: a line per month, mode and version, kept [`KEEP_MONTHS_DAYS`] days;
//! - `years.tsv`: a line per year, mode and version, kept for good.
//!
//! A tier's line holds only what adds up: counts, sums, the largest value, durations as a
//! histogram ([`BUCKETS`] buckets), so percentiles survive folding, and the projects seen.
//! Folding happens at the end of a run, under a lock: a period older than its tier keeps is
//! added to the tier above, which is written whole under a temporary name and renamed into
//! place, and only then is the period removed below. A period already in the tier above is
//! not added again, so a fold cut short leaves nothing counted twice. Writing statistics
//! never fails a run: every error here is passed over.
//!
//! Days are UTC dates.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The buckets durations are counted in: bucket `i` holds a duration up to `BOUNDS[i]`
/// milliseconds, the last everything longer.
pub const BUCKETS: usize = 40;

/// Every context, in the order a tier's line counts them.
pub const CONTEXTS: [Context; 5] = [
  Context::Terminal,
  Context::Pipe,
  Context::Ci,
  Context::Editor,
  Context::Cargo,
];

/// Days a day's line is kept.
pub const KEEP_DAYS_DAYS: i64 = 90;

/// Days a month's line is kept.
pub const KEEP_MONTHS_DAYS: i64 = 730;

/// Days a run's own line is kept.
pub const KEEP_RUNS_DAYS: i64 = 7;

/// The first line of every file, naming its shape.
const HEADER: &str = "# mto stats 1";

/// A lock older than this was left by a run that ended without removing it.
const STALE_LOCK: Duration = Duration::from_secs(60);

/// Where a run was started from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
  /// A terminal.
  Terminal,
  /// Output to a pipe or a file: a script or a hook.
  Pipe,
  /// A CI job (`CI` set).
  Ci,
  /// An editor: stdin with `--stdin-filepath`.
  Editor,
  /// `cargo mto`.
  Cargo,
}

impl Context {
  pub fn id(self) -> &'static str {
    match self {
      Context::Terminal => "terminal",
      Context::Pipe => "pipe",
      Context::Ci => "ci",
      Context::Editor => "editor",
      Context::Cargo => "cargo",
    }
  }

  fn index(self) -> usize {
    CONTEXTS.iter().position(|c| *c == self).unwrap_or(0)
  }

  fn parse(text: &str) -> Option<Context> {
    CONTEXTS.into_iter().find(|c| c.id() == text)
  }
}

/// How a run was asked to treat its files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
  Write,
  Check,
  DryRun,
  /// Read from stdin and printed, as an editor formats a buffer.
  Stdin,
}

impl Mode {
  pub fn id(self) -> &'static str {
    match self {
      Mode::Write => "write",
      Mode::Check => "check",
      Mode::DryRun => "dry_run",
      Mode::Stdin => "stdin",
    }
  }

  fn parse(text: &str) -> Option<Mode> {
    [Mode::Write, Mode::Check, Mode::DryRun, Mode::Stdin]
      .into_iter()
      .find(|m| m.id() == text)
  }
}

/// The counts a run adds up: how many files it took and what came of them, and what it read
/// and changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
  pub files: u64,
  pub changed: u64,
  pub unchanged: u64,
  /// Left alone: layout and formatting off, skipped, held back by the build guard, or
  /// uncommitted with history off.
  pub skipped: u64,
  pub syntax_errors: u64,
  pub failed: u64,
  /// Left as they were because the verifier refused the write: a bug in mto.
  pub refused: u64,
  pub bytes: u64,
  pub lines: u64,
  pub inserted: u64,
  pub removed: u64,
  pub moved: u64,
}

impl Counts {
  const FIELDS: usize = 12;

  fn values(&self) -> [u64; Self::FIELDS] {
    [
      self.files,
      self.changed,
      self.unchanged,
      self.skipped,
      self.syntax_errors,
      self.failed,
      self.refused,
      self.bytes,
      self.lines,
      self.inserted,
      self.removed,
      self.moved,
    ]
  }

  fn from_values(v: &[u64]) -> Option<Counts> {
    let v: [u64; Self::FIELDS] = v.try_into().ok()?;
    Some(Counts {
      files: v[0],
      changed: v[1],
      unchanged: v[2],
      skipped: v[3],
      syntax_errors: v[4],
      failed: v[5],
      refused: v[6],
      bytes: v[7],
      lines: v[8],
      inserted: v[9],
      removed: v[10],
      moved: v[11],
    })
  }

  fn add(&mut self, other: &Counts) {
    let sum: Vec<u64> = self
      .values()
      .iter()
      .zip(other.values())
      .map(|(a, b)| a.saturating_add(b))
      .collect();
    *self = Counts::from_values(&sum).unwrap_or(*self);
  }
}

/// The statistics kept in `dir`.
pub struct Ledger {
  dir: PathBuf,
}

impl Ledger {
  pub fn new(dir: PathBuf) -> Ledger {
    Ledger { dir }
  }

  pub fn dir(&self) -> &Path {
    &self.dir
  }

  /// Appends `run` to its day's file; then folds what has aged, unless another run is
  /// folding. Errors are passed over: statistics never fail a run.
  pub fn record(&self, run: &Run) {
    let _ = self.append(run);
    self.fold(day_number(run.millis));
  }

  fn append(&self, run: &Run) -> std::io::Result<()> {
    let runs = self.dir.join("runs");
    fs::create_dir_all(&runs)?;
    let path = runs.join(format!("{}.tsv", date(day_number(run.millis))));
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    // One write per line, appended: runs at once each keep their line whole.
    let mut text = String::new();
    if file.metadata()?.len() == 0 {
      text.push_str(HEADER);
      text.push('\n');
    }
    text.push_str(&run.line());
    text.push('\n');
    file.write_all(text.as_bytes())
  }

  /// The salt project hashes are made with, made once per machine.
  pub fn salt(&self) -> Option<String> {
    let path = self.dir.join("salt");
    if let Ok(salt) = fs::read_to_string(&path) {
      let salt = salt.trim().to_string();
      if !salt.is_empty() {
        return Some(salt);
      }
    }
    fs::create_dir_all(&self.dir).ok()?;
    let salt = crate::history::id::new();
    let tmp = self.dir.join(format!(".salt.{}", std::process::id()));
    fs::write(&tmp, &salt).ok()?;
    // Two runs making one at once: the first renamed in wins, and both read it back.
    if fs::hard_link(&tmp, &path).is_err() && !path.exists() {
      let _ = fs::rename(&tmp, &path);
    }
    let _ = fs::remove_file(&tmp);
    fs::read_to_string(&path).ok().map(|s| s.trim().to_string())
  }

  /// A project's hash: its folder's path with this machine's salt, 12 hex digits.
  pub fn project(&self, root: &Path) -> Option<String> {
    let salt = self.salt()?;
    let text = format!("{}\0{}", salt, root.display());
    Some(crate::history::sha256::hex(text.as_bytes())[..12].to_string())
  }

  /// Folds every period past its tier's keeping, as of day `today`.
  pub fn fold(&self, today: i64) {
    let Some(_lock) = Lock::take(&self.dir) else {
      return;
    };
    self.fold_runs(today);
    self.fold_tier("days", "months", today, KEEP_DAYS_DAYS, |period| {
      let (year, month) = period.split_once('-')?;
      let (year, month) = (year.parse::<i64>().ok()?, month.parse::<u32>().ok()?);
      // The month's last day: its next's first, less one.
      let (year, month) = if month == 12 {
        (year + 1, 1)
      } else {
        (year, month + 1)
      };
      Some((days_from_civil(year, month, 1) - 1, period[..4].to_string()))
    });
    self.fold_tier("months", "years", today, KEEP_MONTHS_DAYS, |period| {
      let year = period.get(..4)?.parse::<i64>().ok()?;
      Some((days_from_civil(year + 1, 1, 1) - 1, String::new()))
    });
  }

  /// Each day of runs older than [`KEEP_RUNS_DAYS`] summed into the day tier, then removed.
  fn fold_runs(&self, today: i64) {
    let Ok(entries) = fs::read_dir(self.dir.join("runs")) else {
      return;
    };
    let mut old: Vec<(i64, PathBuf)> = entries
      .flatten()
      .filter_map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        let day = parse_date(name.strip_suffix(".tsv")?)?;
        (day <= today - KEEP_RUNS_DAYS).then(|| (day, e.path()))
      })
      .collect();
    old.sort();
    for (day, path) in old {
      let Ok(text) = fs::read_to_string(&path) else {
        continue;
      };
      let period = date(day);
      let totals = sum(
        text
          .lines()
          .filter_map(Run::parse)
          .map(|r| Total::of(&r, &period)),
      );
      let target = self.dir.join("days").join(format!("{}.tsv", &period[..7]));
      if self.merge(&target, &period, totals).is_ok() {
        let _ = fs::remove_file(&path);
      }
    }
  }

  /// Each file of tier `from` whose period ended `keep` days or more before `today` summed
  /// into tier `to`, then removed. `ended` gives a file's period's last day and the name of
  /// the file above it holds (empty: `years.tsv`).
  fn fold_tier(
    &self,
    from: &str,
    to: &str,
    today: i64,
    keep: i64,
    ended: impl Fn(&str) -> Option<(i64, String)>,
  ) {
    let Ok(entries) = fs::read_dir(self.dir.join(from)) else {
      return;
    };
    let mut old: Vec<(String, PathBuf, String)> = entries
      .flatten()
      .filter_map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        let period = name.strip_suffix(".tsv")?.to_string();
        let (last, above) = ended(&period)?;
        (last <= today - keep).then(|| (period, e.path(), above))
      })
      .collect();
    old.sort();
    for (period, path, above) in old {
      let Ok(text) = fs::read_to_string(&path) else {
        continue;
      };
      let totals = sum(text.lines().filter_map(Total::parse).map(|mut t| {
        t.period = period.clone();
        t
      }));
      let target = if above.is_empty() {
        self.dir.join("years.tsv")
      } else {
        self.dir.join(to).join(format!("{}.tsv", above))
      };
      if self.merge(&target, &period, totals).is_ok() {
        let _ = fs::remove_file(&path);
      }
    }
  }

  /// `totals`, all of `period`, added to the file `target`, unless it holds that period
  /// already (a fold cut short after writing it): written whole under a temporary name and
  /// renamed into place.
  fn merge(&self, target: &Path, period: &str, totals: Vec<Total>) -> std::io::Result<()> {
    let existing = fs::read_to_string(target).unwrap_or_default();
    let mut lines: Vec<Total> = existing.lines().filter_map(Total::parse).collect();
    if lines.iter().any(|t| t.period == period) {
      return Ok(());
    }
    lines.extend(totals);
    lines.sort_by(|a, b| (&a.period, a.mode, &a.version).cmp(&(&b.period, b.mode, &b.version)));
    let mut text = format!("{}\n", HEADER);
    for total in &lines {
      text.push_str(&total.line());
      text.push('\n');
    }
    if let Some(parent) = target.parent() {
      fs::create_dir_all(parent)?;
    }
    let tmp = target.with_extension(format!("tsv.{}.tmp", std::process::id()));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, target).inspect_err(|_| {
      let _ = fs::remove_file(&tmp);
    })
  }

  /// Everything recorded.
  pub fn read(&self) -> Read {
    let mut read = Read::default();
    let mut days: BTreeMap<(String, Mode, String), Total> = BTreeMap::new();
    let mut put = |total: Total| {
      days
        .entry((total.period.clone(), total.mode, total.version.clone()))
        .and_modify(|t| t.add(&total))
        .or_insert(total);
    };
    for text in files_in(&self.dir.join("runs")) {
      for run in text.lines().filter_map(Run::parse) {
        put(Total::of(&run, &date(day_number(run.millis))));
      }
    }
    for text in files_in(&self.dir.join("days")) {
      text.lines().filter_map(Total::parse).for_each(&mut put);
    }
    read.days = days.into_values().collect();
    read.months = files_in(&self.dir.join("months"))
      .iter()
      .flat_map(|text| text.lines().filter_map(Total::parse).collect::<Vec<_>>())
      .collect();
    read.years = fs::read_to_string(self.dir.join("years.tsv"))
      .unwrap_or_default()
      .lines()
      .filter_map(Total::parse)
      .collect();
    read.since = read
      .years
      .iter()
      .chain(&read.months)
      .chain(&read.days)
      .map(|t| t.period.clone())
      .min();
    read
  }

  /// Removes everything, the salt too: projects are told apart afresh after.
  pub fn forget(&self) -> std::io::Result<()> {
    match fs::remove_dir_all(&self.dir) {
      Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
      _ => Ok(()),
    }
  }
}

/// The lock folding takes: a file made only if absent, removed when dropped.
struct Lock(PathBuf);

impl Lock {
  fn take(dir: &Path) -> Option<Lock> {
    fs::create_dir_all(dir).ok()?;
    let path = dir.join(".lock");
    let stale = fs::metadata(&path)
      .and_then(|m| m.modified())
      .ok()
      .and_then(|at| at.elapsed().ok())
      .is_some_and(|age| age > STALE_LOCK);
    if stale {
      let _ = fs::remove_file(&path);
    }
    OpenOptions::new()
      .write(true)
      .create_new(true)
      .open(&path)
      .ok()
      .map(|_| Lock(path))
  }
}

impl Drop for Lock {
  fn drop(&mut self) {
    let _ = fs::remove_file(&self.0);
  }
}

/// Everything a ledger holds, each tier's lines as totals; raw runs are summed per day.
#[derive(Debug, Default)]
pub struct Read {
  /// Day totals, from the runs not yet folded and from the day tier, by day.
  pub days: Vec<Total>,
  pub months: Vec<Total>,
  pub years: Vec<Total>,
  /// The first day anything was recorded, if anything was.
  pub since: Option<String>,
}

/// What one run did, as counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
  /// When it ended, in milliseconds since the Unix epoch.
  pub millis: u64,
  pub version: String,
  /// The system and processor, `macos-aarch64`.
  pub system: String,
  pub mode: Mode,
  pub context: Context,
  /// The edition the run's settings gave, `2024`.
  pub edition: String,
  pub exit: i32,
  /// The project's salted hash, if the run named one.
  pub project: Option<String>,
  pub counts: Counts,
  /// How long the run took.
  pub wall_ms: u64,
  /// The worker threads it ran.
  pub threads: u64,
  /// Whether layout was on for any of its files.
  pub layout: bool,
}

impl Run {
  fn line(&self) -> String {
    let mut fields = vec![
      self.millis.to_string(),
      self.version.clone(),
      self.system.clone(),
      self.mode.id().to_string(),
      self.context.id().to_string(),
      self.edition.clone(),
      self.exit.to_string(),
      self.project.clone().unwrap_or_else(|| "-".to_string()),
    ];
    fields.extend(self.counts.values().iter().map(u64::to_string));
    fields.push(self.wall_ms.to_string());
    fields.push(self.threads.to_string());
    fields.push(u64::from(self.layout).to_string());
    fields.join("\t")
  }

  fn parse(line: &str) -> Option<Run> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 8 + Counts::FIELDS + 3 {
      return None;
    }
    let number = |text: &str| text.parse::<u64>().ok();
    let counts: Vec<u64> = fields[8..8 + Counts::FIELDS]
      .iter()
      .map(|f| number(f))
      .collect::<Option<_>>()?;
    let rest = &fields[8 + Counts::FIELDS..];
    Some(Run {
      millis: number(fields[0])?,
      version: fields[1].to_string(),
      system: fields[2].to_string(),
      mode: Mode::parse(fields[3])?,
      context: Context::parse(fields[4])?,
      edition: fields[5].to_string(),
      exit: fields[6].parse().ok()?,
      project: (fields[7] != "-").then(|| fields[7].to_string()),
      counts: Counts::from_values(&counts)?,
      wall_ms: number(rest[0])?,
      threads: number(rest[1])?,
      layout: rest[2] == "1",
    })
  }
}

/// Everything the runs of one period, mode and version add up to: a line of a tier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Total {
  /// `2026-11-07`, `2026-11` or `2026`.
  pub period: String,
  pub mode: Mode,
  pub version: String,
  pub runs: u64,
  pub counts: Counts,
  pub wall_ms: u64,
  /// The longest run.
  pub wall_max_ms: u64,
  /// Runs by how long they took ([`bucket`]).
  pub histogram: [u64; BUCKETS],
  /// Runs by where they were started, in [`CONTEXTS`] order.
  pub contexts: [u64; 5],
  /// Runs with layout on.
  pub layout_runs: u64,
  pub projects: BTreeSet<String>,
}

impl Total {
  fn empty(period: &str, mode: Mode, version: &str) -> Total {
    Total {
      period: period.to_string(),
      mode,
      version: version.to_string(),
      runs: 0,
      counts: Counts::default(),
      wall_ms: 0,
      wall_max_ms: 0,
      histogram: [0; BUCKETS],
      contexts: [0; 5],
      layout_runs: 0,
      projects: BTreeSet::new(),
    }
  }

  /// One run's line, as a total of one.
  pub fn of(run: &Run, period: &str) -> Total {
    let mut total = Total::empty(period, run.mode, &run.version);
    total.runs = 1;
    total.counts = run.counts;
    total.wall_ms = run.wall_ms;
    total.wall_max_ms = run.wall_ms;
    total.histogram[bucket(run.wall_ms)] = 1;
    total.contexts[run.context.index()] = 1;
    total.layout_runs = u64::from(run.layout);
    total.projects.extend(run.project.clone());
    total
  }

  /// `other` added to this one.
  pub fn add(&mut self, other: &Total) {
    self.runs += other.runs;
    self.counts.add(&other.counts);
    self.wall_ms = self.wall_ms.saturating_add(other.wall_ms);
    self.wall_max_ms = self.wall_max_ms.max(other.wall_max_ms);
    for (a, b) in self.histogram.iter_mut().zip(other.histogram) {
      *a += b;
    }
    for (a, b) in self.contexts.iter_mut().zip(other.contexts) {
      *a += b;
    }
    self.layout_runs += other.layout_runs;
    self.projects.extend(other.projects.iter().cloned());
  }

  /// The run time below which `share` of the runs fall, from the histogram: a bucket's
  /// upper bound, so within its width (about 40%) of the true value.
  pub fn percentile_ms(&self, share: f64) -> u64 {
    let wanted = ((self.runs as f64) * share).ceil().max(1.0) as u64;
    let mut seen = 0;
    for (at, count) in self.histogram.iter().enumerate() {
      seen += count;
      if seen >= wanted {
        return bound(at).min(self.wall_max_ms);
      }
    }
    self.wall_max_ms
  }

  fn line(&self) -> String {
    let histogram: Vec<String> = self
      .histogram
      .iter()
      .enumerate()
      .filter(|(_, n)| **n > 0)
      .map(|(at, n)| format!("{}:{}", at, n))
      .collect();
    let mut fields = vec![
      self.period.clone(),
      self.mode.id().to_string(),
      self.version.clone(),
      self.runs.to_string(),
    ];
    fields.extend(self.counts.values().iter().map(u64::to_string));
    fields.push(self.wall_ms.to_string());
    fields.push(self.wall_max_ms.to_string());
    fields.push(histogram.join(","));
    fields.push(
      self
        .contexts
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(","),
    );
    fields.push(self.layout_runs.to_string());
    fields.push(self.projects.iter().cloned().collect::<Vec<_>>().join(","));
    fields.join("\t")
  }

  fn parse(line: &str) -> Option<Total> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 4 + Counts::FIELDS + 6 {
      return None;
    }
    let number = |text: &str| text.parse::<u64>().ok();
    let mut total = Total::empty(fields[0], Mode::parse(fields[1])?, fields[2]);
    total.runs = number(fields[3])?;
    let counts: Vec<u64> = fields[4..4 + Counts::FIELDS]
      .iter()
      .map(|f| number(f))
      .collect::<Option<_>>()?;
    total.counts = Counts::from_values(&counts)?;
    let rest = &fields[4 + Counts::FIELDS..];
    total.wall_ms = number(rest[0])?;
    total.wall_max_ms = number(rest[1])?;
    for pair in rest[2].split(',').filter(|p| !p.is_empty()) {
      let (at, n) = pair.split_once(':')?;
      *total.histogram.get_mut(at.parse::<usize>().ok()?)? = number(n)?;
    }
    for (at, n) in rest[3].split(',').enumerate() {
      *total.contexts.get_mut(at)? = number(n)?;
    }
    total.layout_runs = number(rest[4])?;
    total.projects = rest[5]
      .split(',')
      .filter(|p| !p.is_empty())
      .map(str::to_string)
      .collect();
    Some(total)
  }
}

/// The upper bound of bucket `at`, in milliseconds: from 1 ms, each about 1.41 times the
/// one before, the last ten minutes.
pub fn bound(at: usize) -> u64 {
  let ratio = 600_000f64.powf(1.0 / (BUCKETS as f64 - 1.0));
  ratio.powi(at as i32).round().max(1.0) as u64
}

/// The bucket a duration falls in.
pub fn bucket(ms: u64) -> usize {
  (0..BUCKETS)
    .find(|&at| ms <= bound(at))
    .unwrap_or(BUCKETS - 1)
}

/// Day `day` as `YYYY-MM-DD`.
pub fn date(day: i64) -> String {
  let (year, month, d) = civil_from_days(day);
  format!("{:04}-{:02}-{:02}", year, month, d)
}

/// The UTC day `millis` falls on, as days since 1970-01-01.
pub fn day_number(millis: u64) -> i64 {
  (millis / 86_400_000) as i64
}

/// Milliseconds since the Unix epoch, now.
pub fn now_millis() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0)
}

/// `YYYY-MM-DD` as days since 1970-01-01.
pub fn parse_date(text: &str) -> Option<i64> {
  let mut parts = text.split('-');
  let year = parts.next()?.parse::<i64>().ok()?;
  let month = parts.next()?.parse::<u32>().ok()?;
  let day = parts.next()?.parse::<u32>().ok()?;
  if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
    return None;
  }
  Some(days_from_civil(year, month, day))
}

/// Days since 1970-01-01 split into (year, month, day), proleptic Gregorian (Howard
/// Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
  let z = days + 719_468;
  let era = z.div_euclid(146_097);
  let doe = z.rem_euclid(146_097);
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
  let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
  let year = yoe + era * 400 + i64::from(month <= 2);
  (year, month, day)
}

/// (year, month, day) as days since 1970-01-01.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
  let year = if month <= 2 { year - 1 } else { year };
  let era = year.div_euclid(400);
  let yoe = year.rem_euclid(400);
  let month = i64::from(month);
  let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
  let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  era * 146_097 + doe - 719_468
}

/// The text of each file in `dir`, in name order.
fn files_in(dir: &Path) -> Vec<String> {
  let Ok(entries) = fs::read_dir(dir) else {
    return Vec::new();
  };
  let mut paths: Vec<PathBuf> = entries
    .flatten()
    .map(|e| e.path())
    .filter(|p| p.extension().is_some_and(|e| e == "tsv"))
    .collect();
  paths.sort();
  paths
    .iter()
    .filter_map(|p| fs::read_to_string(p).ok())
    .collect()
}

/// Totals summed by mode and version.
fn sum(totals: impl Iterator<Item = Total>) -> Vec<Total> {
  let mut by: BTreeMap<(Mode, String), Total> = BTreeMap::new();
  for total in totals {
    by.entry((total.mode, total.version.clone()))
      .and_modify(|t| t.add(&total))
      .or_insert(total);
  }
  by.into_values().collect()
}

#[cfg(test)]
mod tests;
