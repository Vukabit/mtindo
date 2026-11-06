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
//! - macOS: `ordered` hands the file to the drive (`fsync`) and waits for a barrier
//!   (`F_BARRIERFSYNC`) issued after that, which keeps the order without emptying the drive's
//!   cache; files written at once share one barrier ([`shared_barrier`]), as a database's
//!   group commit shares one flush, since a barrier orders everything handed to its drive
//!   before it. `settle` hands each folder to the drive and empties its cache once per drive
//!   (`F_FULLFSYNC`). The standard library offers only the full flush, a few milliseconds
//!   each, for every file.
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

  /// A barrier on `file`'s drive.
  pub fn barrier(file: &File) -> io::Result<()> {
    super::apple::barrier(file)
  }

  /// A barrier on each drive `dirs` are on, issued on one of its folders.
  pub fn barrier_for<'a>(dirs: impl IntoIterator<Item = &'a Path>) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let mut drives = std::collections::HashSet::new();
    for dir in dirs {
      let handle = File::open(dir)?;
      if drives.insert(handle.metadata()?.dev()) {
        super::shared_barrier(&handle)?;
      }
    }
    Ok(())
  }

  pub fn handed(file: &File) -> io::Result<()> {
    super::apple::writeout(file)
  }

  pub fn ordered(file: &File) -> io::Result<()> {
    super::apple::writeout(file)?;
    super::shared_barrier(file)
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

  /// A folder renamed into, handed to its drive (`settle` empties the drive's cache after
  /// every folder is).
  pub fn sync_dir(dir: &Path, mode: Mode) -> io::Result<()> {
    let handle = File::open(dir)?;
    match mode {
      Mode::Full => handle.sync_all(),
      Mode::Batch => super::apple::writeout(&handle),
    }
  }
}

#[cfg(all(unix, not(target_vendor = "apple")))]
mod platform {
  use super::*;

  pub fn barrier_for<'a>(_dirs: impl IntoIterator<Item = &'a Path>) -> io::Result<()> {
    Ok(())
  }

  /// `fdatasync`, which lasts once it returns: no barrier is needed after it.
  pub fn handed(file: &File) -> io::Result<()> {
    file.sync_data()
  }

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

  pub fn barrier_for<'a>(_dirs: impl IntoIterator<Item = &'a Path>) -> io::Result<()> {
    Ok(())
  }

  /// `FlushFileBuffers`, which lasts once it returns: no barrier is needed after it.
  pub fn handed(file: &File) -> io::Result<()> {
    file.sync_data()
  }

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

#[cfg(target_vendor = "apple")]
static BARRIER_DONE: std::sync::Condvar = std::sync::Condvar::new();

/// The barriers of each drive (by device number): how many were started and how many have
/// finished, in order, and whether one is running.
#[cfg(target_vendor = "apple")]
static BARRIERS: Mutex<Vec<(u64, Barriers)>> = Mutex::new(Vec::new());

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

#[cfg(target_vendor = "apple")]
#[derive(Clone, Copy, Default)]
struct Barriers {
  started: u64,
  finished: u64,
  running: bool,
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

/// Makes everything [`handed`] to the drives of `dirs` before this call reach them before
/// anything written after it: one barrier for each drive among them.
pub fn barrier_for<'a>(dirs: impl IntoIterator<Item = &'a Path>) -> io::Result<()> {
  if mode() == Mode::Full {
    return Ok(());
  }
  platform::barrier_for(dirs)
}

/// Hands `file`, just written in `dir`, to its drive without waiting for its place in the
/// order: a [`barrier_for`] its folder afterwards gives it that place, for every file handed
/// before it at once. [`ordered`] is the two, for one file.
pub fn handed(file: &File, dir: &Path) -> io::Result<()> {
  if mode() == Mode::Full {
    return file.sync_all();
  }
  platform::handed(file)?;
  lock(&PENDING).touched.insert(dir.to_path_buf());
  Ok(())
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

/// Returns once a barrier on `file`'s drive has finished that was issued after this call
/// began, so after everything handed to the drive before it: the one running when the call
/// begins does not count, the next does, whichever thread issues it. Threads writing at
/// once share each barrier rather than queue one each.
#[cfg(target_vendor = "apple")]
fn shared_barrier(file: &File) -> io::Result<()> {
  use std::os::unix::fs::MetadataExt;
  let drive = file.metadata()?.dev();
  let mut barriers = BARRIERS.lock().unwrap_or_else(|e| e.into_inner());
  let at = match barriers.iter().position(|(d, _)| *d == drive) {
    Some(at) => at,
    None => {
      barriers.push((drive, Barriers::default()));
      barriers.len() - 1
    }
  };
  // The first barrier to start from now on.
  let wanted = barriers[at].1.started + 1;
  loop {
    let state = barriers[at].1;
    if state.finished >= wanted {
      return Ok(());
    }
    if state.running {
      barriers = BARRIER_DONE
        .wait(barriers)
        .unwrap_or_else(|e| e.into_inner());
      continue;
    }
    barriers[at].1.started += 1;
    barriers[at].1.running = true;
    let issued = barriers[at].1.started;
    drop(barriers);
    let result = platform::barrier(file);
    barriers = BARRIERS.lock().unwrap_or_else(|e| e.into_inner());
    barriers[at].1.running = false;
    if result.is_ok() {
      barriers[at].1.finished = issued;
    }
    BARRIER_DONE.notify_all();
    result?;
  }
}

#[cfg(test)]
mod tests;
