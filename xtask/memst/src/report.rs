//! Reading a profile: its addresses named from the binary's symbol table, then the views.
//!
//! - **overview:** totals, peak, what was live at exit, churn, the recorder's own cost, the
//!   pipeline stages (by the mtindo module a stack is in), and the live heap over time.
//! - **functions:** per function, CPU time (self: on top of the stack; total: anywhere in
//!   it) and the allocations it made (by the first frame outside the allocator and the
//!   standard library).
//! - **hot:** the most time, bytes and churn; **cold:** blocks never freed, blocks held long,
//!   and mtindo functions this run never reached.
//! - **tree:** the call tree by bytes, allocations or time, from `main` down.
//! - **sites:** the biggest allocation stacks, with their size and lifetime histograms.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;

use object::{Object, ObjectSection, ObjectSymbol, SectionKind, SymbolKind};

use crate::profile::{HISTOGRAM, Profile, Site};

/// The views, in the order `all` prints them.
pub const VIEWS: &[&str] = &["overview", "functions", "hot", "cold", "tree", "sites"];

/// What a view is ranked by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum By {
  Bytes,
  Allocs,
  Time,
}

impl By {
  pub fn parse(text: &str) -> Option<By> {
    match text {
      "bytes" => Some(By::Bytes),
      "allocs" => Some(By::Allocs),
      "time" => Some(By::Time),
      _ => None,
    }
  }
}

/// Per function: CPU samples on top and anywhere, and its allocation sites merged.
#[derive(Default)]
struct Function {
  self_samples: u64,
  total_samples: u64,
  /// Samples in system code (malloc, free, memcpy, the kernel's entry points) this
  /// function called into: the cost of what it asked of the system.
  system_samples: u64,
  site: Site,
}

/// A profile with its stacks named.
pub struct Named<'a> {
  pub profile: &'a Profile,
  /// Per stack, its functions, innermost first.
  pub names: Vec<Vec<&'a str>>,
}

impl<'a> Named<'a> {
  /// Names every stack; a function repeated by recursion is named once.
  pub fn new(profile: &'a Profile, symbols: &'a Symbols) -> Named<'a> {
    let names = profile
      .stacks
      .iter()
      .map(|frames| {
        let mut names: Vec<&str> = frames.iter().map(|f| symbols.name(*f)).collect();
        names.dedup();
        names
      })
      .collect();
    Named { profile, names }
  }

  /// The function an allocation stack is charged to.
  fn site_function(&self, stack: usize) -> &'a str {
    let names = &self.names[stack];
    names
      .iter()
      .find(|n| !plumbing(n))
      .or(names.first())
      .copied()
      .unwrap_or("?")
  }

  fn time(&self) -> HashMap<u32, u64> {
    self.profile.time_samples.iter().copied().collect()
  }
}

#[derive(Default)]
struct Node<'a> {
  value: u64,
  children: BTreeMap<&'a str, Node<'a>>,
}

/// The binary's functions, by address in memory.
pub struct Symbols {
  /// (start, end, name), sorted by start, as linked.
  ranges: Vec<(u64, u64, String)>,
  /// Added to a linked address to give its address in memory.
  slide: i64,
  /// The binary's code, as linked: an address outside belongs to a system library.
  text: Vec<(u64, u64)>,
}

impl Symbols {
  /// Reads `exe`'s symbol table; `anchor` is where `memst_anchor` was in memory.
  pub fn load(exe: &str, anchor: usize) -> Result<Symbols, String> {
    let data = fs::read(exe).map_err(|e| format!("cannot read {}: {}", exe, e))?;
    let file = object::File::parse(&*data).map_err(|e| format!("cannot parse {}: {}", exe, e))?;
    let mut starts: Vec<(u64, u64, String)> = Vec::new();
    let mut linked_anchor = None;
    for symbol in file.symbols() {
      if symbol.kind() != SymbolKind::Text || symbol.address() == 0 {
        continue;
      }
      let Ok(raw) = symbol.name() else {
        continue;
      };
      if raw.trim_start_matches('_') == "memst_anchor" {
        linked_anchor = Some(symbol.address());
      }
      let name = match rustc_demangle::try_demangle(raw) {
        Ok(demangled) => format!("{:#}", demangled),
        // Mach-O puts an underscore before every C name.
        Err(_) if cfg!(target_os = "macos") => raw.strip_prefix('_').unwrap_or(raw).to_string(),
        Err(_) => raw.to_string(),
      };
      starts.push((symbol.address(), symbol.size(), name));
    }
    let linked_anchor =
      linked_anchor.ok_or_else(|| format!("{} has no memst_anchor symbol", exe))?;
    let text = file
      .sections()
      .filter(|s| s.kind() == SectionKind::Text)
      .map(|s| (s.address(), s.address() + s.size()))
      .collect();
    starts.sort();
    starts.dedup_by_key(|s| s.0);
    let mut ranges = Vec::with_capacity(starts.len());
    for idx in 0..starts.len() {
      let (start, size, name) = &starts[idx];
      let next = starts.get(idx + 1).map_or(u64::MAX, |s| s.0);
      let end = if *size > 0 { start + size } else { next };
      ranges.push((*start, end, name.clone()));
    }
    Ok(Symbols {
      ranges,
      slide: anchor as i64 - linked_anchor as i64,
      text,
    })
  }

  /// The function holding a return address (or a program counter).
  pub fn name(&self, address: usize) -> &str {
    // A return address points after its call; one byte back is inside it.
    let linked = (address as i64 - self.slide - 1) as u64;
    if !self
      .text
      .iter()
      .any(|(start, end)| (*start..*end).contains(&linked))
    {
      return "(system)";
    }
    let idx = self.ranges.partition_point(|r| r.0 <= linked);
    match idx.checked_sub(1).map(|i| &self.ranges[i]) {
      Some((_, end, name)) if linked < *end => name,
      _ => "?",
    }
  }

  /// Every mtindo function in the binary.
  fn mtindo_functions(&self) -> impl Iterator<Item = &str> {
    self
      .ranges
      .iter()
      .map(|r| r.2.as_str())
      .filter(|n| n.starts_with("mtindo::") || n.starts_with("<mtindo::"))
  }
}

pub fn bytes(n: u64) -> String {
  let n = n as f64;
  if n >= 1e9 {
    format!("{:.2} GB", n / 1e9)
  } else if n >= 1e6 {
    format!("{:.1} MB", n / 1e6)
  } else if n >= 1e3 {
    format!("{:.1} KB", n / 1e3)
  } else {
    format!("{} B", n)
  }
}

pub fn cold(named: &Named, symbols: &Symbols, top: usize) -> String {
  let mut out = String::from("COLD\n");
  let table = functions(named);
  let mut held: Vec<(&str, &Function)> = table
    .iter()
    .filter(|(_, f)| f.site.live_count > 0)
    .map(|(n, f)| (*n, f))
    .collect();
  held.sort_by_key(|(n, f)| (std::cmp::Reverse(f.site.live_bytes), *n));
  let _ = writeln!(
    out,
    "  allocated, never freed by the end ({} functions):",
    held.len()
  );
  for (name, f) in held.iter().take(top) {
    let _ = writeln!(
      out,
      "    {:>10} in {:>8} blocks  {}",
      bytes(f.site.live_bytes),
      count(f.site.live_count),
      name
    );
  }
  let mut long: Vec<(&str, u64)> = table
    .iter()
    .filter_map(|(n, f)| median(&f.site.lifetimes).map(|m| (*n, m)))
    .filter(|(_, m)| *m >= 1 << 30)
    .collect();
  long.sort_by_key(|(n, m)| (std::cmp::Reverse(*m), *n));
  let _ = writeln!(
    out,
    "  held long (median lifetime a second or more): {}",
    long.len()
  );
  for (name, m) in long.iter().take(top) {
    let _ = writeln!(out, "    {:>10}  {}", nanos(*m), name);
  }
  let seen: HashSet<&str> = named.names.iter().flatten().copied().collect();
  let mut never: Vec<&str> = symbols
    .mtindo_functions()
    .filter(|n| !seen.contains(n))
    .collect();
  never.sort();
  never.dedup();
  let _ = writeln!(
    out,
    "  mtindo functions this run never reached (not inlined away): {}",
    never.len()
  );
  for name in never.iter().take(top) {
    let _ = writeln!(out, "    {}", name);
  }
  out
}

pub fn count(n: u64) -> String {
  let digits = n.to_string();
  let mut out = String::new();
  for (idx, c) in digits.chars().enumerate() {
    if idx > 0 && (digits.len() - idx).is_multiple_of(3) {
      out.push(',');
    }
    out.push(c);
  }
  out
}

pub fn functions_view(named: &Named, by: By, top: usize) -> String {
  let interval = named.profile.sample_interval_us.max(1) * 1000;
  let table = functions(named);
  let mut rows: Vec<(&&str, &Function)> = table.iter().collect();
  rows.sort_by_key(|(name, f)| {
    let key = match by {
      By::Bytes => f.site.bytes,
      By::Allocs => f.site.allocs,
      By::Time => f.self_samples,
    };
    (std::cmp::Reverse(key), **name)
  });
  let mut out = format!(
    "FUNCTIONS by {:?} ({} with samples or allocations)\n  {:<52}{:>9}{:>9}{:>11}{:>11}{:>11}{:>9}{:>11}\n",
    by,
    count(table.len() as u64),
    "FUNCTION",
    "SELF",
    "TOTAL",
    "ALLOCS",
    "BYTES",
    "FREES",
    "LIFE",
    "LIVE END"
  );
  for (name, f) in rows.into_iter().take(top) {
    let _ = writeln!(
      out,
      "  {:<52}{:>9}{:>9}{:>11}{:>11}{:>11}{:>9}{:>11}",
      fit(name, 51),
      nanos(f.self_samples * interval),
      nanos(f.total_samples * interval),
      count(f.site.allocs),
      bytes(f.site.bytes),
      count(f.site.frees),
      median(&f.site.lifetimes).map_or("-".to_string(), nanos),
      bytes(f.site.live_bytes)
    );
  }
  out
}

pub fn hot(named: &Named, top: usize) -> String {
  let interval = named.profile.sample_interval_us.max(1) * 1000;
  let table = functions(named);
  let total_time: u64 = named.profile.time_samples.iter().map(|(_, c)| c).sum();
  let total_bytes = named.profile.bytes_allocated.max(1);
  let mut out = String::from("HOT\n");
  let ranked = |key: &dyn Fn(&Function) -> u64| {
    let mut rows: Vec<(&str, u64)> = table
      .iter()
      .map(|(n, f)| (*n, key(f)))
      .filter(|(_, v)| *v > 0)
      .collect();
    rows.sort_by_key(|(n, v)| (std::cmp::Reverse(*v), *n));
    rows.truncate(top);
    rows
  };
  let _ = writeln!(out, "  by CPU time on top of the stack:");
  for (name, v) in ranked(&|f| f.self_samples) {
    let _ = writeln!(
      out,
      "    {:>5.1}%  {:>9}  {}",
      100.0 * v as f64 / total_time.max(1) as f64,
      nanos(v * interval),
      name
    );
  }
  let _ = writeln!(
    out,
    "  by CPU time in system code (malloc, free, memcpy...) it called, charged to the caller:"
  );
  for (name, v) in ranked(&|f| f.system_samples) {
    let _ = writeln!(
      out,
      "    {:>5.1}%  {:>9}  {}",
      100.0 * v as f64 / total_time.max(1) as f64,
      nanos(v * interval),
      name
    );
  }
  let _ = writeln!(out, "  by bytes allocated:");
  for (name, v) in ranked(&|f| f.site.bytes) {
    let _ = writeln!(
      out,
      "    {:>5.1}%  {:>9}  {}",
      100.0 * v as f64 / total_bytes as f64,
      bytes(v),
      name
    );
  }
  let _ = writeln!(
    out,
    "  by churn (blocks freed within 1ms: candidates for reuse):"
  );
  for (name, v) in ranked(&|f| churn(&f.site)) {
    let _ = writeln!(out, "    {:>12}  {}", count(v), name);
  }
  out
}

/// The bucket holding the middle value of a log2 histogram, as its lower bound; `None` when
/// empty.
pub fn median(histogram: &[u64; HISTOGRAM]) -> Option<u64> {
  let total: u64 = histogram.iter().sum();
  if total == 0 {
    return None;
  }
  let mut seen = 0;
  for (idx, count) in histogram.iter().enumerate() {
    seen += count;
    if seen * 2 >= total {
      return Some(1u64 << idx);
    }
  }
  None
}

pub fn nanos(n: u64) -> String {
  let n = n as f64;
  if n >= 1e9 {
    format!("{:.2}s", n / 1e9)
  } else if n >= 1e6 {
    format!("{:.1}ms", n / 1e6)
  } else if n >= 1e3 {
    format!("{:.0}µs", n / 1e3)
  } else {
    format!("{}ns", n)
  }
}

pub fn overview(named: &Named) -> String {
  let p = named.profile;
  let mut out = String::new();
  let sampled: u64 = p.time_samples.iter().map(|(_, c)| c).sum();
  let interval = p.sample_interval_us.max(1) * 1000;
  let _ = writeln!(out, "OVERVIEW");
  let _ = writeln!(
    out,
    "  time pass {}: CPU time sampled {} ({} samples, {} dropped)",
    nanos(p.time_elapsed_ns),
    nanos(sampled * interval),
    count(sampled),
    count(p.dropped_samples)
  );
  let _ = writeln!(
    out,
    "  memory pass {}: every allocation recorded, {:.1}x the time pass",
    nanos(p.elapsed_ns),
    p.elapsed_ns as f64 / p.time_elapsed_ns.max(1) as f64
  );
  let _ = writeln!(
    out,
    "  allocations {}  frees {}  reallocs {}",
    count(p.allocs),
    count(p.frees),
    count(p.reallocs)
  );
  let live_count: u64 = p.sites.iter().map(|s| s.live_count).sum();
  let _ = writeln!(
    out,
    "  allocated {}  freed {}  peak live {} at {}  live at end {} in {} blocks",
    bytes(p.bytes_allocated),
    bytes(p.bytes_freed),
    bytes(p.peak_live),
    nanos(p.peak_at),
    bytes(p.live_bytes),
    count(live_count)
  );
  let mut all = Site::default();
  for site in &p.sites {
    merge(&mut all, site);
  }
  let _ = writeln!(out, "  sizes     {}", histogram_line(&all.sizes, bytes));
  let _ = writeln!(out, "  lifetimes {}", histogram_line(&all.lifetimes, nanos));
  let _ = writeln!(
    out,
    "  churn {:.0}% of frees within 1ms",
    100.0 * churn(&all) as f64 / all.frees.max(1) as f64
  );
  // Stages: CPU, allocations, bytes, live at end.
  let mut stages: BTreeMap<&str, (u64, u64, u64, u64)> = BTreeMap::new();
  for (stack, c) in named.time() {
    stages
      .entry(stage(&named.names[stack as usize]))
      .or_default()
      .0 += c;
  }
  for (stack, site) in p.sites.iter().enumerate() {
    let entry = stages.entry(stage(&named.names[stack])).or_default();
    entry.1 += site.allocs;
    entry.2 += site.bytes;
    entry.3 += site.live_bytes;
  }
  let _ = writeln!(
    out,
    "\n  {:<10}{:>10}{:>14}{:>12}{:>14}",
    "STAGE", "CPU", "ALLOCS", "BYTES", "LIVE AT END"
  );
  for (name, (cpu, allocs, b, live)) in &stages {
    let _ = writeln!(
      out,
      "  {:<10}{:>10}{:>14}{:>12}{:>14}",
      name,
      nanos(cpu * interval),
      count(*allocs),
      bytes(*b),
      bytes(*live)
    );
  }
  let live: Vec<u64> = p.buckets.iter().map(|b| b.0).collect();
  if !live.is_empty() {
    let _ = writeln!(
      out,
      "\n  live heap, every 100ms ({} max):\n  {}",
      bytes(live.iter().copied().max().unwrap_or(0)),
      sparkline(&live)
    );
  }
  out
}

/// Whether a frame belongs to the allocator, the recorder or the standard library: not where
/// an allocation is attributed.
pub fn plumbing(name: &str) -> bool {
  let name = name.trim_start_matches('<');
  [
    "memst::",
    "alloc::",
    "core::",
    "std::",
    "hashbrown::",
    "__rust",
    "__rdl",
    "_rust",
    "?",
    "(system)",
  ]
  .iter()
  .any(|p| name.starts_with(p))
}

/// Whether a frame is memst's recorder (or its allocator entry), whose CPU time is the
/// cost of measuring.
pub fn recorder(name: &str) -> bool {
  let name = name.trim_start_matches('<');
  [
    "memst::record::note",
    "memst::record::take_live",
    "memst::record::put_live",
    "memst::record::State",
  ]
  .iter()
  .any(|p| name.starts_with(p))
}

pub fn sites(named: &Named, top: usize) -> String {
  // Stacks that differ only in how deep a recursion went are one site.
  let mut merged: BTreeMap<&[&str], (usize, Site)> = BTreeMap::new();
  for (id, site) in named.profile.sites.iter().enumerate() {
    if site.allocs == 0 {
      continue;
    }
    let entry = merged
      .entry(named.names[id].as_slice())
      .or_insert_with(|| (id, Site::default()));
    merge(&mut entry.1, site);
  }
  let mut rows: Vec<(usize, Site)> = merged.into_values().collect();
  rows.sort_by_key(|(id, s)| (std::cmp::Reverse(s.bytes), *id));
  let mut out = format!("SITES (allocation stacks), the {} largest by bytes\n", top);
  for (id, site) in rows.into_iter().take(top) {
    let site = &site;
    let _ = writeln!(
      out,
      "  {}  {} allocs, {}, {} freed, {} live at end, peak live {}",
      named.site_function(id),
      count(site.allocs),
      bytes(site.bytes),
      count(site.frees),
      bytes(site.live_bytes),
      bytes(site.peak_live)
    );
    let _ = writeln!(out, "    size      {}", histogram_line(&site.sizes, bytes));
    let _ = writeln!(
      out,
      "    lifetime  {}",
      histogram_line(&site.lifetimes, nanos)
    );
    let path: Vec<&str> = named.names[id]
      .iter()
      .filter(|n| !plumbing(n))
      .take(8)
      .copied()
      .collect();
    let _ = writeln!(out, "    stack     {}", path.join(" ← "));
  }
  out
}

/// The pipeline stage a stack is in, by the innermost mtindo module on it; `recording`
/// for CPU time spent in memst's own recorder, which is not mto's.
pub fn stage(names: &[&str]) -> &'static str {
  if names.iter().any(|n| recorder(n)) {
    return "recording";
  }
  const STAGES: &[(&str, &str)] = &[
    ("mtindo::syntax::lexer", "lex"),
    ("mtindo::syntax", "parse"),
    ("mtindo::layout", "layout"),
    ("mtindo::order", "layout"),
    ("mtindo::format", "format"),
    ("mtindo::verify", "verify"),
    ("mtindo::config", "config"),
    ("mtindo::options", "config"),
    ("mtindo::history", "history"),
  ];
  for name in names {
    if let Some((_, stage)) = STAGES.iter().find(|(module, _)| name.contains(module)) {
      return stage;
    }
  }
  "other"
}

pub fn tree(named: &Named, by: By, depth: usize) -> String {
  let mut root = Node::default();
  let mut add = |stack: usize, value: u64| {
    if value == 0 {
      return;
    }
    root.value += value;
    let mut node = &mut root;
    // Outermost first, with the allocator's own frames at the tip folded away.
    let names = &named.names[stack];
    let tip = names.iter().position(|n| !plumbing(n)).unwrap_or(0);
    for name in names[tip..].iter().rev() {
      node = node.children.entry(name).or_default();
      node.value += value;
    }
  };
  match by {
    By::Time => {
      for (stack, c) in named.time() {
        if !named.names[stack as usize].iter().any(|n| recorder(n)) {
          add(stack as usize, c);
        }
      }
    }
    _ => {
      for (stack, site) in named.profile.sites.iter().enumerate() {
        add(
          stack,
          if by == By::Bytes {
            site.bytes
          } else {
            site.allocs
          },
        );
      }
    }
  }
  let mut out = format!("CALL TREE by {:?} (branches under 1% left out)\n", by);
  fn print(out: &mut String, node: &Node, total: u64, indent: usize, depth: usize) {
    if indent >= depth {
      return;
    }
    let mut children: Vec<(&&str, &Node)> = node.children.iter().collect();
    children.sort_by_key(|(n, c)| (std::cmp::Reverse(c.value), **n));
    for (name, child) in children {
      if child.value * 100 < total {
        continue;
      }
      let _ = writeln!(
        out,
        "  {:>5.1}% {}{}",
        100.0 * child.value as f64 / total as f64,
        "  ".repeat(indent),
        fit(name, 90)
      );
      // A chain with one child is printed flat, to keep deep stacks readable.
      print(out, child, total, indent + 1, depth);
    }
  }
  print(&mut out, &root, root.value.max(1), 0, depth);
  out
}

/// Blocks freed within about a millisecond (2^20 ns).
fn churn(site: &Site) -> u64 {
  site.lifetimes[..20].iter().sum()
}

/// Shortens a function name to fit a column, keeping its end.
fn fit(name: &str, width: usize) -> String {
  let chars: Vec<char> = name.chars().collect();
  if chars.len() <= width {
    return name.to_string();
  }
  let tail: String = chars[chars.len() - (width - 1)..].iter().collect();
  format!("…{}", tail)
}

fn functions<'a>(named: &Named<'a>) -> BTreeMap<&'a str, Function> {
  let mut out: BTreeMap<&str, Function> = BTreeMap::new();
  for (stack, count) in named.time() {
    let names = &named.names[stack as usize];
    // The recorder's time is not mto's; the overview shows it.
    if names.iter().any(|n| recorder(n)) {
      continue;
    }
    if let Some(top) = names.first() {
      out.entry(top).or_default().self_samples += count;
    }
    if names.first() == Some(&"(system)")
      && let Some(caller) = names.iter().find(|n| !plumbing(n))
    {
      out.entry(caller).or_default().system_samples += count;
    }
    let mut seen = HashSet::new();
    for name in names {
      if seen.insert(*name) {
        out.entry(name).or_default().total_samples += count;
      }
    }
  }
  for (stack, site) in named.profile.sites.iter().enumerate() {
    if site.allocs == 0 {
      continue;
    }
    merge(
      &mut out.entry(named.site_function(stack)).or_default().site,
      site,
    );
  }
  out
}

/// A histogram as a sparkline from its first to its last non-empty bucket, with the range.
fn histogram_line(histogram: &[u64; HISTOGRAM], unit: fn(u64) -> String) -> String {
  let Some(first) = histogram.iter().position(|v| *v != 0) else {
    return "-".to_string();
  };
  let last = histogram.iter().rposition(|v| *v != 0).unwrap_or(first);
  format!(
    "{}  {} … {}, median {}",
    sparkline(&histogram[first..=last]),
    unit(1 << first),
    unit(1 << last),
    median(histogram).map_or("-".to_string(), unit)
  )
}

fn merge(into: &mut Site, from: &Site) {
  into.allocs += from.allocs;
  into.bytes += from.bytes;
  into.frees += from.frees;
  into.freed_bytes += from.freed_bytes;
  into.reallocs += from.reallocs;
  into.live_count += from.live_count;
  into.live_bytes += from.live_bytes;
  into.peak_live += from.peak_live;
  for i in 0..HISTOGRAM {
    into.lifetimes[i] += from.lifetimes[i];
    into.sizes[i] += from.sizes[i];
  }
}

fn sparkline(values: &[u64]) -> String {
  const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
  let max = values.iter().copied().max().unwrap_or(0).max(1);
  values
    .iter()
    .map(|v| BARS[((*v as f64 / max as f64) * 7.0).round() as usize])
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn named<'a>(profile: &'a Profile, names: Vec<Vec<&'a str>>) -> Named<'a> {
    Named { profile, names }
  }

  fn names() -> Vec<Vec<&'static str>> {
    vec![
      vec![
        "alloc::raw_vec::RawVec::grow",
        "mtindo::format::expr::rewrite_list",
        "mtindo::format::file::format_file",
        "main",
      ],
      vec!["mtindo::syntax::lexer::tokenize", "main"],
      vec![
        "mtindo::verify::format::verify_format",
        "mtindo::format::file::format_file",
        "main",
      ],
    ]
  }

  fn sample() -> Profile {
    Profile {
      elapsed_ns: 1_000_000_000,
      time_elapsed_ns: 250_000_000,
      allocs: 30,
      bytes_allocated: 3000,
      sample_interval_us: 1000,
      stacks: vec![vec![1], vec![2], vec![3]],
      sites: vec![site(20, 2000, 0), site(10, 1000, 2), Site::default()],
      time_samples: vec![(2, 40), (0, 10)],
      buckets: vec![(500, 3000)],
      ..Profile::default()
    }
  }

  fn site(allocs: u64, bytes: u64, live: u64) -> Site {
    let mut site = Site {
      allocs,
      bytes,
      frees: allocs - live,
      live_count: live,
      live_bytes: if live > 0 { bytes / allocs * live } else { 0 },
      peak_live: bytes,
      ..Site::default()
    };
    site.lifetimes[12] = allocs - live;
    site.sizes[6] = allocs;
    site
  }

  #[test]
  fn test_allocations_are_charged_past_the_plumbing() {
    let p = sample();
    let n = named(&p, names());
    assert_eq!(n.site_function(0), "mtindo::format::expr::rewrite_list");
    let table = functions(&n);
    assert_eq!(table["mtindo::format::expr::rewrite_list"].site.bytes, 2000);
    assert_eq!(
      table["mtindo::verify::format::verify_format"].self_samples,
      40
    );
    // format_file is on two sampled stacks: 40 + 10 samples anywhere, none on top.
    assert_eq!(table["mtindo::format::file::format_file"].total_samples, 50);
    assert_eq!(table["mtindo::format::file::format_file"].self_samples, 0);
  }

  #[test]
  fn test_numbers_read_well() {
    assert_eq!(count(12841120), "12,841,120");
    assert_eq!(bytes(2_310_000_000), "2.31 GB");
    assert_eq!(nanos(41_000), "41µs");
    let mut h = [0u64; HISTOGRAM];
    h[3] = 1;
    h[5] = 3;
    assert_eq!(median(&h), Some(32));
    assert!(plumbing(
      "<alloc::vec::Vec<T> as core::clone::Clone>::clone"
    ));
    assert!(!plumbing("<mtindo::syntax::ast::Item>::kind"));
    assert_eq!(fit("abcdefgh", 5), "…efgh");
  }

  #[test]
  fn test_stages_follow_the_innermost_module() {
    assert_eq!(
      stage(&[
        "core::x",
        "mtindo::syntax::lexer::tokenize",
        "mtindo::layout::layout"
      ]),
      "lex"
    );
    assert_eq!(stage(&["<mtindo::syntax::ast::Item>::kind"]), "parse");
    assert_eq!(stage(&["main"]), "other");
    assert_eq!(
      stage(&["memst::record::note", "mtindo::syntax::parser::bump"]),
      "recording"
    );
  }

  #[test]
  fn test_views_render_what_they_should() {
    let p = sample();
    let n = named(&p, names());
    let overview = overview(&n);
    assert!(overview.contains("allocations 30"), "{}", overview);
    assert!(overview.contains("4.0x the time pass"), "{}", overview);
    assert!(overview.contains("format"), "{}", overview);
    let tree = tree(&n, By::Bytes, 10);
    let lines: Vec<&str> = tree.lines().collect();
    assert!(lines[1].contains("100.0% main"), "{}", tree);
    assert!(tree.contains("rewrite_list"), "{}", tree);
    assert!(!tree.contains("RawVec"), "{}", tree);
    let funcs = functions_view(&n, By::Time, 5);
    assert!(
      funcs.lines().nth(2).unwrap().contains("verify_format"),
      "{}",
      funcs
    );
    let sites = sites(&n, 5);
    assert!(
      sites.contains("rewrite_list ← mtindo::format::file::format_file ← main"),
      "{}",
      sites
    );
  }
}
