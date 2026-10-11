//! The log every xtask command writes, to `logs/NAME.log` at the repository root (ignored
//! by git), so what a run did is kept after its terminal output is gone.
//!
//! An entry is `[2026-10-08T21:30:00.123Z] [pid 4242] [INFO] message`, in UTC. The lines of
//! a multi-line message after the first are indented by four spaces. A log rotates when it
//! reaches 5 MiB, keeping three older files (`NAME.log.1` is the newest). Logging never fails
//! a command: if the file cannot be written, that is reported once on stderr and later
//! entries are dropped. A logger can be shared between threads; writes never interleave.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const BACKUP_COUNT: u32 = 3;

const MAX_BYTES: u64 = 5 * 1024 * 1024;

pub struct Logger {
  dir: PathBuf,
  name: String,
  /// Held while writing, so threads sharing a logger never interleave or rotate twice;
  /// true once writing has failed.
  broken: Mutex<bool>,
}

impl Logger {
  /// Opens `logs/NAME.log` under `root` and records the start of a run.
  pub fn new(root: &Path, name: &str) -> Logger {
    let logger = Logger {
      dir: root.join("logs"),
      name: name.to_string(),
      broken: Mutex::new(false),
    };
    logger.info(&format!("=== started {} ===", name));
    logger
  }

  pub fn debug(&self, message: &str) {
    self.write("DEBUG", message);
  }

  pub fn error(&self, message: &str) {
    self.write("ERROR", message);
  }

  pub fn info(&self, message: &str) {
    self.write("INFO", message);
  }

  pub fn warning(&self, message: &str) {
    self.write("WARNING", message);
  }

  fn path(&self) -> PathBuf {
    self.dir.join(format!("{}.log", self.name))
  }

  /// Shifts `NAME.log.1` to `.2` and so on, dropping the oldest, if the log is full.
  fn rotate(&self) -> std::io::Result<()> {
    let path = self.path();
    let size = match fs::metadata(&path) {
      Ok(meta) => meta.len(),
      Err(_) => return Ok(()),
    };
    if size < MAX_BYTES {
      return Ok(());
    }
    for idx in (1..BACKUP_COUNT).rev() {
      let source = self.dir.join(format!("{}.log.{}", self.name, idx));
      if source.exists() {
        fs::rename(
          &source,
          self.dir.join(format!("{}.log.{}", self.name, idx + 1)),
        )?;
      }
    }
    fs::rename(&path, self.dir.join(format!("{}.log.1", self.name)))
  }

  fn write(&self, level: &str, message: &str) {
    let Ok(mut broken) = self.broken.lock() else {
      return;
    };
    if *broken {
      return;
    }
    let entry = format_entry(
      &timestamp(SystemTime::now()),
      std::process::id(),
      level,
      message,
    );
    let result = fs::create_dir_all(&self.dir)
      .and_then(|_| self.rotate())
      .and_then(|_| {
        OpenOptions::new()
          .create(true)
          .append(true)
          .open(self.path())?
          .write_all(entry.as_bytes())
      });
    if let Err(e) = result {
      *broken = true;
      eprintln!(
        "warning: cannot write {}: {}; logging stopped",
        self.path().display(),
        e
      );
    }
  }
}

/// One log entry, ending in a line break.
pub fn format_entry(timestamp: &str, pid: u32, level: &str, message: &str) -> String {
  let normalized = message.replace("\r\n", "\n").replace('\r', "\n");
  let body = normalized.trim_end_matches('\n').replace('\n', "\n    ");
  format!("[{}] [pid {}] [{}] {}\n", timestamp, pid, level, body)
}

/// `2026-10-08T21:30:00.123Z`.
pub fn timestamp(now: SystemTime) -> String {
  let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
  let secs = since.as_secs() as i64;
  let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
  let in_day = secs.rem_euclid(86_400);
  format!(
    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
    year,
    month,
    day,
    in_day / 3600,
    in_day % 3600 / 60,
    in_day % 60,
    since.subsec_millis()
  )
}

/// Splits days since 1970-01-01 into (year, month, day), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
  let z = days + 719_468;
  let era = z.div_euclid(146_097);
  let doe = z.rem_euclid(146_097);
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
  let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
  let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
  (year, month, day)
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use super::*;

  #[test]
  fn test_entry_layout() {
    assert_eq!(
      format_entry("2026-10-08T21:30:00.123Z", 4242, "INFO", "hello"),
      "[2026-10-08T21:30:00.123Z] [pid 4242] [INFO] hello\n"
    );
  }

  #[test]
  fn test_logger_rotates_and_keeps_three_old_files() {
    let root = std::env::temp_dir().join(format!("xtask-log-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let logger = Logger::new(&root, "rotation");
    let dir = root.join("logs");
    for idx in 1..=5 {
      fs::write(dir.join("rotation.log"), vec![b'x'; MAX_BYTES as usize]).unwrap();
      logger.info(&format!("entry {}", idx));
    }
    let newest = fs::read_to_string(dir.join("rotation.log")).unwrap();
    assert!(newest.ends_with("entry 5\n"), "{}", newest);
    assert!(dir.join("rotation.log.3").exists());
    assert!(!dir.join("rotation.log.4").exists());
    assert!(!*logger.broken.lock().unwrap());
    fs::remove_dir_all(&root).unwrap();
  }

  #[test]
  fn test_multi_line_messages_are_indented() {
    assert_eq!(
      format_entry("T", 1, "ERROR", "first\r\nsecond\rthird\n\n"),
      "[T] [pid 1] [ERROR] first\n    second\n    third\n"
    );
  }

  #[test]
  fn test_timestamps_are_utc_with_milliseconds() {
    let epoch = UNIX_EPOCH + Duration::from_millis(1_791_581_400_123);
    assert_eq!(timestamp(epoch), "2026-10-09T21:30:00.123Z");
    assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
    let leap = UNIX_EPOCH + Duration::from_secs(951_782_400);
    assert_eq!(timestamp(leap), "2000-02-29T00:00:00.000Z");
  }

  #[test]
  fn test_unwritable_log_does_not_fail() {
    let root = std::env::temp_dir().join(format!("xtask-log-file-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    // `logs` is a file, so the log directory cannot be created.
    fs::write(root.join("logs"), "").unwrap();
    let logger = Logger::new(&root, "broken");
    logger.error("dropped");
    assert!(*logger.broken.lock().unwrap());
    fs::remove_dir_all(&root).unwrap();
  }
}
