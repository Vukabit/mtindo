use super::*;

/// A folder of its own under the system's temporary folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
  fn new(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!(
      "mtindo-stats-{}-{}-{}",
      name,
      std::process::id(),
      crate::history::id::new()
    ));
    fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
  }
}

impl Drop for Scratch {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.0);
  }
}

/// Midday UTC of `day`, as a run's milliseconds.
fn at(day: &str) -> u64 {
  (parse_date(day).unwrap() as u64) * 86_400_000 + 12 * 3_600_000
}

fn run(day: &str, mode: Mode, wall_ms: u64, changed: u64) -> Run {
  Run {
    millis: at(day),
    version: "0.2.0".to_string(),
    system: "macos-aarch64".to_string(),
    mode,
    context: Context::Terminal,
    edition: "2024".to_string(),
    exit: 0,
    project: Some("3f9a00000000".to_string()),
    counts: Counts {
      files: 10,
      changed,
      unchanged: 10 - changed,
      lines: 1000,
      bytes: 30_000,
      inserted: 4,
      removed: 2,
      ..Counts::default()
    },
    wall_ms,
    threads: 8,
    layout: true,
  }
}

#[test]
fn test_a_fold_another_run_holds_the_lock_for_is_left_to_it() {
  let scratch = Scratch::new("lock");
  let ledger = Ledger::new(scratch.0.clone());
  ledger
    .append(&run("2026-10-01", Mode::Check, 40, 1))
    .unwrap();
  let held = Lock::take(&scratch.0).unwrap();
  ledger.fold(parse_date("2026-11-07").unwrap());
  assert!(scratch.0.join("runs/2026-10-01.tsv").exists());
  drop(held);
  ledger.fold(parse_date("2026-11-07").unwrap());
  assert!(!scratch.0.join("runs/2026-10-01.tsv").exists());
}

#[test]
fn test_a_project_hash_is_salted_per_ledger_and_stable() {
  let (a, b) = (Scratch::new("salt-a"), Scratch::new("salt-b"));
  let (one, two) = (Ledger::new(a.0.clone()), Ledger::new(b.0.clone()));
  let root = Path::new("/home/someone/project");
  let first = one.project(root).unwrap();
  assert_eq!(first.len(), 12);
  assert_eq!(one.project(root), Some(first.clone()));
  assert_ne!(two.project(root), Some(first));
  one.forget().unwrap();
  assert!(!a.0.exists());
}

#[test]
fn test_a_run_and_a_total_read_back_as_written() {
  let one = run("2026-11-07", Mode::Check, 38, 2);
  assert_eq!(Run::parse(&one.line()), Some(one.clone()));
  let mut total = Total::of(&one, "2026-11-07");
  total.add(&Total::of(
    &run("2026-11-07", Mode::Check, 900, 1),
    "2026-11-07",
  ));
  assert_eq!(Total::parse(&total.line()), Some(total.clone()));
  assert_eq!(total.runs, 2);
  assert_eq!(total.counts.changed, 3);
  assert_eq!(total.wall_max_ms, 900);
  // A line of another shape is passed over, not misread.
  assert_eq!(Run::parse("1\t2\t3"), None);
  assert_eq!(Total::parse(HEADER), None);
}

#[test]
fn test_dates_count_days_from_1970_both_ways() {
  assert_eq!(parse_date("1970-01-01"), Some(0));
  assert_eq!(date(0), "1970-01-01");
  assert_eq!(date(20_399), "2025-11-07");
  for day in [-1000, 0, 59, 60, 11_016, 20_764, 40_000] {
    assert_eq!(parse_date(&date(day)), Some(day));
  }
  assert_eq!(parse_date("2026-13-01"), None);
  assert_eq!(parse_date("2026-11"), None);
}

#[test]
fn test_durations_fall_in_rising_buckets_from_a_millisecond_to_ten_minutes() {
  assert_eq!(bound(0), 1);
  assert_eq!(bound(BUCKETS - 1), 600_000);
  assert!((1..BUCKETS).all(|at| bound(at) >= bound(at - 1)));
  assert_eq!(bucket(0), 0);
  assert_eq!(bucket(1), 0);
  assert_eq!(bucket(10_000_000), BUCKETS - 1);
  // A percentile is its bucket's bound: within the bucket's width of the true value.
  let mut total = Total::of(&run("2026-11-07", Mode::Stdin, 10, 0), "2026-11-07");
  for _ in 0..18 {
    total.add(&Total::of(
      &run("2026-11-07", Mode::Stdin, 10, 0),
      "2026-11-07",
    ));
  }
  total.add(&Total::of(
    &run("2026-11-07", Mode::Stdin, 400, 0),
    "2026-11-07",
  ));
  let p50 = total.percentile_ms(0.5);
  assert!((10..=15).contains(&p50), "{}", p50);
  assert_eq!(total.percentile_ms(1.0), 400);
}

#[test]
fn test_old_runs_fold_into_days_months_and_years_counting_each_once() {
  let scratch = Scratch::new("fold");
  let ledger = Ledger::new(scratch.0.clone());
  for day in [
    "2023-05-02",
    "2023-05-03",
    "2026-07-01",
    "2026-10-20",
    "2026-11-06",
  ] {
    ledger.append(&run(day, Mode::Check, 40, 1)).unwrap();
    ledger.append(&run(day, Mode::Write, 60, 1)).unwrap();
  }
  let before = ledger.read();
  let runs = |read: &Read| -> u64 {
    read
      .days
      .iter()
      .chain(&read.months)
      .chain(&read.years)
      .map(|t| t.runs)
      .sum()
  };
  assert_eq!(runs(&before), 10);
  let today = parse_date("2026-11-07").unwrap();
  ledger.fold(today);
  // Six days or younger stay as runs; older days fold up to the tier their age calls for.
  assert!(scratch.0.join("runs/2026-11-06.tsv").exists());
  assert!(!scratch.0.join("runs/2026-10-20.tsv").exists());
  assert!(scratch.0.join("days/2026-10.tsv").exists());
  assert!(!scratch.0.join("days/2026-07.tsv").exists());
  assert!(scratch.0.join("months/2026.tsv").exists());
  assert!(scratch.0.join("years.tsv").exists());
  let after = ledger.read();
  assert_eq!(runs(&after), 10);
  let year = after.years.iter().find(|t| t.mode == Mode::Check).unwrap();
  assert_eq!(
    (year.period.as_str(), year.runs, year.counts.changed),
    ("2023", 2, 2)
  );
  // Folding again, or a fold cut short after writing a tier, counts nothing twice.
  ledger.fold(today);
  ledger
    .append(&run("2026-10-20", Mode::Check, 40, 1))
    .unwrap();
  ledger.fold(today);
  assert_eq!(runs(&ledger.read()), 10);
  assert!(!scratch.0.join("runs/2026-10-20.tsv").exists());
}

#[test]
fn test_runs_recorded_read_back_summed_per_day_and_mode() {
  let scratch = Scratch::new("read");
  let ledger = Ledger::new(scratch.0.clone());
  ledger.record(&run("2026-11-07", Mode::Check, 30, 1));
  ledger.record(&run("2026-11-07", Mode::Check, 50, 0));
  ledger.record(&run("2026-11-07", Mode::Stdin, 9, 1));
  let read = ledger.read();
  assert_eq!(read.since.as_deref(), Some("2026-11-07"));
  let check = read.days.iter().find(|t| t.mode == Mode::Check).unwrap();
  assert_eq!(
    (check.runs, check.counts.changed, check.wall_ms),
    (2, 1, 80)
  );
  assert_eq!(read.days.len(), 2);
  let text = fs::read_to_string(scratch.0.join("runs/2026-11-07.tsv")).unwrap();
  assert!(text.starts_with(HEADER));
  assert_eq!(text.lines().count(), 4);
}
