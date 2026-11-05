//! How far a run over many files has got, so a long one never looks hung: on a terminal,
//! one line on stderr, redrawn in place and cleared before anything else is printed,
//!
//! ```text
//! formatting  ██████████░░░░░░░░░░  4210 of 8420 files  50%  about 12s left
//! ```
//!
//! It shows only once a run has taken a second, so a quick one never flickers, and nowhere
//! but a terminal: a log or CI gets only what mto always prints.

use std::time::{Duration, Instant};

use crate::text::terminal::{self, Stream};

/// The bar's width, in cells.
const BAR: usize = 20;

/// How long a run goes before its progress shows.
const DELAY: Duration = Duration::from_secs(1);

/// The least time between two redraws.
const REDRAW: Duration = Duration::from_millis(100);

pub struct Progress {
  label: &'static str,
  total: usize,
  started: Instant,
  /// When the line was last drawn, and for how many files; `None` while it is not on the
  /// screen.
  drawn: Option<(Instant, usize)>,
}

impl Progress {
  /// Progress over `total` files, labelled `label`: `None` unless stderr is a terminal
  /// that can redraw a line, and there is more than one file.
  pub fn new(label: &'static str, total: usize) -> Option<Progress> {
    if total < 2 || !Stream::Err.is_terminal() || !terminal::redraws() {
      return None;
    }
    Some(Progress {
      label,
      total,
      started: Instant::now(),
      drawn: None,
    })
  }

  /// `done` files are done: the line is drawn again, at most every [`REDRAW`].
  pub fn update(&mut self, done: usize) {
    let elapsed = self.started.elapsed();
    if elapsed < DELAY || done >= self.total {
      return;
    }
    // Results arrive in the files' order, so a slow file holds the count still a while.
    if self
      .drawn
      .is_some_and(|(at, shown)| at.elapsed() < REDRAW || shown == done)
    {
      return;
    }
    terminal::status(&line(self.label, done, self.total, elapsed));
    self.drawn = Some((Instant::now(), done));
  }

  /// Takes the line off the screen, for something else to be printed; the next update
  /// draws it again.
  pub fn clear(&mut self) {
    if self.drawn.take().is_some() {
      terminal::status("");
    }
  }
}

/// `seconds` for a person: `45s`, `3m 10s`, `1h 5m`.
fn duration(seconds: f64) -> String {
  let s = seconds.round() as u64;
  if s < 60 {
    format!("{}s", s)
  } else if s < 3600 {
    format!("{}m {}s", s / 60, s % 60)
  } else {
    format!("{}h {}m", s / 3600, s % 3600 / 60)
  }
}

/// The line for `done` of `total` files after `elapsed`.
fn line(label: &str, done: usize, total: usize, elapsed: Duration) -> String {
  let share = done as f64 / total.max(1) as f64;
  let filled = ((share * BAR as f64).round() as usize).min(BAR);
  let left = if done == 0 {
    String::new()
  } else {
    let rest = elapsed.as_secs_f64() / done as f64 * (total - done.min(total)) as f64;
    format!("  about {} left", duration(rest))
  };
  format!(
    "{}  {}{}  {} of {} files  {:.0}%{}",
    label,
    "█".repeat(filled),
    "░".repeat(BAR - filled),
    done,
    total,
    share * 100.0,
    left
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_the_line_says_how_far_and_how_long() {
    assert_eq!(
      line("formatting", 4210, 8420, Duration::from_secs(12)),
      "formatting  ██████████░░░░░░░░░░  4210 of 8420 files  50%  about 12s left"
    );
    assert_eq!(
      line("checking", 0, 10, Duration::from_secs(2)),
      "checking  ░░░░░░░░░░░░░░░░░░░░  0 of 10 files  0%"
    );
    assert_eq!(duration(3900.0), "1h 5m");
    assert_eq!(duration(190.0), "3m 10s");
  }
}
