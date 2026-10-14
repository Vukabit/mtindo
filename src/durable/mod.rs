//! Layer 0: writing files so that a crash leaves each one whole, and a finished run on disk.
//!
//! Every write goes through three steps, the same on every platform:
//!
//! - [`ordered`]: a file just written reaches the disk before anything written after it, so
//!   a rename that follows can never be kept without the contents it names.
//! - [`renamed`]: a file was renamed into a folder; that folder's entry is made to last.
//! - [`settle`]: at the end of a run, everything written is made to last.
//!
//! In the default `batch` mode ([`Mode::Batch`]), each step does no more than its promise
//! needs, as git's `core.fsyncMethod = batch` does:
//!
//! - macOS: `ordered` is `F_BARRIERFSYNC`, which keeps the order without emptying the
//!   drive's cache, and `settle` empties it once per drive (`F_FULLFSYNC`). The standard
//!   library offers only the second, a few milliseconds each, for every file.
//! - Linux and the other Unixes: `ordered` is `fdatasync`, which lasts already; `settle` syncs
//!   each folder written in, once.
//! - Windows: `ordered` is `FlushFileBuffers`; folders are not synced, as NTFS keeps a
//!   rename with its journal.
//!
//! In `full` mode ([`Mode::Full`]) every step makes what it touched last at once, as mto did
//! before: a crash then loses nothing a step has finished, at the cost of a full flush per
//! file. Either way a crash leaves each file as it was or as written, never part of each, and
//! history holds a file's earlier contents before the file changes.

#[cfg(target_vendor = "apple")]
mod apple;

#[cfg(target_vendor = "apple")]
mod platform {
  use super::*;

  pub fn ordered(file: &File) -> io::Result<()> {
    super::apple::barrier(file)
  }

  /// Empties the cache of each drive written to, once.
  pub fn settle(touched: &BTreeSet<PathBuf>) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let mut drives = std::collections::HashSet::new();
    for dir in touched {
      let Ok(meta) = std::fs::metadata(dir) else {
        continue;
      };
      if drives.insert(meta.dev()) {
        super::apple::full(&File::open(dir)?)?;
      }
    }
    Ok(())
  }

  pub fn sync_dir(dir: &Path, mode: Mode) -> io::Result<()> {
    let handle = File::open(dir)?;
    match mode {
      Mode::Full => handle.sync_all(),
      Mode::Batch => super::apple::barrier(&handle),
    }
  }
}

#[cfg(all(unix, not(target_vendor = "apple")))]
mod platform {
  use super::*;

  pub fn ordered(file: &File) -> io::Result<()> {
    file.sync_data()
  }

  /// `fdatasync` and `fsync` last once they return: nothing is left to do.
  pub fn settle(_touched: &BTreeSet<PathBuf>) -> io::Result<()> {
    Ok(())
  }

  pub fn sync_dir(dir: &Path, _mode: Mode) -> io::Result<()> {
    File::open(dir)?.sync_all()
  }
}

#[cfg(not(unix))]
mod platform {
  use super::*;

  pub fn ordered(file: &File) -> io::Result<()> {
    file.sync_data()
  }

  pub fn settle(_touched: &BTreeSet<PathBuf>) -> io::Result<()> {
    Ok(())
  }

  /// A folder cannot be opened to sync it here, and NTFS keeps a rename with its journal.
  pub fn sync_dir(_dir: &Path, _mode: Mode) -> io::Result<()> {
    Ok(())
  }
}

use std::collections::BTreeSet;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static FULL: AtomicBool = AtomicBool::new(false);

/// The folders written in since the last [`settle`]: `dirs` to sync, `touched` (any folder
/// written in, for the drives to empty the cache of).
static PENDING: Mutex<Pending> = Pending::new();

/// How much each write makes last at once (`safety.sync`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
  /// In order as written, and all of it lasting when the run ends.
  #[default]
  Batch,
  /// Each file lasting as soon as it is written.
  Full,
}

struct Pending {
  dirs: BTreeSet<PathBuf>,
  touched: BTreeSet<PathBuf>,
}

impl Pending {
  const fn new() -> Mutex<Pending> {
    Mutex::new(Pending {
      dirs: BTreeSet::new(),
      touched: BTreeSet::new(),
    })
  }
}

pub fn mode() -> Mode {
  if FULL.load(Ordering::Relaxed) {
    Mode::Full
  } else {
    Mode::Batch
  }
}

/// Makes `file`, just written in `dir`, reach the disk before anything written after it.
pub fn ordered(file: &File, dir: &Path) -> io::Result<()> {
  ordered_in(&PENDING, mode(), file, dir)
}

/// Notes that a file was renamed into `dir`: its entry is made to last now (`full`) or by
/// [`settle`].
pub fn renamed(dir: &Path) -> io::Result<()> {
  renamed_in(&PENDING, mode(), dir)
}

/// Sets the mode for the rest of the process.
pub fn set_mode(mode: Mode) {
  FULL.store(mode == Mode::Full, Ordering::Relaxed);
}

/// Makes everything written since the last call last: each folder renamed into is synced,
/// then each drive written to empties its cache. Called once a run has written everything.
pub fn settle() -> io::Result<()> {
  settle_in(&PENDING)
}

fn lock(pending: &Mutex<Pending>) -> std::sync::MutexGuard<'_, Pending> {
  pending.lock().unwrap_or_else(|e| e.into_inner())
}

// The steps on a given record of what is pending, so that tests keep their own.

fn ordered_in(pending: &Mutex<Pending>, mode: Mode, file: &File, dir: &Path) -> io::Result<()> {
  if mode == Mode::Full {
    return file.sync_all();
  }
  // Synced before the lock is taken: threads sync their files at once.
  platform::ordered(file)?;
  lock(pending).touched.insert(dir.to_path_buf());
  Ok(())
}

fn renamed_in(pending: &Mutex<Pending>, mode: Mode, dir: &Path) -> io::Result<()> {
  if mode == Mode::Full {
    return platform::sync_dir(dir, Mode::Full);
  }
  let mut pending = lock(pending);
  pending.dirs.insert(dir.to_path_buf());
  pending.touched.insert(dir.to_path_buf());
  Ok(())
}

fn settle_in(pending: &Mutex<Pending>) -> io::Result<()> {
  let (dirs, touched) = {
    let mut pending = lock(pending);
    (
      std::mem::take(&mut pending.dirs),
      std::mem::take(&mut pending.touched),
    )
  };
  for dir in &dirs {
    match platform::sync_dir(dir, Mode::Batch) {
      // A folder removed since (a temporary one) has nothing left to keep.
      Err(e) if e.kind() == io::ErrorKind::NotFound => {}
      other => other?,
    }
  }
  platform::settle(&touched)
}

#[cfg(test)]
mod tests;
