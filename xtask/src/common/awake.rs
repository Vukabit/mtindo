//! Keeping the machine awake while a long run is in progress.
//!
//! The guard starts the system's own tool and stops it when dropped: `caffeinate -ims` on
//! macOS (tied to this process with `-w`, so it also ends if the runner is killed), or
//! `systemd-inhibit --what=sleep:idle` on Linux. On Windows it asks the system itself
//! (`SetThreadExecutionState`), for as long as the thread that started it runs, and ends
//! with the process. Elsewhere, or without those tools, nothing keeps the machine awake;
//! callers still detect sleep with [`asleep_seconds`].
//!
//! None of them can stop a laptop sleeping when its lid is closed.

/// Windows' own way of staying awake: no tool to start, a request the system keeps for the
/// calling thread.
#[cfg(windows)]
mod windows {
  /// The state holds until changed, not for this call alone.
  const ES_CONTINUOUS: u32 = 0x8000_0000;

  const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;

  const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

  #[link(name = "kernel32")]
  unsafe extern "system" {
    fn SetThreadExecutionState(flags: u32) -> u32;
  }

  /// Asks the system to stay awake, the display too when `display`; whether it agreed.
  pub fn hold(display: bool) -> bool {
    let display = if display { ES_DISPLAY_REQUIRED } else { 0 };
    // SAFETY: a plain Win32 call with flags only; it returns 0 on failure.
    unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | display) != 0 }
  }

  /// Lets the system sleep again.
  pub fn release() {
    // SAFETY: as in `hold`.
    unsafe {
      SetThreadExecutionState(ES_CONTINUOUS);
    }
  }
}

/// No such request elsewhere: the tools above are used instead.
#[cfg(not(windows))]
mod windows {
  pub fn hold(_display: bool) -> bool {
    false
  }

  pub fn release() {}
}

use std::process::{Child, Command, Stdio};
use std::time::{Instant, SystemTime};

use super::process;

pub struct KeepAwake {
  child: Option<Child>,
  /// Windows was asked to stay awake, and is told it may sleep again on drop.
  execution_state: bool,
  how: &'static str,
}

impl KeepAwake {
  pub fn start() -> KeepAwake {
    KeepAwake::start_with(false)
  }

  /// [`KeepAwake::start`], with the display kept on too (`caffeinate -d` on macOS), for a
  /// run someone watches live.
  pub fn start_watched() -> KeepAwake {
    KeepAwake::start_with(true)
  }

  fn start_with(display: bool) -> KeepAwake {
    if windows::hold(display) {
      return KeepAwake {
        child: None,
        execution_state: true,
        how: if display {
          "SetThreadExecutionState (system and display)"
        } else {
          "SetThreadExecutionState (system)"
        },
      };
    }
    let pid = std::process::id().to_string();
    let flags = if display { "-dims" } else { "-ims" };
    let attempts: [(&'static str, &str, Vec<&str>); 2] = [
      ("caffeinate", "caffeinate", vec![flags, "-w", &pid]),
      (
        "systemd-inhibit",
        "systemd-inhibit",
        vec![
          "--what=sleep:idle",
          "--who=mtindo",
          "--why=a long cargo xtask run is in progress",
          "--mode=block",
          "sleep",
          "infinity",
        ],
      ),
    ];
    for (how, program, args) in attempts {
      if !process::on_path(program) {
        continue;
      }
      // Not under a time limit: it runs as long as the guard lives, and the guard kills it.
      let child = Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
      if let Ok(child) = child {
        return KeepAwake {
          child: Some(child),
          execution_state: false,
          how,
        };
      }
    }
    KeepAwake {
      child: None,
      execution_state: false,
      how: "nothing (no caffeinate or systemd-inhibit)",
    }
  }

  /// What is keeping the machine awake, for the log.
  pub fn how(&self) -> &'static str {
    self.how
  }
}

impl Drop for KeepAwake {
  fn drop(&mut self) {
    if self.execution_state {
      windows::release();
    }
    if let Some(child) = self.child.as_mut() {
      let _ = child.kill();
      let _ = child.wait();
    }
  }
}

/// A point in time on both clocks, to measure sleep since then.
#[derive(Clone, Copy)]
pub struct Stamp {
  wall: SystemTime,
  monotonic: Instant,
}

impl Stamp {
  pub fn now() -> Stamp {
    Stamp {
      wall: SystemTime::now(),
      monotonic: Instant::now(),
    }
  }
}

/// Seconds the machine slept since `stamp`: the monotonic clock stops during sleep (on
/// macOS and Linux) while the wall clock does not. A wall clock set backwards gives 0.
pub fn asleep_seconds(stamp: Stamp) -> u64 {
  let awake = stamp.monotonic.elapsed().as_secs();
  let wall = stamp.wall.elapsed().map(|d| d.as_secs()).unwrap_or(awake);
  wall.saturating_sub(awake)
}

/// `5h 12m`, or `42s` under a minute.
pub fn duration(seconds: u64) -> String {
  if seconds < 60 {
    return format!("{}s", seconds);
  }
  format!("{}h {:02}m", seconds / 3600, seconds % 3600 / 60)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_duration() {
    assert_eq!(duration(42), "42s");
    assert_eq!(duration(3600 * 5 + 12 * 60), "5h 12m");
    assert_eq!(duration(86_400), "24h 00m");
  }

  #[test]
  fn test_guard_starts_and_stops() {
    let guard = KeepAwake::start();
    assert!(!guard.how().is_empty());
    drop(guard);
  }

  #[test]
  fn test_no_sleep_is_measured_while_awake() {
    let stamp = Stamp::now();
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(asleep_seconds(stamp), 0);
  }
}
