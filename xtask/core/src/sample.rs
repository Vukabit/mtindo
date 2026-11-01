//! Sampling a running child's memory and CPU: about once a second, the resident memory and
//! the CPU time of the child and every process below it, read from `ps`, written as a table
//! while the child runs.
//!
//! ```text
//! seconds  rss_kb   cpu_seconds  processes
//! 1.0      48212    0.91         3
//! ```
//!
//! CPU time is cumulative user plus system time over the tree's live processes, so a rate
//! is the difference between two rows over their seconds. A process that exits takes its
//! CPU time with it; the memory column is exact at each sample.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::process;
use crate::store;

pub const HEADER: &str = "seconds\trss_kb\tcpu_seconds\tprocesses\n";

/// How often a sample is taken.
pub const INTERVAL: Duration = Duration::from_secs(1);

/// One reading of a process tree.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
  pub seconds: f64,
  pub rss_kb: u64,
  pub cpu_seconds: f64,
  pub processes: usize,
}

/// Samples a process tree on its own thread until stopped.
pub struct Sampler {
  running: Arc<AtomicBool>,
  handle: Option<JoinHandle<Summary>>,
}

impl Sampler {
  /// Starts sampling `pid` and its descendants into `path` (created with its folder).
  pub fn start(pid: u32, path: &Path) -> std::io::Result<Sampler> {
    if let Some(dir) = path.parent() {
      std::fs::create_dir_all(dir)?;
    }
    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(HEADER.as_bytes())?;
    out.flush()?;
    let running = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&running);
    let handle = thread::spawn(move || {
      let started = Instant::now();
      let mut summary = Summary::default();
      let mut next = started + INTERVAL;
      while flag.load(Ordering::Relaxed) {
        // Short sleeps, so stopping is quick.
        if Instant::now() < next {
          thread::sleep(Duration::from_millis(50));
          continue;
        }
        next += INTERVAL;
        let Some(mut sample) = read_tree(pid) else {
          continue;
        };
        sample.seconds = started.elapsed().as_secs_f64();
        summary.add(sample);
        let _ = writeln!(
          out,
          "{:.1}\t{}\t{:.2}\t{}",
          sample.seconds, sample.rss_kb, sample.cpu_seconds, sample.processes
        );
        let _ = out.flush();
      }
      summary
    });
    Ok(Sampler {
      running,
      handle: Some(handle),
    })
  }

  /// Stops sampling; what the run used.
  pub fn stop(mut self) -> Summary {
    self.running.store(false, Ordering::Relaxed);
    self
      .handle
      .take()
      .and_then(|h| h.join().ok())
      .unwrap_or_default()
  }
}

impl Drop for Sampler {
  fn drop(&mut self) {
    self.running.store(false, Ordering::Relaxed);
  }
}

/// What a sampled run used.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
  pub samples: usize,
  pub peak_rss_kb: u64,
  /// When the peak was reached, in seconds from the start.
  pub peak_at: f64,
  /// CPU time over wall time, in cores (2.0 is two cores busy), from the last sample.
  pub mean_cores: f64,
  pub last: Sample,
}

impl Summary {
  /// `peak 1.42 GB at 2m48s, 0.98 cores`, for messages.
  pub fn describe(&self) -> String {
    format!(
      "peak {} at {}, {:.2} cores over {} samples",
      kilobytes(self.peak_rss_kb),
      process::duration(Duration::from_secs_f64(self.peak_at)),
      self.mean_cores,
      self.samples
    )
  }

  fn add(&mut self, sample: Sample) {
    self.samples += 1;
    if sample.rss_kb > self.peak_rss_kb {
      self.peak_rss_kb = sample.rss_kb;
      self.peak_at = sample.seconds;
    }
    if sample.seconds > 0.0 {
      self.mean_cores = sample.cpu_seconds / sample.seconds;
    }
    self.last = sample;
  }
}

/// The memory column of a samples table as a line of at most `width` bars, each the
/// highest of the samples it covers.
pub fn curve(path: &Path, width: usize) -> String {
  const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
  let rows = read(path);
  if rows.is_empty() || width == 0 {
    return String::new();
  }
  let per = rows.len().div_ceil(width);
  let peaks: Vec<u64> = rows
    .chunks(per)
    .map(|c| c.iter().map(|r| r.rss_kb).max().unwrap_or(0))
    .collect();
  let max = peaks.iter().copied().max().unwrap_or(0).max(1);
  peaks
    .iter()
    .map(|p| BARS[(*p as f64 / max as f64 * 7.0).round() as usize])
    .collect()
}

/// `1.42 GB`, `640 MB`, `512 KB`.
pub fn kilobytes(kb: u64) -> String {
  match kb {
    kb if kb >= 1024 * 1024 => format!("{:.2} GB", kb as f64 / (1024.0 * 1024.0)),
    kb if kb >= 1024 => format!("{} MB", kb / 1024),
    kb => format!("{} KB", kb),
  }
}

/// Reads a samples table back.
pub fn read(path: &Path) -> Vec<Sample> {
  std::fs::read_to_string(path)
    .unwrap_or_default()
    .lines()
    .skip(1)
    .filter_map(|line| {
      let f: Vec<&str> = line.split('\t').collect();
      Some(Sample {
        seconds: f.first()?.parse().ok()?,
        rss_kb: f.get(1)?.parse().ok()?,
        cpu_seconds: f.get(2)?.parse().ok()?,
        processes: f.get(3)?.parse().ok()?,
      })
    })
    .collect()
}

/// Where a run's samples go: `.dev/runs/<UTC>-<name>/samples.tsv`.
pub fn run_path(root: &Path, name: &str) -> PathBuf {
  let stamp: String = crate::log::timestamp(std::time::SystemTime::now())
    .chars()
    .take(19)
    .filter(|c| c.is_ascii_digit() || *c == 'T')
    .map(|c| if c == 'T' { '-' } else { c })
    .collect();
  store::dir(root, store::RUNS)
    .join(format!("{}-{}", stamp, name))
    .join("samples.tsv")
}

/// `ps`'s cumulative CPU time in seconds: `1:02.50` (macOS, minutes), `01:02:03` (Linux),
/// `2-01:02:03` (Linux, with days).
fn cpu_time(text: &str) -> Option<f64> {
  let (days, rest) = match text.split_once('-') {
    Some((d, rest)) => (d.parse::<f64>().ok()?, rest),
    None => (0.0, text),
  };
  let mut seconds = 0.0;
  for part in rest.split(':') {
    seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
  }
  Some(days * 86400.0 + seconds)
}

/// The tree under `pid` from one `ps` listing; `None` once `pid` is gone.
fn read_tree(pid: u32) -> Option<Sample> {
  let listing = process::output(
    Command::new("ps")
      .args(["-A", "-o", "pid=,ppid=,rss=,time="])
      .stdout(Stdio::piped())
      .stderr(Stdio::null()),
    process::QUICK,
  )
  .ok()?;
  tree_sample(&String::from_utf8_lossy(&listing.stdout), pid)
}

/// Sums `pid` and its descendants in a `ps -o pid=,ppid=,rss=,time=` listing.
fn tree_sample(listing: &str, pid: u32) -> Option<Sample> {
  let rows: Vec<(u32, u32, u64, f64)> = listing
    .lines()
    .filter_map(|line| {
      let mut words = line.split_whitespace();
      Some((
        words.next()?.parse().ok()?,
        words.next()?.parse().ok()?,
        words.next()?.parse().ok()?,
        cpu_time(words.next()?)?,
      ))
    })
    .collect();
  if !rows.iter().any(|r| r.0 == pid) {
    return None;
  }
  let mut tree = vec![pid];
  let mut idx = 0;
  while idx < tree.len() {
    let parent = tree[idx];
    tree.extend(rows.iter().filter(|r| r.1 == parent).map(|r| r.0));
    idx += 1;
  }
  let mut sample = Sample::default();
  for row in rows.iter().filter(|r| tree.contains(&r.0)) {
    sample.rss_kb += row.2;
    sample.cpu_seconds += row.3;
    sample.processes += 1;
  }
  Some(sample)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_cpu_time_in_every_ps_form() {
    assert_eq!(cpu_time("0:01.50"), Some(1.5));
    assert_eq!(cpu_time("1:02.50"), Some(62.5));
    assert_eq!(cpu_time("01:02:03"), Some(3723.0));
    assert_eq!(cpu_time("2-00:00:01"), Some(172801.0));
    assert_eq!(cpu_time("x"), None);
  }

  #[test]
  fn test_kilobytes() {
    assert_eq!(kilobytes(512), "512 KB");
    assert_eq!(kilobytes(640 * 1024), "640 MB");
    assert_eq!(kilobytes(1_488_978), "1.42 GB");
  }

  #[test]
  fn test_sampling_a_live_child_and_its_children() {
    let path = std::env::temp_dir()
      .join(format!("xtask-sample-{}", std::process::id()))
      .join("samples.tsv");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let args = ["-c".to_string(), "sleep 3 & sleep 3; wait".to_string()];
    let (outcome, summary) =
      process::run_sampled(root, "sh", &args, &[], process::QUICK, Some(&path));
    assert_eq!(outcome.code, 0);
    let summary = summary.unwrap();
    assert!(summary.samples >= 2, "{:?}", summary);

    assert!(summary.peak_rss_kb > 0);
    let rows = read(&path);
    assert_eq!(rows.len(), summary.samples);
    assert_eq!(curve(&path, 40).chars().count(), rows.len().min(40));
    // The shell and both sleeps, while they ran.
    assert!(rows.iter().any(|r| r.processes == 3), "{:?}", rows);
    assert!(rows[1].seconds > rows[0].seconds);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
  }

  #[test]
  fn test_tree_sums_descendants_only() {
    let listing = "  10   1  1000 0:01.00\n  11  10   200 0:00.50\n  12  11    30 0:00.25\n  13   1  9999 9:00.00\n";
    let sample = tree_sample(listing, 10).unwrap();
    assert_eq!((sample.rss_kb, sample.processes), (1230, 3));
    assert!((sample.cpu_seconds - 1.75).abs() < 1e-9);
    assert_eq!(tree_sample(listing, 99), None);
  }
}
