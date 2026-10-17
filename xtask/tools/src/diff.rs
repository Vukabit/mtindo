//! `cargo xtask diff`: how much of the Rust style mto reproduces, measured against the
//! standard formatter (S3-1, S3-2, S3-6, S3-7).
//!
//! Three input sets, each file scored by whether mto's output equals the answer exactly, and
//! by how many of the answer's lines it has (their longest common subsequence):
//!
//! - **tests:** the standard formatter's own test pairs; the answer is the expected file.
//!   Pairs whose input sets options with `// rustfmt-KEY: VALUE` lines are a set of their
//!   own, since mto does not read those lines yet.
//! - **corpus:** every corpus file as fetched, test fixtures aside (the standard formatter's
//!   own test inputs, rust-analyzer's `test_data`); the answer is the standard formatter's
//!   output, run as `cargo fmt` runs it (the crate's edition, its configuration file), with an
//!   empty home folder so the machine's own settings cannot leak in.
//! - **corpus, unformatted:** every corpus file with its whitespace scrambled
//!   ([`crate::unformat`]); the answer is the standard formatter's output for the scrambled
//!   text.
//!
//! The standard formatter's outputs are cached by input, edition, configuration file and
//! version, so a rerun only runs mto. An input it refuses (a non-zero exit) is cached too,
//! with what it said, and counted as skipped; the skipped count is part of the record and
//! may not grow. A run of it that says nothing about the input (it could not start, was
//! killed, or passed its time limit) is never cached, and fails the run, as does mto
//! panicking on an input: neither may pass for a number. Mismatching files are grouped by the syntax kind of the
//! innermost node at their first differing line. The numbers are compared with the record in
//! `compat/compatibility.txt`, which they may only climb above: a drop fails the run, and
//! `--update` records new numbers.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::history::sha256;
use mtindo::options::format::{FormatConfig, NewlineStyle};
use mtindo::syntax::{Edition, SyntaxElement, SyntaxKind, parse};

use crate::common::edition_for;
use crate::process;
use crate::unformat::unformat;

/// The first line of a cached refusal; a marker without it predates the refusals being
/// told apart from failures to run, so it is run again.
const REFUSED_HEADER: &str = "refused: exit ";

/// The longest the standard formatter may take on one input.
const STANDARD_LIMIT: Duration = Duration::from_secs(120);

/// The mismatch groups the report lists.
const TOP_KINDS: usize = 50;

/// What the standard formatter made of an input.
#[derive(Debug, PartialEq)]
pub enum Answer {
  /// Its output.
  Text(String),
  /// It ran and refused the input (a non-zero exit): its exit code and what it said.
  Refused(String),
  /// It did not run on the input: why. Says nothing about the input, so never cached.
  NotRun(String),
}

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
  /// The answer could not be had: the standard formatter refused the input.
  skipped: bool,
  /// The standard formatter did not run on the input (it could not start, was killed or
  /// passed its time limit): why. Fails the run.
  unanswered: Option<String>,
  /// mto panicked on the input: the panic message. Fails the run.
  panicked: Option<String>,
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

/// How many lines `got` shares with `want` in order: the length of their longest common
/// subsequence, by Myers' O(ND) diff over line ids.
pub fn common_lines<'a>(got: &[&'a str], want: &[&'a str]) -> usize {
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

pub fn edition_year(edition: Edition) -> &'static str {
  match edition {
    Edition::E2015 => "2015",
    Edition::E2018 => "2018",
    Edition::E2021 => "2021",
    _ => "2024",
  }
}

/// Fences `downloads` (compat/cache) off from this repository, whose own settings must not
/// reach the corpus and the test pairs: a `root = true` mto.toml ends mto's search upwards,
/// and an empty rustfmt.toml ends both tools' search for one (empty, so cached answers keep
/// their key).
pub fn fence(downloads: &Path) {
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

/// The kind of the outermost syntax node that starts where line `line` (0-based) of `text`
/// starts its text, an expression statement standing for its expression; `end of file` past
/// the end. A line starting inside a node (a continuation) gives the innermost node holding
/// it.
pub fn kind_at_line(text: &str, line: usize, edition: Edition) -> String {
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

/// The text a panic carried.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
  payload
    .downcast_ref::<&str>()
    .map(|s| s.to_string())
    .or_else(|| payload.downcast_ref::<String>().cloned())
    .unwrap_or_else(|| "a panic without a message".to_string())
}

pub fn run(args: &[String]) -> ExitCode {
  let options = match parse_args(args) {
    Ok(options) => options,
    Err(e) => {
      eprintln!("xtask-tools diff: {}", e);
      return ExitCode::from(2);
    }
  };
  if let Some(downloads) = options.cache.parent() {
    fence(downloads);
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
    if score.unanswered.is_some() || score.panicked.is_some() {
      continue;
    }
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
  // This run's numbers, for `cargo xtask diff` to keep as series.
  if let Some(dir) = options.broke_log.as_ref().and_then(|log| log.parent()) {
    let mut out = String::new();
    for (set, t) in &tallies {
      out.push_str(&format!(
        "{}\t{}\t{}\t{}\t{}\t{}\n",
        set.key(),
        t.matched_files,
        t.files,
        t.matched_lines,
        t.lines,
        t.skipped
      ));
    }
    let _ = fs::write(dir.join("diff-tallies.tsv"), out);
  }
  let not_scored: Vec<String> = cases
    .iter()
    .zip(&scores)
    .filter_map(|(case, score)| {
      let (what, why) = match (&score.panicked, &score.unanswered) {
        (Some(message), _) => ("mto panicked", message),
        (None, Some(reason)) => ("no answer", reason),
        (None, None) => return None,
      };
      Some(format!(
        "{} [{}]: {}: {}",
        case.name,
        case.set.key(),
        what,
        why
      ))
    })
    .collect();
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
    .map(|path| exceptions(path, false))
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
  // The default-settings test pairs mto does not match, but those excused with a reason
  // (the standard formatter's own bugs): the v1.0 gate wants none left.
  let excused = options
    .exceptions
    .as_ref()
    .map(|path| exceptions(path, true))
    .unwrap_or_default();
  let pairs: Vec<(String, bool)> = cases
    .iter()
    .zip(&scores)
    .filter(|(case, score)| case.set == Set::Tests && !score.skipped)
    .map(|(case, score)| (format!("{} [{}]", case.name, case.set.key()), score.matches))
    .collect();
  let open = pairs
    .iter()
    .filter(|(key, matches)| !matches && !excused.contains(key))
    .count();
  println!(
    "default-settings test pairs not matching: {}, {} of them excused in \
     compat/meaning-exceptions.txt",
    pairs.iter().filter(|(_, matches)| !matches).count(),
    pairs.iter().filter(|(_, matches)| !matches).count() - open,
  );
  for key in &excused {
    if pairs.iter().any(|(k, matches)| k == key && *matches) {
      println!("  excused but now matching (remove it): {}", key);
    }
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
  if let Some(dir) = options.broke_log.as_ref().and_then(|log| log.parent()) {
    write_findings(dir, &cases, &scores);
  }
  if !not_scored.is_empty() {
    println!(
      "{} input(s) could not be scored, so these numbers are not a measurement:",
      not_scored.len()
    );
    for line in not_scored.iter().take(20) {
      println!("  {}", line.chars().take(200).collect::<String>());
    }
    if let Some(dir) = options.broke_log.as_ref().and_then(|log| log.parent()) {
      let path = dir.join("diff-not-scored.txt");
      let _ = fs::write(&path, not_scored.join("\n") + "\n");
      println!("  (all of them in {})", path.display());
    }
    return ExitCode::from(1);
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

/// The standard formatter on `text` from `dir`, with an empty home folder. rustup is told
/// where it lives and which toolchain to run: the empty home hides its own settings, and a
/// repository's `rust-toolchain` file would ask for another (the reference is stable).
pub fn run_standard(text: &str, edition: &str, dir: &Path, home: &Path) -> Answer {
  let rustup_home = std::env::var_os("RUSTUP_HOME")
    .map(PathBuf::from)
    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".rustup")));
  let mut command = Command::new("rustfmt");
  if let Some(rustup_home) = rustup_home {
    command.env("RUSTUP_HOME", rustup_home);
  }
  let done = process::output_with_input(
    command
      .args(["--edition", edition, "--emit", "stdout"])
      .current_dir(dir)
      .env("HOME", home)
      .env_remove("XDG_CONFIG_HOME")
      .env("RUSTUP_TOOLCHAIN", "stable")
      .env("RUSTUP_AUTO_INSTALL", "0")
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    Some(text.as_bytes()),
    STANDARD_LIMIT,
  );
  match done {
    Ok(done) => classify(done),
    Err(e) => Answer::NotRun(format!("cannot run rustfmt: {}", e)),
  }
}

/// `diff-show [--scrambled] [--twice] [--save] FILE [CONTEXT]`: where mto's formatting of FILE first differs from
/// the standard formatter's, as the harness compares them (the corpus file as fetched, or
/// with `--scrambled` as the unformatted set scrambles it): the lines around the first
/// difference from each, side by side as two blocks. `--twice` compares with mto run again;
/// `--save` also writes the input and both outputs to `.dev/logs/diff-show` (as `.txt`).
pub fn show(args: &[String]) -> ExitCode {
  let scrambled = args.iter().any(|a| a == "--scrambled");
  let twice = args.iter().any(|a| a == "--twice");
  let save = args.iter().any(|a| a == "--save");
  let rest: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
  let Some(file) = rest.first().map(|f| PathBuf::from(f.as_str())) else {
    eprintln!("diff-show [--scrambled] [--twice] [--save] FILE [CONTEXT]");
    return ExitCode::from(2);
  };
  let context: usize = rest.get(1).and_then(|n| n.parse().ok()).unwrap_or(6);
  let Ok(text) = fs::read_to_string(&file) else {
    eprintln!("cannot read {}", file.display());
    return ExitCode::from(1);
  };
  let edition = edition_for(&file, &text);
  let input = if scrambled {
    match unformat(&text, edition) {
      Some(messy) => messy,
      None => {
        eprintln!("{} is not scrambled by the harness", file.display());
        return ExitCode::from(1);
      }
    }
  } else {
    text
  };
  let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
  let format = Resolver::new(&Overrides::default(), Environment::default())
    .ok()
    .and_then(|mut r| r.config_for(&dir).ok())
    .map(|c| c.stable_only().format())
    .unwrap_or_default();
  let ours = mto_output(&input, edition, &format);
  let (theirs, name) = if twice {
    (mto_output(&ours, edition, &format), "mto, run again")
  } else {
    let home = std::env::temp_dir().join("mtindo-diff-show-home");
    let _ = fs::create_dir_all(&home);
    match run_standard(&input, edition_year(edition), &dir, &home) {
      Answer::Text(text) => (text, "standard"),
      Answer::Refused(why) | Answer::NotRun(why) => {
        println!("the standard formatter gave no answer: {}", why);
        return ExitCode::from(1);
      }
    }
  };
  // The input and both outputs, for running other builds on the same input; not `.rs`, so
  // that nothing formatting the repository takes them for sources.
  if save {
    let out = Path::new(".dev/logs/diff-show");
    let _ = fs::create_dir_all(out);
    for (name, text) in [
      ("input.txt", &input),
      ("mto.txt", &ours),
      ("other.txt", &theirs),
    ] {
      let _ = fs::write(out.join(name), text);
    }
    println!("saved to {}", out.display());
  }
  let (a, b): (Vec<&str>, Vec<&str>) = (ours.lines().collect(), theirs.lines().collect());
  let Some(first) = (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i)) else {
    println!("{}: the same", file.display());
    return ExitCode::SUCCESS;
  };
  let from = first.saturating_sub(context / 2);
  let block = |lines: &[&str]| -> String {
    lines
      .iter()
      .enumerate()
      .skip(from)
      .take(context + 1)
      .map(|(i, l)| format!("{}{:5} {}\n", if i == first { ">" } else { " " }, i + 1, l))
      .collect()
  };
  println!(
    "{}: first difference at line {}\n--- mto\n{}--- {}\n{}",
    file.display(),
    first + 1,
    block(&a),
    name,
    block(&b)
  );
  ExitCode::from(1)
}

/// The standard formatter's answer for `input` formatted from `dir`, from the cache or a
/// run.
pub fn standard_output(
  input: &str,
  edition: Edition,
  dir: &Path,
  reference: &str,
  cache: &Path,
  home: &Path,
) -> Answer {
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
    return Answer::Text(text);
  }
  if let Ok(said) = fs::read_to_string(&failed) {
    if said.starts_with(REFUSED_HEADER) {
      return Answer::Refused(said);
    }
    let _ = fs::remove_file(&failed);
  }
  let answer = run_standard(input, edition, dir, home);
  let _ = fs::create_dir_all(cached.parent().unwrap_or(cache));
  match &answer {
    Answer::Text(text) => {
      let _ = fs::write(&cached, text);
    }
    Answer::Refused(said) => {
      let _ = fs::write(&failed, said);
    }
    Answer::NotRun(_) => {}
  }
  answer
}

/// `diff-why FILE LINE`: in mto's formatting of the scrambled FILE, what the statement
/// starting on LINE (1-based, as the harness reports it) holds that may decline it: the
/// blocks and other kinds in it, as `kind<parent` names.
pub fn why(args: &[String]) -> ExitCode {
  let (Some(file), Some(line)) = (
    args.first(),
    args.get(1).and_then(|l| l.parse::<usize>().ok()),
  ) else {
    eprintln!("diff-why FILE LINE");
    return ExitCode::from(2);
  };
  let file = PathBuf::from(file);
  let Ok(text) = fs::read_to_string(&file) else {
    return ExitCode::from(1);
  };
  let edition = edition_for(&file, &text);
  let Some(scrambled) = unformat(&text, edition) else {
    return ExitCode::from(1);
  };
  // The line is one of mto's output, as the harness reports it.
  let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
  let format = Resolver::new(&Overrides::default(), Environment::default())
    .ok()
    .and_then(|mut r| r.config_for(&dir).ok())
    .map(|c| c.stable_only().format())
    .unwrap_or_default();
  let input = mto_output(&scrambled, edition, &format);
  let offset: usize = input.lines().take(line - 1).map(|l| l.len() + 1).sum();
  let root = mtindo::syntax::parse(&input, edition).syntax_node();
  let statement = root
    .descendants_iter()
    .filter(|n| {
      matches!(n.kind(), SyntaxKind::LetStmt | SyntaxKind::ExprStmt)
        && n.text_range().start as usize >= offset
    })
    .min_by_key(|n| n.text_range().start);
  let Some(statement) = statement else {
    println!("none");
    return ExitCode::SUCCESS;
  };
  let mut found: Vec<String> = statement
    .descendants_iter()
    .filter_map(|n| match n.kind() {
      SyntaxKind::BlockExpr => Some(format!(
        "block<{:?}",
        n.parent().map(|p| p.kind()).unwrap_or(SyntaxKind::Error)
      )),
      SyntaxKind::LetElse | SyntaxKind::MatchArmList | SyntaxKind::Attr => {
        Some(format!("{:?}", n.kind()))
      }
      SyntaxKind::MacroCall if n.text().trim_end().ends_with('}') => {
        Some("braced-macro".to_string())
      }
      SyntaxKind::GenericArgList => Some("generics".to_string()),
      _ => None,
    })
    .collect();
  if statement
    .token_texts()
    .any(|(k, _, _)| matches!(k, SyntaxKind::LineComment | SyntaxKind::BlockComment))
  {
    found.push("comment".into());
  }
  found.sort();
  found.dedup();
  println!(
    "{}",
    if found.is_empty() {
      "plain".to_string()
    } else {
      found.join(" ")
    }
  );
  ExitCode::SUCCESS
}

/// What a finished run of the standard formatter says about its input.
fn classify(done: process::Separate) -> Answer {
  let stderr = String::from_utf8_lossy(&done.stderr);
  if done.timed_out {
    return Answer::NotRun(format!(
      "rustfmt passed its time limit of {}s",
      STANDARD_LIMIT.as_secs()
    ));
  }
  match done.code {
    0 => match String::from_utf8(done.stdout) {
      Ok(text) => Answer::Text(text),
      Err(_) => Answer::NotRun("rustfmt wrote invalid UTF-8".to_string()),
    },
    // A shell's 128 + signal: killed, not refusing.
    code if !(0..=128).contains(&code) => Answer::NotRun(format!(
      "rustfmt ended without an exit status (code {}): {}",
      code,
      stderr.trim()
    )),
    code => Answer::Refused(format!("{}{}\n{}", REFUSED_HEADER, code, stderr)),
  }
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
      if never_formatted(relative) {
        continue;
      }
      let input = fs::read_to_string(source_dir.join(relative)).unwrap_or(expected.clone());
      let set = if input.lines().any(|l| l.starts_with("// rustfmt-")) {
        Set::TestsWithSettings
      } else {
        Set::Tests
      };
      let mut format = header_settings(&input, &tests.join("config"));
      // A module beside a crate root is formatted as the root's run reaches it, with the
      // macros and attributes the root's own attributes skip.
      let folder = source_dir.join(relative).parent().map(Path::to_path_buf);
      if let Some(root) = folder.map(|f| f.join("lib.rs"))
        && root != source_dir.join(relative)
        && let Ok(text) = fs::read_to_string(&root)
      {
        (format.crate_skipped_macros, format.crate_skipped_attributes) =
          mtindo::cli::modules::crate_skips(&text);
      }
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
      if fixture(&shown) {
        continue;
      }
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

/// Every way `tallies` fall below the record: a lower share of files or lines matching, or
/// more inputs the standard formatter refuses.
fn drops(
  recorded: &BTreeMap<String, (Tally, bool)>,
  tallies: &BTreeMap<Set, Tally>,
) -> Vec<String> {
  let mut drops = Vec::new();
  for (set, now) in tallies {
    let Some((then, has_skipped)) = recorded.get(set.key()) else {
      continue;
    };
    if *has_skipped && now.skipped > then.skipped {
      drops.push(format!(
        "{}: {} inputs the standard formatter refuses, recorded {} (the files cannot be \
         scored; see the .failed markers in the cache)",
        set.title(),
        now.skipped,
        then.skipped
      ));
    }
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
  drops
}

/// The `PATH [SET]` keys listed in the exceptions file, the meaning check's (`excused`
/// false) or the test pairs' mto need not match (`excused` true, lines starting `excused `).
fn exceptions(path: &Path, excused: bool) -> Vec<String> {
  let Ok(text) = fs::read_to_string(path) else {
    return Vec::new();
  };
  text
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty() && !l.starts_with('#'))
    .filter_map(|l| match l.strip_prefix("excused ") {
      Some(rest) if excused => Some(rest),
      None if !excused => Some(l),
      _ => None,
    })
    .filter_map(|l| l.find(']').map(|end| l[..=end].trim().to_string()))
    .collect()
}

/// Whether a corpus file, named as the report shows it, is a test fixture, left out of the
/// corpus sets: the standard formatter's own test inputs (scored as test pairs, with their
/// settings) and rust-analyzer's `test_data` folders, written to be odd rather than to be
/// formatted.
fn fixture(shown: &str) -> bool {
  shown.starts_with("rustfmt/tests/")
    || shown.starts_with("rust-analyzer/") && shown.contains("/test_data/")
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

/// Whether a test pair, named relative to the suite's `target` folder, is a module its
/// parent's run never formats, whose expected text is its source left alone: the standard
/// formatter's own test runner leaves these out.
fn never_formatted(relative: &Path) -> bool {
  [
    "configs/skip_children/foo/mod.rs",
    "issue-3253/paths/excluded.rs",
  ]
  .iter()
  .any(|child| relative == Path::new(child))
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

/// The reference and each set's numbers; the flag says whether the record has the set's
/// skipped count.
fn parse_record(text: &str) -> (String, BTreeMap<String, (Tally, bool)>) {
  let mut reference = String::new();
  let mut sets = BTreeMap::new();
  for line in text.lines() {
    if let Some(r) = line.strip_prefix("reference ") {
      reference = r.to_string();
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let n = |s: &str| s.parse::<usize>().unwrap_or(0);
    let (name, mf, f, ml, l, skipped) = match words.as_slice() {
      ["set", name, "files", mf, f, "lines", ml, l, "skipped", k] => {
        (name, mf, f, ml, l, Some(n(k)))
      }
      // Records from before the skipped count was kept.
      ["set", name, "files", mf, f, "lines", ml, l] => (name, mf, f, ml, l, None),
      _ => continue,
    };
    sets.insert(
      name.to_string(),
      (
        Tally {
          matched_files: n(mf),
          files: n(f),
          matched_lines: n(ml),
          lines: n(l),
          skipped: skipped.unwrap_or(0),
        },
        skipped.is_some(),
      ),
    );
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
  let drops = drops(&recorded, tallies);
  if reference != options.reference {
    println!(
      "the record was made with {}, this run with {}; check the numbers, then run \
             `cargo xtask diff --update`",
      reference, options.reference
    );
    return ExitCode::from(1);
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
         # formatter exactly, per input set (`cargo xtask diff`), and the inputs it refuses\n\
         # (skipped). Matches may only go up, skipped only down; `cargo xtask diff --update`\n\
         # records new numbers.\n",
  );
  out.push_str(&format!("reference {}\n", reference));
  for (set, t) in tallies {
    out.push_str(&format!(
      "set {} files {} {} lines {} {} skipped {}\n",
      set.key(),
      t.matched_files,
      t.files,
      t.matched_lines,
      t.lines,
      t.skipped
    ));
  }
  out
}

/// The standard formatter's answer for a case, from the cache or a run.
fn reference_output(case: &Case, options: &Options, home: &Path) -> Answer {
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

fn score(case: &Case, options: &Options, home: &Path) -> Score {
  let answer = match &case.expected {
    Some(expected) => expected.clone(),
    None => match reference_output(case, options, home) {
      Answer::Text(text) => text,
      Answer::Refused(_) => {
        return Score {
          skipped: true,
          ..Score::default()
        };
      }
      Answer::NotRun(why) => {
        return Score {
          unanswered: Some(why),
          ..Score::default()
        };
      }
    },
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
    unanswered: None,
    panicked: None,
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
            // A panic in mto is a finding about that input, not a reason to lose the batch.
            let scored =
              std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| score(case, options, home)))
                .unwrap_or_else(|payload| Score {
                  panicked: Some(panic_message(&*payload)),
                  ..Score::default()
                });
            mine.push((at, scored));
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
  scores
    .into_iter()
    .map(|score| {
      score.unwrap_or_else(|| Score {
        panicked: Some("its worker thread died before scoring it".to_string()),
        ..Score::default()
      })
    })
    .collect()
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

/// What `cargo xtask diff` files as incidents, beside the logs: `diff-findings.tsv`, one
/// line per finding (kind, set, file, message), and each finding's input under
/// `diff-inputs/` (cleared on every run). The first line says whether every input was
/// scored: only a complete run can say a finding is gone.
fn write_findings(dir: &Path, cases: &[Case], scores: &[Score]) {
  let inputs = dir.join("diff-inputs");
  let _ = fs::remove_dir_all(&inputs);
  let _ = fs::create_dir_all(&inputs);
  let complete = scores.iter().all(|s| s.unanswered.is_none());
  let mut out = format!("complete\t{}\n", complete);
  let clean = |text: &str| text.replace(['\t', '\n', '\r'], " ");
  for (case, score) in cases.iter().zip(scores) {
    let found = if let Some(message) = &score.panicked {
      Some(("panic", message.clone()))
    } else if let Some(message) = &score.own {
      let kind = if message == "a second run changes it" {
        "unstable"
      } else {
        "meaning"
      };
      Some((kind, message.clone()))
    } else if score.broke {
      let (line, kind) = score.first.clone().unwrap_or_default();
      Some((
        "compat-break",
        format!("mto changes line {} ({})", line, kind),
      ))
    } else {
      None
    };
    let Some((kind, message)) = found else {
      continue;
    };
    let file = format!("{}-{}.input", case.set.key(), case.name.replace('/', "__"));
    let _ = fs::write(inputs.join(&file), &case.input);
    out.push_str(&format!(
      "{}\t{}\t{}\t{}\t{}\n",
      kind,
      case.set.key(),
      clean(&case.name),
      file,
      clean(&message)
    ));
  }
  let _ = fs::write(dir.join("diff-findings.tsv"), out);
}

#[cfg(test)]
mod tests {
  use super::*;

  fn finished(code: i32, stdout: &str, stderr: &str, timed_out: bool) -> process::Separate {
    process::Separate {
      code,
      stdout: stdout.as_bytes().to_vec(),
      stderr: stderr.as_bytes().to_vec(),
      timed_out,
    }
  }

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
  fn test_only_a_real_refusal_is_a_refusal() {
    assert_eq!(
      classify(finished(0, "fn f() {}\n", "", false)),
      Answer::Text("fn f() {}\n".into())
    );
    assert_eq!(
      classify(finished(1, "", "error: expected item", false)),
      Answer::Refused(format!("{}1\nerror: expected item", REFUSED_HEADER))
    );
    assert!(matches!(
      classify(finished(137, "", "", false)),
      Answer::NotRun(_)
    ));
    assert!(matches!(
      classify(finished(-1, "", "", false)),
      Answer::NotRun(_)
    ));
    assert!(matches!(
      classify(finished(process::TIMED_OUT, "", "", true)),
      Answer::NotRun(_)
    ));
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
    assert_eq!(sets["corpus"].0.matched_lines, 90);
    assert!(sets["corpus"].1);
    let old = parse_record("set corpus files 4 10 lines 90 100\n").1;
    assert_eq!((old["corpus"].0.files, old["corpus"].1), (10, false));
    assert!(drops(&sets, &tallies).is_empty());
    let mut worse = tallies.clone();
    worse.get_mut(&Set::Corpus).unwrap().matched_lines = 89;
    worse.get_mut(&Set::Corpus).unwrap().skipped = 1;
    let found = drops(&sets, &worse);
    assert_eq!(found.len(), 2, "{:?}", found);
    assert!(found[0].contains("refuses"), "{:?}", found);
    // A record without skipped counts does not hold them against a run.
    assert_eq!(drops(&old, &worse).len(), 1);
  }
}
