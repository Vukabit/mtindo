//! A recorded profile and its file: plain text, one fact per line, so it can be read by
//! hand, diffed and kept beside a run.
//!
//! ```text
//! memst 1
//! exe /path/to/memst            the binary, for its symbols
//! anchor 0x104a3c120            where `memst_anchor` was in memory, for the load offset
//! elapsed_ns 8412000000        the memory pass
//! time_elapsed_ns 2104000000   the time pass
//! totals ALLOCS FREES REALLOCS ALLOCATED FREED PEAK_LIVE PEAK_AT LIVE ALLOCATOR_NS
//! samples INTERVAL_US DROPPED
//! stack ID 0x... 0x...          return addresses, innermost first
//! site ID ALLOCS BYTES FREES FREED REALLOCS LIVE_COUNT LIVE_BYTES PEAK_LIVE
//!      lifetimes=... sizes=...  log2 histograms (nanoseconds, bytes)
//! time ID COUNT                 CPU-time samples whose stack is ID
//! bucket INDEX LIVE ALLOCATED   per 100 ms
//! bucket_stack INDEX ID BYTES
//! ```

use std::fmt::Write as _;

/// Buckets of the live-heap timeline, in nanoseconds.
pub const BUCKET_NS: u64 = 100_000_000;

/// Log2 histogram buckets.
pub const HISTOGRAM: usize = 48;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
  pub exe: String,
  pub anchor: usize,
  /// How long the memory pass ran (recording slows it).
  pub elapsed_ns: u64,
  /// How long the time pass ran, unslowed.
  pub time_elapsed_ns: u64,
  pub allocs: u64,
  pub frees: u64,
  pub reallocs: u64,
  pub bytes_allocated: u64,
  pub bytes_freed: u64,
  pub peak_live: u64,
  pub peak_at: u64,
  pub live_bytes: u64,
  pub allocator_ns: u64,
  pub sample_interval_us: u64,
  pub dropped_samples: u64,
  pub stacks: Vec<Vec<usize>>,
  /// By stack id; stacks only seen by the time sampler have empty sites.
  pub sites: Vec<Site>,
  pub time_samples: Vec<(u32, u64)>,
  /// Per bucket: live bytes at its end, bytes allocated in it.
  pub buckets: Vec<(u64, u64)>,
  pub bucket_stacks: std::collections::HashMap<(u32, u32), u64>,
}

impl Profile {
  pub fn to_text(&self) -> String {
    let mut out = String::from("memst 1\n");
    let _ = writeln!(out, "exe {}", self.exe);
    let _ = writeln!(out, "anchor {:#x}", self.anchor);
    let _ = writeln!(out, "elapsed_ns {}", self.elapsed_ns);
    let _ = writeln!(out, "time_elapsed_ns {}", self.time_elapsed_ns);
    let _ = writeln!(
      out,
      "totals {} {} {} {} {} {} {} {} {}",
      self.allocs,
      self.frees,
      self.reallocs,
      self.bytes_allocated,
      self.bytes_freed,
      self.peak_live,
      self.peak_at,
      self.live_bytes,
      self.allocator_ns
    );
    let _ = writeln!(
      out,
      "samples {} {}",
      self.sample_interval_us, self.dropped_samples
    );
    for (id, frames) in self.stacks.iter().enumerate() {
      let frames: Vec<String> = frames.iter().map(|f| format!("{:#x}", f)).collect();
      let _ = writeln!(out, "stack {} {}", id, frames.join(" "));
    }
    for (id, s) in self.sites.iter().enumerate() {
      if s.allocs == 0 {
        continue;
      }
      let _ = writeln!(
        out,
        "site {} {} {} {} {} {} {} {} {} lifetimes={} sizes={}",
        id,
        s.allocs,
        s.bytes,
        s.frees,
        s.freed_bytes,
        s.reallocs,
        s.live_count,
        s.live_bytes,
        s.peak_live,
        histogram(&s.lifetimes),
        histogram(&s.sizes)
      );
    }
    for (id, count) in &self.time_samples {
      let _ = writeln!(out, "time {} {}", id, count);
    }
    for (idx, (live, allocated)) in self.buckets.iter().enumerate() {
      let _ = writeln!(out, "bucket {} {} {}", idx, live, allocated);
    }
    let mut keys: Vec<&(u32, u32)> = self.bucket_stacks.keys().collect();
    keys.sort();
    for key in keys {
      let _ = writeln!(
        out,
        "bucket_stack {} {} {}",
        key.0, key.1, self.bucket_stacks[key]
      );
    }
    out
  }

  /// Takes a time pass's samples (recorded in this same process, so at the same
  /// addresses) into this memory pass's profile.
  pub fn absorb_time(&mut self, time: Profile) {
    let mut ids: std::collections::HashMap<Vec<usize>, u32> = self
      .stacks
      .iter()
      .enumerate()
      .map(|(id, frames)| (frames.clone(), id as u32))
      .collect();
    let mut samples = Vec::new();
    for (stack, count) in time.time_samples {
      let frames = &time.stacks[stack as usize];
      let id = *ids.entry(frames.clone()).or_insert_with(|| {
        self.stacks.push(frames.clone());
        self.sites.push(Site::default());
        (self.stacks.len() - 1) as u32
      });
      samples.push((id, count));
    }
    samples.sort();
    self.time_samples = samples;
    self.sample_interval_us = time.sample_interval_us;
    self.dropped_samples = time.dropped_samples;
    self.time_elapsed_ns = time.elapsed_ns;
  }

  pub fn from_text(text: &str) -> Result<Profile, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
      Some((_, "memst 1")) => {}
      _ => return Err("not a memst profile (no `memst 1` first line)".to_string()),
    }
    let mut p = Profile::default();
    for (n, line) in lines {
      let bad = || format!("line {}: cannot read `{}`", n + 1, line);
      let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
      let words: Vec<&str> = rest.split_whitespace().collect();
      let num = |i: usize| -> Result<u64, String> {
        words.get(i).and_then(|w| w.parse().ok()).ok_or_else(bad)
      };
      let hex = |w: &str| usize::from_str_radix(w.trim_start_matches("0x"), 16).map_err(|_| bad());
      match key {
        "exe" => p.exe = rest.to_string(),
        "anchor" => p.anchor = hex(rest)?,
        "elapsed_ns" => p.elapsed_ns = num(0)?,
        "time_elapsed_ns" => p.time_elapsed_ns = num(0)?,
        "totals" => {
          p.allocs = num(0)?;
          p.frees = num(1)?;
          p.reallocs = num(2)?;
          p.bytes_allocated = num(3)?;
          p.bytes_freed = num(4)?;
          p.peak_live = num(5)?;
          p.peak_at = num(6)?;
          p.live_bytes = num(7)?;
          p.allocator_ns = num(8)?;
        }
        "samples" => {
          p.sample_interval_us = num(0)?;
          p.dropped_samples = num(1)?;
        }
        "stack" => {
          let id = num(0)? as usize;
          if id != p.stacks.len() {
            return Err(bad());
          }
          p.stacks.push(
            words[1..]
              .iter()
              .map(|w| hex(w))
              .collect::<Result<_, _>>()?,
          );
          p.sites.push(Site::default());
        }
        "site" => {
          let id = num(0)? as usize;
          let field = |prefix: &str| {
            words
              .iter()
              .find_map(|w| w.strip_prefix(prefix))
              .and_then(parse_histogram)
              .ok_or_else(bad)
          };
          let site = Site {
            allocs: num(1)?,
            bytes: num(2)?,
            frees: num(3)?,
            freed_bytes: num(4)?,
            reallocs: num(5)?,
            live_count: num(6)?,
            live_bytes: num(7)?,
            peak_live: num(8)?,
            lifetimes: field("lifetimes=")?,
            sizes: field("sizes=")?,
          };
          *p.sites.get_mut(id).ok_or_else(bad)? = site;
        }
        "time" => p.time_samples.push((num(0)? as u32, num(1)?)),
        "bucket" => {
          let idx = num(0)? as usize;
          if p.buckets.len() <= idx {
            p.buckets.resize(idx + 1, (0, 0));
          }
          p.buckets[idx] = (num(1)?, num(2)?);
        }
        "bucket_stack" => {
          p.bucket_stacks
            .insert((num(0)? as u32, num(1)? as u32), num(2)?);
        }
        _ => return Err(bad()),
      }
    }
    Ok(p)
  }
}

/// What one allocation stack did.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
  pub allocs: u64,
  pub bytes: u64,
  pub frees: u64,
  pub freed_bytes: u64,
  pub reallocs: u64,
  pub live_count: u64,
  pub live_bytes: u64,
  pub peak_live: u64,
  /// How long blocks lived before being freed, by log2 of nanoseconds.
  pub lifetimes: [u64; HISTOGRAM],
  /// Block sizes, by log2 of bytes.
  pub sizes: [u64; HISTOGRAM],
}

impl Default for Site {
  fn default() -> Site {
    Site {
      allocs: 0,
      bytes: 0,
      frees: 0,
      freed_bytes: 0,
      reallocs: 0,
      live_count: 0,
      live_bytes: 0,
      peak_live: 0,
      lifetimes: [0; HISTOGRAM],
      sizes: [0; HISTOGRAM],
    }
  }
}

fn histogram(values: &[u64; HISTOGRAM]) -> String {
  let last = values.iter().rposition(|v| *v != 0).map_or(0, |i| i + 1);
  values[..last]
    .iter()
    .map(u64::to_string)
    .collect::<Vec<_>>()
    .join(",")
}

fn parse_histogram(text: &str) -> Option<[u64; HISTOGRAM]> {
  let mut out = [0; HISTOGRAM];
  if text.is_empty() {
    return Some(out);
  }
  for (idx, value) in text.split(',').enumerate() {
    *out.get_mut(idx)? = value.parse().ok()?;
  }
  Some(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_profile_round_trips() {
    let mut site = Site {
      allocs: 3,
      bytes: 300,
      frees: 2,
      freed_bytes: 200,
      reallocs: 1,
      live_count: 1,
      live_bytes: 100,
      peak_live: 200,
      ..Site::default()
    };
    site.lifetimes[10] = 2;
    site.sizes[6] = 3;
    let mut p = Profile {
      exe: "/bin/memst run".to_string(),
      anchor: 0x1000,
      elapsed_ns: 5,
      allocs: 3,
      peak_live: 200,
      stacks: vec![vec![0x10, 0x20], vec![0x30]],
      sites: vec![site, Site::default()],
      time_samples: vec![(1, 7)],
      buckets: vec![(100, 300)],
      ..Profile::default()
    };
    p.bucket_stacks.insert((0, 0), 300);
    let text = p.to_text();
    assert_eq!(Profile::from_text(&text), Ok(p));
  }

  #[test]
  fn test_errors_name_the_line() {
    assert!(Profile::from_text("hello").is_err());
    let err = Profile::from_text("memst 1\nstack 3 0x1\n").unwrap_err();
    assert!(err.starts_with("line 2:"), "{}", err);
  }

  #[test]
  fn test_time_samples_join_the_memory_pass() {
    let mut memory = Profile {
      stacks: vec![vec![1, 2]],
      sites: vec![Site::default()],
      elapsed_ns: 9,
      ..Profile::default()
    };
    let time = Profile {
      stacks: vec![vec![5], vec![1, 2]],
      time_samples: vec![(0, 3), (1, 4)],
      elapsed_ns: 2,
      sample_interval_us: 1000,
      ..Profile::default()
    };
    memory.absorb_time(time);
    assert_eq!(memory.stacks, vec![vec![1, 2], vec![5]]);
    assert_eq!(memory.time_samples, vec![(0, 4), (1, 3)]);
    assert_eq!((memory.elapsed_ns, memory.time_elapsed_ns), (9, 2));
    assert_eq!(memory.sites.len(), 2);
  }
}
