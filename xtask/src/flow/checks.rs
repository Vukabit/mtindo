//! The checks the stages run, each by the name `flow.toml`'s `[stages]` gives it.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use super::config::Flow;
use super::git;
use crate::common::process;
use crate::common::store;

/// Every check a stage may list, with what it checks: the registry `cargo xtask checks`
/// shows. A stage may also list any gate of `cargo xtask gates` by its id.
pub const CHECKS: &[(&str, &str)] = &[
  (
    "no-internal",
    "nothing under docs/internal/ (working notes) is committed",
  ),
  (
    "secrets",
    "nothing that looks like a credential is committed",
  ),
  ("fmt", "`cargo fmt --all --check` agrees with the layout"),
  ("gates", "every repository rule of `cargo xtask gates`"),
  (
    "actions-pinned",
    "every workflow action is pinned to a full commit hash",
  ),
  (
    "messages",
    "the commit messages follow CONTRIBUTING.md's rules",
  ),
  (
    "hold",
    "`main` and release tags are pushed only as a release left them",
  ),
  (
    "tools-pinned",
    "flow.toml's tool pins agree with the files that name the same versions",
  ),
  ("clippy-lib", "clippy on mtindo, warnings are errors"),
  ("tests-lib", "mtindo's tests"),
  (
    "tests-near",
    "the unit tests of each package the uncommitted changes touch",
  ),
  (
    "suite",
    "`cargo xtask test`, unless the last green run was on this tree",
  ),
  ("msrv", "mtindo's tests on the oldest supported Rust"),
  ("clean", "nothing uncommitted, untracked files included"),
  (
    "pushed",
    "HEAD is on its remote branch, which a release fast-forwards to",
  ),
  ("ci-green", "every CI run for HEAD finished and passed"),
  (
    "compat",
    "the compatibility record holds for this tree, running `cargo xtask diff` unless proven",
  ),
  (
    "compat-record",
    "`cargo xtask diff` passed on exactly this tree",
  ),
  (
    "version-agrees",
    "the release tag, Cargo.toml's version and a CHANGELOG.md section agree",
  ),
  (
    "tokens",
    "every expiring credential CI uses is more than 14 days from expiry",
  ),
  (
    "package",
    "the crate builds as crates.io gets it (`cargo package --locked`)",
  ),
  (
    "history-clean",
    "no commit ever made holds a credential or a working note",
  ),
  (
    "commands-doc",
    "docs/generated/commands.md matches every command's help",
  ),
  (
    "tests-xtask",
    "the unit tests of xtask, xtask-core and xtask-tools",
  ),
  (
    "github-settings",
    "the repository's GitHub settings are those MAINTAINING.md lists",
  ),
  (
    "deny",
    "`cargo deny` on the dependency graph: advisories, licenses, sources",
  ),
  (
    "mutants",
    "the meaning check and comment handling mutated: which mutants the tests miss",
  ),
];

/// The files `mutants` puts bugs in: the meaning check, its rewrites, and comment handling.
const MUTATED: &[&str] = &[
  "src/verify/format.rs",
  "src/verify/rewrites.rs",
  "src/format/comment.rs",
];

/// What a stage checks: the staged files for `pre-commit`, the refs being pushed for
/// `pre-push`, a range of commits for `ci`.
pub struct Scope {
  pub stage: String,
  /// Files to look in; `None` for the whole tree.
  pub files: Option<Vec<String>>,
  /// Whether to read the index (what is staged) rather than the work tree.
  pub staged: bool,
  /// Commit ranges whose messages are checked (`rev-list` arguments each).
  pub ranges: Vec<Vec<String>>,
  /// Refs being pushed: (remote ref, local sha).
  pub pushed: Vec<(String, String)>,
}

/// Every repository rule `cargo xtask gates` checks.
/// What is staged, written out as a tree of its own under `target/`, so checks of the
/// whole tree see the commit being made and not what is left unstaged. Its `.git` file
/// names the repository, so git there reads the same history and index, and the
/// machine-local records the gates read (`.dev/`) are linked in. Removed when dropped.
struct Staged {
  dir: std::path::PathBuf,
}

impl Staged {
  fn new(root: &Path) -> Result<Staged, String> {
    let dir = root.join("target/flow-staged");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    let staged = Staged { dir };
    let prefix = format!("{}/", staged.dir.display());
    super::git::git(
      root,
      &["checkout-index", "--all", "--force", "--prefix", &prefix],
    )?;
    let git_dir = super::git::git(root, &["rev-parse", "--absolute-git-dir"])?;
    std::fs::write(
      staged.dir.join(".git"),
      format!("gitdir: {}\n", git_dir.trim()),
    )
    .map_err(|e| format!("the staged tree's .git: {}", e))?;
    let records = store::root(root);
    if records.is_dir() {
      #[cfg(unix)]
      let linked = std::os::unix::fs::symlink(&records, store::root(&staged.dir));
      #[cfg(windows)]
      let linked = std::os::windows::fs::symlink_dir(&records, store::root(&staged.dir));
      linked.map_err(|e| format!("the staged tree's .dev: {}", e))?;
    }
    Ok(staged)
  }
}

impl Drop for Staged {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.dir);
  }
}

/// Whether `name` is a check a stage may list: one of [`CHECKS`] or a gate.
pub fn known(name: &str) -> bool {
  CHECKS.iter().any(|(n, _)| *n == name) || crate::gates::GATES.iter().any(|(n, _, _)| *n == name)
}

/// Runs `stage`'s checks in order, a line each; stops at the first that fails. With
/// `scope.staged`, the checks that read the whole tree read what is staged.
pub fn run_stage(root: &Path, flow: &Flow, names: &[String], scope: &Scope) -> Result<(), String> {
  let snapshot = if scope.staged {
    Some(Staged::new(root)?)
  } else {
    None
  };
  let tree = snapshot.as_ref().map_or(root, |s| s.dir.as_path());
  let mut rows: Vec<(String, Result<(), String>, f64)> = Vec::new();
  let mut stopped = None;
  for name in names {
    let started = Instant::now();
    eprint!("{}  {} ", scope.stage, name);
    let outcome = run_check(root, tree, flow, name, scope);
    let took = started.elapsed().as_secs_f64();
    match &outcome {
      Ok(()) => eprintln!("ok ({:.1}s)", took),
      Err(e) => {
        eprintln!("failed ({:.1}s)", took);
        eprintln!("{}", indent(e));
        stopped = Some(name.clone());
      }
    }
    rows.push((name.clone(), outcome, took));
    if stopped.is_some() {
      break;
    }
  }
  // On GitHub Actions, the stage's checks as a table on the run's summary page.
  if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
    let text = summary(&scope.stage, names, &rows);
    let _ = std::fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(path)
      .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()));
  }
  match stopped {
    Some(name) => Err(format!("{} stopped at `{}`", scope.stage, name)),
    None => Ok(()),
  }
}

/// Each `uses:` in the workflows names its action by a full commit hash, which a moved
/// tag cannot change.
fn actions_pinned(root: &Path) -> Result<(), String> {
  let dir = root.join(".github/workflows");
  let mut found = Vec::new();
  for entry in std::fs::read_dir(&dir).map_err(|e| format!(".github/workflows: {}", e))? {
    let path = entry.map_err(|e| e.to_string())?.path();
    let Ok(text) = std::fs::read_to_string(&path) else {
      continue;
    };
    let name = path
      .file_name()
      .unwrap_or_default()
      .to_string_lossy()
      .into_owned();
    for (i, line) in text.lines().enumerate() {
      let Some(used) = line
        .trim_start()
        .trim_start_matches("- ")
        .strip_prefix("uses:")
      else {
        continue;
      };
      let used = used.split('#').next().unwrap_or("").trim();
      let pinned = used.starts_with("./")
        || used
          .rsplit_once('@')
          .is_some_and(|(_, at)| at.len() == 40 && at.chars().all(|c| c.is_ascii_hexdigit()));
      if !pinned {
        found.push(format!(
          "{}:{}: `{}` (pin it: `owner/action@<commit> # vN`)",
          name,
          i + 1,
          used
        ));
      }
    }
  }
  if found.is_empty() {
    Ok(())
  } else {
    Err(lines(&found))
  }
}

/// Every CI run for HEAD finished, and passed.
fn ci_green(root: &Path) -> Result<(), String> {
  let head = git::git(root, &["rev-parse", "HEAD"])?;
  let done = process::output(
    Command::new("gh")
      .args([
        "run",
        "list",
        "--commit",
        head.trim(),
        "--json",
        "name,status,conclusion,databaseId",
        "--jq",
        ".[] | [.name, .status, .conclusion, .databaseId] | @tsv",
      ])
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  )
  .map_err(|e| format!("gh could not be run: {}", e))?;
  if done.code != 0 {
    return Err(format!(
      "gh run list failed: {}",
      String::from_utf8_lossy(&done.stderr).trim()
    ));
  }
  let text = String::from_utf8_lossy(&done.stdout).into_owned();
  let runs: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
  if runs.is_empty() {
    return Err("no CI run for HEAD yet".to_string());
  }
  let failing: Vec<String> = runs
    .iter()
    .filter(|r| r.get(1) != Some(&"completed") || r.get(2) != Some(&"success"))
    .map(|r| {
      format!(
        "{} ({}, {}): run {}",
        r[0],
        r.get(1).unwrap_or(&""),
        r.get(2).unwrap_or(&""),
        r.get(3).unwrap_or(&"")
      )
    })
    .collect();
  if failing.is_empty() {
    Ok(())
  } else {
    Err(lines(&failing))
  }
}

/// Nothing uncommitted, untracked files included: a release is the tree as committed.
fn clean(root: &Path) -> Result<(), String> {
  let status = git::git(root, &["status", "--porcelain", "--untracked-files=normal"])?;
  if status.trim().is_empty() {
    Ok(())
  } else {
    Err(format!("commit or put aside first:\n{}", status.trim_end()))
  }
}

/// A command that passes or fails; its output shown when it fails.
fn command(root: &Path, program: &str, args: &[&str]) -> Result<(), String> {
  let done = process::output(
    Command::new(program)
      .args(args)
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::HEAVY,
  )
  .map_err(|e| format!("{} could not be run: {}", program, e))?;
  if done.code == 0 {
    return Ok(());
  }
  let stdout = String::from_utf8_lossy(&done.stdout);
  let stderr = String::from_utf8_lossy(&done.stderr);
  Err(format!(
    "`{} {}` failed:\n{}",
    program,
    args.join(" "),
    failure_text(&stdout, &stderr)
  ))
}

/// `compat`: the compatibility record holds for this tree; `cargo xtask diff` runs to
/// show it unless a proof already does.
fn compat(root: &Path) -> Result<(), String> {
  if compat_record(root).is_ok() {
    println!("  (proven: `cargo xtask diff` passed on this tree)");
    return Ok(());
  }
  command(root, "cargo", &["xtask", "diff"])
}

/// `cargo xtask diff` passed on exactly this tree: the corpus is at or above the record.
fn compat_record(root: &Path) -> Result<(), String> {
  use crate::common::proof;
  // A proof for what the comparison reads, as it is now: a commit that changed none of
  // it (docs, the changelog) keeps the proof.
  let hash =
    proof::inputs(root, &proof::COMPAT).ok_or("cannot list the files the comparison reads")?;
  if proof::holds(root, &proof::COMPAT, &hash).is_some() {
    return Ok(());
  }
  Err(format!(
    "no passing `cargo xtask diff` for what it reads now (inputs {}): run it first",
    &hash[..12]
  ))
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date (what follows it ignored).
fn days(date: &str) -> Option<i64> {
  let mut parts = date.get(..10)?.split('-');
  let (y, m, d): (i64, i64, i64) = (
    parts.next()?.parse().ok()?,
    parts.next()?.parse().ok()?,
    parts.next()?.parse().ok()?,
  );
  if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
    return None;
  }
  // The civil-from-days count, from March so that leap days fall last.
  let y = if m <= 2 { y - 1 } else { y };
  let era = y.div_euclid(400);
  let yoe = y - era * 400;
  let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
  let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  Some(era * 146097 + doe - 719468)
}

/// What a failed command said that matters: each failing test's report (cargo test
/// prints them on stdout, from `failures:` to `test result: FAILED`), then the end of
/// stderr, where a build error or the failed target is named. Without the first, a build
/// with many warnings on stderr pushes the failures out of sight.
fn failure_text(stdout: &str, stderr: &str) -> String {
  let mut out: Vec<&str> = Vec::new();
  let mut inside = false;
  for line in stdout.lines() {
    if line == "failures:" && !inside {
      inside = true;
    }
    if inside {
      out.push(line);
    }
    if line.starts_with("test result: FAILED") {
      inside = false;
    }
  }
  out.truncate(300);
  if out.is_empty() {
    // Not a test run: the end of what it printed, where failures are summed up.
    out.extend(
      stdout
        .lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev(),
    );
  }
  out.extend(
    stderr
      .lines()
      .rev()
      .take(20)
      .collect::<Vec<_>>()
      .into_iter()
      .rev(),
  );
  out.join("\n")
}

fn gates(root: &Path) -> Result<(), String> {
  let problems: Vec<String> = crate::gates::GATES
    .iter()
    .flat_map(|(_, _, check)| check(root))
    .collect();
  if problems.is_empty() {
    Ok(())
  } else {
    Err(problems.join("\n"))
  }
}

/// The repository's settings on GitHub are those MAINTAINING.md lists: public, `main`
/// and release tags guarded by rulesets, the crates-io environment behind approval,
/// workflows read-only by default, private vulnerability reporting on.
fn github_settings(root: &Path, flow: &Flow) -> Result<(), String> {
  let repo = &flow.release.repository;
  let ask = |path: &str, jq: &str| -> Result<String, String> {
    let done = process::output(
      Command::new("gh")
        .args(["api", path, "--jq", jq])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()),
      process::QUICK,
    )
    .map_err(|e| format!("gh could not be run: {}", e))?;
    if done.code == 0 {
      Ok(String::from_utf8_lossy(&done.stdout).trim().to_string())
    } else {
      Err(format!(
        "gh api {}: {}",
        path,
        String::from_utf8_lossy(&done.stderr).trim()
      ))
    }
  };
  let mut found = Vec::new();
  if ask(&format!("repos/{}", repo), ".visibility")? != "public" {
    found.push("the repository is not public".to_string());
  }
  // Each active ruleset's target, the refs it covers and its rule types.
  let ids = ask(
    &format!("repos/{}/rulesets", repo),
    ".[] | select(.enforcement == \"active\") | .id",
  )?;
  let mut branch = false;
  let mut tags = false;
  for id in ids.lines() {
    let line = ask(
      &format!("repos/{}/rulesets/{}", repo, id),
      "[.target, (.conditions.ref_name.include | join(\",\")), ([.rules[].type] | join(\",\"))] | @tsv",
    )?;
    let parts: Vec<&str> = line.split('\t').collect();
    let (target, refs, rules) = (
      parts.first().copied().unwrap_or(""),
      parts.get(1).copied().unwrap_or(""),
      parts.get(2).copied().unwrap_or(""),
    );
    let has = |rule: &str| rules.split(',').any(|r| r == rule);
    if target == "branch"
      && (refs.contains("refs/heads/main") || refs.contains("~DEFAULT_BRANCH"))
      && has("non_fast_forward")
      && has("deletion")
      && has("required_status_checks")
    {
      branch = true;
    }
    if target == "tag" && refs.contains("refs/tags/v") && has("update") && has("deletion") {
      tags = true;
    }
  }
  if !branch {
    found.push(
      "no active ruleset on main requiring the CI checks and refusing force pushes and deletion"
        .to_string(),
    );
  }
  if !tags {
    found.push("no active ruleset on tags v* refusing updates and deletion".to_string());
  }
  let environments = ask(
    &format!("repos/{}/environments", repo),
    ".environments[] | [.name, ([.protection_rules[].type] | join(\",\"))] | @tsv",
  )
  .unwrap_or_default();
  // Publishing waits for approval.
  let guarded = environments.lines().any(|l| {
    l.split_once('\t')
      .is_some_and(|(n, rules)| n == "crates-io" && rules.contains("required_reviewers"))
  });
  if !guarded {
    found.push("environment `crates-io` does not require approval".to_string());
  }

  if ask(
    &format!("repos/{}/actions/permissions/workflow", repo),
    ".default_workflow_permissions",
  )? != "read"
  {
    found.push("workflows are not read-only by default".to_string());
  }
  let reporting = ask(
    &format!("repos/{}/private-vulnerability-reporting", repo),
    ".enabled",
  )
  .unwrap_or_default();
  if reporting != "true" {
    found.push("private vulnerability reporting is off".to_string());
  }
  if found.is_empty() {
    Ok(())
  } else {
    Err(format!(
      "make these as MAINTAINING.md says:\n{}",
      lines(&found)
    ))
  }
}

/// No commit ever made holds what looks like a credential or a working note: what going
/// public shows is the whole history, not only the tree.
fn history_clean(root: &Path, flow: &Flow) -> Result<(), String> {
  let mut found = Vec::new();
  let notes = git::git(
    root,
    &[
      "log",
      "--all",
      "--name-only",
      "--format=",
      "--",
      "docs/internal",
    ],
  )?;
  for path in notes.lines().filter(|l| !l.is_empty()) {
    found.push(format!("{} was committed (docs/internal/ never is)", path));
  }
  let commits = git::git(root, &["rev-list", "--all"])?;
  let mut args: Vec<String> = vec!["grep".into(), "-l".into(), "-I".into(), "-E".into()];
  for pattern in &flow.secrets.patterns {
    args.push("-e".into());
    args.push(pattern.clone());
  }
  args.extend(commits.lines().map(String::from));
  let args: Vec<&str> = args.iter().map(String::as_str).collect();
  let hits = git::git(root, &args).unwrap_or_default();
  for hit in hits.lines() {
    // `commit:path`.
    let path = hit.split_once(':').map_or(hit, |(_, p)| p);
    if !flow
      .secrets
      .allow
      .iter()
      .any(|g| super::glob::matches(g, path))
    {
      found.push(format!("{}: looks like a credential", hit));
    }
  }
  found.sort();
  found.dedup();
  if found.is_empty() {
    Ok(())
  } else {
    Err(lines(&found))
  }
}

/// A held ref (`main`, release tags) is pushed only as `flow release` left it.
fn hold(root: &Path, flow: &Flow, scope: &Scope) -> Result<(), String> {
  let marked = super::release_mark(root);
  let mut found = Vec::new();
  for (remote_ref, sha) in &scope.pushed {
    let held = flow
      .release
      .hold
      .iter()
      .any(|h| super::glob::matches(h, remote_ref));
    if held && !marked.iter().any(|m| m == sha) {
      found.push(format!(
        "`{}` is held: cut a release with `cargo xtask flow release`",
        remote_ref.trim_start_matches("refs/heads/")
      ));
    }
  }
  if found.is_empty() {
    Ok(())
  } else {
    Err(lines(&found))
  }
}

fn indent(text: &str) -> String {
  text
    .lines()
    .map(|l| format!("  {}", l))
    .collect::<Vec<_>>()
    .join("\n")
}

fn lines<T: AsRef<str>>(items: &[T]) -> String {
  items
    .iter()
    .map(|i| format!("- {}", i.as_ref()))
    .collect::<Vec<_>>()
    .join("\n")
}

/// `mutants`: cargo-mutants on [`MUTATED`] against the library's tests. A mutant the tests
/// miss is listed (in `mutants.out/missed.txt` too), not failed: the run is a report. A
/// build that fails, or a run that cannot finish, fails it.
fn mutants(root: &Path) -> Result<(), String> {
  let mut args: Vec<String> = vec!["mutants".into(), "--package".into(), "mtindo".into()];
  for file in MUTATED {
    args.extend(["--file".to_string(), file.to_string()]);
  }
  args.extend(["--".to_string(), "--lib".to_string()]);
  let done = process::output(
    Command::new("cargo")
      .args(&args)
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::HEAVY,
  )
  .map_err(|e| format!("cargo mutants could not be run: {}", e))?;
  let missed = std::fs::read_to_string(root.join("mutants.out/missed.txt")).unwrap_or_default();
  match done.code {
    // 0: every mutant caught; 2: some missed; 3: some timed out, which counts as caught.
    0 | 2 | 3 => {
      let missed: Vec<&str> = missed.lines().filter(|l| !l.trim().is_empty()).collect();
      println!("  {} mutant(s) the tests miss", missed.len());
      for line in missed.iter().take(50) {
        println!("    {}", line);
      }
      Ok(())
    }
    _ => Err(failure_text(
      &String::from_utf8_lossy(&done.stdout),
      &String::from_utf8_lossy(&done.stderr),
    )),
  }
}

/// The packages the uncommitted changes (against HEAD, untracked files included) fall in,
/// each with the `cargo test` arguments for its unit tests.
fn near(changed: &[String]) -> Vec<(&'static str, &'static [&'static str])> {
  const PACKAGES: &[(&str, &[&str], &[&str])] = &[
    ("xtask/core/", &[], &["test", "--package", "xtask-core"]),
    ("xtask/tools/", &[], &["test", "--package", "xtask-tools"]),
    ("xtask/", &[], &["test", "--package", "xtask"]),
    (
      "",
      &["src/", "tests/", "build.rs", "Cargo.toml", "Cargo.lock"],
      &["test", "--package", "mtindo", "--lib", "--bins"],
    ),
  ];
  let mut out: Vec<(&'static str, &'static [&'static str])> = Vec::new();
  for file in changed {
    let hit = PACKAGES.iter().find(|(prefix, also, _)| {
      (!prefix.is_empty() && file.starts_with(prefix)) || also.iter().any(|a| file.starts_with(a))
    });
    if let Some((prefix, _, args)) = hit
      && !out.iter().any(|(p, _)| p == prefix)
    {
      out.push((prefix, args));
    }
  }
  out
}

/// `docs/internal/` holds working notes that never enter the history.
fn no_internal(root: &Path, scope: &Scope) -> Result<(), String> {
  let files = match &scope.files {
    Some(files) => files.clone(),
    None => git::tree_files(root, None)?.into_iter().collect(),
  };
  let found: Vec<&String> = files
    .iter()
    .filter(|f| f.starts_with("docs/internal/"))
    .collect();
  if found.is_empty() {
    return Ok(());
  }
  Err(format!(
    "docs/internal/ is never committed; unstage:\n{}",
    lines(&found)
  ))
}

/// `text` without terminal escape sequences (colors, character sets), for a page.
fn plain(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let mut chars = text.chars().peekable();
  while let Some(c) = chars.next() {
    if c != '\x1b' {
      out.push(c);
      continue;
    }
    match chars.next() {
      // CSI: parameters, then one final letter.
      Some('[') => {
        for c in chars.by_ref() {
          if c.is_ascii_alphabetic() {
            break;
          }
        }
      }
      // A character set: one more character.
      Some('(') | Some(')') => {
        chars.next();
      }
      _ => {}
    }
  }
  out
}

/// HEAD is on its remote branch, and the branch a release lands on fast-forwards to it
/// (a `release/X.Y` branch is its own).
fn pushed(root: &Path, flow: &Flow) -> Result<(), String> {
  let head = git::git(root, &["rev-parse", "HEAD"])?;
  let upstream = git::git(root, &["rev-parse", "@{upstream}"])
    .map_err(|_| "the branch has no remote branch: push it first".to_string())?;
  if head.trim() != upstream.trim() {
    return Err("HEAD is not what the remote branch holds: push it first".to_string());
  }
  let branch = git::git(root, &["symbolic-ref", "--short", "HEAD"]).unwrap_or_default();
  if branch.trim().starts_with("release/") {
    return Ok(());
  }
  let target = format!("origin/{}", flow.release.branch);
  git::git(root, &["merge-base", "--is-ancestor", &target, "HEAD"])
    .map(|_| ())
    .map_err(|_| {
      format!(
        "{} does not fast-forward to HEAD: bring it in first",
        target
      )
    })
}

/// One check, at `root`; those that read the tree read `tree` (what is staged, for
/// `pre-commit`).
fn run_check(
  root: &Path,
  tree: &Path,
  flow: &Flow,
  name: &str,
  scope: &Scope,
) -> Result<(), String> {
  match name {
    "no-internal" => no_internal(root, scope),
    "secrets" => secrets(root, flow, scope),
    "fmt" => command(tree, "cargo", &["fmt", "--all", "--check"]),
    "gates" => gates(tree),
    "actions-pinned" => actions_pinned(root),
    "messages" => super::messages_in(root, flow, &scope.ranges),
    "hold" => hold(root, flow, scope),
    "clippy-lib" => command(
      root,
      "cargo",
      &[
        "clippy",
        "--package",
        "mtindo",
        "--all-targets",
        "--",
        "-D",
        "warnings",
      ],
    ),
    "tests-xtask" => command(
      root,
      "cargo",
      &[
        "test",
        "--package",
        "xtask",
        "--package",
        "xtask-core",
        "--package",
        "xtask-tools",
      ],
    ),
    "deny" => command(root, "cargo", &["deny", "--workspace", "check"]),
    "mutants" => mutants(root),
    "github-settings" => github_settings(root, flow),
    "tests-lib" => command(
      root,
      "cargo",
      &["test", "--package", "mtindo", "--no-fail-fast"],
    ),
    "tests-near" => tests_near(root),
    "suite" => suite(root),
    "compat" => compat(root),
    "tools-pinned" => tools_pinned(root, flow),
    "msrv" => match flow.tool("msrv") {
      Some(msrv) => command(
        root,
        "cargo",
        &[&format!("+{}", msrv.version), "test", "--package", "mtindo"],
      ),
      None => Err("flow.toml has no [tools.msrv]".to_string()),
    },
    "clean" => clean(root),
    "pushed" => pushed(root, flow),
    "ci-green" => ci_green(root),
    "compat-record" => compat_record(root),
    "version-agrees" => version_agrees(root),
    "tokens" => tokens(flow),
    // The crate as crates.io gets it, built from its own files.
    "package" => command(root, "cargo", &["package", "--locked"]),
    "history-clean" => history_clean(root, flow),
    "commands-doc" => {
      let written =
        std::fs::read_to_string(tree.join("docs/generated/commands.md")).unwrap_or_default();
      if written == crate::cli::reference() {
        Ok(())
      } else {
        Err("docs/generated/commands.md is out of date: run `cargo xtask docs`".to_string())
      }
    }
    gate if crate::gates::GATES.iter().any(|(n, _, _)| *n == gate) => {
      let problems: Vec<String> = crate::gates::GATES
        .iter()
        .filter(|(n, _, _)| *n == gate)
        .flat_map(|(_, _, check)| check(tree))
        .collect();
      if problems.is_empty() {
        Ok(())
      } else {
        Err(problems.join("\n"))
      }
    }
    other => Err(format!("no check `{}` (flow.toml [stages])", other)),
  }
}

/// Text that looks like a credential, in what is staged (or the whole tree).
fn secrets(root: &Path, flow: &Flow, scope: &Scope) -> Result<(), String> {
  let mut args: Vec<String> = vec!["grep".into(), "-n".into(), "-I".into(), "-E".into()];
  if scope.staged {
    args.push("--cached".into());
  }
  for pattern in &flow.secrets.patterns {
    args.push("-e".into());
    args.push(pattern.clone());
  }
  let allowed = |path: &str| {
    flow
      .secrets
      .allow
      .iter()
      .any(|g| super::glob::matches(g, path))
  };
  if let Some(files) = &scope.files {
    let files: Vec<&String> = files.iter().filter(|f| !allowed(f)).collect();
    if files.is_empty() {
      return Ok(());
    }
    args.push("--".into());
    args.extend(files.into_iter().cloned());
  }
  let args: Vec<&str> = args.iter().map(String::as_str).collect();
  // `git grep` exits 1 when nothing matches.
  let text = git::git(root, &args).unwrap_or_default();
  let found: Vec<&str> = text
    .lines()
    .filter(|l| l.split(':').next().is_some_and(|p| !allowed(p)))
    .collect();
  if found.is_empty() {
    return Ok(());
  }
  // The line's place only: the match itself is not printed again.
  let places: Vec<String> = found
    .iter()
    .map(|l| l.splitn(3, ':').take(2).collect::<Vec<_>>().join(":"))
    .collect();
  Err(format!(
    "what looks like a credential (flow.toml [secrets]); remove it, rotate it if it was real:\n{}",
    lines(&places)
  ))
}

/// `suite`: `cargo xtask test`, unless its last green run was on exactly this tree.
fn suite(root: &Path) -> Result<(), String> {
  let here = crate::common::tree::identity(root);
  if crate::test::last_green(root).is_some_and(|(_, tree)| tree == here) {
    println!("  (proven: the last green `cargo xtask test` ran on this tree)");
    return Ok(());
  }
  command(root, "cargo", &["xtask", "test"])
}

/// A stage's checks as Markdown for a job's summary: each with its result and time, those
/// not reached after a failure marked so, and the failure's report under the table.
fn summary(stage: &str, names: &[String], rows: &[(String, Result<(), String>, f64)]) -> String {
  let failed = rows.iter().any(|(_, outcome, _)| outcome.is_err());
  let mut out = format!(
    "### `{}` {}\n\n| check | result | time |\n|---|---|---|\n",
    stage,
    if failed { "failed" } else { "passed" }
  );
  for name in names {
    match rows.iter().find(|(n, _, _)| n == name) {
      Some((_, Ok(()), took)) => out.push_str(&format!("| {} | ok | {:.1}s |\n", name, took)),
      Some((_, Err(_), took)) => {
        out.push_str(&format!("| {} | **failed** | {:.1}s |\n", name, took))
      }
      None => out.push_str(&format!("| {} | not run | |\n", name)),
    }
  }
  for (name, outcome, _) in rows {
    if let Err(e) = outcome {
      out.push_str(&format!(
        "\n<details open><summary>{}</summary>\n\n```text\n{}\n```\n</details>\n",
        name,
        plain(e.trim_end())
      ));
    }
  }
  out.push('\n');
  out
}

/// `tests-near`: the unit tests of the packages the uncommitted changes touch.
fn tests_near(root: &Path) -> Result<(), String> {
  let mut changed: Vec<String> = git::git(root, &["diff", "--name-only", "HEAD"])?
    .lines()
    .map(str::to_string)
    .collect();
  changed.extend(
    git::git(root, &["ls-files", "--others", "--exclude-standard"])?
      .lines()
      .map(str::to_string),
  );
  let packages = near(&changed);
  if packages.is_empty() {
    println!("  (no package changed since HEAD)");
  }
  for (_, args) in packages {
    command(root, "cargo", args)?;
  }
  Ok(())
}

/// Each expiring credential CI uses has a recorded expiry, more than 14 days off (warned
/// within them, refused once passed).
fn tokens(flow: &Flow) -> Result<(), String> {
  let today = days(&crate::common::log::timestamp(std::time::SystemTime::now()));
  let mut found = Vec::new();
  for (name, written) in &flow.release.tokens {
    match (today, days(written)) {
      (_, None) => found.push(format!(
        "{}: no expiry in flow.toml [release.tokens]: make the token (MAINTAINING.md) and write its expiry",
        name
      )),
      (Some(today), Some(expires)) if expires <= today => {
        found.push(format!("{} expired on {}: rotate it (MAINTAINING.md)", name, written))
      }
      (Some(today), Some(expires)) if expires - today <= 14 => eprintln!(
        "  warning: {} expires on {}, in {} days: rotate it soon (MAINTAINING.md)",
        name,
        written,
        expires - today
      ),
      _ => {}
    }
  }
  if found.is_empty() {
    Ok(())
  } else {
    Err(lines(&found))
  }
}

/// The pins agree with the files that name the same versions: rust-toolchain.toml's
/// channel and components with `[tools.stable]`, Cargo.toml's rust-version with
/// `[tools.msrv]`; and every `needed_by` names a stage or a command.
fn tools_pinned(root: &Path, flow: &Flow) -> Result<(), String> {
  let mut found = Vec::new();
  let read = |file: &str| {
    std::fs::read_to_string(root.join(file))
      .map_err(|e| format!("{}: {}", file, e))
      .and_then(|t| crate::toml::parse(&t).map_err(|e| format!("{}: {}", file, e)))
  };
  let toolchain = read("rust-toolchain.toml")?;
  let toolchain = toolchain
    .get("toolchain")
    .and_then(|t| t.as_table())
    .ok_or("rust-toolchain.toml has no [toolchain]")?;
  match flow.tool("stable") {
    Some(stable) => {
      let channel = toolchain
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("");
      if channel != stable.version {
        found.push(format!(
          "rust-toolchain.toml's channel is {}, [tools.stable] pins {}",
          channel, stable.version
        ));
      }
      let components: Vec<&str> = toolchain
        .get("components")
        .and_then(|v| v.as_array())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
      if components
        != stable
          .components
          .iter()
          .map(String::as_str)
          .collect::<Vec<_>>()
      {
        found.push(format!(
          "rust-toolchain.toml's components are {:?}, [tools.stable] lists {:?}",
          components, stable.components
        ));
      }
    }
    None => found.push("flow.toml has no [tools.stable]".to_string()),
  }
  let manifest = read("Cargo.toml")?;
  let msrv = manifest
    .get("package")
    .and_then(|p| p.as_table())
    .and_then(|p| p.get("rust-version"))
    .and_then(|v| v.as_str())
    .unwrap_or("");
  match flow.tool("msrv") {
    Some(pin) if pin.version == msrv => {}
    Some(pin) => found.push(format!(
      "Cargo.toml's rust-version is {}, [tools.msrv] pins {}",
      msrv, pin.version
    )),
    None => found.push("flow.toml has no [tools.msrv]".to_string()),
  }
  for tool in &flow.tools {
    for user in &tool.needed_by {
      if flow.stage(user).is_none() && !super::tools::COMMANDS.contains(&user.as_str()) {
        found.push(format!(
          "[tools.{}] is needed by `{}`, neither a stage nor a command",
          tool.name, user
        ));
      }
    }
  }
  if found.is_empty() {
    Ok(())
  } else {
    Err(lines(&found))
  }
}

/// The release tag at HEAD, Cargo.toml's version and a CHANGELOG.md section agree.
fn version_agrees(root: &Path) -> Result<(), String> {
  let tags = git::git(root, &["tag", "--points-at", "HEAD", "v[0-9]*"])?;
  let tag = tags.lines().next().ok_or("no release tag at HEAD")?.trim();
  let manifest = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
  let version = crate::toml::parse(&manifest)
    .ok()
    .and_then(|t| {
      t.get("package")?
        .as_table()?
        .get("version")?
        .as_str()
        .map(String::from)
    })
    .ok_or("Cargo.toml has no [package] version")?;
  let mut found = Vec::new();
  if tag.trim_start_matches('v') != version {
    found.push(format!(
      "the tag is {}, Cargo.toml's version {}",
      tag, version
    ));
  }
  let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap_or_default();
  if !changelog.contains(&format!("## [{}]", version)) {
    found.push(format!("CHANGELOG.md has no ## [{}] section", version));
  }
  if found.is_empty() {
    Ok(())
  } else {
    Err(lines(&found))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_failed_test_run_shows_the_failure_before_the_warnings() {
    let stdout = "running 2 tests\ntest a ... ok\ntest b ... FAILED\n\nfailures:\n\n---- b stdout ----\nthread 'b' panicked: left != right\n\nfailures:\n    b\n\ntest result: FAILED. 1 passed; 1 failed\n";
    let stderr = format!(
      "{}error: test failed, to rerun pass `--test x`\n",
      "warning: unused\n".repeat(400)
    );
    let shown = failure_text(stdout, &stderr);
    assert!(shown.starts_with("failures:"), "{}", shown);
    assert!(shown.contains("panicked: left != right"));
    assert!(
      shown.ends_with("error: test failed, to rerun pass `--test x`"),
      "{}",
      shown
    );
    assert!(shown.lines().count() < 40, "{}", shown);
  }

  #[test]
  fn test_a_stage_summary_names_every_check_and_the_failure() {
    let names: Vec<String> = ["fmt", "gates", "tests"]
      .iter()
      .map(|s| s.to_string())
      .collect();
    let rows = vec![
      ("fmt".to_string(), Ok(()), 0.7),
      (
        "gates".to_string(),
        Err("naming: 2 problems".to_string()),
        0.6,
      ),
    ];
    let text = summary("ci", &names, &rows);
    assert!(text.starts_with("### `ci` failed\n"), "{}", text);
    assert!(text.contains("| fmt | ok | 0.7s |"), "{}", text);
    assert!(text.contains("| gates | **failed** | 0.6s |"), "{}", text);
    assert!(text.contains("| tests | not run | |"), "{}", text);
    assert!(text.contains("<summary>gates</summary>") && text.contains("naming: 2 problems"));
    assert_eq!(plain("\x1b[31m-a\x1b(B\x1b[m b"), "-a b");
    let passed = summary("ci", &names[..1], &rows[..1]);
    assert!(passed.starts_with("### `ci` passed\n"), "{}", passed);
  }

  #[test]
  fn test_dates_count_days() {
    assert_eq!(days("1970-01-01"), Some(0));
    assert_eq!(days("2000-03-01"), Some(11017));
    assert_eq!(
      days("2027-01-28")
        .zip(days("2026-12-29"))
        .map(|(a, b)| a - b),
      Some(30)
    );
    assert_eq!(
      days("2024-03-01")
        .zip(days("2024-02-28"))
        .map(|(a, b)| a - b),
      Some(2)
    );
    assert_eq!(days(""), None);
    assert_eq!(days("2026-13-01"), None);
  }

  /// The staged tree holds what is staged, not what the work tree has since, and git run
  /// in it reads the same repository.
  #[test]
  fn test_the_staged_tree_is_what_is_staged() {
    let root = std::env::temp_dir().join(format!("xtask-staged-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let git = |args: &[&str]| super::super::git::git(&root, args).unwrap();
    git(&["init", "--quiet"]);
    std::fs::write(root.join("a.md"), "staged\n").unwrap();
    git(&["add", "a.md"]);
    std::fs::write(root.join("a.md"), "changed since\n").unwrap();
    std::fs::write(root.join("b.md"), "never staged\n").unwrap();
    let staged = Staged::new(&root).unwrap();
    assert_eq!(
      std::fs::read_to_string(staged.dir.join("a.md")).unwrap(),
      "staged\n"
    );
    assert!(!staged.dir.join("b.md").exists());
    let listed = super::super::git::git(&staged.dir, &["ls-files"]).unwrap();
    assert_eq!(listed.trim(), "a.md");
    let dir = staged.dir.clone();
    drop(staged);
    assert!(!dir.exists());
    let _ = std::fs::remove_dir_all(&root);
  }
}
