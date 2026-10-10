//! Keeping the machine awake while a long run is in progress.
//!
//! The guard starts the system's own tool and stops it when dropped: `caffeinate -ims` on
//! macOS (tied to this process with `-w`, so it also ends if the runner is killed), or
//! `systemd-inhibit --what=sleep:idle` on Linux. Elsewhere, or without those tools, nothing
//! keeps the machine awake; callers still detect sleep with [`asleep_seconds`].
//!
//! Neither tool can stop a laptop sleeping when its lid is closed.

use std::process::{Child, Command, Stdio};
use std::time::{Instant, SystemTime};

use super::process;

pub struct KeepAwake {
    child: Option<Child>,
    how: &'static str,
}

impl KeepAwake {
    pub fn start() -> KeepAwake {
        let pid = std::process::id().to_string();
        let attempts: [(&'static str, &str, Vec<&str>); 2] = [
            ("caffeinate", "caffeinate", vec!["-ims", "-w", &pid]),
            (
                "systemd-inhibit",
                "systemd-inhibit",
                vec![
                    "--what=sleep:idle",
                    "--who=mtindo",
                    "--why=cargo xtask test is running",
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
            let child = Command::new(program)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(child) = child {
                return KeepAwake {
                    child: Some(child),
                    how,
                };
            }
        }
        KeepAwake {
            child: None,
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
