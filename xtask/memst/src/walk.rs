//! Walking the stack by frame pointers: each frame starts with the caller's frame pointer,
//! then the return address. The binary is built with frame pointers forced on, so the walk
//! is exact through its own code; it stops at the first frame that does not look like one
//! (outside the thread's stack, not above the last, misaligned), so code built without them
//! ends a stack early instead of sending the walk astray.

/// The most frames kept for one stack, innermost first.
pub const DEPTH: usize = 48;

/// This thread's stack, `(lowest, highest)` address; `(0, 0)` when unknown. Reading them
/// may allocate (on Linux), so the recorder keeps each thread's once read (see
/// `record::bounds`).
#[cfg(target_os = "macos")]
pub fn bounds() -> (usize, usize) {
  // SAFETY: both read the calling thread's own record.
  unsafe {
    let me = libc::pthread_self();
    let top = libc::pthread_get_stackaddr_np(me) as usize;
    let size = libc::pthread_get_stacksize_np(me);
    (top.saturating_sub(size), top)
  }
}

#[cfg(target_os = "linux")]
pub fn bounds() -> (usize, usize) {
  // SAFETY: the attributes are initialized by pthread_getattr_np before they are read, and
  // destroyed after.
  unsafe {
    let mut attr: libc::pthread_attr_t = std::mem::zeroed();
    if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
      return (0, 0);
    }
    let mut low: *mut libc::c_void = std::ptr::null_mut();
    let mut size: libc::size_t = 0;
    let ok = libc::pthread_attr_getstack(&attr, &mut low, &mut size) == 0;
    libc::pthread_attr_destroy(&mut attr);
    if ok {
      (low as usize, low as usize + size)
    } else {
      (0, 0)
    }
  }
}

/// The current frame pointer.
#[inline(always)]
pub fn frame_pointer() -> usize {
  let fp: usize;
  // SAFETY: reads a register.
  #[cfg(target_arch = "aarch64")]
  unsafe {
    std::arch::asm!("mov {}, x29", out(reg) fp, options(nomem, nostack, preserves_flags));
  }
  #[cfg(target_arch = "x86_64")]
  unsafe {
    std::arch::asm!("mov {}, rbp", out(reg) fp, options(nomem, nostack, preserves_flags));
  }
  fp
}

/// Walks from frame pointer `fp`, writing return addresses into `out` (after any already
/// there, up to its length); returns how many frames `out` holds. With bounds `(0, 0)` a
/// frame must only lie above the last, within a megabyte of it.
///
/// # Safety
///
/// `fp` must be a frame pointer of the current thread (or of the thread a signal
/// interrupted, from the handler on that thread).
pub unsafe fn walk(
  mut fp: usize,
  bounds: (usize, usize),
  out: &mut [usize],
  mut len: usize,
) -> usize {
  let word = std::mem::size_of::<usize>();
  let inside = |at: usize| {
    if bounds.1 == 0 {
      true
    } else {
      at >= bounds.0 && at + 2 * word <= bounds.1
    }
  };
  while len < out.len() {
    if fp == 0 || !fp.is_multiple_of(word) || !inside(fp) {
      break;
    }
    // SAFETY: `fp` is aligned and inside this thread's stack (or, without bounds, was the
    // previous frame's saved pointer, which the caller vouches for).
    let (next, ret) = unsafe { (*(fp as *const usize), *((fp + word) as *const usize)) };
    if ret == 0 {
      break;
    }
    out[len] = ret;
    len += 1;
    // The stack grows down, so each caller's frame is higher; a frame further than a
    // megabyte up is not a frame.
    if next <= fp || next - fp > 1 << 20 {
      break;
    }
    fp = next;
  }
  len
}

#[cfg(test)]
mod tests {
  use super::*;

  #[inline(never)]
  fn depth_here(extra: usize) -> usize {
    if extra > 0 {
      return depth_here(extra - 1) + std::hint::black_box(0);
    }
    let mut out = [0usize; DEPTH];
    // SAFETY: this thread's own frame pointer.
    unsafe { walk(frame_pointer(), bounds(), &mut out, 0) }
  }

  #[test]
  fn test_a_bad_frame_pointer_stops_the_walk() {
    let mut out = [0usize; DEPTH];
    // SAFETY: rejected before it is read: misaligned, then outside the bounds.
    assert_eq!(unsafe { walk(3, bounds(), &mut out, 0) }, 0);
    assert_eq!(unsafe { walk(8, bounds(), &mut out, 0) }, 0);
  }

  #[test]
  fn test_bounds_hold_this_frame() {
    let (low, high) = bounds();
    let here = frame_pointer();
    assert!(
      low < here && here < high,
      "{:x} not in {:x}..{:x}",
      here,
      low,
      high
    );
  }

  #[test]
  fn test_deeper_calls_walk_deeper() {
    let shallow = depth_here(0);
    let deep = depth_here(5);
    assert!(shallow >= 2, "{}", shallow);
    assert!(
      deep >= shallow + 5 || deep == DEPTH,
      "{} then {}",
      shallow,
      deep
    );
  }
}
