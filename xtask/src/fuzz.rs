//! `cargo xtask fuzz`: runs the fuzz targets and keeps every crash they find, with what it
//! said at the time.
//!
//! libFuzzer writes each input it fails on as `crash-<sha1>`, `oom-<sha1>`, `timeout-<sha1>`,
//! `leak-<sha1>` or `slow-unit-<sha1>`, a name that does not say which target, when, from
//! which commit or what went wrong. Runs here aim libFuzzer at a staging folder,
//! `fuzz/crashes/.incoming/<target>/`, and save the run's failure output beside the input it
//! failed on. Each input staged there, any in cargo-fuzz's default `fuzz/artifacts/<target>/`,
//! and any libFuzzer wrote to the repository root when run by hand there, is then filed under
//! `fuzz/crashes/` (ignored by git):
//!
//! ```text
//! fuzz/crashes/INDEX.tsv                           one line per crash
//! fuzz/crashes/<target>/<UTC date-time>-<kind>-<hash>/
//!     input      the bytes, as libFuzzer wrote them
//!     info.txt   when it crashed and at which commit, what it said then, how to replay it,
//!                the input, and a history of replays with what the latest one said
//! ```
//!
//! - `cargo xtask fuzz run TARGET [--time SECONDS]`: fuzz one target (60 s by default), then
//!   file what it found.
//! - `cargo xtask fuzz collect`: file whatever is waiting.
//! - `cargo xtask fuzz recheck`: replay every crash at the current commit, adding the result
//!   to its history and the index.
//! - `cargo xtask fuzz list`: print the index.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use crate::common::log::{Logger, timestamp};
use crate::common::process;

const INDEX_HEADER: &str = "crashed\ttarget\tkind\tstatus\tcommit\tfolder\n";

/// The kinds of input libFuzzer writes, by their names' prefixes.
const KINDS: &[&str] = &["crash-", "oom-", "timeout-", "leak-", "slow-unit-"];

/// The longest a replay may take before it counts as a timeout.
const REPLAY_TIMEOUT: &str = "-timeout=60";

/// The memory a run may use: the sanitizer's growth under libFuzzer passes the 2 GB default
/// on the `format` target within minutes.
const RSS_LIMIT_MB: &str = "-rss_limit_mb=4096";

/// Beside a staged input: the output of the run that found it.
const SAID_SUFFIX: &str = ".said";

/// Files every input waiting in the staging folders, cargo-fuzz's default artifacts folder
/// and the repository root; returns how many were filed.
pub fn collect(root: &Path, log: &Logger) -> usize {
  let mut waiting: Vec<(String, PathBuf)> = Vec::new();
  for dir in [
    crashes(root).join(".incoming"),
    root.join("fuzz").join("artifacts"),
  ] {
    let Ok(targets) = fs::read_dir(&dir) else {
      continue;
    };
    for target in targets.flatten() {
      let Ok(files) = fs::read_dir(target.path()) else {
        continue;
      };
      for file in files.flatten() {
        let name = file.file_name().to_string_lossy().into_owned();
        if file.path().is_file() && !name.ends_with(SAID_SUFFIX) {
          waiting.push((
            target.file_name().to_string_lossy().into_owned(),
            file.path(),
          ));
        }
      }
    }
  }
  // libFuzzer run by hand from the repository root writes there; its target is unknown.
  if let Ok(files) = fs::read_dir(root) {
    for file in files.flatten() {
      let name = file.file_name().to_string_lossy().into_owned();
      if file.path().is_file() && KINDS.iter().any(|kind| name.starts_with(kind)) {
        waiting.push(("unknown-target".to_string(), file.path()));
      }
    }
  }
  waiting.sort();
  let commit = commit(root);
  let mut filed = 0;
  for (target, path) in waiting {
    let said_path = PathBuf::from(format!("{}{}", path.display(), SAID_SUFFIX));
    let said = fs::read_to_string(&said_path).ok();
    match file_one(root, &target, &path, said.as_deref(), &commit) {
      Ok(folder) => {
        log.info(&format!("filed {} as {}", path.display(), folder.display()));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&said_path);
        filed += 1;
      }
      Err(e) => {
        log.error(&format!("cannot file {}: {}", path.display(), e));
        println!("cannot file {}: {}", path.display(), e);
      }
    }
  }
  filed
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
  let log = Logger::new(root, "fuzz");
  log.info(&format!("cargo xtask fuzz {}", args.join(" ")));
  match args.first().map(String::as_str) {
    Some("run") => {
      let Some(target) = args.get(1) else {
        return usage();
      };
      let seconds = match args.get(2).map(String::as_str) {
        None => 60,
        Some("--time") => match args.get(3).and_then(|s| s.parse::<u32>().ok()) {
          Some(seconds) => seconds,
          None => return usage(),
        },
        Some(_) => return usage(),
      };
      ExitCode::from(run_target(root, target, seconds, &log) as u8)
    }
    Some("collect") => {
      let filed = collect(root, &log);
      println!("filed {} crash(es) under fuzz/crashes/", filed);
      ExitCode::SUCCESS
    }
    Some("recheck") => {
      recheck(root, &log);
      ExitCode::SUCCESS
    }
    Some("list") => {
      match fs::read_to_string(crashes(root).join("INDEX.tsv")) {
        Ok(index) => print!("{}", index),
        Err(_) => println!("no crashes filed yet"),
      }
      ExitCode::SUCCESS
    }
    _ => usage(),
  }
}

/// Fuzzes `target` for `seconds` with libFuzzer's inputs staged for filing and the run's
/// failure output saved beside the one it failed on, then files them. The exit code is
/// libFuzzer's (0 when nothing was found), clamped to a byte.
pub fn run_target(root: &Path, target: &str, seconds: u32, log: &Logger) -> i32 {
  let incoming = crashes(root).join(".incoming").join(target);
  if let Err(e) = fs::create_dir_all(&incoming) {
    println!("cannot create {}: {}", incoming.display(), e);
    return 1;
  }
  let before: Vec<PathBuf> = staged(&incoming);
  let args: Vec<String> = vec![
    "+nightly".into(),
    "fuzz".into(),
    "run".into(),
    target.into(),
    "--".into(),
    format!("-max_total_time={}", seconds),
    RSS_LIMIT_MB.into(),
    // libFuzzer joins the prefix and the name as given, so it ends with a separator.
    format!("-artifact_prefix={}/", incoming.display()),
  ];
  log.info(&format!("fuzzing {} for {}s", target, seconds));
  let (code, output) = process::capture(root, "cargo", &args, &[]);
  let said = failure_lines(&output);
  if code != 0 {
    println!("{}", said.join("\n"));
  }
  // What the run said goes beside each input it left (libFuzzer stops at the first crash).
  for input in staged(&incoming)
    .into_iter()
    .filter(|p| !before.contains(p))
  {
    let note = format!(
      "run: cargo {}\nexit: {}\n\n{}\n",
      args.join(" "),
      code,
      said.join("\n")
    );
    let _ = fs::write(format!("{}{}", input.display(), SAID_SUFFIX), note);
  }
  let filed = collect(root, log);
  if filed > 0 {
    println!(
      "filed {} crash(es) under fuzz/crashes/{}/ (cargo xtask fuzz list)",
      filed, target
    );
  }
  code.clamp(0, 255)
}

/// The commit checked out; with uncommitted changes, `-dirty.` and a digest of them.
fn commit(root: &Path) -> String {
  let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
  let (code, hash) = process::capture(root, "git", &args(&["rev-parse", "--short", "HEAD"]), &[]);
  if code != 0 {
    return "unknown".to_string();
  }
  let (_, status) = process::capture(root, "git", &args(&["status", "--porcelain"]), &[]);
  if status.trim().is_empty() {
    return hash.trim().to_string();
  }
  // Uncommitted changes, named by a digest of the diff against HEAD and of the untracked
  // files, so that a status recorded on a dirty tree says which tree it was.
  let (_, diff) = process::capture(root, "git", &args(&["diff", "HEAD", "--binary"]), &[]);
  let (_, untracked) = process::capture(
    root,
    "git",
    &args(&["ls-files", "--others", "--exclude-standard"]),
    &[],
  );
  let mut changes = diff.into_bytes();
  for file in untracked.lines() {
    changes.extend_from_slice(file.as_bytes());
    changes.extend(fs::read(root.join(file)).unwrap_or_default());
  }
  let digest = crate::sha256::hex_digest(&changes);
  format!("{}-dirty.{}", hash.trim(), &digest[..8])
}

/// `2026-10-11T15:17:30.123Z` as `20261011-151730`, for folder names.
fn compact(time: SystemTime) -> String {
  timestamp(time)
    .chars()
    .take(19)
    .filter(|c| c.is_ascii_digit() || *c == 'T')
    .map(|c| if c == 'T' { '-' } else { c })
    .collect()
}

fn crashes(root: &Path) -> PathBuf {
  root.join("fuzz").join("crashes")
}

/// The end of a run's output worth keeping: from the panic or libFuzzer's error on, or the
/// last lines when there is neither.
fn failure_lines(output: &str) -> Vec<String> {
  let lines: Vec<&str> = output.lines().collect();
  let from = lines
    .iter()
    .position(|l| l.contains("panicked at") || l.contains("ERROR: libFuzzer"))
    .unwrap_or(lines.len().saturating_sub(20));
  lines[from..]
    .iter()
    .take(40)
    .map(|l| l.to_string())
    .collect()
}

/// Files one input: its folder with `input` and `info.txt`, and a line in the index.
fn file_one(
  root: &Path,
  target: &str,
  path: &Path,
  said: Option<&str>,
  commit: &str,
) -> Result<PathBuf, String> {
  let name = path
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .ok_or("no file name")?;
  // `slow-unit-<sha1>`, `crash-<sha1>`: the kind is everything before the hash.
  let (kind, hash) = name.rsplit_once('-').unwrap_or(("unknown", &name));
  let short: String = hash.chars().take(8).collect();
  let bytes = fs::read(path).map_err(|e| e.to_string())?;
  let crashed = fs::metadata(path)
    .and_then(|m| m.modified())
    .unwrap_or_else(|_| SystemTime::now());
  let folder = crashes(root)
    .join(target)
    .join(format!("{}-{}-{}", compact(crashed), kind, short));
  fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
  let input = folder.join("input");
  fs::write(&input, &bytes).map_err(|e| e.to_string())?;
  let (status, replay_output) = replay(root, target, &input);
  let now = timestamp(SystemTime::now());
  let mut info = String::new();
  info.push_str(&format!("target:   {}\n", target));
  info.push_str(&format!("kind:     {}\n", kind));
  info.push_str(&format!("crashed:  {}\n", timestamp(crashed)));
  info.push_str(&format!(
    "commit:   {} (when filed; `-dirty.<digest>` names uncommitted changes)\n",
    commit
  ));
  info.push_str(&format!("found:    {}\n", relative(root, path)));
  info.push_str(&format!("size:     {} bytes\n", bytes.len()));
  info.push_str(&format!("status:   {} at {}\n", status, commit));
  info.push_str(&format!(
    "replay:   {}\n",
    replay_command(root, target, &input)
  ));
  info.push_str("\nwhat it said when it crashed:\n");
  match said {
    Some(said) => {
      for line in said.lines() {
        info.push_str(&format!("  {}\n", line));
      }
    }
    None => info.push_str("  (not seen: found as a file; the replay below is the nearest)\n"),
  }
  info.push_str("\nfirst bytes, hex:\n");
  let head: Vec<String> = bytes
    .iter()
    .take(32)
    .map(|b| format!("{:02x}", b))
    .collect();
  info.push_str(&format!("  {}\n", head.join(" ")));
  if let Ok(text) = std::str::from_utf8(&bytes) {
    info.push_str("\ninput as text:\n");
    for line in text.lines().take(60) {
      info.push_str(&format!("  | {}\n", line));
    }
  }
  info.push_str("\nhistory:\n");
  info.push_str(&format!("  {}  {}  {}\n", now, commit, status));
  info.push_str(&latest_replay(&now, commit, &replay_output));
  fs::write(folder.join("info.txt"), info).map_err(|e| e.to_string())?;
  let index = crashes(root).join("INDEX.tsv");
  let mut text = fs::read_to_string(&index).unwrap_or_else(|_| INDEX_HEADER.to_string());
  text.push_str(&format!(
    "{}\t{}\t{}\t{} at {}\t{}\t{}\n",
    timestamp(crashed),
    target,
    kind,
    status,
    commit,
    commit,
    relative(root, &folder)
  ));
  fs::write(&index, text).map_err(|e| e.to_string())?;
  Ok(folder)
}

/// The last section of `info.txt`: what the latest replay printed.
fn latest_replay(when: &str, commit: &str, output: &[String]) -> String {
  let mut out = format!("\nlatest replay ({}, at {}):\n", when, commit);
  for line in output {
    out.push_str(&format!("  {}\n", line));
  }
  out
}

/// Replays every filed crash, adding the result to its history and the index.
fn recheck(root: &Path, log: &Logger) {
  let index_path = crashes(root).join("INDEX.tsv");
  let Ok(index) = fs::read_to_string(&index_path) else {
    println!("no crashes filed yet");
    return;
  };
  let commit = commit(root);
  let now = timestamp(SystemTime::now());
  let mut out = String::from(INDEX_HEADER);
  for line in index.lines().skip(1) {
    let mut fields: Vec<String> = line.split('\t').map(str::to_string).collect();
    if fields.len() != 6 {
      out.push_str(line);
      out.push('\n');
      continue;
    }
    let folder = root.join(&fields[5]);
    let (status, replay_output) = replay(root, &fields[1], &folder.join("input"));
    println!("{}  {} at {}", fields[5], status, commit);
    log.info(&format!(
      "rechecked {}: {} at {}",
      fields[5], status, commit
    ));
    fields[3] = format!("{} at {}", status, commit);
    if let Ok(info) = fs::read_to_string(folder.join("info.txt")) {
      // Up to the latest replay: the status updated, the history extended; then the new
      // latest replay.
      let kept = info
        .split("\nlatest replay (")
        .next()
        .unwrap_or(&info)
        .trim_end();
      let mut updated: Vec<String> = kept
        .lines()
        .map(|l| {
          if l.starts_with("status:") {
            format!("status:   {} at {}", status, commit)
          } else {
            l.to_string()
          }
        })
        .collect();
      updated.push(format!("  {}  {}  {}", now, commit, status));
      let text = format!(
        "{}\n{}",
        updated.join("\n"),
        latest_replay(&now, &commit, &replay_output)
      );
      let _ = fs::write(folder.join("info.txt"), text);
    }
    out.push_str(&fields.join("\t"));
    out.push('\n');
  }
  let _ = fs::write(&index_path, out);
}

fn relative(root: &Path, path: &Path) -> String {
  path
    .strip_prefix(root)
    .unwrap_or(path)
    .display()
    .to_string()
}

/// Replays an input once: whether it still fails, and the end of what the run printed.
fn replay(root: &Path, target: &str, input: &Path) -> (&'static str, Vec<String>) {
  let args: Vec<String> = vec![
    "+nightly".into(),
    "fuzz".into(),
    "run".into(),
    target.into(),
    input.display().to_string(),
    "--".into(),
    RSS_LIMIT_MB.into(),
    REPLAY_TIMEOUT.into(),
  ];
  let (code, output) = process::capture(root, "cargo", &args, &[]);
  // Only what libFuzzer says counts: a clean run reports `Executed <input> in`, a failing
  // one a panic or its own error. Anything else (a build error, no nightly toolchain) means
  // the replay did not run, which proves nothing either way.
  let failed = output.contains("panicked at") || output.contains("ERROR: libFuzzer");
  let status = if failed {
    "fails"
  } else if code == 0 && output.contains("Executed ") {
    "no longer fails"
  } else {
    "replay did not run"
  };
  (status, failure_lines(&output))
}

fn replay_command(root: &Path, target: &str, input: &Path) -> String {
  format!(
    "cargo +nightly fuzz run {} {} -- {}",
    target,
    relative(root, input),
    RSS_LIMIT_MB
  )
}

/// The inputs staged in a folder, notes aside.
fn staged(folder: &Path) -> Vec<PathBuf> {
  fs::read_dir(folder)
    .map(|files| {
      files
        .flatten()
        .map(|f| f.path())
        .filter(|p| p.is_file() && !p.display().to_string().ends_with(SAID_SUFFIX))
        .collect()
    })
    .unwrap_or_default()
}

fn usage() -> ExitCode {
  eprintln!("Usage: cargo xtask fuzz run TARGET [--time SECONDS] | collect | recheck | list");
  ExitCode::from(2)
}
