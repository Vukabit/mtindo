//! What else the machine is doing while a bench times something: how much of the processor
//! other work is using, and whether the system is holding the processor back for heat.
//!
//! A timing cannot be corrected for load (contention is not proportional to it), so a
//! bench waits for a quiet machine first ([`settle`]), and every figure is kept with the
//! conditions it was taken in. A timing taken while the machine was busy or throttled is
//! kept but never judged: the ratchet and `metrics compare` pass over it.
//!
//! The processor in use is measured over one second while the bench itself is idle, not
//! taken from the load average, which trails a minute behind (it still counts the bench's
//! own build) and counts processes waiting on disks.
//!
//! - macOS: the second sample of `top -l 2 -s 1`; throttling from `pmset -g therm` (a CPU
//!   speed limit under 100, or a thermal warning recorded).
//! - Linux: `/proc/stat` read twice a second apart; throttling is not read.
//! - Elsewhere: neither is known, and figures are judged as before.

use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::process;

/// How often [`settle`] looks again.
const POLL: Duration = Duration::from_secs(5);

/// The share of the processor in use (percent) a machine counts as quiet up to.
const QUIET_BUSY: f64 = 20.0;

/// How long [`settle`] waits for a quiet machine before timing anyway.
const WAIT: Duration = Duration::from_secs(120);

/// The conditions this process's bench settled on, for every figure it records.
static SETTLED: OnceLock<Conditions> = OnceLock::new();

#[derive(Clone, Debug, PartialEq)]
pub struct Conditions {
  /// The share of the processor in use over a second, in percent, where the system says.
  pub busy: Option<f64>,
  pub cores: usize,
  /// Whether the system is slowing the processor for heat, where it says.
  pub throttled: Option<bool>,
}

impl Conditions {
  /// The machine as it is now.
  pub fn now(root: &Path) -> Conditions {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    Conditions {
      busy: busy(root),
      cores,
      throttled: throttled(root),
    }
  }

  /// `quiet`, `busy` (more than [`QUIET_BUSY`] percent of the processor in use) or
  /// `throttled`; empty when nothing is known.
  pub fn state(&self) -> &'static str {
    if self.throttled == Some(true) {
      "throttled"
    } else if self.busy.is_some_and(|b| b > QUIET_BUSY) {
      "busy"
    } else if self.busy.is_some() || self.throttled.is_some() {
      "quiet"
    } else {
      ""
    }
  }

  /// Whether timings taken now are not to be judged.
  pub fn noisy(&self) -> bool {
    matches!(self.state(), "busy" | "throttled")
  }

  pub fn describe(&self) -> String {
    let load = self.busy.map_or("processor use unknown".to_string(), |b| {
      format!("{:.0}% of the processor in use", b)
    });
    let heat = match self.throttled {
      Some(true) => ", throttled for heat",
      Some(false) => ", not throttled",
      None => "",
    };
    format!("{} on {} cores{}", load, self.cores, heat)
  }
}

/// Waits, up to [`WAIT`], for the machine to be quiet before `what` is timed, saying so
/// while it waits; the conditions it settled on are kept for every figure this process
/// records ([`settled`]).
pub fn settle(root: &Path, what: &str) -> Conditions {
  let started = Instant::now();
  let mut said = false;
  let conditions = loop {
    let now = Conditions::now(root);
    if !now.noisy() || started.elapsed() >= WAIT {
      break now;
    }
    if !said {
      println!(
        "waiting for a quiet machine before {} ({}; up to {} s)...",
        what,
        now.describe(),
        WAIT.as_secs()
      );
      said = true;
    }
    std::thread::sleep(POLL);
  };
  if conditions.noisy() {
    println!(
      "the machine is still {} ({}): {} is timed anyway, and its timings are kept but not judged",
      conditions.state(),
      conditions.describe(),
      what
    );
  }
  let _ = SETTLED.set(conditions.clone());
  conditions
}

/// The conditions this process's bench settled on, if it settled.
pub fn settled() -> Option<&'static Conditions> {
  SETTLED.get()
}

/// The share of the processor in use over one second, in percent.
fn busy(root: &Path) -> Option<f64> {
  if cfg!(target_os = "macos") {
    // `CPU usage: 4.32% user, 2.25% sys, 93.41% idle`, the second sample over the second.
    let args = ["-l", "2", "-s", "1", "-n", "0"].map(String::from);
    let (code, output) = process::capture(root, "top", &args, &[], process::QUICK);
    if code != 0 {
      return None;
    }
    let idle = output
      .lines()
      .rfind(|l| l.starts_with("CPU usage:"))?
      .split(',')
      .find(|part| part.contains("idle"))?
      .trim()
      .trim_end_matches("idle")
      .trim()
      .trim_end_matches('%')
      .parse::<f64>()
      .ok()?;
    return Some(100.0 - idle);
  }
  if cfg!(target_os = "linux") {
    // The first line of /proc/stat: time spent in each state; idle and iowait are idle.
    let read = || -> Option<(u64, u64)> {
      let text = std::fs::read_to_string("/proc/stat").ok()?;
      let times: Vec<u64> = text
        .lines()
        .next()?
        .split_whitespace()
        .skip(1)
        .filter_map(|t| t.parse().ok())
        .collect();
      let idle = times.get(3)? + times.get(4).unwrap_or(&0);
      Some((times.iter().sum(), idle))
    };
    let (total, idle) = read()?;
    std::thread::sleep(Duration::from_secs(1));
    let (total_after, idle_after) = read()?;
    let spent = total_after.saturating_sub(total).max(1);
    let idle = idle_after.saturating_sub(idle);
    return Some(100.0 * (spent.saturating_sub(idle)) as f64 / spent as f64);
  }
  None
}

/// Whether `pmset -g therm`'s output says the processor is held back: a CPU speed limit
/// under 100, or a thermal or performance warning level recorded.
fn therm_says_throttled(output: &str) -> bool {
  output.lines().any(|line| {
    let line = line.trim();
    if let Some(limit) = line.strip_prefix("CPU_Speed_Limit") {
      return limit
        .trim_start_matches([' ', '\t', '='])
        .trim()
        .parse::<u32>()
        .is_ok_and(|limit| limit < 100);
    }
    (line.contains("warning level") && !line.starts_with("Note: No")) && !line.ends_with("= 0")
  })
}

fn throttled(root: &Path) -> Option<bool> {
  if !cfg!(target_os = "macos") {
    return None;
  }
  let (code, output) = process::capture(
    root,
    "pmset",
    &["-g".to_string(), "therm".to_string()],
    &[],
    process::QUICK,
  );
  (code == 0).then(|| therm_says_throttled(&output))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_states() {
    let at = |busy, throttled| Conditions {
      busy,
      cores: 10,
      throttled,
    };
    assert_eq!(at(Some(7.0), Some(false)).state(), "quiet");
    assert_eq!(at(Some(35.0), Some(false)).state(), "busy");
    assert_eq!(at(Some(5.0), Some(true)).state(), "throttled");
    assert_eq!(at(None, None).state(), "");
    assert!(at(Some(35.0), None).noisy() && !at(None, None).noisy());
  }

  #[test]
  fn test_therm_output() {
    let calm = "Note: No thermal warning level has been recorded\n\
                Note: No performance warning level has been recorded\n\
                Note: No CPU power status has been recorded\n";
    assert!(!therm_says_throttled(calm));
    let limited = "CPU Power notify\n\tCPU_Scheduler_Limit \t= 100\n\
                   \tCPU_Available_CPUs \t= 10\n\tCPU_Speed_Limit \t= 72\n";
    assert!(therm_says_throttled(limited));
    let full = "\tCPU_Speed_Limit \t= 100\n";
    assert!(!therm_says_throttled(full));
    assert!(therm_says_throttled("Thermal warning level set to 2.\n"));
  }
}
