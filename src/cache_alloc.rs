//! A thread cache in front of the system allocator. Laying out a file makes and drops
//! millions of small blocks (tree handles, short strings, small lists), and the system
//! allocator's bookkeeping for them costs about a fifth of a run. Each thread keeps the
//! small blocks it frees and hands them back out, without a lock or a call.
//!
//! A small block (at most `LARGEST` bytes, aligned to at most `STEP`) belongs to one of
//! `CLASSES` size classes, `STEP` bytes apart, and always comes from the system with its
//! class's size and alignment. So a block freed on another thread than the one that made
//! it (a report string, say) simply joins that thread's list, and one handed back to the
//! system goes back with the layout it was made with. Larger or more aligned blocks go
//! straight to the system. The blocks a thread still holds when it ends are not given back:
//! at most `KEPT` per class.
//!
//! The binaries install it with `#[global_allocator]`; the library does not, so tools that
//! link it can count allocations with their own.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::ptr;

/// The number of size classes.
const CLASSES: usize = 32;

/// The most blocks a thread keeps of one class; the rest go back to the system.
const KEPT: u32 = 2048;

/// The largest block cached.
const LARGEST: usize = CLASSES * STEP;

/// The size step between classes, and the alignment every cached block has.
const STEP: usize = 16;

/// The allocator: [`System`], with a free list per thread and size class.
pub struct Cached;

thread_local! {
  // No drop, so the thread registers no destructor and the lists are never torn down.
  static LISTS: UnsafeCell<Lists> = const {
    UnsafeCell::new(Lists { heads: [ptr::null_mut(); CLASSES], counts: [0; CLASSES] })
  };
}

// SAFETY: every cached block was made by `System` with its class layout, is handed out to
// one owner at a time, and goes back to `System` with that same layout.
unsafe impl GlobalAlloc for Cached {
  #[inline]
  unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
    let Some(c) = class(&layout) else {
      return unsafe { System.alloc(layout) };
    };
    let reused = LISTS
      .try_with(|lists| {
        // SAFETY: the lists belong to this thread, and nothing here allocates.
        let lists = unsafe { &mut *lists.get() };
        let head = lists.heads[c];
        if !head.is_null() {
          // SAFETY: a free block holds the next block's address in its first word.
          lists.heads[c] = unsafe { *head.cast::<*mut u8>() };
          lists.counts[c] -= 1;
        }
        head
      })
      .unwrap_or(ptr::null_mut());
    if reused.is_null() {
      unsafe { System.alloc(class_layout(c)) }
    } else {
      reused
    }
  }

  #[inline]
  unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
    let Some(c) = class(&layout) else {
      return unsafe { System.dealloc(block, layout) };
    };
    let kept = LISTS
      .try_with(|lists| {
        // SAFETY: as in `alloc`.
        let lists = unsafe { &mut *lists.get() };
        if lists.counts[c] >= KEPT {
          return false;
        }
        // SAFETY: the block is at least `STEP` bytes, aligned to `STEP`, and no longer used.
        unsafe { *block.cast::<*mut u8>() = lists.heads[c] };
        lists.heads[c] = block;
        lists.counts[c] += 1;
        true
      })
      .unwrap_or(false);
    if !kept {
      unsafe { System.dealloc(block, class_layout(c)) }
    }
  }

  #[inline]
  unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    // SAFETY: the caller guarantees `new_size`, rounded to the alignment, does not overflow.
    let new = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
    match (class(&layout), class(&new)) {
      (None, None) => unsafe { System.realloc(block, layout, new_size) },
      // The block already has its class's full size.
      (Some(a), Some(b)) if a == b => block,
      _ => unsafe {
        let moved = self.alloc(new);
        if !moved.is_null() {
          ptr::copy_nonoverlapping(block, moved, layout.size().min(new_size));
          self.dealloc(block, layout);
        }
        moved
      },
    }
  }
}

/// A thread's free blocks: one list per class, linked through each block's first word.
struct Lists {
  heads: [*mut u8; CLASSES],
  counts: [u32; CLASSES],
}

/// The class of a block of `layout`, or `None` when the system serves it directly.
#[inline]
fn class(layout: &Layout) -> Option<usize> {
  let size = layout.size();
  (layout.align() <= STEP && size > 0 && size <= LARGEST).then(|| size.div_ceil(STEP) - 1)
}

/// The layout every block of class `c` is made with.
#[inline]
fn class_layout(c: usize) -> Layout {
  // SAFETY: `STEP` is a power of two and the size, at most `LARGEST`, cannot overflow.
  unsafe { Layout::from_size_align_unchecked((c + 1) * STEP, STEP) }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Fills a block with a pattern from `seed`, so a block handed out twice shows.
  fn fill(block: *mut u8, size: usize, seed: u8) {
    for i in 0..size {
      unsafe { *block.add(i) = seed.wrapping_add(i as u8) };
    }
  }

  /// Whether a block still holds the pattern [`fill`] wrote for its first `size` bytes.
  fn holds(block: *const u8, size: usize, seed: u8) -> bool {
    (0..size).all(|i| unsafe { *block.add(i) } == seed.wrapping_add(i as u8))
  }

  /// The number of blocks this thread keeps of class `c`.
  fn kept(c: usize) -> u32 {
    LISTS.with(|lists| unsafe { (*lists.get()).counts[c] })
  }

  #[test]
  fn test_a_thread_keeps_at_most_kept_blocks_of_a_class() {
    std::thread::spawn(|| {
      let layout = Layout::from_size_align(100, 4).unwrap();
      let c = class(&layout).unwrap();
      let blocks: Vec<*mut u8> = (0..KEPT + 50)
        .map(|_| unsafe { Cached.alloc(layout) })
        .collect();
      for block in blocks {
        unsafe { Cached.dealloc(block, layout) };
      }
      assert_eq!(kept(c), KEPT);
    })
    .join()
    .unwrap();
  }

  #[test]
  fn test_blocks_freed_on_another_thread_serve_that_thread() {
    let layout = Layout::from_size_align(48, 8).unwrap();
    let made: Vec<usize> = std::thread::spawn(move || {
      (0..100)
        .map(|i| {
          let block = unsafe { Cached.alloc(layout) };
          fill(block, 48, i);
          block as usize
        })
        .collect()
    })
    .join()
    .unwrap();
    std::thread::spawn(move || {
      let c = class(&layout).unwrap();
      let before = kept(c);
      for (i, &block) in made.iter().enumerate() {
        assert!(holds(block as *const u8, 48, i as u8));
        unsafe { Cached.dealloc(block as *mut u8, layout) };
      }
      assert_eq!(kept(c), before + 100);
      // Handed back out last freed first, each one whole.
      for &block in made.iter().rev() {
        let again = unsafe { Cached.alloc(layout) };
        assert_eq!(again as usize, block);
        fill(again, 48, 9);
      }
      assert_eq!(kept(c), before);
      for &block in &made {
        unsafe { Cached.dealloc(block as *mut u8, layout) };
      }
    })
    .join()
    .unwrap();
  }

  #[test]
  fn test_every_class_and_alignment_is_reused_whole() {
    std::thread::spawn(|| {
      for size in 1..=LARGEST {
        for align in [1, 2, 4, 8, 16] {
          let layout = Layout::from_size_align(size, align).unwrap();
          let c = class(&layout).unwrap();
          let block = unsafe { Cached.alloc(layout) };
          assert!(!block.is_null());
          assert_eq!(block as usize % STEP, 0);
          // The whole class size is usable, not only what was asked for.
          fill(block, (c + 1) * STEP, size as u8);
          unsafe { Cached.dealloc(block, layout) };
          let again = unsafe { Cached.alloc(layout) };
          assert_eq!(again, block, "size {size} align {align}");
          unsafe { Cached.dealloc(again, layout) };
        }
      }
    })
    .join()
    .unwrap();
  }

  #[test]
  fn test_large_and_highly_aligned_blocks_go_to_the_system() {
    for (size, align) in [(LARGEST + 1, 8), (4096, 16), (8, 32), (64, 4096)] {
      let layout = Layout::from_size_align(size, align).unwrap();
      assert_eq!(class(&layout), None);
      let block = unsafe { Cached.alloc(layout) };
      assert!(!block.is_null());
      assert_eq!(block as usize % align, 0);
      fill(block, size, 7);
      assert!(holds(block, size, 7));
      unsafe { Cached.dealloc(block, layout) };
    }
  }

  #[test]
  fn test_mixed_traffic_on_many_threads_never_shares_a_live_block() {
    let threads: Vec<_> = (0..8u64)
      .map(|t| {
        std::thread::spawn(move || {
          let mut state = 0x9e37_79b9_7f4a_7c15u64 ^ t;
          let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
          };
          let mut live: Vec<(*mut u8, Layout, u8)> = Vec::new();
          for round in 0..20_000u32 {
            let roll = next();
            if live.len() < 64 && roll % 3 != 0 {
              let size = 1 + (next() % (LARGEST as u64 + 200)) as usize;
              let layout = Layout::from_size_align(size, 1 << (next() % 5)).unwrap();
              let block = unsafe { Cached.alloc(layout) };
              assert!(!block.is_null());
              fill(block, size, round as u8);
              live.push((block, layout, round as u8));
            } else if !live.is_empty() {
              let at = (next() % live.len() as u64) as usize;
              let (block, layout, seed) = live.swap_remove(at);
              assert!(holds(block, layout.size(), seed));
              if roll % 2 == 0 {
                let size = 1 + (next() % (LARGEST as u64 + 200)) as usize;
                let moved = unsafe { Cached.realloc(block, layout, size) };
                assert!(holds(moved, layout.size().min(size), seed));
                let layout = Layout::from_size_align(size, layout.align()).unwrap();
                fill(moved, size, seed);
                live.push((moved, layout, seed));
              } else {
                unsafe { Cached.dealloc(block, layout) };
              }
            }
          }
          for (block, layout, seed) in live {
            assert!(holds(block, layout.size(), seed));
            unsafe { Cached.dealloc(block, layout) };
          }
        })
      })
      .collect();
    for thread in threads {
      thread.join().unwrap();
    }
  }

  #[test]
  fn test_realloc_keeps_the_contents_across_classes_and_the_system() {
    let small = Layout::from_size_align(17, 8).unwrap();
    let block = unsafe { Cached.alloc(small) };
    fill(block, 17, 3);
    // Within its class the block already has room.
    let same = unsafe { Cached.realloc(block, small, 30) };
    assert_eq!(same, block);
    assert!(holds(same, 17, 3));
    let at_30 = Layout::from_size_align(30, 8).unwrap();
    // To a larger class, then past every class, then back down.
    let grown = unsafe { Cached.realloc(same, at_30, 200) };
    assert!(holds(grown, 17, 3));
    let at_200 = Layout::from_size_align(200, 8).unwrap();
    let big = unsafe { Cached.realloc(grown, at_200, 5000) };
    assert!(holds(big, 17, 3));
    let at_5000 = Layout::from_size_align(5000, 8).unwrap();
    let shrunk = unsafe { Cached.realloc(big, at_5000, 24) };
    assert!(holds(shrunk, 17, 3));
    unsafe { Cached.dealloc(shrunk, Layout::from_size_align(24, 8).unwrap()) };
  }
}
