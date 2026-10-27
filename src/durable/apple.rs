//! The two `fcntl` commands macOS (and the other Apple systems) sync with, which the
//! standard library does not offer: unsafe code, held to this file by `cargo xtask gates`.
#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

/// `fsync`, then a barrier: nothing written after reaches the disk before what was written
/// before. Much cheaper than [`full`], since the drive's cache is not emptied.
const F_BARRIERFSYNC: i32 = 85;

/// Everything the drive holds in its cache is written to the disk.
const F_FULLFSYNC: i32 = 51;

unsafe extern "C" {
  fn fcntl(fd: i32, cmd: i32, ...) -> i32;
}

/// `F_BARRIERFSYNC` on `file`; where a file system does not take it (a network share),
/// a full sync instead.
pub fn barrier(file: &File) -> io::Result<()> {
  if command(file, F_BARRIERFSYNC) {
    Ok(())
  } else {
    file.sync_all()
  }
}

/// `F_FULLFSYNC` on `file`, which empties the cache of its drive; where a file system does
/// not take it, the standard library's sync.
pub fn full(file: &File) -> io::Result<()> {
  if command(file, F_FULLFSYNC) {
    Ok(())
  } else {
    file.sync_all()
  }
}

/// Runs `cmd` on `file`'s descriptor; `false` when it failed.
fn command(file: &File, cmd: i32) -> bool {
  // SAFETY: both commands take no third argument and only act on the descriptor, which
  // `file` keeps open for the call.
  unsafe { fcntl(file.as_raw_fd(), cmd) != -1 }
}
