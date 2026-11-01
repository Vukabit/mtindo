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
//! `records/results/compatibility.txt`, which they may only climb above: a drop fails the run, and
//! `--update` records new numbers.

mod key;
pub(crate) mod lexer;
mod measure;
mod record;
mod source;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub(crate) use key::*;
pub(crate) use measure::*;
use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::syntax::{Edition, SyntaxKind};
pub(crate) use record::*;
pub(crate) use source::*;

use crate::common::edition_for;
use crate::unformat::unformat;

/// How many errors parsing shows, the commonest first.
const SHOWN_MESSAGES: usize = 25;

/// How many differences or errors a file shows.
const SHOWN_PER_FILE: usize = 3;

pub(crate) struct Options {
  pub(crate) tests: Option<PathBuf>,
  pub(crate) reference: String,
  pub(crate) cache: PathBuf,
  pub(crate) record: PathBuf,
  pub(crate) update: bool,
  pub(crate) markdown: bool,
  pub(crate) repos: Vec<(String, PathBuf)>,
  /// Where the files the meaning check flags are listed.
  pub(crate) meaning_log: Option<PathBuf>,
  /// Where the files mto breaks are listed in full.
  pub(crate) broke_log: Option<PathBuf>,
  /// The files whose flags are known: the formatter's bugs, each with its reason.
  pub(crate) exceptions: Option<PathBuf>,
  /// Where a scrambled pair's texts are written, beside each other.
  pub(crate) out: Option<PathBuf>,
  /// The recorded sets to score; empty for all of them.
  pub(crate) sources: Vec<Source>,
}

impl Options {
  /// Options with nothing set, for the sources that need few of them.
  pub(crate) fn empty() -> Options {
    Options {
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
      out: None,
      sources: Vec::new(),
    }
  }
}

/// What `pairs` is asked for: the seed, where the texts go, files per crate (0 for all),
/// and the crates by name.
struct PairsArgs {
  seed: u64,
  out: PathBuf,
  files: usize,
  crates: Vec<(String, PathBuf)>,
}

/// `lexdiff [--edition YEAR] [PATH...]`: every file held against rustc's lexer, token for
/// token, at one edition (default 2021); fails when one differs unexplained, or when there
/// are no files.
pub fn lexer(root: &Path, args: &[String]) -> ExitCode {
  let mut edition = Edition::E2021;
  let mut paths = Vec::new();
  let mut at = 0;
  while at < args.len() {
    match args[at].as_str() {
      "--edition" if at + 1 < args.len() => {
        edition = match Edition::from_year(&args[at + 1]) {
          Some(edition) => edition,
          None => {
            eprintln!("Unknown edition `{}`.", args[at + 1]);
            return ExitCode::from(2);
          }
        };
        at += 1;
      }
      path => paths.push(PathBuf::from(path)),
    }
    at += 1;
  }
  let cases = match file_cases(root, &paths, Source::Lexer, Some(edition), true) {
    Ok(cases) => cases,
    Err(e) => {
      eprintln!("lexdiff: {}", e);
      return ExitCode::from(1);
    }
  };
  let scores = score_all(&cases, &Options::empty(), Path::new("."));
  let (mut compared, mut tokens, mut differing) = (0, 0, 0);
  let mut expected_lines: Vec<String> = Vec::new();
  for (case, score) in cases.iter().zip(&scores) {
    if let Some((_, why)) = &score.skip {
      differing += 1;
      println!("{}: cannot read: {}", case.name, why);
      continue;
    }
    compared += 1;
    let Some((differences, known, count)) = &score.tokens else {
      differing += 1;
      println!(
        "{}: {}",
        case.name,
        score.panicked.as_deref().unwrap_or("not compared")
      );
      continue;
    };
    tokens += count;
    expected_lines.extend(known.iter().map(|line| format!("{}: {}", case.name, line)));
    if !differences.is_empty() {
      differing += 1;
      println!("{}:", case.name);
      for line in differences.iter().take(SHOWN_PER_FILE) {
        println!("  {}", line);
      }
      if differences.len() > SHOWN_PER_FILE {
        println!("  ... {} more", differences.len() - SHOWN_PER_FILE);
      }
    }
  }
  if !expected_lines.is_empty() {
    println!("expected differences (rustc's lexer ignores editions and nightly gates):");
    for line in &expected_lines {
      println!("  {}", line);
    }
  }
  println!(
    "lexdiff (edition {}): {} files, {} tokens, {} file(s) differ, {} expected difference(s)",
    edition.year(),
    compared,
    tokens,
    differing,
    expected_lines.len()
  );
  if compared == 0 {
    println!("lexdiff: no files compared, so nothing was checked");
    return ExitCode::from(1);
  }
  if differing == 0 {
    ExitCode::SUCCESS
  } else {
    ExitCode::from(1)
  }
}

/// `pairs --seed N --out DIR [--files N] NAME=DIR...`: the scrambled pairs a seed draws
/// from each crate, scored against the standard formatter's first output, a line each:
///
/// ```text
/// file<TAB>NAME<TAB>FILE<TAB>MOVED<TAB>MATCHED<TAB>LINES<TAB>SAME
/// refused<TAB>NAME<TAB>FILE<TAB>WHY      (the standard formatter refused or did not run)
/// skipped<TAB>NAME<TAB>FILE<TAB>WHY      (not scrambled: syntax errors, carriage returns)
/// scrambler-bug<TAB>NAME<TAB>FILE<TAB>WHAT
/// panic<TAB>NAME<TAB>FILE<TAB>MESSAGE    (mto panicked)
/// ```
///
/// with each file's scrambled, standard and mto texts under `DIR/scrambled|standard|mto/
/// NAME/FILE`, for `cargo xtask pairs` to build the copies it compiles.
pub fn pairs(args: &[String]) -> ExitCode {
  let PairsArgs {
    seed,
    out,
    files,
    crates,
  } = match parse_pairs_args(args) {
    Ok(parsed) => parsed,
    Err(e) => {
      eprintln!("xtask-tools pairs: {}", e);
      return ExitCode::from(2);
    }
  };
  let options = Options {
    out: Some(out.clone()),
    ..Options::empty()
  };
  let cases = pair_cases(&crates, seed, files);
  let home = out.join("home");
  let _ = fs::create_dir_all(&home);
  let scores = score_all(&cases, &options, &home);
  for (case, score) in cases.iter().zip(&scores) {
    let line = |what: &str, rest: &str| {
      format!(
        "{}\t{}\t{}\t{}",
        what,
        case.group,
        case.name,
        rest.lines().next().unwrap_or("").trim()
      )
    };
    let shown = match (&score.skip, &score.refused, &score.panicked) {
      (Some((what, why)), _, _) => line(what, why),
      (None, Some(why), _) => line("refused", why),
      (None, None, Some(message)) => line("panic", message),
      (None, None, None) => format!(
        "file\t{}\t{}\t{}\t{}\t{}\t{}",
        case.group,
        case.name,
        case.moved,
        score.matched_lines,
        score.lines,
        u8::from(score.matches)
      ),
    };
    println!("{}", shown);
  }
  ExitCode::SUCCESS
}

/// `parsecheck [PATH...]`: every file parsed with its crate's edition (or a `// rustfmt-
/// edition:` header's), its syntax errors listed, the commonest first; the repository's
/// files but its fixtures by default. Fails on any error, a parse that does not give back
/// the text, or no files.
pub fn parse(root: &Path, args: &[String]) -> ExitCode {
  let paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();
  let cases = match file_cases(root, &paths, Source::Parse, None, !paths.is_empty()) {
    Ok(cases) => cases,
    Err(e) => {
      eprintln!("parsecheck: {}", e);
      return ExitCode::from(1);
    }
  };
  let scores = score_all(&cases, &Options::empty(), Path::new("."));
  let (mut parsed, mut failing) = (0, 0);
  let mut messages: BTreeMap<String, usize> = BTreeMap::new();
  for (case, score) in cases.iter().zip(&scores) {
    if let Some((_, why)) = &score.skip {
      failing += 1;
      println!("{}: cannot read: {}", case.name, why);
      continue;
    }
    parsed += 1;
    if let Some(message) = &score.panicked {
      failing += 1;
      println!("{}: {}", case.name, message);
      continue;
    }
    if score.errors.is_empty() {
      continue;
    }
    failing += 1;
    println!("{} (edition {}):", case.name, case.edition.year());
    for error in score.errors.iter().take(2) {
      println!("  {}", error);
    }
    // `LINE:COLUMN: MESSAGE`, counted by message.
    for error in &score.errors {
      *messages
        .entry(
          error
            .split_once(": ")
            .map_or(error.as_str(), |(_, m)| m)
            .to_string(),
        )
        .or_insert(0) += 1;
    }
  }
  if !messages.is_empty() {
    let mut ranked: Vec<(&String, &usize)> = messages.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));
    println!("most common errors:");
    for (message, count) in ranked.into_iter().take(SHOWN_MESSAGES) {
      println!("  {:>6}  {}", count, message);
    }
  }
  println!("parsecheck: {} files, {} with errors", parsed, failing);
  if parsed == 0 {
    println!("parsecheck: no files parsed, so nothing was checked");
    return ExitCode::from(1);
  }
  if failing == 0 {
    ExitCode::SUCCESS
  } else {
    ExitCode::from(1)
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
  if let Some(downloads) = options.cache.parent() {
    fence(downloads);
  }
  let mut cases = collect_cases(&options);
  if !options.sources.is_empty() {
    cases.retain(|c| options.sources.contains(&c.source));
  }
  eprintln!(
    "scoring {} inputs against {}...",
    cases.len(),
    options.reference
  );
  let home = options.cache.join("home");
  let _ = fs::create_dir_all(&home);
  let scores = score_all(&cases, &options, &home);
  let mut tallies: BTreeMap<Source, Tally> = BTreeMap::new();
  let mut kinds: BTreeMap<String, (usize, String)> = BTreeMap::new();
  for (case, score) in cases.iter().zip(&scores) {
    let tally = tallies.entry(case.source).or_default();
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
        case.source.key(),
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
        .map(|m| format!("{} [{}]: {}", case.name, case.source.key(), m))
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
  // One not listed fails the run, but where mto's own output for the file is right: the
  // standard formatter is what is wrong there (or the check lacks a rewrite), so it is
  // listed for triage and the case kept in `.dev/upstream/rustfmt/`, not failed.
  let mut unexpected: Vec<String> = Vec::new();
  let mut found: Vec<Found<'_>> = Vec::new();
  for (case, score) in cases.iter().zip(&scores) {
    let Some(message) = &score.meaning else {
      continue;
    };
    let key = format!("{} [{}]", case.name, case.source.key());
    if known.contains(&key) {
      continue;
    }
    if options.exceptions.is_some() && score.own.is_none() && score.panicked.is_none() {
      found.push(Found {
        case,
        key,
        message: message.clone(),
        excused: case.source == Source::Tests && !score.matches,
      });
    } else {
      unexpected.push(format!("{}: {}", key, message));
    }
  }
  let stale: Vec<&String> = known.iter().filter(|k| !keys.contains(k)).collect();
  println!(
    "\nmeaning check on the standard formatter's output: {} of {} files flagged, {} \
         of them listed in records/policy/exceptions.txt",
    flagged.len(),
    checked,
    flagged.len() - unexpected.len()
  );
  for entry in &stale {
    println!("  listed but no longer flagged (remove it): {}", entry);
  }
  if let Some(path) = &options.exceptions
    && !found.is_empty()
  {
    println!(
      "  {} more the standard formatter gets wrong where mto is right, now listed for \
       triage:",
      found.len()
    );
    for (entry, case) in record_found(path, &found) {
      println!("    {} ({})", entry, case);
    }
  }
  // The default-settings test pairs mto does not match, but those excused with a reason
  // (the standard formatter's own bugs): the v0.1.0 gate wants none left.
  let excused = options
    .exceptions
    .as_ref()
    .map(|path| exceptions(path, true))
    .unwrap_or_default();
  let pairs: Vec<(String, bool)> = cases
    .iter()
    .zip(&scores)
    .filter(|(case, score)| case.source == Source::Tests && !score.skipped)
    .map(|(case, score)| {
      (
        format!("{} [{}]", case.name, case.source.key()),
        score.matches,
      )
    })
    .collect();
  let open = pairs
    .iter()
    .filter(|(key, matches)| !matches && !excused.contains(key))
    .count();
  println!(
    "default-settings test pairs not matching: {}, {} of them excused in \
     records/policy/exceptions.txt",
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
      format!("{}:{} [{}] {}", case.name, line, case.source.key(), kind)
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
  // Files whose standard answer is not stable: scored against where it settles, listed to
  // do better on than the standard formatter's first run.
  let unsettled: Vec<String> = cases
    .iter()
    .zip(&scores)
    .filter(|(_, score)| score.unsettled)
    .map(|(case, score)| {
      format!(
        "{} [{}]{}",
        case.name,
        case.source.key(),
        if score.matches { " (matched)" } else { "" }
      )
    })
    .collect();
  println!(
    "files the standard formatter changes again on a second run, scored against where it \
     settles: {} ({} matched)",
    unsettled.len(),
    unsettled
      .iter()
      .filter(|l| l.ends_with("(matched)"))
      .count()
  );
  if let Some(dir) = options.broke_log.as_ref().and_then(|log| log.parent()) {
    let _ = fs::write(dir.join("diff-unsettled.txt"), unsettled.join("\n") + "\n");
  }
  let own: Vec<String> = cases
    .iter()
    .zip(&scores)
    .filter_map(|(case, score)| {
      score
        .own
        .as_ref()
        .map(|m| format!("{} [{}]: {}", case.name, case.source.key(), m))
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
        let name = format!(
          "{}-{}.input",
          case.source.key(),
          case.name.replace('/', "__")
        );
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
             records/policy/exceptions.txt does not list: a rewrite the allowed list lacks \
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
      // Where its runs settle, as the diff scores it.
      Answer::Text(text) => {
        let mut text = text;
        let mut name = "standard";
        for _ in 0..3 {
          match run_standard(&text, edition_year(edition), &dir, &home) {
            Answer::Text(next) if next != text => {
              text = next;
              name = "standard, run until it settles";
            }
            _ => break,
          }
        }
        (text, name)
      }
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

pub(crate) fn parse_args(args: &[String]) -> Result<Options, String> {
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
    out: None,
    sources: Vec::new(),
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
      "--sources" => {
        for name in value(i)?.split(',') {
          match Source::ALL.iter().find(|s| s.key() == name) {
            Some(source) => options.sources.push(*source),
            None => return Err(format!("no recorded set `{}`", name)),
          }
        }
      }
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
  if options.update && !options.sources.is_empty() && options.sources.len() < Source::ALL.len() {
    return Err("--update records every set at once; leave out --sources".into());
  }
  Ok(options)
}

fn parse_pairs_args(args: &[String]) -> Result<PairsArgs, String> {
  let (mut seed, mut out, mut files, mut crates) = (None, None, 0, Vec::new());
  let mut at = 0;
  while at < args.len() {
    let value = || {
      args
        .get(at + 1)
        .ok_or(format!("{} needs a value", args[at]))
    };
    match args[at].as_str() {
      "--seed" => {
        seed = Some(
          value()?
            .parse::<u64>()
            .map_err(|_| "--seed takes a number")?,
        )
      }
      "--out" => out = Some(PathBuf::from(value()?)),
      "--files" => files = value()?.parse().map_err(|_| "--files takes a number")?,
      other => {
        let (name, dir) = other
          .split_once('=')
          .ok_or(format!("unexpected `{}`", other))?;
        crates.push((name.to_string(), PathBuf::from(dir)));
        at += 1;
        continue;
      }
    }
    at += 2;
  }
  match (seed, out) {
    (Some(seed), Some(out)) if !crates.is_empty() => Ok(PairsArgs {
      seed,
      out,
      files,
      crates,
    }),
    _ => Err("usage: pairs --seed N --out DIR [--files N] NAME=DIR...".to_string()),
  }
}

#[cfg(test)]
mod tests {
  use mtindo::options::format::FormatConfig;
  use xtask_core::store;

  use super::*;
  use crate::process;

  fn finished(code: i32, stdout: &str, stderr: &str, timed_out: bool) -> process::Separate {
    process::Separate {
      code,
      stdout: stdout.as_bytes().to_vec(),
      stderr: stderr.as_bytes().to_vec(),
      timed_out,
    }
  }

  #[test]
  fn test_a_seed_draws_the_same_files_and_another_seed_others() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let first = draw(&dir, "tools", 1, 4);
    assert_eq!(first.len(), 4);
    assert_eq!(first, draw(&dir, "tools", 1, 4));
    assert!((2..10).any(|seed| draw(&dir, "tools", seed, 4) != first));
    assert!(draw(&dir, "tools", 1, 0).len() > 4);
    // A file's scramble does not depend on which others were drawn with it.
    let file = Path::new("main.rs");
    assert_eq!(file_seed(1, "tools", file), file_seed(1, "tools", file));
    assert_ne!(file_seed(1, "tools", file), file_seed(2, "tools", file));
  }

  #[test]
  fn test_common_lines_is_the_longest_common_subsequence() {
    assert_eq!(common_lines(&["a", "b", "c"], &["a", "b", "c"]), 3);
    assert_eq!(common_lines(&["a", "x", "c"], &["a", "b", "c"]), 2);
    assert_eq!(common_lines(&[], &["a"]), 0);
    assert_eq!(common_lines(&["b", "a"], &["a", "b"]), 1);
  }

  /// A flag on the standard formatter's output where mto is right is listed for triage,
  /// before the excused pairs, a test pair excused too, and its case kept once.
  #[test]
  fn test_found_flags_are_listed_and_kept() {
    let root = std::env::temp_dir().join(format!("mtindo-found-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let upstream = store::dir(&root, store::UPSTREAM).join("rustfmt");
    fs::create_dir_all(upstream.join("01-old")).unwrap();
    fs::write(
      upstream.join("README.md"),
      "| Case | What | Upstream |\n|---|---|---|\n| 01 old | x | y |\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("records/policy")).unwrap();
    let path = root.join("records/policy/exceptions.txt");
    fs::write(
      &path,
      "# header\n\na.rs [tests]  old\n\nexcused a.rs [tests]  old\n",
    )
    .unwrap();
    let case = Case {
      source: Source::Tests,
      name: "b.rs".to_string(),
      input: "// rustfmt-max_width: 50\nfn f() {}\n".to_string(),
      edition: Edition::E2021,
      dir: root.clone(),
      expected: None,
      format: FormatConfig::default(),
      ..Case::empty()
    };
    let found = [Found {
      case: &case,
      key: "b.rs [tests]".to_string(),
      message: "the tokens differ".to_string(),
      excused: true,
    }];
    let places = record_found(&path, &found);
    assert_eq!(places[0].1, ".dev/upstream/rustfmt/02-auto-b");
    let listed = fs::read_to_string(&path).unwrap();
    let at = |needle: &str| listed.find(needle).unwrap();
    assert!(at("b.rs [tests]  auto-recorded") < at("excused a.rs"));
    assert!(listed.contains("excused b.rs [tests]  auto-recorded"));
    let case_dir = upstream.join("02-auto-b");
    assert_eq!(
      fs::read_to_string(case_dir.join("rustfmt.toml")).unwrap(),
      "max_width = 50\n"
    );
    assert!(
      fs::read_to_string(upstream.join("README.md"))
        .unwrap()
        .contains("02 auto-b")
    );
    // Kept once.
    assert_eq!(record_found(&path, &found)[0].1, "no case kept");
    let _ = fs::remove_dir_all(&root);
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
  fn test_pairs_arguments() {
    let args = |words: &[&str]| -> Vec<String> { words.iter().map(|w| w.to_string()).collect() };
    let parsed = parse_pairs_args(&args(&[
      "--seed", "42", "--out", "/o", "--files", "3", "a=/x", "b=/y",
    ]))
    .unwrap();
    assert_eq!((parsed.seed, parsed.files), (42, 3));
    assert_eq!(parsed.crates[1], ("b".to_string(), PathBuf::from("/y")));
    assert!(parse_pairs_args(&args(&["--out", "/o", "a=/x"])).is_err());
    assert!(parse_pairs_args(&args(&["--seed", "x", "--out", "/o", "a=/x"])).is_err());
    assert!(parse_pairs_args(&args(&["--seed", "1", "--out", "/o", "stray"])).is_err());
  }

  #[test]
  fn test_records_round_trip_and_drops_are_found() {
    let mut tallies = BTreeMap::new();
    tallies.insert(
      Source::Corpus,
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
    worse.get_mut(&Source::Corpus).unwrap().matched_lines = 89;
    worse.get_mut(&Source::Corpus).unwrap().skipped = 1;
    let found = drops(&sets, &worse);
    assert_eq!(found.len(), 2, "{:?}", found);
    assert!(found[0].contains("refuses"), "{:?}", found);
    // A record without skipped counts does not hold them against a run.
    assert_eq!(drops(&old, &worse).len(), 1);
  }

  #[test]
  fn test_the_reference_is_the_pinned_toolchain() {
    let pinned =
      fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rust-toolchain.toml"))
        .unwrap();
    let toolchain = reference_toolchain();
    assert_ne!(toolchain, "stable");
    assert!(pinned.contains(&format!("channel = \"{}\"", toolchain)));
  }
}
