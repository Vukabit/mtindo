//! `doctor-bench DIR...`: what the build guard costs mto, and how quickly a narrowing finds
//! what to hold back, measured on the corpus's files with a pretend build. Prints each
//! figure as `metric NAME VALUE UNIT` for `cargo xtask speed doctor` to keep.
//!
//! - **Shaping:** every file laid out and formatted as an ordinary run does, then again
//!   holding back two moves and a run of lines by content, as remembered holds are; the
//!   second over the first is what a file with holds costs.
//! - **Narrowing:** `n` files drawn from the corpus, `k` of their moves planted as culprits.
//!   The pretend build breaks while a culprit's item comes before the item it was moved in
//!   front of, so it costs nothing to run and the figures are mto's own: how many checks a
//!   narrowing takes, how long mto spends between them, and whether it ends holding back
//!   no more than the `k` culprits. The draws are seeded, so the figures repeat.
//!   `DOCTOR_TRACE=1` prints each check and what each narrowing held back.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mtindo::doctor::{File, Held, Tried, hunks, move_id, narrow};
use mtindo::layout;
use mtindo::options::format::FormatConfig;
use mtindo::{LayoutConfig, Settings, shape};

/// (files, culprits) of each narrowing measured.
const SIZES: &[(usize, usize)] = &[(10, 1), (10, 3), (100, 1), (100, 3)];

/// A move that can be planted as a culprit: its item's text and the text of the item it was
/// moved in front of, each found once in the file as laid out.
#[derive(Clone)]
struct Plant {
  item: String,
  passed: String,
}

impl Plant {
  /// Whether the pretend build breaks on `text`: the item comes before the one it passed.
  fn breaks(&self, text: &str) -> bool {
    matches!((text.find(&self.item), text.find(&self.passed)), (Some(i), Some(p)) if i < p)
  }
}

/// A seeded xorshift, so the draws repeat from run to run.
struct Random(u64);

impl Random {
  fn next(&mut self) -> u64 {
    self.0 ^= self.0 << 13;
    self.0 ^= self.0 >> 7;
    self.0 ^= self.0 << 17;
    self.0
  }

  /// `count` distinct numbers below `below`, in the order drawn.
  fn pick(&mut self, below: usize, count: usize) -> Vec<usize> {
    let mut all: Vec<usize> = (0..below).collect();
    for at in 0..count.min(below) {
      let swap = at + (self.next() % (below - at) as u64) as usize;
      all.swap(at, swap);
    }
    all.truncate(count.min(below));
    all
  }
}

pub fn run(args: &[String]) -> std::process::ExitCode {
  let mut trials = 5;
  let mut dirs = Vec::new();
  let mut at = 0;
  while at < args.len() {
    if args[at] == "--trials" {
      trials = args.get(at + 1).and_then(|n| n.parse().ok()).unwrap_or(5);
      at += 2;
      continue;
    }
    dirs.push(PathBuf::from(&args[at]));
    at += 1;
  }
  let mut paths = Vec::new();
  for dir in &dirs {
    crate::corpus::collect(dir, &mut paths);
  }
  paths.sort();
  let sources: Vec<(PathBuf, String)> = paths
    .into_iter()
    .filter_map(|p| fs::read_to_string(&p).ok().map(|t| (p, t)))
    .collect();
  if sources.is_empty() {
    eprintln!("doctor-bench: no .rs files under {:?}", dirs);
    return std::process::ExitCode::from(2);
  }
  shaping(&sources);
  narrowing(&sources, trials);
  std::process::ExitCode::SUCCESS
}

fn metric(name: &str, value: f64, unit: &str) {
  println!("metric {} {} {}", name, value, unit);
}

fn narrowing(sources: &[(PathBuf, String)], trials: usize) {
  // Generated files laid out too, as `layout::layout` lays out every file the moves are
  // drawn from.
  let settings = Arc::new(Settings {
    layout: Some(LayoutConfig {
      layout_generated_files: true,
      ..LayoutConfig::default()
    }),
    format: None,
    edition: mtindo::syntax::Edition::E2021,
  });
  // The files with moves that can be planted, each with those moves.
  let mut pool: Vec<(usize, Vec<Plant>)> = Vec::new();
  for (i, (_, source)) in sources.iter().enumerate() {
    let Ok(laid) = layout::layout(source, &LayoutConfig::default()) else {
      continue;
    };
    let once = |t: &str| laid.text.matches(t).count() == 1 && source.matches(t).count() == 1;
    let plants: Vec<Plant> = laid
      .moves
      .iter()
      .filter_map(|m| {
        let at = laid.after.iter().position(|u| u.span == m.span)?;
        let (item, passed) = (&laid.after[at], laid.after.get(at + 1)?);
        // Moved up past it: as written, it came after the item it now precedes.
        (item.index > passed.index && once(&item.text) && once(&passed.text)).then(|| Plant {
          item: item.text.clone(),
          passed: passed.text.clone(),
        })
      })
      .collect();
    if !plants.is_empty() {
      pool.push((i, plants));
    }
  }
  println!("narrowing: {} corpus files have moves to plant", pool.len());
  let mut random = Random(0x5eed_d0c7);
  let mut found_all = (0usize, 0usize);
  for &(n, k) in SIZES {
    if pool.len() < n {
      continue;
    }
    let (mut checks, mut time, mut passed, mut extra, mut edits) =
      (0usize, Duration::ZERO, 0usize, 0usize, 0usize);
    for _ in 0..trials {
      let chosen = random.pick(pool.len(), n);
      let mut files: Vec<File> = chosen
        .iter()
        .map(|&p| {
          let (path, source) = &sources[pool[p].0];
          File {
            path: path.clone(),
            source: source.clone(),
            settings: settings.clone(),
            held: Held::default(),
          }
        })
        .collect();
      let candidates: Vec<(usize, &Plant)> = chosen
        .iter()
        .enumerate()
        .flat_map(|(slot, &p)| pool[p].1.iter().map(move |plant| (slot, plant)))
        .collect();
      edits += candidates.len();
      let culprits: Vec<(usize, Plant)> = random
        .pick(candidates.len(), k.min(candidates.len()))
        .into_iter()
        .map(|c| (candidates[c].0, candidates[c].1.clone()))
        .collect();
      let breaks = |texts: &[String]| {
        culprits
          .iter()
          .any(|(slot, plant)| plant.breaks(&texts[*slot]))
      };
      let mut runs = 0;
      let started = Instant::now();
      let narrowed = narrow(
        &mut files,
        Some(Tried {
          broke: true,
          errors: Vec::new(),
        }),
        &mut |texts: &[String]| {
          runs += 1;
          let broke = breaks(texts);
          if std::env::var_os("DOCTOR_TRACE").is_some() {
            let applied = texts
              .iter()
              .enumerate()
              .filter(|(i, t)| **t != sources[pool[chosen[*i]].0].1)
              .count();
            eprintln!(
              "    check {}: {} files changed, broke {}",
              runs, applied, broke
            );
          }
          Some(Tried {
            broke,
            errors: Vec::new(),
          })
        },
      );
      time += started.elapsed();
      checks += runs;
      if std::env::var_os("DOCTOR_TRACE").is_some() {
        for f in &narrowed.found {
          eprintln!("    held: {} {}", files[f.file].path.display(), f.what);
        }
        for (slot, plant) in &culprits {
          eprintln!(
            "    culprit in {}: {}",
            files[*slot].path.display(),
            plant.item.lines().last().unwrap_or("")
          );
        }
      }
      if narrowed.passed && narrowed.found.len() <= k {
        passed += 1;
      }
      extra += narrowed.found.len().saturating_sub(k);
    }
    let mean = |x: f64| x / trials as f64;
    println!(
      "  {:>3} files, {} culprit(s): {:.1} checks, {:.0} ms of mto between them, {}/{} \
       held back no more than the culprits ({:.1} extra holds; {:.0} plantable moves)",
      n,
      k,
      mean(checks as f64),
      mean(time.as_secs_f64() * 1000.0),
      passed,
      trials,
      mean(extra as f64),
      mean(edits as f64)
    );
    let name = format!("doctor.narrow.n{}.k{}", n, k);
    metric(
      &format!("{}.checks", name),
      round(mean(checks as f64), 2),
      "checks",
    );
    metric(
      &format!("{}.seconds", name),
      round(mean(time.as_secs_f64()), 4),
      "s",
    );
    found_all.0 += passed;
    found_all.1 += trials;
  }
  if found_all.1 > 0 {
    metric(
      "doctor.narrow.found_rate",
      round(found_all.0 as f64 / found_all.1 as f64, 3),
      "",
    );
  }
}

fn round(value: f64, places: i32) -> f64 {
  let scale = 10f64.powi(places);
  (value * scale).round() / scale
}

/// An ordinary run's shaping against one with remembered holds.
fn shaping(sources: &[(PathBuf, String)]) {
  let settings = Settings {
    layout: Some(LayoutConfig::default()),
    format: Some(FormatConfig::default()),
    edition: mtindo::syntax::Edition::E2021,
  };
  let (mut bytes, mut plain, mut holding) = (0usize, Duration::ZERO, Duration::ZERO);
  let (mut files, mut moves, mut runs) = (0usize, 0usize, 0usize);
  for (_, source) in sources {
    let started = Instant::now();
    let Ok(free) = shape(source, &settings, &Held::default(), None) else {
      continue;
    };
    plain += started.elapsed();
    bytes += source.len();
    files += 1;
    let changed = hunks(&free.laid_out, &free.text);
    moves += free.moves.len();
    runs += changed.len();
    let held = Held {
      move_ids: free
        .moves
        .iter()
        .take(2)
        .map(|m| move_id(source, m))
        .collect(),
      lines: changed.iter().take(1).map(|h| h.id.clone()).collect(),
      ..Held::default()
    };
    let started = Instant::now();
    let _ = shape(source, &settings, &held, None);
    holding += started.elapsed();
  }
  let mb_per_s = bytes as f64 / plain.as_secs_f64().max(1e-9) / 1e6;
  let cost = holding.as_secs_f64() / plain.as_secs_f64().max(1e-9);
  println!(
    "shaping: {} files, {:.1} MB: {:.1} MB/s on one thread; with remembered holds {:.2}x \
     as long; {:.1} moves and {:.1} runs of changed lines a file",
    files,
    bytes as f64 / 1e6,
    mb_per_s,
    cost,
    moves as f64 / files.max(1) as f64,
    runs as f64 / files.max(1) as f64
  );
  metric("doctor.shape.mb_per_s", round(mb_per_s, 2), "MB/s");
  metric("doctor.held.cost.x", round(cost, 3), "x");
  metric(
    "doctor.edits.per_file",
    round((moves + runs) as f64 / files.max(1) as f64, 2),
    "edits",
  );
}
