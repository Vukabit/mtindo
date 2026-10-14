//! Recording: the counting allocator and the CPU-time sampler.
//!
//! [`Counting`] passes every request to the system allocator and, while recording, notes it
//! with the stack it came from: allocations, frees and reallocations, their sizes, how long
//! each block lived, the live total over time. Stacks are interned once; everything else is
//! aggregated as it happens, so the profile stays small whatever the run's length.
//!
//! Every thread allocates at once, so the recorder shares as little as it can. What only
//! adds up (counts, bytes, histograms, bytes per 100 ms) is kept per thread and summed when
//! recording stops. What depends on the order of events across threads:
//!
//! - The live total, its peak and its value every 100 ms: each thread logs its events'
//!   times and what they did to the total, and the logs are played merged in time order,
//!   now and then and at the end, by whichever thread fills its log first. One counter
//!   every thread updated would cost more than all the rest of the recording.
//! - Each site's live bytes and their peak: an atomic per site, on a cache line of its own.
//! - The blocks still live: a map split by address into shards, each under its own lock,
//!   so a block freed on another thread than the one that allocated it is still matched.
//!
//! Each thread remembers the stacks it has seen; only a stack new to the thread takes the
//! shared table's lock.
//!
//! The sampler is a SIGPROF timer firing every millisecond of CPU time; its handler walks
//! the interrupted thread's stack into a preallocated ring (a handler may not allocate or
//! lock), which is interned when recording stops.
//!
//! The recorder's own work allocates; a per-thread flag sends those requests straight to
//! the system allocator, unrecorded. Per-thread state lives in pthread keys, not Rust
//! thread locals: on macOS a thread's first touch of a thread local allocates, which
//! would re-enter the allocator before the flag could say so.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::profile::{BUCKET_NS, HISTOGRAM, Profile, Site};
use crate::walk::{self, DEPTH};

/// Sites per chunk of [`SITE_LIVE`].
const CHUNK: usize = 1 << 12;

/// Chunks of [`SITE_LIVE`]: room for sixteen million stacks.
const CHUNKS: usize = 1 << 12;

/// The sampling interval, in microseconds of CPU time.
const INTERVAL_US: i64 = 1000;

/// Events a thread logs before it plays the logs into [`TIMELINE`].
const LOG: usize = 1 << 16;

/// A multiplier with well-spread bits, for hashing.
const MIX: u64 = 0x9e37_79b9_7f4a_7c15;

/// Time samples the ring holds: a little over a minute of CPU time at one a millisecond.
const RING: usize = 1 << 16;

/// Shards of the live-block map: enough that threads rarely meet on one.
const SHARDS: usize = 256;

/// Frames dropped from the top of every allocation stack: the recorder and the allocator
/// entry points, which every stack shares.
const SKIP: usize = 2;

/// Where the current recording started on [`now_ns`].
static BASE_NS: AtomicU64 = AtomicU64::new(0);

/// Per thread: its stack bounds, once read (a leaked `(usize, usize)`).
static BOUNDS: OnceLock<libc::pthread_key_t> = OnceLock::new();

/// Per thread: non-null while the thread is inside the recorder.
static BUSY: OnceLock<libc::pthread_key_t> = OnceLock::new();

/// The blocks allocated while recording and not yet freed.
static LIVE_BLOCKS: OnceLock<Box<[Shard]>> = OnceLock::new();

/// Per thread: its [`Local`] (a leaked `Spin<Local>`, also in [`LOCALS`]).
static LOCAL: OnceLock<libc::pthread_key_t> = OnceLock::new();

/// Every thread's [`Local`] that has recorded, kept after the thread ends so its counts
/// are summed at [`finish`].
static LOCALS: Mutex<Vec<&'static Spin<Local>>> = Mutex::new(Vec::new());

/// Time spent playing logs into [`TIMELINE`], part of the recorder's cost.
static PLAY_NS: AtomicU64 = AtomicU64::new(0);

static RECORDING: AtomicBool = AtomicBool::new(false);

static SAMPLED: AtomicUsize = AtomicUsize::new(0);

static SAMPLES: Ring = Ring(UnsafeCell::new([[0; DEPTH]; RING]));

/// Each site's live bytes and their peak, by stack id, in chunks allocated as ids are
/// given out (never freed, so a reference to one stays good).
static SITE_LIVE: [AtomicPtr<SiteLive>; CHUNKS] =
  [const { AtomicPtr::new(std::ptr::null_mut()) }; CHUNKS];

/// Every stack seen, by id.
static STACKS: Mutex<Interner> = Mutex::new(Interner::new());

/// The nanoseconds in one tick of [`now_ns`]'s clock, in 32.32 fixed point.
static TIMEBASE: OnceLock<u64> = OnceLock::new();

/// The live total over the events played so far.
static TIMELINE: Spin<Timeline> = Spin::new(Timeline::new());

enum Event {
  Alloc(usize, usize),
  Free(usize),
  Realloc(Option<Live>, usize, usize),
}

/// Marks this thread as inside the recorder until dropped; `None` if it already was.
struct Busy;

impl Busy {
  fn enter() -> Busy {
    set(&BUSY, std::ptr::dangling::<libc::c_void>());
    Busy
  }

  fn try_enter() -> Option<Busy> {
    get(&BUSY).is_null().then(Busy::enter)
  }
}

impl Drop for Busy {
  fn drop(&mut self) {
    set(&BUSY, std::ptr::null());
  }
}

/// The system allocator, recording while [`start`] is in effect.
pub struct Counting;

// SAFETY: every request is the system allocator's; recording only reads the result.
unsafe impl GlobalAlloc for Counting {
  unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
    // SAFETY: forwarded as given.
    let ptr = unsafe { System.alloc(layout) };
    if !ptr.is_null() {
      note(Event::Alloc(ptr as usize, layout.size()));
    }
    ptr
  }

  unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
    // SAFETY: forwarded as given.
    let ptr = unsafe { System.alloc_zeroed(layout) };
    if !ptr.is_null() {
      note(Event::Alloc(ptr as usize, layout.size()));
    }
    ptr
  }

  unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
    // Before the block is released: once it is, another thread may be given its address.
    note(Event::Free(ptr as usize));
    // SAFETY: forwarded as given.
    unsafe { System.dealloc(ptr, layout) }
  }

  unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    let old = take_live(ptr as usize);
    // SAFETY: forwarded as given.
    let new = unsafe { System.realloc(ptr, layout, new_size) };
    if new.is_null() {
      // The old block stands.
      if let Some(old) = old {
        put_live(ptr as usize, old);
      }
    } else {
      note(Event::Realloc(old, new as usize, new_size));
    }
    new
  }
}

#[derive(Clone, Copy)]
struct Entry {
  hash: u64,
  start: usize,
  len: u32,
  value: u32,
}

/// Stacks by their frames: open addressing over a flat array of frames, so a lookup
/// hashes the walked frames once and allocates nothing.
struct Interner {
  /// Per slot, an index into `entries` plus one; `0` is empty. A power of two long.
  slots: Vec<u32>,
  entries: Vec<Entry>,
  frames: Vec<usize>,
}

impl Interner {
  const fn new() -> Interner {
    Interner {
      slots: Vec::new(),
      entries: Vec::new(),
      frames: Vec::new(),
    }
  }

  /// The value `frames` (whose hash is `hash`) was inserted with.
  fn get(&self, hash: u64, frames: &[usize]) -> Option<u32> {
    if self.slots.is_empty() {
      return None;
    }
    let mask = self.slots.len() - 1;
    let mut at = self.home(hash);
    loop {
      let slot = self.slots[at];
      if slot == 0 {
        return None;
      }
      let entry = &self.entries[slot as usize - 1];
      if entry.hash == hash && self.frames_of(entry) == frames {
        return Some(entry.value);
      }
      at = (at + 1) & mask;
    }
  }

  /// Adds `frames`, not yet present, with `value`; returns its index (entries are
  /// numbered in the order they were inserted).
  fn insert(&mut self, hash: u64, frames: &[usize], value: u32) -> u32 {
    // At most three quarters full, so probes stay short.
    if (self.entries.len() + 1) * 4 > self.slots.len() * 3 {
      self.grow();
    }
    let idx = self.entries.len() as u32;
    self.entries.push(Entry {
      hash,
      start: self.frames.len(),
      len: frames.len() as u32,
      value,
    });
    self.frames.extend_from_slice(frames);
    self.place(hash, idx + 1);
    idx
  }

  /// The id of `frames`, interned if new: ids are entries' indexes.
  fn intern(&mut self, hash: u64, frames: &[usize]) -> u32 {
    match self.get(hash, frames) {
      Some(id) => id,
      None => {
        let id = self.entries.len() as u32;
        self.insert(hash, frames, id)
      }
    }
  }

  fn len(&self) -> usize {
    self.entries.len()
  }

  /// The frames of entry `idx`.
  fn frames(&self, idx: usize) -> &[usize] {
    self.frames_of(&self.entries[idx])
  }

  fn frames_of(&self, entry: &Entry) -> &[usize] {
    &self.frames[entry.start..entry.start + entry.len as usize]
  }

  fn grow(&mut self) {
    let len = (self.slots.len() * 2).max(64);
    self.slots = vec![0; len];
    for idx in 0..self.entries.len() {
      self.place(self.entries[idx].hash, idx as u32 + 1);
    }
  }

  /// The slot a probe for `hash` starts at: the hash's top bits, its best mixed.
  fn home(&self, hash: u64) -> usize {
    (hash >> (64 - self.slots.len().trailing_zeros())) as usize
  }

  fn place(&mut self, hash: u64, slot: u32) {
    let mask = self.slots.len() - 1;
    let mut at = self.home(hash);
    while self.slots[at] != 0 {
      at = (at + 1) & mask;
    }
    self.slots[at] = slot;
  }
}

impl Default for Interner {
  fn default() -> Interner {
    Interner::new()
  }
}

/// What a block was when it was allocated: its stack, size and time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Live {
  stack: u32,
  size: usize,
  at: u64,
}

/// The live blocks, split by address so threads seldom wait on one another.
struct LiveBlocks<'a>(&'a [Shard]);

impl LiveBlocks<'_> {
  fn new_shards() -> Box<[Shard]> {
    (0..SHARDS).map(|_| Shard::default()).collect()
  }

  fn clear(&self) {
    for shard in self.0 {
      *shard.0.lock() = HashMap::default();
    }
  }

  fn insert(&self, ptr: usize, live: Live) {
    self.shard(ptr).lock().insert(ptr, live);
  }

  #[cfg(test)]
  fn len(&self) -> usize {
    self.0.iter().map(|s| s.0.lock().len()).sum()
  }

  fn remove(&self, ptr: usize) -> Option<Live> {
    self.shard(ptr).lock().remove(&ptr)
  }

  fn shard(&self, ptr: usize) -> &Spin<HashMap<usize, Live, BuildHasherDefault<PtrHasher>>> {
    // Bits apart from those the map inside the shard picks its buckets with.
    &self.0[(PtrHasher::mix(ptr) >> 40) as usize % self.0.len()].0
  }
}

/// What a thread has recorded: summed with every other thread's at [`finish`].
#[derive(Default)]
struct Local {
  /// Stacks this thread has seen, to the index of their site in `sites`.
  stacks: Interner,
  sites: Vec<LocalSite>,
  /// Per stack id, the index of its site in `sites` plus one; `0` if none yet.
  by_id: Vec<u32>,
  /// Per 100 ms, the bytes allocated in it.
  buckets: Vec<u64>,
  /// Each event's time and what it did to the live total, not yet played into
  /// [`TIMELINE`]: in time order, as they happened on this thread.
  log: Vec<(u64, i64)>,
  /// Bytes allocated per (bucket, stack), as each site moves to a later bucket.
  bucket_stacks: Vec<(u32, u32, u64)>,
  allocs: u64,
  frees: u64,
  reallocs: u64,
  bytes_allocated: u64,
  bytes_freed: u64,
  allocator_ns: u64,
}

impl Local {
  fn allocated(&mut self, ptr: usize, size: usize, site: usize, at: u64) {
    let size = size as u64;
    self.allocs += 1;
    self.bytes_allocated += size;
    let bucket = (at / BUCKET_NS) as u32;
    let s = &mut self.sites[site];
    s.allocs += 1;
    s.bytes += size;
    s.live_count = s.live_count.wrapping_add(1);
    s.sizes[log2(size)] += 1;
    if s.bucket != bucket {
      if s.bucket != u32::MAX {
        self.bucket_stacks.push((s.bucket, s.id, s.bucket_bytes));
      }
      s.bucket = bucket;
      s.bucket_bytes = 0;
    }
    s.bucket_bytes += size;
    let id = s.id;
    if let Some(shared) = site_live(id) {
      let now = shared.live.fetch_add(size, Ordering::Relaxed) + size;
      if now > shared.peak.load(Ordering::Relaxed) {
        shared.peak.fetch_max(now, Ordering::Relaxed);
      }
    }
    let bucket = bucket as usize;
    if self.buckets.len() <= bucket {
      self.buckets.resize(bucket + 1, 0);
    }
    self.buckets[bucket] += size;
    // Last: once the block is in the map another thread may free it, and its counts must
    // be in before that free takes them out.
    if let Some(blocks) = live_blocks() {
      blocks.insert(
        ptr,
        Live {
          stack: id,
          size: size as usize,
          at,
        },
      );
    }
  }

  fn freed(&mut self, live: Live, at: u64) {
    let size = live.size as u64;
    self.frees += 1;
    self.bytes_freed += size;
    if let Some(shared) = site_live(live.stack) {
      shared.live.fetch_sub(size, Ordering::Relaxed);
    }
    let site = self.site_of(live.stack);
    let s = &mut self.sites[site];
    s.frees += 1;
    s.freed_bytes += size;
    s.live_count = s.live_count.wrapping_sub(1);
    s.lifetimes[log2(at.saturating_sub(live.at))] += 1;
  }

  /// The index in `sites` of the site of `frames`, interning the stack if new.
  fn site(&mut self, frames: &[usize]) -> usize {
    let hash = hash_frames(frames);
    if let Some(site) = self.stacks.get(hash, frames) {
      return site as usize;
    }
    let id = {
      let mut stacks = STACKS.lock().unwrap_or_else(|e| e.into_inner());
      let id = stacks.intern(hash, frames);
      ensure_site_live(id);
      id
    };
    let site = self.site_of(id);
    self.stacks.insert(hash, frames, site as u32);
    site
  }

  /// The index in `sites` of stack `id`'s site, added if new.
  fn site_of(&mut self, id: u32) -> usize {
    let id = id as usize;
    if self.by_id.len() <= id {
      self.by_id.resize(id + 1, 0);
    }
    if self.by_id[id] == 0 {
      self.sites.push(LocalSite {
        id: id as u32,
        ..LocalSite::default()
      });
      self.by_id[id] = self.sites.len() as u32;
    }
    self.by_id[id] as usize - 1
  }
}

/// One thread's share of a site: what only adds up.
struct LocalSite {
  id: u32,
  allocs: u64,
  bytes: u64,
  frees: u64,
  freed_bytes: u64,
  reallocs: u64,
  /// Wraps below zero when this thread frees more of the site's blocks than it allocated.
  live_count: u64,
  lifetimes: [u64; HISTOGRAM],
  sizes: [u64; HISTOGRAM],
  /// The bucket this thread last allocated from the site in (`u32::MAX`: none) and the
  /// bytes since, until a later bucket moves them to `Local::bucket_stacks`.
  bucket: u32,
  bucket_bytes: u64,
}

impl Default for LocalSite {
  fn default() -> LocalSite {
    LocalSite {
      id: 0,
      allocs: 0,
      bytes: 0,
      frees: 0,
      freed_bytes: 0,
      reallocs: 0,
      live_count: 0,
      lifetimes: [0; HISTOGRAM],
      sizes: [0; HISTOGRAM],
      bucket: u32::MAX,
      bucket_bytes: 0,
    }
  }
}

/// Hashes a pointer for [`LiveBlocks`]: a multiply, then the high half folded into the
/// low, since blocks of one size class differ only in a few middle bits.
#[derive(Default)]
struct PtrHasher(u64);

impl PtrHasher {
  fn mix(ptr: usize) -> u64 {
    let h = (ptr as u64).wrapping_mul(MIX);
    h ^ (h >> 32)
  }
}

impl Hasher for PtrHasher {
  fn finish(&self) -> u64 {
    self.0
  }

  fn write(&mut self, bytes: &[u8]) {
    for byte in bytes {
      self.0 = (self.0.rotate_left(5) ^ *byte as u64).wrapping_mul(MIX);
    }
  }

  fn write_usize(&mut self, ptr: usize) {
    self.0 = PtrHasher::mix(ptr);
  }
}

/// The time samples' stacks, written by the signal handler only.
struct Ring(UnsafeCell<[[usize; DEPTH]; RING]>);

// SAFETY: each slot is written by one handler (claimed through `SAMPLED`) and read only
// after the timer has stopped.
unsafe impl Sync for Ring {}

/// One shard of [`LiveBlocks`], on a cache line of its own.
#[derive(Default)]
#[repr(align(128))]
struct Shard(Spin<HashMap<usize, Live, BuildHasherDefault<PtrHasher>>>);

/// One site's live bytes, shared by every thread, and their peak; on a cache line of its
/// own, so threads busy with different sites do not slow one another.
#[derive(Default)]
#[repr(align(128))]
struct SiteLive {
  live: AtomicU64,
  peak: AtomicU64,
}

/// A lock that spins instead of sleeping, for the recorder's short sections: it lives
/// where it is put (std's mutex on macOS allocates its own, beside the others, so locks
/// meant to be apart share cache lines), and a thread seldom waits on it.
struct Spin<T> {
  held: AtomicBool,
  value: UnsafeCell<T>,
}

impl<T> Spin<T> {
  const fn new(value: T) -> Spin<T> {
    Spin {
      held: AtomicBool::new(false),
      value: UnsafeCell::new(value),
    }
  }

  fn try_lock(&self) -> Option<SpinGuard<'_, T>> {
    self
      .held
      .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
      .is_ok()
      // Lazily: a guard made and dropped would let the lock go.
      .then(|| SpinGuard { lock: self })
  }

  fn lock(&self) -> SpinGuard<'_, T> {
    let mut spins = 0u32;
    while self
      .held
      .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
      .is_err()
    {
      while self.held.load(Ordering::Relaxed) {
        spins += 1;
        if spins < 64 {
          std::hint::spin_loop();
        } else {
          // The holder may have been preempted: let it run.
          std::thread::yield_now();
        }
      }
    }
    SpinGuard { lock: self }
  }
}

impl<T: Default> Default for Spin<T> {
  fn default() -> Spin<T> {
    Spin::new(T::default())
  }
}

// SAFETY: the value is reached only through a guard, and one guard at a time exists.
unsafe impl<T: Send> Sync for Spin<T> {}

struct SpinGuard<'a, T> {
  lock: &'a Spin<T>,
}

impl<T> std::ops::Deref for SpinGuard<'_, T> {
  type Target = T;

  fn deref(&self) -> &T {
    // SAFETY: this guard holds the lock.
    unsafe { &*self.lock.value.get() }
  }
}

impl<T> std::ops::DerefMut for SpinGuard<'_, T> {
  fn deref_mut(&mut self) -> &mut T {
    // SAFETY: this guard holds the lock.
    unsafe { &mut *self.lock.value.get() }
  }
}

impl<T> Drop for SpinGuard<'_, T> {
  fn drop(&mut self) {
    self.lock.held.store(false, Ordering::Release);
  }
}

/// The live total, played from every thread's log in time order: one order of events
/// as they could have happened, the same as a single lock on every event would give,
/// without every thread meeting on one counter.
struct Timeline {
  /// Signed: two threads' events in one tick of the clock may be played free first.
  live: i64,
  peak: i64,
  peak_at: u64,
  /// Per 100 ms, the live total after its last event (`0` when none).
  buckets: Vec<u64>,
}

impl Timeline {
  const fn new() -> Timeline {
    Timeline {
      live: 0,
      peak: 0,
      peak_at: 0,
      buckets: Vec::new(),
    }
  }

  fn apply(&mut self, at: u64, delta: i64) {
    self.live += delta;
    if self.live > self.peak {
      self.peak = self.live;
      self.peak_at = at;
    }
    let bucket = (at / BUCKET_NS) as usize;
    if self.buckets.len() <= bucket {
      self.buckets.resize(bucket + 1, 0);
    }
    self.buckets[bucket] = self.live.max(0) as u64;
  }

  /// Plays `runs`, each in time order, merged into one: by time, then by run, each run's
  /// events kept in their order.
  fn play(&mut self, runs: &[Vec<(u64, i64)>]) {
    let mut next = vec![0usize; runs.len()];
    let mut heads: BinaryHeap<Reverse<(u64, usize)>> = runs
      .iter()
      .enumerate()
      .filter_map(|(idx, run)| run.first().map(|e| Reverse((e.0, idx))))
      .collect();
    while let Some(Reverse((_, idx))) = heads.pop() {
      // This run's events up to the next run's head need no heap.
      let until = heads.peek().map_or((u64::MAX, usize::MAX), |h| h.0);
      let run = &runs[idx];
      let mut at = next[idx];
      while let Some(&(time, delta)) = run.get(at)
        && (time, idx) < until
      {
        self.apply(time, delta);
        at += 1;
      }
      next[idx] = at;
      if let Some(&(time, _)) = run.get(at) {
        heads.push(Reverse((time, idx)));
      }
    }
  }
}

/// Stops recording: the profile, with the time samples and what was still live.
pub fn finish() -> Profile {
  stop_sampling();
  RECORDING.store(false, Ordering::SeqCst);
  let _busy = Busy::enter();
  let now = elapsed();
  let mut profile = Profile::default();
  // Locking each thread's share waits out an event in flight; one that starts later sees
  // recording stopped. Only then is everything shared still.
  let locals = LOCALS.lock().unwrap_or_else(|e| e.into_inner()).clone();
  for local in &locals {
    drop(local.lock());
  }
  let mut stacks = std::mem::take(&mut *STACKS.lock().unwrap_or_else(|e| e.into_inner()));
  let mut sites = vec![Site::default(); stacks.len()];
  let mut timeline = TIMELINE.lock();
  play(&mut timeline, u64::MAX);
  for local in locals {
    let mut local = local.lock();
    let local = std::mem::take(&mut *local);
    profile.allocs += local.allocs;
    profile.frees += local.frees;
    profile.reallocs += local.reallocs;
    profile.bytes_allocated += local.bytes_allocated;
    profile.bytes_freed += local.bytes_freed;
    profile.allocator_ns += local.allocator_ns;
    for s in &local.sites {
      let site = &mut sites[s.id as usize];
      site.allocs += s.allocs;
      site.bytes += s.bytes;
      site.frees += s.frees;
      site.freed_bytes += s.freed_bytes;
      site.reallocs += s.reallocs;
      site.live_count = site.live_count.wrapping_add(s.live_count);
      for idx in 0..HISTOGRAM {
        site.lifetimes[idx] += s.lifetimes[idx];
        site.sizes[idx] += s.sizes[idx];
      }
      if s.bucket != u32::MAX {
        *profile.bucket_stacks.entry((s.bucket, s.id)).or_default() += s.bucket_bytes;
      }
    }
    for (bucket, id, bytes) in local.bucket_stacks {
      *profile.bucket_stacks.entry((bucket, id)).or_default() += bytes;
    }
    if profile.buckets.len() < local.buckets.len() {
      profile.buckets.resize(local.buckets.len(), (0, 0));
    }
    for (idx, allocated) in local.buckets.into_iter().enumerate() {
      profile.buckets[idx].1 += allocated;
    }
  }
  if profile.buckets.len() < timeline.buckets.len() {
    profile.buckets.resize(timeline.buckets.len(), (0, 0));
  }
  for (idx, live) in timeline.buckets.iter().enumerate() {
    profile.buckets[idx].0 = *live;
  }
  for (id, site) in sites.iter_mut().enumerate() {
    if let Some(shared) = site_live(id as u32) {
      site.live_bytes = shared.live.load(Ordering::Relaxed);
      site.peak_live = shared.peak.load(Ordering::Relaxed);
    }
  }
  profile.live_bytes = timeline.live.max(0) as u64;
  (profile.peak_live, profile.peak_at) = (timeline.peak as u64, timeline.peak_at);
  profile.allocator_ns += PLAY_NS.load(Ordering::Relaxed);
  drop(timeline);
  // The live total at the end, as at every event.
  let bucket = (now / BUCKET_NS) as usize;
  if profile.buckets.len() <= bucket {
    profile.buckets.resize(bucket + 1, (0, 0));
  }
  profile.buckets[bucket].0 = profile.live_bytes;
  profile.elapsed_ns = now;
  // The time samples, interned like allocation stacks.
  let taken = SAMPLED.load(Ordering::SeqCst).min(RING);
  let mut samples: HashMap<u32, u64> = HashMap::new();
  for slot in 0..taken {
    // SAFETY: the timer is stopped, so no handler writes the ring any more.
    let frames = unsafe { &(*SAMPLES.0.get())[slot] };
    let len = frames.iter().position(|f| *f == 0).unwrap_or(DEPTH);
    if len > 0 {
      let id = stacks.intern(hash_frames(&frames[..len]), &frames[..len]);
      *samples.entry(id).or_default() += 1;
    }
  }
  sites.resize(stacks.len(), Site::default());
  profile.stacks = (0..stacks.len())
    .map(|id| stacks.frames(id).to_vec())
    .collect();
  profile.sites = sites;
  profile.time_samples = samples.into_iter().collect();
  profile.time_samples.sort();
  profile.dropped_samples = SAMPLED.load(Ordering::SeqCst).saturating_sub(RING) as u64;
  profile.sample_interval_us = INTERVAL_US as u64;
  profile.anchor = crate::anchor_address();
  profile
}

/// `floor(log2(n))`, `0` for `0` and `1`, at most the histogram's last bucket.
pub fn log2(n: u64) -> usize {
  ((u64::BITS - n.max(1).leading_zeros() - 1) as usize).min(HISTOGRAM - 1)
}

/// Starts recording: CPU-time samples when `time`, allocations when `memory`; [`finish`]
/// ends it and returns the profile. Recording allocations costs several times the work
/// itself, so time is best sampled in a pass of its own.
pub fn start(time: bool, memory: bool) {
  BUSY.get_or_init(new_key);
  BOUNDS.get_or_init(new_key);
  LOCAL.get_or_init(new_key);
  {
    let _busy = Busy::enter();
    TIMEBASE.get_or_init(timebase);
    *STACKS.lock().unwrap_or_else(|e| e.into_inner()) = Interner::new();
    for chunk in &SITE_LIVE {
      let chunk = chunk.load(Ordering::Acquire);
      if !chunk.is_null() {
        // SAFETY: a chunk `ensure_site_live` leaked, `CHUNK` sites long.
        for site in unsafe { std::slice::from_raw_parts(chunk, CHUNK) } {
          site.live.store(0, Ordering::Relaxed);
          site.peak.store(0, Ordering::Relaxed);
        }
      }
    }
    match LIVE_BLOCKS.get() {
      Some(shards) => LiveBlocks(shards).clear(),
      None => {
        let _ = LIVE_BLOCKS.set(LiveBlocks::new_shards());
      }
    }
    for local in LOCALS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
      *local.lock() = Local::default();
    }
    *TIMELINE.lock() = Timeline::new();
    PLAY_NS.store(0, Ordering::SeqCst);
  }
  BASE_NS.store(now_ns(), Ordering::SeqCst);
  SAMPLED.store(0, Ordering::SeqCst);
  RECORDING.store(memory, Ordering::SeqCst);
  if time {
    start_sampling();
  }
}

/// This thread's stack bounds, read the first time (inside the recorder, so what reading
/// them allocates is not recorded).
fn bounds() -> (usize, usize) {
  let known = get(&BOUNDS) as *const (usize, usize);
  if !known.is_null() {
    // SAFETY: set below to a leaked, never-freed pair.
    return unsafe { *known };
  }
  let read = walk::bounds();
  set(
    &BOUNDS,
    Box::into_raw(Box::new(read)) as *const libc::c_void,
  );
  read
}

/// This thread's stack bounds if already read, else `(0, 0)`; for the signal handler.
fn cached_bounds() -> (usize, usize) {
  let known = get(&BOUNDS) as *const (usize, usize);
  if known.is_null() {
    (0, 0)
  } else {
    // SAFETY: as in `bounds`.
    unsafe { *known }
  }
}

fn elapsed() -> u64 {
  now_ns().saturating_sub(BASE_NS.load(Ordering::Relaxed))
}

/// Makes sure stack `id` has its shared live bytes; called as ids are given out, under
/// the [`STACKS`] lock.
fn ensure_site_live(id: u32) {
  let Some(chunk) = SITE_LIVE.get(id as usize / CHUNK) else {
    return;
  };
  if chunk.load(Ordering::Acquire).is_null() {
    let sites: Box<[SiteLive]> = (0..CHUNK).map(|_| SiteLive::default()).collect();
    chunk.store(Box::leak(sites).as_mut_ptr(), Ordering::Release);
  }
}

fn get(key: &OnceLock<libc::pthread_key_t>) -> *mut libc::c_void {
  match key.get() {
    // SAFETY: a key this module created.
    Some(key) => unsafe { libc::pthread_getspecific(*key) },
    None => std::ptr::null_mut(),
  }
}

/// Hashes a stack: four independent lanes, so the multiplies overlap instead of waiting on
/// one another.
fn hash_frames(frames: &[usize]) -> u64 {
  let mut lanes = [1u64, 2, 3, 4];
  let (chunks, rest) = frames.as_chunks::<4>();
  for chunk in chunks {
    for (lane, frame) in lanes.iter_mut().zip(chunk) {
      *lane = (lane.rotate_left(5) ^ *frame as u64).wrapping_mul(MIX);
    }
  }
  for (lane, frame) in lanes.iter_mut().zip(rest) {
    *lane = (lane.rotate_left(5) ^ *frame as u64).wrapping_mul(MIX);
  }
  lanes.iter().fold(frames.len() as u64, |h, lane| {
    (h.rotate_left(5) ^ lane).wrapping_mul(MIX)
  })
}

fn live_blocks() -> Option<LiveBlocks<'static>> {
  LIVE_BLOCKS.get().map(|shards| LiveBlocks(shards))
}

/// This thread's [`Local`], made and registered the first time (inside the recorder).
fn local() -> &'static Spin<Local> {
  let known = get(&LOCAL) as *const Spin<Local>;
  if !known.is_null() {
    // SAFETY: set below to a leaked, never-freed `Spin<Local>`.
    return unsafe { &*known };
  }
  let made: &'static Spin<Local> = Box::leak(Box::new(Spin::new(Local::default())));
  LOCALS.lock().unwrap_or_else(|e| e.into_inner()).push(made);
  set(&LOCAL, made as *const Spin<Local> as *const libc::c_void);
  made
}

fn new_key() -> libc::pthread_key_t {
  let mut key: libc::pthread_key_t = 0;
  // SAFETY: creates a key with no destructor.
  unsafe {
    libc::pthread_key_create(&mut key, None);
  }
  key
}

// Never inlined: `SKIP` counts on this frame and the allocator entry point's above it.
#[inline(never)]
fn note(event: Event) {
  if !RECORDING.load(Ordering::Relaxed) {
    return;
  }
  let Some(_busy) = Busy::try_enter() else {
    return;
  };
  let mut local = local().lock();
  // `finish` may have run since the first look.
  if !RECORDING.load(Ordering::Relaxed) {
    return;
  }
  // Read holding the lock, so `play` can tell which events are all in.
  let started = now_ns();
  let at = started.saturating_sub(BASE_NS.load(Ordering::Relaxed));
  let mut frames = [0usize; DEPTH + SKIP];
  let mut delta = 0i64;
  match event {
    Event::Alloc(ptr, size) => {
      let site = local.site(walk_here(&mut frames));
      local.allocated(ptr, size, site, at);
      delta += size as i64;
    }
    Event::Free(ptr) => {
      if let Some(live) = live_blocks().and_then(|b| b.remove(ptr)) {
        local.freed(live, at);
        delta -= live.size as i64;
      }
    }
    Event::Realloc(old, ptr, size) => {
      local.reallocs += 1;
      if let Some(old) = old {
        local.freed(old, at);
        delta -= old.size as i64;
      }
      let site = local.site(walk_here(&mut frames));
      local.sites[site].reallocs += 1;
      local.allocated(ptr, size, site, at);
      delta += size as i64;
    }
  }
  // Every event, even one that changed nothing: it still says what was live when.
  local.log.push((at, delta));
  let full = local.log.len() >= LOG;
  local.allocator_ns += now_ns().saturating_sub(started);
  drop(local);
  // Unless another thread is at it already, in which case it takes this log too.
  if full && let Some(mut timeline) = TIMELINE.try_lock() {
    play(&mut timeline, elapsed().saturating_sub(1_000_000));
  }
}

/// A monotonic clock, in nanoseconds, read twice an event, so the cheapest there is: on
/// ARM the virtual counter, which user code may read (one instruction; macOS's
/// `mach_absolute_time` adds a barrier that costs more than the rest of an event).
#[cfg(target_arch = "aarch64")]
fn now_ns() -> u64 {
  let ticks: u64;
  // SAFETY: reads a register user code is allowed to.
  unsafe {
    // Not `nomem`: the read must stay after the lock taken before it (see `play`).
    std::arch::asm!("mrs {}, cntvct_el0", out(reg) ticks, options(nostack, preserves_flags));
  }
  let per_tick = TIMEBASE.get().copied().unwrap_or(1 << 32);
  ((ticks as u128 * per_tick as u128) >> 32) as u64
}

#[cfg(all(not(target_arch = "aarch64"), target_os = "macos"))]
fn now_ns() -> u64 {
  unsafe extern "C" {
    fn mach_absolute_time() -> u64;
  }
  // SAFETY: reads the clock; no arguments.
  let ticks = unsafe { mach_absolute_time() };
  let per_tick = TIMEBASE.get().copied().unwrap_or(1 << 32);
  ((ticks as u128 * per_tick as u128) >> 32) as u64
}

#[cfg(all(not(target_arch = "aarch64"), not(target_os = "macos")))]
fn now_ns() -> u64 {
  let mut now = libc::timespec {
    tv_sec: 0,
    tv_nsec: 0,
  };
  // SAFETY: writes the time into `now`.
  unsafe {
    libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now);
  }
  now.tv_sec as u64 * 1_000_000_000 + now.tv_nsec as u64
}

extern "C" fn on_sample(_: libc::c_int, _: *mut libc::siginfo_t, context: *mut libc::c_void) {
  let slot = SAMPLED.fetch_add(1, Ordering::Relaxed);
  if slot >= RING || context.is_null() {
    return;
  }
  // SAFETY: the kernel passes the interrupted thread's context; the slot is ours alone.
  unsafe {
    let (pc, fp) = registers(context);
    let frames = &mut (*SAMPLES.0.get())[slot];
    frames[0] = pc;
    let len = walk::walk(fp, cached_bounds(), frames, 1);
    // The slot may hold a longer stack from an earlier recording.
    if len < DEPTH {
      frames[len] = 0;
    }
  }
}

/// Plays every thread's events before `horizon` into `timeline`, leaving the rest logged.
///
/// An event's time is read under its thread's lock and logged before it is let go, so
/// once each thread's lock has been taken after `horizon`, every event before it is in a
/// log, and any still to come is later. Mid-run, `horizon` is a millisecond back from
/// now, well beyond how far a processor may read the clock ahead of the lock.
fn play(timeline: &mut Timeline, horizon: u64) {
  let started = now_ns();
  let locals = LOCALS.lock().unwrap_or_else(|e| e.into_inner()).clone();
  let runs: Vec<Vec<(u64, i64)>> = locals
    .iter()
    .map(|local| {
      let mut local = local.lock();
      let split = local.log.partition_point(|e| e.0 < horizon);
      let rest = local.log.split_off(split);
      std::mem::replace(&mut local.log, rest)
    })
    .collect();
  timeline.play(&runs);
  PLAY_NS.fetch_add(now_ns().saturating_sub(started), Ordering::Relaxed);
}

fn put_live(ptr: usize, live: Live) {
  if let Some(_busy) = Busy::try_enter()
    && let Some(blocks) = live_blocks()
  {
    blocks.insert(ptr, live);
  }
}

/// The interrupted program counter and frame pointer.
///
/// # Safety
///
/// `context` is the `ucontext_t` a signal handler was given.
unsafe fn registers(context: *mut libc::c_void) -> (usize, usize) {
  let context = context as *const libc::ucontext_t;
  // SAFETY: as the caller vouches.
  unsafe {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
      let state = &(*(*context).uc_mcontext).__ss;
      (state.__pc as usize, state.__fp as usize)
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
      let state = &(*(*context).uc_mcontext).__ss;
      (state.__rip as usize, state.__rbp as usize)
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
      let state = &(*context).uc_mcontext;
      (state.pc as usize, state.regs[29] as usize)
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
      let regs = &(*context).uc_mcontext.gregs;
      (
        regs[libc::REG_RIP as usize] as usize,
        regs[libc::REG_RBP as usize] as usize,
      )
    }
  }
}

fn set(key: &OnceLock<libc::pthread_key_t>, value: *const libc::c_void) {
  if let Some(key) = key.get() {
    // SAFETY: a key this module created.
    unsafe {
      libc::pthread_setspecific(*key, value);
    }
  }
}

unsafe fn set_timer(microseconds: i64) {
  let interval = libc::timeval {
    tv_sec: 0,
    tv_usec: microseconds as _,
  };
  let timer = libc::itimerval {
    it_interval: interval,
    it_value: interval,
  };
  // SAFETY: a plain system call on a valid struct.
  unsafe {
    libc::setitimer(libc::ITIMER_PROF, &timer, std::ptr::null_mut());
  }
}

/// Stack `id`'s shared live bytes, once [`ensure_site_live`] has made them.
fn site_live(id: u32) -> Option<&'static SiteLive> {
  let id = id as usize;
  let chunk = SITE_LIVE.get(id / CHUNK)?.load(Ordering::Acquire);
  // SAFETY: a chunk `ensure_site_live` leaked, `CHUNK` sites long; never freed.
  (!chunk.is_null()).then(|| unsafe { &*chunk.add(id % CHUNK) })
}

fn start_sampling() {
  // SAFETY: installs a handler that only touches the preallocated ring and atomics, then
  // arms the process's CPU-time timer.
  unsafe {
    let mut action: libc::sigaction = std::mem::zeroed();
    action.sa_sigaction = on_sample as *const () as usize;
    action.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
    libc::sigemptyset(&mut action.sa_mask);
    libc::sigaction(libc::SIGPROF, &action, std::ptr::null_mut());
    set_timer(INTERVAL_US);
  }
}

fn stop_sampling() {
  // SAFETY: disarms the timer; the handler stays installed but no longer fires.
  unsafe {
    set_timer(0);
  }
}

fn take_live(ptr: usize) -> Option<Live> {
  if !RECORDING.load(Ordering::Relaxed) {
    return None;
  }
  let _busy = Busy::try_enter()?;
  live_blocks()?.remove(ptr)
}

/// The clock's nanoseconds per tick, in 32.32 fixed point.
#[cfg(target_arch = "aarch64")]
fn timebase() -> u64 {
  let hertz: u64;
  // SAFETY: reads a register user code is allowed to.
  unsafe {
    std::arch::asm!("mrs {}, cntfrq_el0", out(reg) hertz, options(nomem, nostack, preserves_flags));
  }
  if hertz == 0 {
    1 << 32
  } else {
    ((1_000_000_000u128 << 32) / hertz as u128) as u64
  }
}

#[cfg(all(not(target_arch = "aarch64"), target_os = "macos"))]
fn timebase() -> u64 {
  #[repr(C)]
  struct Timebase {
    numer: u32,
    denom: u32,
  }
  unsafe extern "C" {
    fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
  }
  let mut info = Timebase { numer: 0, denom: 0 };
  // SAFETY: writes into `info`.
  let ok = unsafe { mach_timebase_info(&mut info) } == 0;
  if ok && info.denom != 0 {
    ((info.numer as u64) << 32) / info.denom as u64
  } else {
    1 << 32
  }
}

#[cfg(all(not(target_arch = "aarch64"), not(target_os = "macos")))]
fn timebase() -> u64 {
  1 << 32
}

/// This thread's stack, without the recorder's frames.
#[inline(always)]
fn walk_here(frames: &mut [usize; DEPTH + SKIP]) -> &[usize] {
  // SAFETY: this thread's own frame pointer.
  let len = unsafe { walk::walk(walk::frame_pointer(), bounds(), frames, 0) };
  &frames[SKIP.min(len)..len]
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Enough small blocks, made and freed one by one, to fill a thread's log.
  #[inline(never)]
  fn churn(n: usize) -> usize {
    (0..n)
      .map(|i| std::hint::black_box(Box::new(i)).count_ones() as usize)
      .sum()
  }

  #[inline(never)]
  fn make(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| vec![0u8; 1000 + i]).collect()
  }

  #[inline(never)]
  fn make_odd(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|_| vec![1u8; 12_345]).collect()
  }

  #[test]
  fn test_a_spin_lock_tried_while_held_stays_held() {
    let lock = Spin::new(1);
    let mut held = lock.lock();
    assert!(lock.try_lock().is_none());
    assert!(lock.try_lock().is_none());
    *held += 1;
    drop(held);
    let again = lock.try_lock().map(|g| *g);
    assert_eq!(again, Some(2));
  }

  #[test]
  fn test_interner_finds_what_it_was_given() {
    let mut stacks = Interner::new();
    let all: Vec<Vec<usize>> = (0..1000)
      .map(|i| (0..1 + i % 50).map(|f| f * 7 + i).collect())
      .collect();
    for (idx, frames) in all.iter().enumerate() {
      assert_eq!(stacks.get(hash_frames(frames), frames), None);
      assert_eq!(stacks.intern(hash_frames(frames), frames), idx as u32);
    }
    for (idx, frames) in all.iter().enumerate() {
      assert_eq!(stacks.intern(hash_frames(frames), frames), idx as u32);
      assert_eq!(stacks.frames(idx), &frames[..]);
    }
    assert_eq!(stacks.len(), 1000);
  }

  #[test]
  fn test_interner_tells_apart_stacks_that_share_a_hash() {
    let mut stacks = Interner::new();
    for value in 0..100u32 {
      stacks.insert(42, &[value as usize, 1], value * 3);
    }
    for value in 0..100u32 {
      assert_eq!(stacks.get(42, &[value as usize, 1]), Some(value * 3));
    }
    assert_eq!(stacks.get(42, &[100, 1]), None);
    assert_eq!(stacks.get(43, &[0, 1]), None);
  }

  #[test]
  fn test_live_blocks_match_frees_from_any_thread() {
    let shards = LiveBlocks::new_shards();
    let blocks = LiveBlocks(&shards);
    let live = |n: usize| Live {
      stack: n as u32,
      size: n * 16,
      at: n as u64,
    };
    std::thread::scope(|scope| {
      for t in 0..4 {
        let blocks = &blocks;
        scope.spawn(move || {
          for n in (t * 1000..(t + 1) * 1000).map(|n| n * 16 + 0x1000) {
            blocks.insert(n, live(n));
          }
        });
      }
    });
    assert_eq!(blocks.len(), 4000);
    std::thread::scope(|scope| {
      for t in 0..4 {
        let blocks = &blocks;
        // Each thread frees another's blocks.
        scope.spawn(move || {
          let other = (t + 1) % 4;
          for n in (other * 1000..(other + 1) * 1000).map(|n| n * 16 + 0x1000) {
            assert_eq!(blocks.remove(n), Some(live(n)));
            assert_eq!(blocks.remove(n), None);
          }
        });
      }
    });
    assert_eq!(blocks.len(), 0);
    blocks.insert(0x10, live(1));
    blocks.clear();
    assert_eq!(blocks.remove(0x10), None);
  }

  /// One test for both recordings: the recorder is global, so tests of it cannot overlap.
  #[test]
  fn test_recording_counts_frees_and_stacks() {
    start(false, true);
    let blocks = make(50);
    let kept = vec![7u8; 4096];
    drop(blocks);
    // Blocks made on threads that end before recording does, freed on this one.
    let made: Vec<Vec<Vec<u8>>> = std::thread::scope(|scope| {
      let handles: Vec<_> = (0..4).map(|_| scope.spawn(|| make_odd(100))).collect();
      handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    drop(made);
    // Logs filled and played on several threads while recording.
    std::thread::scope(|scope| {
      for _ in 0..3 {
        scope.spawn(|| churn(LOG + 1000));
      }
    });
    let profile = finish();
    drop(kept);
    // Other tests run beside this one, so their allocations may be counted too.
    assert!(
      profile.allocs as usize >= 451 + 3 * LOG,
      "{}",
      profile.allocs
    );
    assert!(profile.bytes_allocated >= 50 * 1000 + 4096 + 400 * 12_345);
    let mine = profile
      .sites
      .iter()
      .filter(|s| s.allocs >= 50 && s.bytes >= 50 * 1000)
      .count();
    assert!(mine >= 1, "no site holds the 50 blocks");
    // Every thread's share is in, and their frees were matched on this thread.
    let odd = profile.sites.iter().filter(|s| s.sizes[log2(12_345)] > 0);
    let (allocs, frees) = odd.fold((0, 0), |(a, f), s| (a + s.allocs, f + s.frees));
    assert!(
      allocs >= 400 && frees >= 400,
      "{} allocs, {} frees",
      allocs,
      frees
    );
    // The kept block was live when recording stopped.
    assert!(profile.sites.iter().any(|s| s.live_bytes >= 4096));
    // The sites add up to the totals.
    let sum = |f: fn(&Site) -> u64| profile.sites.iter().map(f).sum::<u64>();
    assert_eq!(sum(|s| s.allocs), profile.allocs);
    assert_eq!(sum(|s| s.frees), profile.frees);
    assert_eq!(sum(|s| s.reallocs), profile.reallocs);
    assert_eq!(sum(|s| s.bytes), profile.bytes_allocated);
    assert_eq!(sum(|s| s.freed_bytes), profile.bytes_freed);
    // Every event was played into the timeline.
    assert_eq!(sum(|s| s.live_bytes), profile.live_bytes);
    assert_eq!(
      profile.buckets.last().map(|b| b.0),
      Some(profile.live_bytes)
    );
    assert_eq!(
      profile.bytes_allocated - profile.bytes_freed,
      profile.live_bytes
    );
    assert_eq!(
      profile.buckets.iter().map(|b| b.1).sum::<u64>(),
      profile.bytes_allocated
    );
    assert_eq!(
      profile.bucket_stacks.values().sum::<u64>(),
      profile.bytes_allocated
    );
    assert!(profile.peak_live >= profile.live_bytes);
    assert!(profile.stacks.iter().all(|s| s.len() <= DEPTH));
    assert!(profile.time_samples.is_empty());
    assert_eq!(log2(1), 0);
    assert_eq!(log2(1024), 10);
    assert_eq!(log2(u64::MAX), crate::profile::HISTOGRAM - 1);
  }

  #[test]
  fn test_timeline_plays_runs_in_time_order() {
    let mut timeline = Timeline::new();
    // Two threads: the second's free at 15 comes between the first's allocations.
    let runs = vec![
      vec![(10, 100), (20, 50), (BUCKET_NS + 5, -150)],
      vec![(5, 30), (15, -30), (20, 7)],
    ];
    timeline.play(&runs);
    // 30, 130, 100, then 150 at 20 (the first run before the second on a tie), 157.
    assert_eq!((timeline.peak, timeline.peak_at), (157, 20));
    assert_eq!(timeline.live, 7);
    assert_eq!(timeline.buckets, vec![157, 7]);
    // Played in rounds, the same.
    let mut rounds = Timeline::new();
    rounds.play(&[vec![(10, 100)], vec![(5, 30)]]);
    rounds.play(&[
      vec![(20, 50), (BUCKET_NS + 5, -150)],
      vec![(15, -30), (20, 7)],
    ]);
    assert_eq!((rounds.peak, rounds.peak_at, rounds.live), (157, 20, 7));
    // A free played before its allocation, in one tick, does not wrap the total.
    let mut tie = Timeline::new();
    tie.play(&[vec![(3, -8)], vec![(3, 8)]]);
    assert_eq!((tie.peak, tie.live), (0, 0));
  }
}
