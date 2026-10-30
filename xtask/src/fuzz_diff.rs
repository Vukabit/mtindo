//! `cargo xtask fuzz diff`: differential fuzzing against the standard formatter (in
//! `xtask-tools diff-fuzz`, which links mtindo). Each way mto and the standard formatter
//! disagree is filed once as a `mismatch` incident, keyed by its name (`MatchExpr / line
//! breaks`), with the shrunk input that shows it. Before fuzzing, the open mismatches are
//! tried again and those both formatters now agree on are marked fixed. Mismatches never
//! fail the suite: formatting is a preview, and the compatibility ratchet holds it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::common::log::Logger;
use crate::common::{process, tree};
use crate::{corpus, incidents, metrics};

pub fn run(root: &Path, sub: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "fuzz-diff");
  let tools = |args: Vec<String>| {
    let mut all: Vec<String> = [
      "run",
      "--quiet",
      "--release",
      "--package",
      "xtask-tools",
      "--",
      "diff-fuzz",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    all.extend(args);
    process::capture(root, "cargo", &all, &[], process::LONG)
  };
  if let Some(file) = crate::cli::text(sub, "replay") {
    let edition = crate::cli::text(sub, "edition").unwrap_or_else(|| "2021".into());
    let (code, output) = tools(vec!["--replay".into(), file, "--edition".into(), edition]);
    print!("{}", output);
    return ExitCode::from(code.clamp(0, 255) as u8);
  }
  let tree = tree::identity(root);
  let fixed = recheck(root, &tree, &tools);
  if fixed > 0 {
    println!(
      "{} mismatch(es) fixed: both formatters agree on them now",
      fixed
    );
  }
  let seconds = sub.get_one::<u32>("time").copied().unwrap_or(120);
  let seed = sub
    .get_one::<u64>("seed")
    .copied()
    .unwrap_or_else(|| std::process::id() as u64 ^ now_seconds());
  let dirs: Vec<PathBuf> = fs::read_dir(corpus::cache_dir(root))
    .into_iter()
    .flatten()
    .flatten()
    .map(|e| e.path())
    .filter(|p| p.is_dir())
    .collect();
  if dirs.is_empty() {
    println!("the corpus is not checked out: run `cargo xtask corpus check` first");
    return ExitCode::from(2);
  }
  // The shrunk inputs are only kept until they are filed, each in its incident.
  let out =
    std::env::temp_dir()
      .join("mtindo-fuzz-diff")
      .join(format!("{}-{}", std::process::id(), seed));
  println!(
    "fuzzing against the standard formatter for {}s (seed {})",
    seconds, seed
  );
  log.info(&format!("diff-fuzz for {}s, seed {}", seconds, seed));
  let mut args = vec![
    "--time".to_string(),
    seconds.to_string(),
    "--seed".to_string(),
    seed.to_string(),
    "--out".to_string(),
    out.display().to_string(),
  ];
  args.extend(dirs.iter().map(|d| d.display().to_string()));
  let (code, output) = tools(args);
  if code != 0 {
    println!("xtask-tools diff-fuzz failed (exit {}):\n{}", code, output);
    return ExitCode::from(1);
  }
  let run = std::env::var(crate::runs::RUN_DIR)
    .ok()
    .and_then(|d| {
      Path::new(&d)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    })
    .unwrap_or_default();
  let (mut new, mut again) = (0, 0);
  for line in output.lines() {
    if let Some(figure) = line.strip_prefix("metric ") {
      let words: Vec<&str> = figure.split_whitespace().collect();
      if let [name, value, unit, ..] = words.as_slice()
        && let Ok(value) = value.parse::<f64>()
      {
        metrics::record(root, name, value, unit);
      }
      continue;
    }
    let fields: Vec<&str> = line.split('\t').collect();
    let ["mismatch", name, file, edition] = fields.as_slice() else {
      if !line.trim().is_empty() {
        println!("{}", line);
      }
      continue;
    };
    let Ok(input) = fs::read(file) else {
      continue;
    };
    let said = format!(
      "mto and the standard formatter disagree: {} (edition {})\n{}",
      name,
      edition,
      String::from_utf8_lossy(&input)
    );
    let report = incidents::Report {
      kind: "mismatch",
      target: "format",
      said: &said,
      input: Some(&input),
      replay: "",
      tree: &tree,
      run: &run,
      key: Some(name),
    };
    match incidents::file(root, &report) {
      Ok(filed) => {
        match filed {
          incidents::Filed::New(_) => new += 1,
          _ => again += 1,
        }
        if let Some((dir, mut incident)) = incidents::find(root, filed.id())
          && incident.replay.is_empty()
        {
          let input = dir.join("input");
          let shown = input.strip_prefix(root).unwrap_or(&input);
          incident.replay = format!(
            "cargo xtask fuzz diff --replay {} --edition {}",
            shown.display(),
            edition
          );
          incidents::note(
            root,
            &dir,
            &mut incident,
            &tree,
            "found by differential fuzzing",
          );
        }
      }
      Err(e) => println!("cannot file {}: {}", name, e),
    }
  }
  let _ = fs::remove_dir_all(&out);
  println!(
    "{} new way(s) mto and the standard formatter disagree, {} seen before (cargo xtask \
     incidents --kind mismatch)",
    new, again
  );
  ExitCode::SUCCESS
}

fn now_seconds() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_secs())
}

/// Tries every open mismatch again; marks fixed those both formatters agree on now, and
/// wont-fix those whose standard output changes the meaning.
fn recheck(root: &Path, tree: &str, tools: &dyn Fn(Vec<String>) -> (i32, String)) -> usize {
  let open: Vec<(PathBuf, incidents::Incident)> = incidents::all(root)
    .into_iter()
    .filter(|(_, i)| i.kind == "mismatch" && i.status == "open")
    .collect();
  if open.is_empty() {
    return 0;
  }
  let mut args = vec!["--recheck".to_string()];
  for (dir, incident) in &open {
    let edition = incident
      .replay
      .rsplit_once("--edition ")
      .map_or("2021", |(_, year)| year.trim());
    args.push(format!("{}:{}", edition, dir.join("input").display()));
  }
  let (_, output) = tools(args);
  let mut fixed = 0;
  for line in output.lines() {
    let mut words = line.split('\t');
    let (Some(said), Some(file)) = (words.next(), words.next()) else {
      continue;
    };
    let Some((dir, mut incident)) = open
      .iter()
      .find(|(dir, _)| dir.join("input").display().to_string() == file)
      .cloned()
    else {
      continue;
    };
    match said {
      "agree" => {
        incidents::mark(root, &dir, &mut incident, "fixed", tree);
        fixed += 1;
      }
      // No answer to match: the standard formatter's output means something else.
      "broken" => {
        incidents::mark(root, &dir, &mut incident, "wont-fix", tree);
        incidents::note(
          root,
          &dir,
          &mut incident,
          tree,
          &format!(
            "the standard formatter's output changes the meaning: {}",
            words.next().unwrap_or_default()
          ),
        );
      }
      _ => {}
    }
  }
  fixed
}
