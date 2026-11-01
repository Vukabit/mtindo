//! Timing: one way to repeat a measurement and read its runs, for every command that
//! measures speed. Each command keeps the statistic its figures are recorded by: the
//! middle run of a few for the slow ones (a whole project, `cargo check`), the median with
//! its spread for rounds of a race, the fastest of enough runs for microbenchmarks, a
//! percentile for latency.

use std::time::{Duration, Instant};

/// The fastest of as many runs of `f` as take `total`, and at least `runs`, in seconds.
pub fn fastest_of(runs: usize, total: Duration, mut f: impl FnMut()) -> f64 {
  let mut best = f64::MAX;
  let mut spent = Duration::ZERO;
  let mut done = 0;
  while done < runs || spent < total {
    let started = Instant::now();
    f();
    let took = started.elapsed();
    best = best.min(took.as_secs_f64());
    spent += took;
    done += 1;
  }
  best
}

/// The median of `samples` (the mean of the two middle ones for an even count) and their
/// spread: half the range, as a percentage of the median. `None` when there are none.
pub fn median_spread(samples: &[f64]) -> Option<(f64, f64)> {
  if samples.is_empty() {
    return None;
  }
  let mut sorted = samples.to_vec();
  sorted.sort_by(|a, b| a.total_cmp(b));
  let mid = sorted.len() / 2;
  let median = if sorted.len().is_multiple_of(2) {
    (sorted[mid - 1] + sorted[mid]) / 2.0
  } else {
    sorted[mid]
  };
  let spread = if median > 0.0 {
    (sorted[sorted.len() - 1] - sorted[0]) / 2.0 / median * 100.0
  } else {
    0.0
  };
  Some((median, spread))
}

/// The middle of `values` ([`middle_by`]); 0 when there are none.
pub fn middle(values: Vec<f64>) -> f64 {
  middle_by(values, |v| *v).unwrap_or(0.0)
}

/// The middle run of `runs` by `key`, as the upper of the two middle ones for an even
/// count: of three runs, the second fastest. `None` when there are none.
pub fn middle_by<T>(mut runs: Vec<T>, key: impl Fn(&T) -> f64) -> Option<T> {
  if runs.is_empty() {
    return None;
  }
  runs.sort_by(|a, b| key(a).total_cmp(&key(b)));
  let mid = runs.len() / 2;
  Some(runs.swap_remove(mid))
}

/// The value `percent` percent of the way up `samples` sorted, by index: of 100 samples,
/// the 99th percentile is the 100th. 0 when there are none.
pub fn percentile(samples: &[f64], percent: usize) -> f64 {
  let mut sorted = samples.to_vec();
  sorted.sort_by(|a, b| a.total_cmp(b));
  sorted
    .get((sorted.len() * percent / 100).min(sorted.len().saturating_sub(1)))
    .copied()
    .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_the_fastest_takes_at_least_the_runs_asked() {
    let mut calls = 0;
    let best = fastest_of(3, Duration::ZERO, || calls += 1);
    assert_eq!(calls, 3);
    assert!(best >= 0.0);
  }

  #[test]
  fn test_the_statistics_read_runs_as_the_commands_record_them() {
    assert_eq!(middle(vec![3.0, 1.0, 2.0]), 2.0);
    assert_eq!(middle(vec![4.0, 1.0, 3.0, 2.0]), 3.0);
    assert_eq!(middle(Vec::new()), 0.0);
    let pairs = vec![(3.0, "c"), (1.0, "a"), (2.0, "b")];
    assert_eq!(middle_by(pairs, |p| p.0), Some((2.0, "b")));
    let (median, spread) = median_spread(&[0.10, 0.12, 0.11]).unwrap();
    assert!((median - 0.11).abs() < 1e-9);
    assert!((spread - 9.090909).abs() < 1e-3);
    assert_eq!(median_spread(&[1.0, 3.0]).map(|(m, _)| m), Some(2.0));
    assert!(median_spread(&[]).is_none());
    let hundred: Vec<f64> = (1..=100).map(f64::from).collect();
    assert_eq!(percentile(&hundred, 99), 100.0);
    assert_eq!(percentile(&hundred, 50), 51.0);
    assert_eq!(percentile(&[], 99), 0.0);
  }
}
