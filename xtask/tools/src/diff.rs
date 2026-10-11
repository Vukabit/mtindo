//! `cargo xtask diff`: how much of the Rust style mto reproduces, measured against the
//! standard formatter (S3-1, S3-2, S3-6, S3-7).
//!
//! Three input sets, each file scored by whether mto's output equals the answer exactly, and
//! by how many of the answer's lines it has (their longest common subsequence):
//!
//! - **tests:** the standard formatter's own test pairs; the answer is the expected file.
//!   Pairs whose input sets options with `// rustfmt-KEY: VALUE` lines are a set of their
//!   own, since mto does not read those lines yet.
//! - **corpus:** every corpus file as fetched; the answer is the standard formatter's output,
//!   run as `cargo fmt` runs it (the crate's edition, its configuration file), with an empty
//!   home folder so the machine's own settings cannot leak in.
//! - **corpus, unformatted:** every corpus file with its whitespace scrambled
//!   ([`crate::unformat`]); the answer is the standard formatter's output for the scrambled
//!   text.
//!
//! The standard formatter's outputs are cached by input, edition, configuration file and
//! version, so a rerun only runs mto. Mismatching files are grouped by the syntax kind of the
//! innermost node at their first differing line. The numbers are compared with the record in
//! `compat/compatibility.txt`, which they may only climb above: a drop fails the run, and
//! `--update` records new numbers.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::history::sha256;
use mtindo::options::format::{FormatConfig, NewlineStyle};
use mtindo::syntax::{Edition, SyntaxElement, SyntaxKind, parse};

use crate::common::edition_for;
use crate::unformat::unformat;

/// The mismatch groups the report lists.
const TOP_KINDS: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Set {
  Tests,
  TestsWithSettings,
  Corpus,
  Unformatted,
}

impl Set {
  const ALL: [Set; 4] = [
    Set::Tests,
    Set::TestsWithSettings,
    Set::Corpus,
    Set::Unformatted,
  ];

  fn key(self) -> &'static str {
    match self {
      Set::Tests => "tests",
      Set::TestsWithSettings => "tests-with-settings",
      Set::Corpus => "corpus",
      Set::Unformatted => "corpus-unformatted",
    }
  }

  fn title(self) -> &'static str {
    match self {
      Set::Tests => "test pairs, default settings",
      Set::TestsWithSettings => "test pairs with settings",
      Set::Corpus => "corpus",
      Set::Unformatted => "corpus, unformatted",
    }
  }
}

/// One input to score.
struct Case {
  set: Set,
  name: String,
  input: String,
  edition: Edition,
  /// Where the standard formatter runs, for its configuration lookup.
  dir: PathBuf,
  /// The answer, when it is given rather than computed (test pairs).
  expected: Option<String>,
  /// The formatting settings mto uses for the case: its folder's, as mto resolves them.
  format: FormatConfig,
}

struct Options {
  tests: Option<PathBuf>,
  reference: String,
  cache: PathBuf,
  record: PathBuf,
  update: bool,
  markdown: bool,
  repos: Vec<(String, PathBuf)>,
  /// Where the files the meaning check flags are listed.
  meaning_log: Option<PathBuf>,
  /// Where the files mto breaks are listed in full.
  broke_log: Option<PathBuf>,
  /// The files whose flags are known: the formatter's bugs, each with its reason.
  exceptions: Option<PathBuf>,
}

/// One scored case.
#[derive(Default)]
struct Score {
  /// The answer could not be had: the standard formatter failed on the input.
  skipped: bool,
  matches: bool,
  lines: usize,
  matched_lines: usize,
  /// For a mismatch: the first differing line (1-based) and the syntax kind there.
  first: Option<(usize, String)>,
  /// The meaning check (S3-3) run on the answer itself: what it reports, if anything.
  meaning: Option<String>,
  /// What is wrong with mto's own output: the meaning check fails on it, or a second run
  /// changes it (S3-3, S3-4).
  own: Option<String>,
  /// The answer is the input as written, and mto changed it: a file mto breaks.
  broke: bool,
}

/// The numbers for one set: files matched of scored, lines matched of the answers' lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
  files: usize,
  matched_files: usize,
  lines: usize,
  matched_lines: usize,
  skipped: usize,
}

/// Every `.rs` file under `dir`, without `.git` and `target` folders.
pub fn collect_rs(dir: &Path, files: &mut Vec<PathBuf>) {
  let Ok(entries) = fs::read_dir(dir) else {
    return;
  };
  for entry in entries.flatten() {
    let path = entry.path();
    let Ok(kind) = entry.file_type() else {
      continue;
    };
    if kind.is_dir() {
      let name = entry.file_name();
      if name != ".git" && name != "target" {
        collect_rs(&path, files);
      }
    } else if kind.is_file() && path.extension().is_some_and(|e| e == "rs") {
      files.push(path);
    }
  }
}

pub fn run(args: &[String]) -> ExitCode {
  let options = match parse_args(args) {
    Ok(options) => options,
    Err(e) => {
      eprintln!("xtask-tools diff: {}", e);
      return ExitCode::from(2);
    }
  };
  // The corpus and the test pairs sit inside this repository, whose own settings must not
  // reach them: a `root = true` mto.toml ends mto's search upwards, and an empty
  // rustfmt.toml ends both tools' search for one (empty, so cached answers keep their key).
  if let Some(downloads) = options.cache.parent() {
    let fences = [
      (
        "mto.toml",
        "# Written by `cargo xtask diff`: nothing here takes settings from above.\nroot = true\n",
      ),
      ("rustfmt.toml", ""),
    ];
    for (name, text) in fences {
      let fence = downloads.join(name);
      if !fence.exists() {
        let _ = fs::write(&fence, text);
      }
    }
  }
  let cases = collect_cases(&options);
  eprintln!(
    "scoring {} inputs against {}...",
    cases.len(),
    options.reference
  );
  let home = options.cache.join("home");
  let _ = fs::create_dir_all(&home);
  let scores = score_all(&cases, &options, &home);
  let mut tallies: BTreeMap<Set, Tally> = BTreeMap::new();
  let mut kinds: BTreeMap<String, (usize, String)> = BTreeMap::new();
  for (case, score) in cases.iter().zip(&scores) {
    let tally = tallies.entry(case.set).or_default();
    if score.skipped {
      tally.skipped += 1;
      continue;
    }
    tally.files += 1;
    tally.lines += score.lines;
    tally.matched_lines += score.matched_lines;
    if score.matches {
      tally.matched_files += 1;
    } else if let Some((line, kind)) = &score.first {
      let entry = kinds
        .entry(kind.clone())
        .or_insert_with(|| (0, format!("{}:{}", case.name, line)));
      entry.0 += 1;
    }
  }
  print!(
    "{}",
    report(&tallies, &kinds, &options.reference, options.markdown)
  );
  // The meaning check on the standard formatter's own output: anything it flags is a
  // rewrite the allowed list lacks, or a bug of the formatter's (F2).
  let flagged: Vec<String> = cases
    .iter()
    .zip(&scores)
    .filter_map(|(case, score)| {
      score
        .meaning
        .as_ref()
        .map(|m| format!("{} [{}]: {}", case.name, case.set.key(), m))
    })
    .collect();
  let checked = scores.iter().filter(|s| !s.skipped).count();
  if let Some(log) = &options.meaning_log {
    let _ = fs::write(log, flagged.join("\n") + "\n");
  }
  let known = options
    .exceptions
    .as_ref()
    .map(|path| exceptions(path))
    .unwrap_or_default();
  let keys: Vec<String> = flagged
    .iter()
    .map(|f| f.split("]:").next().unwrap_or(f).to_string() + "]")
    .collect();
  let unexpected: Vec<&String> = flagged
    .iter()
    .zip(&keys)
    .filter(|(_, key)| !known.contains(key))
    .map(|(line, _)| line)
    .collect();
  let stale: Vec<&String> = known.iter().filter(|k| !keys.contains(k)).collect();
  println!(
    "\nmeaning check on the standard formatter's output: {} of {} files flagged, {} \
         of them listed in compat/meaning-exceptions.txt",
    flagged.len(),
    checked,
    flagged.len() - unexpected.len()
  );
  for entry in &stale {
    println!("  listed but no longer flagged (remove it): {}", entry);
  }
  // Files the standard formatter leaves as written and mto changes: where a rule is wrong,
  // not missing. Each with the first line that differs and the syntax there.
  let broke: Vec<String> = cases
    .iter()
    .zip(&scores)
    .filter(|(_, score)| score.broke)
    .map(|(case, score)| {
      let (line, kind) = score.first.clone().unwrap_or_default();
      format!("{}:{} [{}] {}", case.name, line, case.set.key(), kind)
    })
    .collect();
  println!(
    "files the standard formatter leaves as written and mto changes: {}",
    broke.len()
  );
  for line in broke.iter().take(20) {
    println!("  {}", line);
  }
  if let Some(log) = &options.broke_log {
    let _ = fs::write(log, broke.join("\n") + "\n");
    if let Some(dir) = log.parent() {
      compare_scores(&dir.join("diff-scores.txt"), &cases, &scores);
    }
  }
  let own: Vec<String> = cases
    .iter()
    .zip(&scores)
    .filter_map(|(case, score)| {
      score
        .own
        .as_ref()
        .map(|m| format!("{} [{}]: {}", case.name, case.set.key(), m))
    })
    .collect();
  println!(
    "mto's own output: {} file(s) fail the meaning check or change on a second run",
    own.len()
  );
  // The inputs, saved beside the log for reproducing each failure (not as `.rs`, so walks
  // pass over them); cleared on every run.
  if let Some(dir) = options.broke_log.as_ref().and_then(|log| log.parent()) {
    let dir = dir.join("diff-own");
    let _ = fs::remove_dir_all(&dir);
    if !own.is_empty() {
      let _ = fs::create_dir_all(&dir);
      for (case, _) in cases.iter().zip(&scores).filter(|(_, s)| s.own.is_some()) {
        let name = format!("{}-{}.input", case.set.key(), case.name.replace('/', "__"));
        let _ = fs::write(dir.join(name), &case.input);
      }
    }
  }
  if !own.is_empty() {
    for line in own.iter().take(20) {
      println!("  {}", line.chars().take(200).collect::<String>());
    }
    let _ = ratchet(&options, &tallies);
    return ExitCode::from(1);
  }
  if !unexpected.is_empty() {
    println!(
      "the meaning check flags the standard formatter's output in {} file(s) that \
             compat/meaning-exceptions.txt does not list: a rewrite the allowed list lacks \
             (src/verify/rewrites.rs), or a bug of the formatter's to list with its reason:",
      unexpected.len()
    );
    for line in unexpected.iter().take(20) {
      println!("  {}", line.chars().take(200).collect::<String>());
    }
    let _ = ratchet(&options, &tallies);
    return ExitCode::from(1);
  }
  ratchet(&options, &tallies)
}

/// The standard formatter's output for `input` formatted from `dir`, from the cache or a
/// run; `None` when it fails on the input.
pub fn standard_output(
  input: &str,
  edition: Edition,
  dir: &Path,
  reference: &str,
  cache: &Path,
  home: &Path,
) -> Option<String> {
  let config = nearest_config(dir);
  let edition = edition_year(edition);
  let mut key = String::new();
  key.push_str(reference);
  key.push('\0');
  key.push_str(edition);
  key.push('\0');
  key.push_str(&config);
  key.push('\0');
  key.push_str(input);
  let hash = sha256::hex(key.as_bytes());
  let cached = cache.join(&hash[..2]).join(&hash[2..]);
  let failed = cached.with_extension("failed");
  if let Ok(text) = fs::read_to_string(&cached) {
    return Some(text);
  }
  if failed.exists() {
    return None;
  }
  let result = run_standard(input, edition, dir, home);
  let _ = fs::create_dir_all(cached.parent().unwrap_or(cache));
  match &result {
    Some(text) => {
      let _ = fs::write(&cached, text);
    }
    None => {
      let _ = fs::write(&failed, "");
    }
  }
  result
}

/// Every input, in a fixed order: the test pairs, then the corpus as fetched, then unformatted.
fn collect_cases(options: &Options) -> Vec<Case> {
  let mut cases = Vec::new();
  // Settings as mto resolves them for each folder, with no user configuration.
  let mut resolver = Resolver::new(&Overrides::default(), Environment::default()).ok();
  // The corpus is answered by the stable release, which ignores nightly-only settings; mto
  // honors them on every channel, so they are dropped here (S3-2).
  let mut format_for = |dir: &Path| -> FormatConfig {
    resolver
      .as_mut()
      .and_then(|r| r.config_for(dir).ok())
      .map(|c| c.stable_only().format())
      .unwrap_or_default()
  };
  if let Some(tests) = &options.tests {
    let target_dir = tests.join("target");
    let source_dir = tests.join("source");
    let mut targets = Vec::new();
    collect_rs(&target_dir, &mut targets);
    targets.sort();
    for target in targets {
      let Ok(expected) = fs::read_to_string(&target) else {
        continue;
      };
      let relative = target.strip_prefix(&target_dir).unwrap_or(&target);
      let input = fs::read_to_string(source_dir.join(relative)).unwrap_or(expected.clone());
      let set = if input.lines().any(|l| l.starts_with("// rustfmt-")) {
        Set::TestsWithSettings
      } else {
        Set::Tests
      };
      let format = header_settings(&input, &tests.join("config"));
      // The pairs are stored with `\n` endings whatever the header asks for; the
      // standard formatter's own test runner ignores endings.
      let edition = edition_for(&target, &input);
      let expected = if format.newline_style == NewlineStyle::Windows
        && !strings_span_lines(&expected, edition)
      {
        expected.replace("\r\n", "\n").replace('\n', "\r\n")
      } else {
        expected
      };
      cases.push(Case {
        set,
        name: format!("tests/{}", relative.display()),
        edition,
        format,
        input,
        dir: tests.to_path_buf(),
        expected: Some(expected),
      });
    }
  }
  let mut corpus = Vec::new();
  for (name, dir) in &options.repos {
    let mut files = Vec::new();
    collect_rs(dir, &mut files);
    files.sort();
    for file in files {
      let Ok(text) = fs::read_to_string(&file) else {
        continue;
      };
      let shown = format!(
        "{}/{}",
        name,
        file.strip_prefix(dir).unwrap_or(&file).display()
      );
      corpus.push((file, shown, text));
    }
  }
  for (file, shown, text) in &corpus {
    cases.push(Case {
      set: Set::Corpus,
      name: shown.clone(),
      edition: edition_for(file, text),
      input: text.clone(),
      dir: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
      expected: None,
      format: format_for(file.parent().unwrap_or(Path::new("."))),
    });
  }
  for (file, shown, text) in &corpus {
    let edition = edition_for(file, text);
    let Some(messy) = unformat(text, edition) else {
      continue;
    };
    cases.push(Case {
      set: Set::Unformatted,
      name: shown.clone(),
      edition,
      input: messy,
      dir: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
      expected: None,
      format: format_for(file.parent().unwrap_or(Path::new("."))),
    });
  }
  cases
}

/// How many lines `got` shares with `want` in order: the length of their longest common
/// subsequence, by Myers' O(ND) diff over line ids.
fn common_lines<'a>(got: &[&'a str], want: &[&'a str]) -> usize {
  let mut ids: BTreeMap<&'a str, u32> = BTreeMap::new();
  let mut id = |line: &'a str| -> u32 {
    let next = ids.len() as u32;
    *ids.entry(line).or_insert(next)
  };
  let a: Vec<u32> = got.iter().map(|l| id(l)).collect();
  let b: Vec<u32> = want.iter().map(|l| id(l)).collect();
  let (n, m) = (a.len() as isize, b.len() as isize);
  let max = (n + m) as usize;
  let offset = max as isize;
  let mut v = vec![0isize; 2 * max + 2];
  for d in 0..=max as isize {
    let mut k = -d;
    while k <= d {
      let index = (k + offset) as usize;
      let mut x = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
        v[index + 1]
      } else {
        v[index - 1] + 1
      };
      let mut y = x - k;
      while x < n && y < m && a[x as usize] == b[y as usize] {
        x += 1;
        y += 1;
      }
      v[index] = x;
      if x >= n && y >= m {
        return ((n + m - d) / 2) as usize;
      }
      k += 2;
    }
  }
  0
}

/// Writes each file's matched lines to `log` and lists the files that matched fewer lines
/// than at the run before, which a total alone hides behind gains elsewhere; beside it,
/// `diff-first.txt` names each mismatching file's first differing line and its kind.
fn compare_scores(log: &Path, cases: &[Case], scores: &[Score]) {
  let before: HashMap<String, usize> = fs::read_to_string(log)
    .unwrap_or_default()
    .lines()
    .filter_map(|line| {
      let (key, matched) = line.rsplit_once(' ')?;
      Some((key.to_string(), matched.split('/').next()?.parse().ok()?))
    })
    .collect();
  let mut now = String::new();
  let mut first = String::new();
  let mut lost = Vec::new();
  for (case, score) in cases.iter().zip(scores).filter(|(_, s)| !s.skipped) {
    let key = format!("[{}] {}", case.set.key(), case.name);
    if let Some((line, kind)) = &score.first {
      first.push_str(&format!("{} {}:{}\n", kind, key, line));
    }
    if let Some(&old) = before.get(&key)
      && score.matched_lines < old
    {
      lost.push(format!("{} {} -> {}", key, old, score.matched_lines));
    }
    now.push_str(&format!(
      "{} {}/{}\n",
      key, score.matched_lines, score.lines
    ));
  }
  let _ = fs::write(log, now);
  // Each mismatching file's first differing line and its syntax kind, to sample a kind by.
  let _ = fs::write(log.with_file_name("diff-first.txt"), first);
  if !lost.is_empty() {
    println!(
      "files matching fewer lines than at the last run: {}",
      lost.len()
    );
    for line in lost.iter().take(20) {
      println!("  {}", line);
    }
  }
}

fn edition_year(edition: Edition) -> &'static str {
  match edition {
    Edition::E2015 => "2015",
    Edition::E2018 => "2018",
    Edition::E2021 => "2021",
    _ => "2024",
  }
}

/// The `PATH [SET]` keys listed in the exceptions file.
fn exceptions(path: &Path) -> Vec<String> {
  let Ok(text) = fs::read_to_string(path) else {
    return Vec::new();
  };
  text
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty() && !l.starts_with('#'))
    .filter_map(|l| l.find(']').map(|end| l[..=end].to_string()))
    .collect()
}

/// The formatting settings a test pair's `// rustfmt-NAME: VALUE` header lines set, resolved
/// as mto resolves `--config NAME=VALUE`: the same widths, heuristics and defaults a user
/// gets. A header mto cannot read leaves the defaults.
fn header_settings(input: &str, config_dir: &Path) -> FormatConfig {
  // Each header mto accepts on its own command line; one it does not know is left out
  // rather than failing the rest.
  let accepted = |pair: &(String, String)| {
    let one = Overrides {
      settings: vec![pair.clone()],
      ..Overrides::default()
    };
    Resolver::new(&one, Environment::default()).is_ok()
  };
  let settings: Vec<(String, String)> = input
    .lines()
    .filter_map(|line| line.strip_prefix("// rustfmt-"))
    .filter_map(|rest| rest.split_once(':'))
    .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
    .filter(accepted)
    .collect();
  // `// rustfmt-config: file.toml` names a file in the suite's `config` folder, read as
  // `--config-path` reads one.
  let config_path = input
    .lines()
    .filter_map(|line| line.strip_prefix("// rustfmt-config:"))
    .map(|name| config_dir.join(name.trim()))
    .next();
  let overrides = Overrides {
    config_path,
    settings,
    ..Overrides::default()
  };
  // An empty folder, so that no file's settings mix in.
  let dir = std::env::temp_dir().join("mtindo-diff-headers");
  let _ = fs::create_dir_all(&dir);
  Resolver::new(&overrides, Environment::default())
    .ok()
    .and_then(|mut r| r.config_for(&dir).ok())
    .map(|c| c.format())
    .unwrap_or_default()
}

/// The kind of the outermost syntax node that starts where line `line` (0-based) of `text`
/// starts its text, an expression statement standing for its expression; `end of file` past
/// the end. A line starting inside a node (a continuation) gives the innermost node holding
/// it.
fn kind_at_line(text: &str, line: usize, edition: Edition) -> String {
  let mut offset = 0;
  for (i, l) in text.split('\n').enumerate() {
    if i == line {
      offset += l.len() - l.trim_start().len();
      break;
    }
    offset += l.len() + 1;
  }
  if offset >= text.len() {
    return "end of file".to_string();
  }
  let mut node = parse(text, edition).syntax_node();
  loop {
    let child = node.children().into_iter().find(|c| {
      let range = c.text_range();
      (range.start as usize) <= offset && offset < range.end as usize
    });
    let Some(child) = child else {
      return format!("{:?}", node.kind());
    };
    if child.text_range().start as usize == offset {
      // The outermost node starting here; a statement is named by its expression.
      if child.kind() == SyntaxKind::ExprStmt
        && let Some(expr) = child.children().into_iter().next()
      {
        return format!("{:?}", expr.kind());
      }
      return format!("{:?}", child.kind());
    }
    node = child;
  }
}

/// mto's formatting of `text`, as `format = true` runs it.
fn mto_output(text: &str, edition: Edition, config: &FormatConfig) -> String {
  mtindo::format::file::format_file(text, edition, config)
}

/// The text of the configuration file the standard formatter finds from `dir`, for the
/// cache key: the nearest `.rustfmt.toml` or `rustfmt.toml` in it or a parent.
fn nearest_config(dir: &Path) -> String {
  let mut current = Some(dir);
  while let Some(here) = current {
    for name in [".rustfmt.toml", "rustfmt.toml"] {
      if let Ok(text) = fs::read_to_string(here.join(name)) {
        return text;
      }
    }
    current = here.parent();
  }
  String::new()
}

fn parse_args(args: &[String]) -> Result<Options, String> {
  let mut options = Options {
    tests: None,
    reference: String::new(),
    cache: PathBuf::new(),
    record: PathBuf::new(),
    update: false,
    markdown: false,
    repos: Vec::new(),
    meaning_log: None,
    broke_log: None,
    exceptions: None,
  };
  let mut i = 0;
  while i < args.len() {
    let value = |i: usize| args.get(i + 1).cloned().ok_or("a flag needs a value");
    match args[i].as_str() {
      "--tests" => options.tests = Some(PathBuf::from(value(i)?)),
      "--reference" => options.reference = value(i)?,
      "--cache" => options.cache = PathBuf::from(value(i)?),
      "--record" => options.record = PathBuf::from(value(i)?),
      "--meaning-log" => options.meaning_log = Some(PathBuf::from(value(i)?)),
      "--broke-log" => options.broke_log = Some(PathBuf::from(value(i)?)),
      "--exceptions" => options.exceptions = Some(PathBuf::from(value(i)?)),
      "--update" => {
        options.update = true;
        i += 1;
        continue;
      }
      "--markdown" => {
        options.markdown = true;
        i += 1;
        continue;
      }
      other => match other.split_once('=') {
        Some((name, path)) => {
          options.repos.push((name.to_string(), PathBuf::from(path)));
          i += 1;
          continue;
        }
        None => return Err(format!("unexpected argument `{}`", other)),
      },
    }
    i += 2;
  }
  if options.reference.is_empty() || options.record.as_os_str().is_empty() {
    return Err("--reference and --record are required".into());
  }
  Ok(options)
}

fn parse_record(text: &str) -> (String, BTreeMap<String, Tally>) {
  let mut reference = String::new();
  let mut sets = BTreeMap::new();
  for line in text.lines() {
    if let Some(r) = line.strip_prefix("reference ") {
      reference = r.to_string();
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    if let ["set", name, "files", mf, f, "lines", ml, l] = words.as_slice() {
      let n = |s: &str| s.parse::<usize>().unwrap_or(0);
      sets.insert(
        name.to_string(),
        Tally {
          matched_files: n(mf),
          files: n(f),
          matched_lines: n(ml),
          lines: n(l),
          skipped: 0,
        },
      );
    }
  }
  (reference, sets)
}

/// Compares with the record, or writes it with `--update`.
fn ratchet(options: &Options, tallies: &BTreeMap<Set, Tally>) -> ExitCode {
  let current = record_text(&options.reference, tallies);
  if options.update {
    return match fs::write(&options.record, &current) {
      Ok(()) => {
        println!("recorded in {}", options.record.display());
        ExitCode::SUCCESS
      }
      Err(e) => {
        println!("cannot write {}: {}", options.record.display(), e);
        ExitCode::from(1)
      }
    };
  }
  let Ok(text) = fs::read_to_string(&options.record) else {
    println!(
      "no record at {}; run `cargo xtask diff --update` to start one",
      options.record.display()
    );
    return ExitCode::from(1);
  };
  let (reference, recorded) = parse_record(&text);
  if reference != options.reference {
    println!(
      "the record was made with {}, this run with {}; check the numbers, then run \
             `cargo xtask diff --update`",
      reference, options.reference
    );
    return ExitCode::from(1);
  }
  let mut drops = Vec::new();
  for (set, now) in tallies {
    let Some(then) = recorded.get(set.key()) else {
      continue;
    };
    // a/b < c/d without division: a*d < c*b.
    let below = |a: usize, b: usize, c: usize, d: usize| a * d.max(1) < c * b.max(1);
    if below(now.matched_files, now.files, then.matched_files, then.files) {
      drops.push(format!(
        "{}: files {} of {}, recorded {} of {}",
        set.title(),
        now.matched_files,
        now.files,
        then.matched_files,
        then.files
      ));
    }
    if below(now.matched_lines, now.lines, then.matched_lines, then.lines) {
      drops.push(format!(
        "{}: lines {} of {}, recorded {} of {}",
        set.title(),
        now.matched_lines,
        now.lines,
        then.matched_lines,
        then.lines
      ));
    }
  }
  if drops.is_empty() {
    println!("compatibility: ok, at or above the record");
    ExitCode::SUCCESS
  } else {
    println!(
      "compatibility dropped below the record (S3-7):\n  {}",
      drops.join("\n  ")
    );
    ExitCode::from(1)
  }
}

/// The record file's text.
fn record_text(reference: &str, tallies: &BTreeMap<Set, Tally>) -> String {
  let mut out = String::from(
    "# How much of the Rust style mto reproduces: files and lines matching the standard\n\
         # formatter exactly, per input set (`cargo xtask diff`). The numbers may only go up;\n\
         # `cargo xtask diff --update` records new ones.\n",
  );
  out.push_str(&format!("reference {}\n", reference));
  for (set, t) in tallies {
    out.push_str(&format!(
      "set {} files {} {} lines {} {}\n",
      set.key(),
      t.matched_files,
      t.files,
      t.matched_lines,
      t.lines
    ));
  }
  out
}

/// The standard formatter's output for a case, from the cache or a run; `None` when it
/// fails on the input.
fn reference_output(case: &Case, options: &Options, home: &Path) -> Option<String> {
  standard_output(
    &case.input,
    case.edition,
    &case.dir,
    &options.reference,
    &options.cache,
    home,
  )
}

fn report(
  tallies: &BTreeMap<Set, Tally>,
  kinds: &BTreeMap<String, (usize, String)>,
  reference: &str,
  markdown: bool,
) -> String {
  let percent = |a: usize, b: usize| {
    if b == 0 {
      0.0
    } else {
      a as f64 * 100.0 / b as f64
    }
  };
  let mut out = String::new();
  if markdown {
    out.push_str(&format!(
      "# mto compatibility\n\nAgainst {}.\n\n| input set | files matching | lines \
             matching | left out |\n|---|---:|---:|---:|\n",
      reference
    ));
  } else {
    out.push_str(&format!(
      "against {}\n{:<30} {:>24} {:>28} {:>9}\n",
      reference, "input set", "files matching", "lines matching", "left out"
    ));
  }
  for set in Set::ALL {
    let Some(t) = tallies.get(&set) else {
      continue;
    };
    let files = format!(
      "{} of {} ({:.1}%)",
      t.matched_files,
      t.files,
      percent(t.matched_files, t.files)
    );
    let lines = format!(
      "{} of {} ({:.2}%)",
      t.matched_lines,
      t.lines,
      percent(t.matched_lines, t.lines)
    );
    if markdown {
      out.push_str(&format!(
        "| {} | {} | {} | {} |\n",
        set.title(),
        files,
        lines,
        t.skipped
      ));
    } else {
      out.push_str(&format!(
        "{:<30} {:>24} {:>28} {:>9}\n",
        set.title(),
        files,
        lines,
        t.skipped
      ));
    }
  }
  let mut ranked: Vec<(&String, &(usize, String))> = kinds.iter().collect();
  ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
  if !ranked.is_empty() {
    if markdown {
      out.push_str(
        "\n## Mismatches by syntax kind at the first differing line\n\n| kind | files \
                 | for example |\n|---|---:|---|\n",
      );
    } else {
      out.push_str("\nmismatches by syntax kind at the first differing line:\n");
    }
    for (kind, (count, example)) in ranked.into_iter().take(TOP_KINDS) {
      if markdown {
        out.push_str(&format!("| `{}` | {} | `{}` |\n", kind, count, example));
      } else {
        out.push_str(&format!("  {:<28} {:>6}  e.g. {}\n", kind, count, example));
      }
    }
  }
  out
}

/// The standard formatter on `text` from `dir`, with an empty home folder.
fn run_standard(text: &str, edition: &str, dir: &Path, home: &Path) -> Option<String> {
  let mut child = Command::new("rustfmt")
    .args(["--edition", edition, "--emit", "stdout"])
    .current_dir(dir)
    .env("HOME", home)
    .env_remove("XDG_CONFIG_HOME")
    .env("RUSTUP_AUTO_INSTALL", "0")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .ok()?;
  let mut stdin = child.stdin.take()?;
  let input = text.to_string();
  let writer = std::thread::spawn(move || {
    let _ = stdin.write_all(input.as_bytes());
  });
  let output = child.wait_with_output().ok()?;
  let _ = writer.join();
  if !output.status.success() {
    return None;
  }
  String::from_utf8(output.stdout).ok()
}

fn score(case: &Case, options: &Options, home: &Path) -> Score {
  let answer = match &case.expected {
    Some(expected) => Some(expected.clone()),
    None => reference_output(case, options, home),
  };
  let Some(answer) = answer else {
    return Score {
      skipped: true,
      ..Score::default()
    };
  };
  let meaning = {
    let found = mtindo::verify::format::verify_format(&case.input, &answer, case.edition);
    (!found.is_empty()).then(|| {
      found
        .iter()
        .map(|v| format!("{}: {}", v.rule.name(), v.detail))
        .collect::<Vec<_>>()
        .join("; ")
    })
  };
  let output = mto_output(&case.input, case.edition, &case.format);
  let own = {
    let found = mtindo::verify::format::verify_format(&case.input, &output, case.edition);
    if !found.is_empty() {
      Some(
        found
          .iter()
          .map(|v| format!("{}: {}", v.rule.name(), v.detail))
          .collect::<Vec<_>>()
          .join("; "),
      )
    } else if mto_output(&output, case.edition, &case.format) != output {
      Some("a second run changes it".to_string())
    } else {
      None
    }
  };
  let expected: Vec<&str> = answer.split('\n').collect();
  let got: Vec<&str> = output.split('\n').collect();
  let matches = output == answer;
  let broke = !matches && case.input == answer;
  let matched_lines = if matches {
    expected.len()
  } else {
    common_lines(&got, &expected)
  };
  let first = (!matches).then(|| {
    let line = expected
      .iter()
      .zip(&got)
      .position(|(a, b)| a != b)
      .unwrap_or(expected.len().min(got.len()));
    (line + 1, kind_at_line(&answer, line, case.edition))
  });
  Score {
    skipped: false,
    matches,
    lines: expected.len(),
    matched_lines,
    first,
    meaning,
    own,
    broke,
  }
}

fn score_all(cases: &[Case], options: &Options, home: &Path) -> Vec<Score> {
  let next = AtomicUsize::new(0);
  let done = AtomicUsize::new(0);
  let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
  let mut scores: Vec<Option<Score>> = (0..cases.len()).map(|_| None).collect();
  let results: Vec<(usize, Score)> = std::thread::scope(|scope| {
    let handles: Vec<_> = (0..threads)
      .map(|_| {
        scope.spawn(|| {
          let mut mine = Vec::new();
          loop {
            let at = next.fetch_add(1, Ordering::Relaxed);
            let Some(case) = cases.get(at) else {
              break;
            };
            mine.push((at, score(case, options, home)));
            let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
            if finished.is_multiple_of(2000) {
              eprintln!("  {} of {}", finished, cases.len());
            }
          }
          mine
        })
      })
      .collect();
    handles
      .into_iter()
      .flat_map(|h| h.join().unwrap_or_default())
      .collect()
  });
  for (at, score) in results {
    scores[at] = Some(score);
  }
  scores.into_iter().map(Option::unwrap_or_default).collect()
}

/// Whether a string literal holds a line break: line endings inside it are its contents.
fn strings_span_lines(text: &str, edition: Edition) -> bool {
  parse(text, edition)
    .syntax_node()
    .descendants_with_tokens()
    .iter()
    .any(|e| match e {
      SyntaxElement::Token(t) => t.kind().is_literal() && t.text().contains('\n'),
      SyntaxElement::Node(_) => false,
    })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_common_lines_is_the_longest_common_subsequence() {
    assert_eq!(common_lines(&["a", "b", "c"], &["a", "b", "c"]), 3);
    assert_eq!(common_lines(&["a", "x", "c"], &["a", "b", "c"]), 2);
    assert_eq!(common_lines(&[], &["a"]), 0);
    assert_eq!(common_lines(&["b", "a"], &["a", "b"]), 1);
  }

  #[test]
  fn test_header_settings_are_read() {
    let config = header_settings(
      "// rustfmt-tab_spaces: 2\n// rustfmt-hard_tabs: true\n",
      Path::new("."),
    );
    assert_eq!((config.tab_spaces, config.hard_tabs), (2, true));
    assert_eq!(header_settings("fn f() {}\n", Path::new(".")).tab_spaces, 4);
  }

  #[test]
  fn test_kind_at_a_line() {
    let text = "fn f() {\n    call(1);\n}\n";
    assert_eq!(kind_at_line(text, 1, Edition::E2021), "CallExpr");
    assert_eq!(kind_at_line(text, 9, Edition::E2021), "end of file");
  }

  #[test]
  fn test_records_round_trip_and_drops_are_found() {
    let mut tallies = BTreeMap::new();
    tallies.insert(
      Set::Corpus,
      Tally {
        files: 10,
        matched_files: 4,
        lines: 100,
        matched_lines: 90,
        skipped: 0,
      },
    );
    let text = record_text("rustfmt 1.0", &tallies);
    let (reference, sets) = parse_record(&text);
    assert_eq!(reference, "rustfmt 1.0");
    assert_eq!(sets["corpus"].matched_lines, 90);
  }
}
