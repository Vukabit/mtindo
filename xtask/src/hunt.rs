//! `cargo xtask hunt prepare|collect`: the ground a bug hunt runs on, and what it found
//! filed. A hunt is agents testing a frozen build as users would (no source) or with the
//! source, each finding reproduced by another from its script alone; this command does
//! the parts that are not an agent's.
//!
//! `prepare` makes `.dev/hunt/<id>/`:
//!
//! ```text
//! src/            the commit, a detached worktree (`git worktree remove` it when done)
//! bin/mto         its release build, what every hunter runs
//! guide/          the user guide and README of that commit, for the arms given them
//! known-gaps.txt  documented limits (capabilities partial or missing, open hunt incidents):
//!                 not findings
//! env.sh          `. env.sh NAME`: a sandbox of its own under play/NAME, with a decoy HOME
//!                 (rc files any change to is a finding), its own config, state and git
//!                 config, a fake-PATH folder, and helpers (mkrepo, commit, m)
//! hunt.toml       the commit, when, and the build's version
//! ```
//!
//! `collect` reads what the hunt returned (`{coverage, ledger}`, or the ledger alone),
//! files each finding a verifier reproduced as a `hunt` incident with its repro script as
//! the input, and adds the hunters' coverage to `.dev/hunt/map.txt`, so the next hunt
//! starts from what has gone longest without a look.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use crate::common::json::{self, Json};
use crate::common::log::timestamp;
use crate::common::process;
use crate::incidents;

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let code = match m.subcommand() {
    Some(("prepare", sub)) => prepare(
      root,
      crate::cli::text(sub, "rev").as_deref().unwrap_or("HEAD"),
    ),
    Some(("collect", sub)) => collect(
      root,
      &crate::cli::text(sub, "result").unwrap_or_default(),
      crate::cli::text(sub, "hunt").as_deref(),
    ),
    _ => Err("give prepare or collect".to_string()),
  };
  match code {
    Ok(()) => ExitCode::SUCCESS,
    Err(why) => {
      println!("error: {}", why);
      ExitCode::from(1)
    }
  }
}

fn collect(root: &Path, result: &str, hunt: Option<&str>) -> Result<(), String> {
  if result.is_empty() {
    return Err("give the hunt's result: a JSON file".to_string());
  }
  let text = fs::read_to_string(result).map_err(|e| format!("cannot read {}: {}", result, e))?;
  let value = json::parse(&text).map_err(|e| format!("{}: {}", result, e))?;
  // A workflow's whole output holds its result under `result`.
  let value = value.get("result").cloned().unwrap_or(value);
  let ledger = match &value {
    Json::List(items) => items.clone(),
    _ => match value.get("ledger") {
      Some(Json::List(items)) => items.clone(),
      _ => return Err(format!("{}: no ledger in it", result)),
    },
  };
  let manifest = hunt
    .map(|id| fs::read_to_string(hunts(root).join(id).join("hunt.toml")).unwrap_or_default())
    .unwrap_or_default();
  let commit = manifest
    .lines()
    .find_map(|l| l.strip_prefix("commit = \""))
    .map(|c| c.trim_end_matches('"').to_string())
    .unwrap_or_else(|| "unknown".to_string());
  let run = hunt.unwrap_or("hunt");
  let field = |f: &Json, key: &str| f.get(key).and_then(Json::as_str).unwrap_or("").to_string();
  let (mut new, mut again, mut skipped) = (0, 0, 0);
  for finding in &ledger {
    let verdict = finding.get("verdict");
    let reproduced = matches!(
      verdict.and_then(|v| v.get("reproduced")),
      Some(Json::Bool(true))
    );
    let severity = verdict
      .and_then(|v| v.get("severity"))
      .and_then(Json::as_str)
      .unwrap_or_else(|| finding.get("severity").and_then(Json::as_str).unwrap_or(""))
      .to_string();
    if !reproduced || severity == "not-a-bug" {
      skipped += 1;
      continue;
    }
    let title = field(finding, "title");
    let said = format!(
      "{}\n\nseverity: {}\nfound by: {} ({})\ninvariant: {}\nexpected: {}\nactual: {}\ncause: {}\nverifier: {}\n",
      title,
      severity,
      field(finding, "hunter"),
      field(finding, "arm"),
      field(finding, "invariant"),
      field(finding, "expected"),
      field(finding, "actual"),
      field(finding, "cause"),
      verdict.map(|v| field(v, "judgement")).unwrap_or_default(),
    );
    let repro = field(finding, "repro");
    let report = incidents::Report {
      kind: "hunt",
      target: &field(finding, "hunter"),
      said: &said,
      input: Some(repro.as_bytes()),
      replay: "bash the input (its repro script), the hunt's env.sh beside it",
      tree: &commit,
      run,
      key: Some(&title),
    };
    match incidents::file(root, &report).map_err(|e| e.to_string())? {
      incidents::Filed::New(_) => new += 1,
      _ => again += 1,
    }
  }
  let _ = incidents::write_board(root);
  // The coverage, one line a hunter: when, what commit, who, what they covered.
  if let Some(Json::List(coverage)) = value.get("coverage") {
    let mut map = fs::read_to_string(hunts(root).join("map.txt")).unwrap_or_else(|_| {
      "# Each hunt's hunters and what they covered, oldest first: the next starts from what has gone longest without a look.\n".to_string()
    });
    let when = timestamp(SystemTime::now());
    for hunter in coverage {
      let covered: String = field(hunter, "covered")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
      let cut: String = covered.chars().take(400).collect();
      map.push_str(&format!(
        "{}\t{}\t{}\t{}\t{} found\t{}\n",
        &when[..10.min(when.len())],
        &commit[..10.min(commit.len())],
        field(hunter, "hunter"),
        field(hunter, "arm"),
        hunter.get("found").map(Json::render).unwrap_or_default(),
        cut
      ));
    }
    fs::create_dir_all(hunts(root)).map_err(|e| e.to_string())?;
    fs::write(hunts(root).join("map.txt"), map).map_err(|e| e.to_string())?;
  }
  println!(
    "hunt: {} new incident(s), {} seen before, {} not filed (not reproduced, or not a bug); \
     .dev/incidents/INCIDENTS.md lists them",
    new, again, skipped
  );
  Ok(())
}

/// The sandbox script each hunter sources.
fn env_script(dir: &Path) -> String {
  format!(
    r#"# Source with the hunter's name: `. env.sh NAME`. Everything lives under $HUNT/play/NAME.
HUNT={hunt}
P=$HUNT/play/${{1:?name}}
mkdir -p "$P/home/.config/fish" "$P/xdg/config" "$P/xdg/state" "$P/fakebin" "$P/r" "$P/repros"
[ -f "$P/home/.zshrc" ] || {{ echo '# decoy zshrc: any change here is a finding' > "$P/home/.zshrc"; cp "$P/home/.zshrc" "$P/home/.bashrc"; echo '# decoy' > "$P/home/.config/fish/config.fish"; }}
[ -f "$P/gitconfig" ] || printf '[user]\n\tname = t\n\temail = t@t\n[init]\n\tdefaultBranch = main\n[protocol "file"]\n\tallow = always\n' > "$P/gitconfig"
export HOME="$P/home" XDG_CONFIG_HOME="$P/xdg/config" XDG_STATE_HOME="$P/xdg/state" GIT_CONFIG_GLOBAL="$P/gitconfig" GIT_CONFIG_NOSYSTEM=1 CI=
unset MTINDO_CONFIG
M=$HUNT/bin/mto
# A fresh git repository under $P/r, entered.
mkrepo() {{ rm -rf "$P/r/$1"; mkdir -p "$P/r/$1"; cd "$P/r/$1" && git init -q .; }}
commit() {{ git add -A && git commit -q -m "${{1:-c}}"; }}
# mto with its exit code shown.
m() {{ "$M" "$@"; echo "[exit $?]"; }}
"#,
    hunt = dir.display()
  )
}

/// `git ARGS` in `dir`, or why it failed.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
  let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
  let (code, out) = process::capture(dir, "git", &args, &[], process::QUICK);
  if code != 0 {
    return Err(format!("git {}: {}", args.join(" "), out.trim()));
  }
  Ok(out)
}

/// Where hunts are kept.
fn hunts(root: &Path) -> PathBuf {
  crate::common::store::dir(root, crate::common::store::HUNT)
}

/// The documented limits a hunter should not report: settings and flags mto honours in part
/// or not yet, and what open hunt incidents already hold.
fn known_gaps(root: &Path, src: &Path) -> String {
  let mut out = String::from(
    "# Known gaps: documented limits and open findings, not new findings.\n\n## Not honoured, \
     or in part (records/policy/capabilities.txt)\n",
  );
  let capabilities =
    fs::read_to_string(src.join("records/policy/capabilities.txt")).unwrap_or_default();
  for line in capabilities.lines() {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() >= 3 && matches!(words[2], "partial" | "missing" | "not-planned") {
      out.push_str(line);
      out.push('\n');
    }
  }
  out.push_str("\n## Open hunt findings (.dev/incidents/hunt)\n");
  for (_, incident) in incidents::all(root) {
    if incident.kind == "hunt" && incident.status == "open" {
      out.push_str(&format!("{}\n", incident.title));
    }
  }
  out
}

fn prepare(root: &Path, rev: &str) -> Result<(), String> {
  let commit = git(
    root,
    &["rev-parse", "--verify", &format!("{}^{{commit}}", rev)],
  )?
  .trim()
  .to_string();
  let short = &commit[..10.min(commit.len())];
  let stamp: String = timestamp(SystemTime::now())
    .chars()
    .take(19)
    .filter(|c| c.is_ascii_digit() || *c == 'T')
    .map(|c| if c == 'T' { '-' } else { c })
    .collect();
  let id = format!("{}-{}", stamp, short);
  let dir = hunts(root).join(&id);
  fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {}", dir.display(), e))?;
  let src = dir.join("src");
  println!("hunt {}: the worktree of {}", id, short);
  git(
    root,
    &[
      "worktree",
      "add",
      "--detach",
      &src.display().to_string(),
      &commit,
    ],
  )?;
  println!("hunt {}: building mto (release)", id);
  let args: Vec<String> = ["build", "--quiet", "--release", "--bin", "mto"]
    .iter()
    .map(|s| s.to_string())
    .collect();
  let target = dir.join("target");
  let (code, out) = process::capture(
    &src,
    "cargo",
    &args,
    &[("CARGO_TARGET_DIR".to_string(), target.display().to_string())],
    process::LONG,
  );
  if code != 0 {
    return Err(format!("cannot build mto at {}:\n{}", short, out));
  }
  let exe = format!("mto{}", std::env::consts::EXE_SUFFIX);
  fs::create_dir_all(dir.join("bin")).map_err(|e| e.to_string())?;
  fs::copy(
    target.join("release").join(&exe),
    dir.join("bin").join(&exe),
  )
  .map_err(|e| format!("cannot copy the build: {}", e))?;
  // The guide as the commit has it.
  let guide = dir.join("guide");
  fs::create_dir_all(&guide).map_err(|e| e.to_string())?;
  if let Ok(entries) = fs::read_dir(src.join("docs").join("guide")) {
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().is_some_and(|e| e == "md") {
        let _ = fs::copy(&path, guide.join(entry.file_name()));
      }
    }
  }
  let _ = fs::copy(src.join("README.md"), guide.join("README.md"));
  fs::write(dir.join("known-gaps.txt"), known_gaps(root, &src)).map_err(|e| e.to_string())?;
  fs::write(dir.join("env.sh"), env_script(&dir)).map_err(|e| e.to_string())?;
  let (_, version) = process::capture(
    &dir,
    &dir.join("bin").join(&exe).display().to_string(),
    &["--version".to_string()],
    &[],
    process::QUICK,
  );
  fs::write(
    dir.join("hunt.toml"),
    format!(
      "id = \"{}\"\ncommit = \"{}\"\nprepared = \"{}\"\nmto = \"{}\"\n",
      id,
      commit,
      timestamp(SystemTime::now()),
      version.trim()
    ),
  )
  .map_err(|e| e.to_string())?;
  println!("hunt {}: ready in {}", id, dir.display());
  println!(
    "  each hunter: `. {}/env.sh NAME`, then $M is the build",
    dir.display()
  );
  println!(
    "  afterwards: cargo xtask hunt collect RESULT.json --hunt {}; git worktree remove {}",
    id,
    src.display()
  );
  Ok(())
}
