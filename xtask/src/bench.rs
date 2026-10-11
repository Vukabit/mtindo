//! `cargo xtask corpus bench`: the speed target (S2-1) on this machine, as a table.
//!
//! Every tool is timed on the pinned corpus, end to end, by the wall clock and by CPU time
//! (children's user plus system time, from `/usr/bin/time -p`):
//!
//! - **rustfmt stable and nightly** as `cargo fmt --all --check` runs them, one repository
//!   at a time; the bytes are those of the files rustfmt says it formats (`-v`). A toolchain
//!   that is not installed is skipped;
//! - **mto** with `--check` on a copy of the corpus already laid out (the case CI sees), and
//!   on the corpus as fetched, where most files would change and each is also verified and
//!   diffed; median of three runs each.
//!
//! The copies live outside this repository (Cargo would otherwise take the corpus for part
//! of mtindo's workspace) and are reused while `compat/corpus.txt` is unchanged.
//!
//! S2-1: mto's throughput on the laid-out corpus is at least [`MIN_SPEEDUP`] times rustfmt
//! stable's and nightly's, by the wall clock; mto may use every core. The walk-and-read
//! floor is shown for reference.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::common::log::Logger;
use crate::common::process;
use crate::sha256;

/// S2-1: mto's wall-clock throughput over rustfmt's.
pub const MIN_SPEEDUP: f64 = 2.0;

/// The settings mto is timed with: layout on, and no configuration from outside the copy.
const MTO_CONFIG: &str = "layout = true\nroot = true\n";

/// The longest one timed command may run.
const TIMED_LIMIT: Duration = Duration::from_secs(30 * 60);

struct Copies {
  base: PathBuf,
  raw: PathBuf,
  laid: PathBuf,
}

/// One row of the table.
pub struct Row {
  pub tool: String,
  pub files: usize,
  pub bytes: u64,
  pub wall: f64,
  /// `None` when CPU time could not be measured.
  pub cpu: Option<f64>,
}

impl Row {
  fn mb(&self) -> f64 {
    self.bytes as f64 / 1e6
  }

  fn throughput(&self) -> f64 {
    self.mb() / self.wall.max(1e-9)
  }
}

/// What [`timed`] measured.
struct Timed {
  wall: f64,
  cpu: Option<f64>,
  /// stdout (when kept) and stderr, without time's own lines.
  output: String,
  code: Option<i32>,
}

/// Times everything and prints the table; returns the process exit code.
pub fn run(root: &Path, repos: &[(String, PathBuf)], markdown: bool, log: &Logger) -> u8 {
  let copies = match prepare(root, repos, log) {
    Ok(copies) => copies,
    Err(e) => {
      println!("cannot prepare the bench copies: {}", e);
      log.error(&e);
      return 1;
    }
  };
  let mto = root.join("target").join("release").join("mto");
  let mut rows = Vec::new();
  for toolchain in ["stable", "nightly"] {
    match rustfmt_row(&copies.raw, toolchain, log) {
      Ok(row) => rows.push(row),
      Err(e) => {
        println!("rustfmt {}: skipped ({})", toolchain, e);
        log.warning(&format!("rustfmt {} skipped: {}", toolchain, e));
      }
    }
  }
  let config = copies.base.join("mto-bench.toml");
  for (name, dir) in [
    ("mto, laid-out corpus", &copies.laid),
    ("mto, as fetched", &copies.raw),
  ] {
    match mto_row(&mto, dir, &config, name) {
      Ok(row) => rows.push(row),
      Err(e) => {
        println!("{}: {}", name, e);
        log.error(&format!("{}: {}", name, e));
        return 1;
      }
    }
  }
  print!("{}", table(&rows, markdown));
  for row in rows.iter() {
    log.info(&format!(
      "bench row: {}: {} files, {} bytes, wall {:.3} s, cpu {:?}",
      row.tool, row.files, row.bytes, row.wall, row.cpu
    ));
  }
  let laid = rows
    .iter()
    .find(|r| r.tool.starts_with("mto, laid-out"))
    .map(Row::throughput)
    .unwrap_or(0.0);
  let references: Vec<&Row> = rows
    .iter()
    .filter(|r| r.tool.starts_with("rustfmt") && r.bytes > 0 && r.wall > 0.0)
    .collect();
  if references.is_empty() {
    println!(
      "S2-1: not checked, so this fails: no rustfmt toolchain formatted anything to compare \
       against. Install one with: rustup toolchain install stable --component rustfmt"
    );
    log.error("speed target not checked: no rustfmt reference row");
    return 1;
  }
  let mut failed = false;
  for row in references {
    let speedup = laid / row.throughput();
    let verdict = if speedup >= MIN_SPEEDUP {
      "ok"
    } else {
      failed = true;
      "below target"
    };
    println!(
      "S2-1: mto (laid out) is {:.1}x {} by throughput (at least {:.0}x): {}",
      speedup, row.tool, MIN_SPEEDUP, verdict
    );
  }
  if failed { 1 } else { 0 }
}

/// The table, aligned for a terminal or as Markdown.
pub fn table(rows: &[Row], markdown: bool) -> String {
  let header = [
    "tool",
    "files",
    "MB",
    "wall s",
    "MB/s",
    "CPU s",
    "MB/s/core",
  ];
  let cells: Vec<Vec<String>> = rows
    .iter()
    .map(|r| {
      vec![
        r.tool.clone(),
        r.files.to_string(),
        format!("{:.1}", r.mb()),
        format!("{:.2}", r.wall),
        format!("{:.1}", r.throughput()),
        r.cpu.map_or("-".to_string(), |c| format!("{:.2}", c)),
        r.cpu
          .map_or("-".to_string(), |c| format!("{:.1}", r.mb() / c.max(1e-9))),
      ]
    })
    .collect();
  let mut out = String::new();
  if markdown {
    out.push_str(&format!("| {} |\n", header.join(" | ")));
    out.push_str(&format!(
      "|{}\n",
      header
        .iter()
        .enumerate()
        .map(|(i, _)| if i == 0 { "---|" } else { "---:|" })
        .collect::<String>()
    ));
    for row in cells {
      out.push_str(&format!("| {} |\n", row.join(" | ")));
    }
    return out;
  }
  let widths: Vec<usize> = (0..header.len())
    .map(|i| {
      cells
        .iter()
        .map(|r| r[i].len())
        .chain([header[i].len()])
        .max()
        .unwrap_or(0)
    })
    .collect();
  let line = |row: &[String]| {
    row
      .iter()
      .enumerate()
      .map(|(i, c)| {
        if i == 0 {
          format!("{:<w$}", c, w = widths[i])
        } else {
          format!("{:>w$}", c, w = widths[i])
        }
      })
      .collect::<Vec<_>>()
      .join("  ")
      .trim_end()
      .to_string()
  };
  let header: Vec<String> = header.iter().map(|h| h.to_string()).collect();
  out.push_str(&line(&header));
  out.push('\n');
  for row in cells {
    out.push_str(&line(&row));
    out.push('\n');
  }
  out
}

/// Copies a repository (no `.git`, no `target`), following symlinked files.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
  fs::create_dir_all(to)?;
  for entry in fs::read_dir(from)?.flatten() {
    let name = entry.file_name();
    if name == ".git" || name == "target" {
      continue;
    }
    // Symlinks are followed: repositories link shared modules into several crates.
    let Ok(meta) = fs::metadata(entry.path()) else {
      continue;
    };
    if meta.is_dir() {
      if !entry.file_type()?.is_symlink() {
        copy_tree(&entry.path(), &to.join(&name))?;
      }
    } else if meta.is_file() {
      fs::copy(entry.path(), to.join(&name))?;
    }
  }
  Ok(())
}

fn mto_row(mto: &Path, dir: &Path, config: &Path, name: &str) -> Result<Row, String> {
  let args = vec![
    "--check".to_string(),
    "--config-path".to_string(),
    config.display().to_string(),
    ".".to_string(),
  ];
  let mut runs = Vec::new();
  for _ in 0..3 {
    let run = timed(mto, &args, dir, false)?;
    if run.code == Some(4) {
      return Err("mto reported a verifier failure".to_string());
    }
    runs.push((run.wall, run.cpu));
  }
  runs.sort_by(|a, b| a.0.total_cmp(&b.0));
  let (files, bytes) = rust_files(dir);
  Ok(Row {
    tool: name.to_string(),
    files,
    bytes,
    wall: runs[1].0,
    cpu: runs[1].1,
  })
}

/// The raw and laid-out copies, made or refreshed as needed.
fn prepare(root: &Path, repos: &[(String, PathBuf)], log: &Logger) -> Result<Copies, String> {
  let pins = fs::read(root.join("compat").join("corpus.txt")).map_err(|e| e.to_string())?;
  let key = sha256::hex_digest(&pins)[..12].to_string();
  let base = std::env::temp_dir().join("mtindo-bench");
  let copies = Copies {
    raw: base.join(&key).join("raw"),
    laid: base.join(&key).join("laid"),
    base: base.clone(),
  };
  fs::create_dir_all(&base).map_err(|e| e.to_string())?;
  fs::write(base.join("mto-bench.toml"), MTO_CONFIG).map_err(|e| e.to_string())?;
  // Copies of an older pin list are removed.
  if let Ok(entries) = fs::read_dir(&base) {
    for entry in entries.flatten() {
      if entry.path().is_dir() && entry.file_name() != key.as_str() {
        let _ = fs::remove_dir_all(entry.path());
      }
    }
  }
  for copy in [&copies.raw, &copies.laid] {
    if copy.join(".complete").exists() {
      continue;
    }
    println!("copying the corpus to {}...", copy.display());
    let _ = fs::remove_dir_all(copy);
    for (name, dir) in repos {
      copy_tree(dir, &copy.join(name)).map_err(|e| format!("{}: {}", name, e))?;
    }
    fs::write(copy.join(".complete"), "").map_err(|e| e.to_string())?;
  }
  // Lay the copy out with the mto being measured; on a laid-out copy this changes little.
  println!("building mto (release) and laying out the copy...");
  let build = ["build", "--quiet", "--release", "--bin", "mto"].map(String::from);
  if process::capture(root, "cargo", &build, &[], process::LONG).0 != 0 {
    return Err("cannot build mto".to_string());
  }
  let mto = root.join("target").join("release").join("mto");
  let laid_out = process::output(
    Command::new(&mto)
      .arg("--config-path")
      .arg(base.join("mto-bench.toml"))
      .arg(".")
      .current_dir(&copies.laid)
      .stdout(Stdio::null())
      .stderr(Stdio::null()),
    TIMED_LIMIT,
  )
  .map_err(|e| e.to_string())?;
  if laid_out.timed_out {
    return Err("laying out the bench copy passed its time limit".to_string());
  }
  if laid_out.code == 4 {
    return Err("mto reported a verifier failure; run `cargo xtask corpus check`".to_string());
  }
  log.info(&format!("bench copies ready under {}", base.display()));
  Ok(copies)
}

/// The `.rs` files mto reads under `dir`, skipping the folders it skips by default.
fn rust_files(dir: &Path) -> (usize, u64) {
  const SKIPPED: &[&str] = &[".git", "target", "node_modules", "vendor", "dist", ".venv"];
  let (mut files, mut bytes) = (0, 0);
  let mut stack = vec![dir.to_path_buf()];
  while let Some(folder) = stack.pop() {
    let Ok(entries) = fs::read_dir(&folder) else {
      continue;
    };
    for entry in entries.flatten() {
      let Ok(file_type) = entry.file_type() else {
        continue;
      };
      let name = entry.file_name();
      if file_type.is_dir() {
        if !SKIPPED.iter().any(|s| name == *s) {
          stack.push(entry.path());
        }
      } else if file_type.is_file() && entry.path().extension().is_some_and(|e| e == "rs") {
        files += 1;
        bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
      }
    }
  }
  (files, bytes)
}

fn rustfmt_row(raw: &Path, toolchain: &str, log: &Logger) -> Result<Row, String> {
  let version = process::output(
    Command::new("rustfmt")
      .arg(format!("+{}", toolchain))
      .arg("--version")
      .env("RUSTUP_AUTO_INSTALL", "0")
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  )
  .map_err(|e| format!("rustfmt cannot be run: {}", e))?;
  if version.code != 0 {
    return Err(format!("toolchain {} is not installed", toolchain));
  }
  let version = String::from_utf8_lossy(&version.stdout).trim().to_string();
  println!("timing {} as cargo fmt runs it...", version);
  let cargo = Path::new("cargo");
  let args: Vec<String> = [
    &format!("+{}", toolchain),
    "fmt",
    "--all",
    "--check",
    "--",
    "-v",
  ]
  .iter()
  .map(|s| s.to_string())
  .collect();
  let mut formatted = BTreeSet::new();
  let (mut wall, mut cpu) = (0.0, Some(0.0));
  let mut repos: Vec<PathBuf> = fs::read_dir(raw)
    .map_err(|e| e.to_string())?
    .flatten()
    .map(|e| e.path())
    .filter(|p| p.join("Cargo.toml").is_file())
    .collect();
  repos.sort();
  for repo in repos {
    let run = timed(cargo, &args, &repo, true)?;
    let (seconds, cpu_seconds, output) = (run.wall, run.cpu, run.output);
    let found: Vec<&str> = output
      .lines()
      .filter_map(|l| l.strip_prefix("Formatting "))
      .collect();
    if found.is_empty() {
      log.warning(&format!(
        "rustfmt {} formatted nothing in {}: {}",
        toolchain,
        repo.display(),
        output.lines().last().unwrap_or("")
      ));
      continue;
    }
    formatted.extend(found.into_iter().map(PathBuf::from));
    wall += seconds;
    cpu = match (cpu, cpu_seconds) {
      (Some(total), Some(more)) => Some(total + more),
      _ => None,
    };
  }
  let bytes = formatted
    .iter()
    .filter_map(|f| fs::metadata(f).ok())
    .map(|m| m.len())
    .sum();
  Ok(Row {
    tool: format!("rustfmt {} (cargo fmt)", toolchain),
    files: formatted.len(),
    bytes,
    wall,
    cpu,
  })
}

/// Runs a command under `/usr/bin/time -p`. Its stdout is kept only when asked: mto's diffs
/// over the whole corpus are hundreds of megabytes.
fn timed(program: &Path, args: &[String], dir: &Path, keep_stdout: bool) -> Result<Timed, String> {
  let time = Path::new("/usr/bin/time");
  let mut command = if time.exists() {
    let mut c = Command::new(time);
    c.arg("-p").arg(program);
    c
  } else {
    Command::new(program)
  };
  let started = Instant::now();
  let output = process::output(
    command
      .args(args)
      .current_dir(dir)
      .env("CI", "1")
      .stdout(if keep_stdout {
        Stdio::piped()
      } else {
        Stdio::null()
      })
      .stderr(Stdio::piped()),
    TIMED_LIMIT,
  )
  .map_err(|e| format!("cannot run {}: {}", program.display(), e))?;
  if output.timed_out {
    return Err(format!(
      "{} was stopped at its time limit of {}",
      program.display(),
      process::duration(TIMED_LIMIT)
    ));
  }
  let wall = started.elapsed().as_secs_f64();
  let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
  let mut cpu = None;
  let mut rest = String::from_utf8_lossy(&output.stdout).into_owned();
  for line in stderr.lines() {
    let mut words = line.split_whitespace();
    match (
      words.next(),
      words.next().and_then(|v| v.parse::<f64>().ok()),
    ) {
      (Some("user" | "sys"), Some(seconds)) if words.next().is_none() => {
        *cpu.get_or_insert(0.0) += seconds;
      }
      (Some("real"), Some(_)) if words.next().is_none() => {}
      _ => {
        rest.push_str(line);
        rest.push('\n');
      }
    }
  }
  Ok(Timed {
    wall,
    cpu,
    output: rest,
    code: Some(output.code),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn row(tool: &str, wall: f64, cpu: Option<f64>) -> Row {
    Row {
      tool: tool.to_string(),
      files: 10,
      bytes: 8_000_000,
      wall,
      cpu,
    }
  }

  #[test]
  fn test_table_columns() {
    let rows = [row("rustfmt stable", 1.0, Some(0.8)), row("mto", 0.1, None)];
    let text = table(&rows, false);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("tool"), "{}", text);
    assert!(
      lines[1].contains("8.0") && lines[1].ends_with("10.0"),
      "{}",
      text
    );
    assert!(lines[2].ends_with("-"), "{}", text);
    let md = table(&rows, true);
    assert!(md.starts_with("| tool | files |"), "{}", md);
    assert!(
      md.lines().nth(1).unwrap().starts_with("|---|---:|"),
      "{}",
      md
    );
  }
}
