//! The checks the stages run, each by the name `flow.toml`'s `[stages]` gives it.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use super::config::Flow;
use super::git;
use crate::common::process;

/// Every check name a stage may list.
pub const NAMES: &[&str] = &[
  "no-internal",
  "secrets",
  "fmt",
  "gates",
  "actions-pinned",
  "messages",
  "hold",
  "tools-pinned",
  "clippy",
  "tests",
  "msrv",
  "clean",
  "pushed",
  "ci-green",
  "compat-record",
  "version-agrees",
  "tokens",
  "package",
  "history-clean",
  "commands-doc",
  "tests-xtask",
  "github-settings",
  "deny",
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
    let records = root.join(".dev");
    if records.is_dir() {
      #[cfg(unix)]
      let linked = std::os::unix::fs::symlink(&records, staged.dir.join(".dev"));
      #[cfg(windows)]
      let linked = std::os::windows::fs::symlink_dir(&records, staged.dir.join(".dev"));
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

/// Runs `stage`'s checks in order, a line each; stops at the first that fails. With
/// `scope.staged`, the checks that read the whole tree read what is staged.
pub fn run_stage(root: &Path, flow: &Flow, names: &[String], scope: &Scope) -> Result<(), String> {
  let snapshot = if scope.staged {
    Some(Staged::new(root)?)
  } else {
    None
  };
  let tree = snapshot.as_ref().map_or(root, |s| s.dir.as_path());
  for name in names {
    let started = Instant::now();
    eprint!("{}  {} ", scope.stage, name);
    let outcome = run_check(root, tree, flow, name, scope);
    let took = started.elapsed().as_secs_f64();
    match outcome {
      Ok(()) => eprintln!("ok ({:.1}s)", took),
      Err(e) => {
        eprintln!("failed ({:.1}s)", took);
        eprintln!("{}", indent(&e));
        return Err(format!("{} stopped at `{}`", scope.stage, name));
      }
    }
  }
  Ok(())
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
  let mut text = String::from_utf8_lossy(&done.stdout).into_owned();
  text.push_str(&String::from_utf8_lossy(&done.stderr));
  // The end, where the failures are summed up.
  let tail: Vec<&str> = text.lines().rev().take(40).collect();
  let tail: Vec<&str> = tail.into_iter().rev().collect();
  Err(format!(
    "`{} {}` failed:\n{}",
    program,
    args.join(" "),
    tail.join("\n")
  ))
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
    "clippy" => command(
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
    "tests-xtask" => command(root, "cargo", &["test", "--package", "xtask"]),
    "deny" => command(root, "cargo", &["deny", "--workspace", "check"]),
    "github-settings" => github_settings(root, flow),
    "tests" => command(
      root,
      "cargo",
      &["test", "--package", "mtindo", "--no-fail-fast"],
    ),
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
