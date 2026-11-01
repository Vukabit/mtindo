//! Progress for a long run, so it never looks hung: what it is doing, how far along, and
//! about how long is left. On a terminal, one line on stderr redrawn in place,
//!
//! ```text
//! scoring  ██████████░░░░░░░░░░  9876 of 19751  50%  about 1m 40s left
//! ```
//!
//! and cleared when the run ends; elsewhere (a log, CI), a plain line at each tenth, so a
//! log shows the pace without a line per step. Safe to share between threads.

use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The bar's width, in cells.
const BAR: usize = 20;

/// The least time between two redraws on a terminal.
const REDRAW: Duration = Duration::from_millis(100);

pub struct Progress {
  label: String,
  total: usize,
  done: AtomicUsize,
  started: Instant,
  terminal: bool,
  /// When the line was last drawn, and the last tenth a plain line was written for.
  shown: Mutex<(Instant, usize)>,
}

impl Progress {
  /// Progress of `total` steps, labelled `label`; drawn in place when stderr is a terminal.
  pub fn new(label: &str, total: usize) -> Progress {
    Progress::with_terminal(label, total, std::io::stderr().is_terminal())
  }

  fn with_terminal(label: &str, total: usize, terminal: bool) -> Progress {
    Progress {
      label: label.to_string(),
      total,
      done: AtomicUsize::new(0),
      started: Instant::now(),
      terminal,
      shown: Mutex::new((Instant::now() - REDRAW, 0)),
    }
  }

  /// One more step done.
  pub fn tick(&self) {
    let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
    let Ok(mut shown) = self.shown.lock() else {
      return;
    };
    if self.terminal {
      if shown.0.elapsed() >= REDRAW || done == self.total {
        shown.0 = Instant::now();
        eprint!("\r{}\x1b[K", self.line(done, self.started.elapsed()));
        let _ = std::io::stderr().flush();
      }
    } else if self.total > 0 {
      let tenth = done * 10 / self.total;
      if tenth > shown.1 {
        shown.1 = tenth;
        eprintln!("{}", self.line(done, self.started.elapsed()));
      }
    }
  }

  /// The run is over: the line is cleared from the terminal.
  pub fn finish(&self) {
    if self.terminal {
      eprint!("\r\x1b[K");
      let _ = std::io::stderr().flush();
    }
  }

  /// The line for `done` steps after `elapsed`.
  fn line(&self, done: usize, elapsed: Duration) -> String {
    let share = if self.total == 0 {
      1.0
    } else {
      done as f64 / self.total as f64
    };
    let filled = ((share * BAR as f64).round() as usize).min(BAR);
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(BAR - filled));
    let left = if done == 0 || done >= self.total {
      String::new()
    } else {
      let rest = elapsed.as_secs_f64() / done as f64 * (self.total - done) as f64;
      format!("  about {} left", duration(rest))
    };
    format!(
      "{}  {}  {} of {}  {:.0}%{}",
      self.label,
      bar,
      done,
      self.total,
      share * 100.0,
      left
    )
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_the_line_says_how_far_and_how_long() {
    let p = Progress::with_terminal("scoring", 200, false);
    assert_eq!(
      p.line(100, Duration::from_secs(100)),
      "scoring  ██████████░░░░░░░░░░  100 of 200  50%  about 1m 40s left"
    );
    assert_eq!(
      p.line(200, Duration::from_secs(5)),
      "scoring  ████████████████████  200 of 200  100%"
    );
    assert_eq!(duration(3900.0), "1h 5m");
  }

  #[test]
  fn test_ticks_count_from_every_thread() {
    let p = Progress::with_terminal("x", 1000, false);
    std::thread::scope(|s| {
      for _ in 0..4 {
        s.spawn(|| {
          for _ in 0..250 {
            p.tick();
          }
        });
      }
    });
    assert_eq!(p.done.load(Ordering::Relaxed), 1000);
    assert_eq!(p.shown.lock().unwrap().1, 10);
  }
}
