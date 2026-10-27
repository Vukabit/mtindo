//! Windows consoles: a console shows color only once a program asks it to read escape
//! codes (virtual terminal processing). Windows Terminal reads them anyway; the classic
//! console prints them as text unless asked. The standard library cannot ask, so this file
//! holds the two calls that do, with mto's other unsafe code in `src/durable/apple.rs` and
//! `src/cache_alloc.rs`.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;

const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

#[link(name = "kernel32")]
unsafe extern "system" {
  fn GetConsoleMode(console: *mut c_void, mode: *mut u32) -> i32;
  fn SetConsoleMode(console: *mut c_void, mode: u32) -> i32;
}

/// Asks the consoles stdout and stderr write to, if any, to read escape codes. `false` when
/// one of them refuses (a Windows older than 10), which then gets no color.
pub fn enable_colors() -> bool {
  let ready = |stream: &dyn AsRawHandle| {
    let console = stream.as_raw_handle();
    let mut mode = 0u32;
    // SAFETY: both read or set the mode of a handle the process holds, and `mode` is
    // a valid place to write it. A handle that is not a console fails the first call.
    unsafe {
      if GetConsoleMode(console, &mut mode) == 0 {
        // A file or a pipe: nothing to ask.
        return true;
      }
      mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
        || SetConsoleMode(console, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
  };
  // Both asked, whatever the first says.
  let out = ready(&std::io::stdout());
  let err = ready(&std::io::stderr());
  out && err
}
