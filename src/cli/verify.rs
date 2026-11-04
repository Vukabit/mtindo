//! `mto verify REV`: whether a commit (a layout commit above all, the kind a project lists
//! in `.git-blame-ignore-revs`) is mto's work and nothing else. Each Rust file it changed is
//! taken from the commit's parent and run through mto: the commit's text is reproduced, the
//! checks passing on the way, or at least the change is one layout or formatting may make.
//! What mto never does (create, delete, rename a file, change its mode or a link) is not
//! safe; what nothing can be proved about (a file not Rust, not UTF-8, a submodule) could
//! not be verified.
//!
//! The settings are the ones the commit holds (its mto.toml, rustfmt.toml and Cargo.toml
//! files, a link among them followed inside the commit), not the working tree's, which may
//! have changed since, and none from outside the repository. git is asked a fixed number of
//! times whatever the commit's size, every file's contents in one `git cat-file --batch`;
//! without git, nothing can be verified.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::{Committed, Resolver, is_format_settings_file, is_settings_file};
use crate::settle::{Verdict, verdict};

/// A file's mode in git: a link.
const LINK: &str = "120000";

/// A file's mode in git: a submodule.
const SUBMODULE: &str = "160000";

/// What was found for one file.
pub enum Found {
  Verdict(Verdict),
  /// An mto.toml or the standard formatter's file: a layout commit may change them.
  Settings,
  /// Nothing can be proved about it either way: why.
  Unverified(String),
  /// Something mto never does: what.
  Never(String),
}

/// One file `git diff-tree --raw` lists.
struct Change {
  old_mode: String,
  new_mode: String,
  old_blob: String,
  new_blob: String,
  /// `A`, `D`, `M`, `R`, `T`.
  status: char,
  path: String,
  /// For a rename, the path it had.
  from: Option<String>,
}

/// One file a commit changed, and what was found.
pub struct Checked {
  pub path: String,
  pub found: Found,
}

/// A file a check runs on.
struct Task {
  at: usize,
  before: String,
  after: String,
  settings: Option<crate::Settings>,
}

/// A commit, checked.
pub struct Verified {
  /// The commit's full id.
  pub commit: String,
  /// It has more than one parent: it was compared with the first.
  pub merge: bool,
  pub files: Vec<Checked>,
  /// What reading its settings warned of, its files named as `COMMIT:PATH`.
  pub warnings: Vec<String>,
}

/// The commits `git blame` is told to look past: the file `blame.ignoreRevsFile` names (from
/// the repository's top), else `.git-blame-ignore-revs` there; one per line, `#` starting
/// a comment. The file as named for messages, and the commits it lists.
pub fn ignored_revs(here: &Path) -> Result<(String, Vec<String>), String> {
  let root = PathBuf::from(
    git(here, &["rev-parse", "--show-toplevel"])
      .map_err(|_| "not in a git work tree: verifying a commit needs one".to_string())?
      .trim(),
  );
  let named = git(&root, &["config", "--get", "blame.ignoreRevsFile"])
    .ok()
    .map(|name| name.trim().to_string())
    .filter(|name| !name.is_empty())
    .unwrap_or_else(|| ".git-blame-ignore-revs".to_string());
  let text = std::fs::read_to_string(root.join(&named)).map_err(|e| {
    format!(
      "cannot read {} ({}); it lists the commits `git blame` looks past",
      named, e
    )
  })?;
  let revs = text
    .lines()
    .map(|line| {
      line
        .split('#')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
    })
    .filter(|line| !line.is_empty())
    .collect();
  Ok((named, revs))
}

/// The commit `rev` names, checked file by file; an error saying why it cannot be.
pub fn verify(rev: &str, resolver: &mut Resolver, here: &Path) -> Result<Verified, String> {
  let root = PathBuf::from(
    git(here, &["rev-parse", "--show-toplevel"])
      .map_err(|_| "not in a git work tree: verifying a commit needs one".to_string())?
      .trim(),
  );
  let commit = commit_named(&root, rev)?;
  let short = commit[..10.min(commit.len())].to_string();
  let listed = git(&root, &["rev-list", "--parents", "-n", "1", &commit])?;
  let parents: Vec<String> = listed
    .split_whitespace()
    .skip(1)
    .map(str::to_string)
    .collect();
  let Some(parent) = parents.first() else {
    let shallow =
      git(&root, &["rev-parse", "--is-shallow-repository"]).is_ok_and(|said| said.trim() == "true");
    return Err(if shallow {
      format!(
        "{}'s parent is not in this clone, which is shallow; `git fetch --deepen=1` fetches \
         it",
        rev
      )
    } else {
      format!("{} has no parent to compare it with", rev)
    });
  };
  let changes = changes(&root, parent, &commit)?;
  // The settings as the commit holds them: each file, and the file a link among them
  // names inside the commit.
  let tree = tree(&root, &commit)?;
  let mut settings_blobs = Vec::new();
  for (path, (mode, blob)) in &tree {
    let name = path.rsplit('/').next().unwrap_or(path);
    if is_settings_file(name) {
      settings_blobs.push((path.clone(), mode.clone(), blob.clone()));
    }
  }
  let mut wanted: Vec<String> = settings_blobs.iter().map(|(_, _, b)| b.clone()).collect();
  for change in &changes {
    if change.path.ends_with(".rs") && change.status == 'M' {
      wanted.push(change.old_blob.clone());
      wanted.push(change.new_blob.clone());
    }
  }
  let mut contents = blobs(&root, &wanted)?;
  let mut warnings = Vec::new();
  let mut settings_files = Vec::new();
  for (path, mode, blob) in settings_blobs {
    let blob = if mode == LINK {
      let target = contents
        .get(&blob)
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
      match linked(&path, &target).and_then(|to| tree.get(&to)) {
        Some((_, to_blob)) => {
          if !contents.contains_key(to_blob) {
            contents.extend(blobs(&root, std::slice::from_ref(to_blob))?);
          }
          to_blob.clone()
        }
        None => {
          warnings.push(format!(
            "{}:{} links to {}, which the commit does not hold; it is not read",
            short, path, target
          ));
          continue;
        }
      }
    } else {
      blob
    };
    match contents.get(&blob).map(|b| String::from_utf8(b.clone())) {
      Some(Ok(text)) => settings_files.push((path, text)),
      _ => warnings.push(format!(
        "{}:{} is not UTF-8 text; it is not read",
        short, path
      )),
    }
  }
  resolver.read_committed(Committed::new(&root, settings_files));
  // Errors and warnings name the commit's files, not the working tree's.
  let real_root = crate::real_path(&root).unwrap_or_else(|_| root.clone());
  let named = |text: String| -> String {
    let mut text = text;
    for base in [&real_root, &root] {
      for separator in ["/", "\\"] {
        text = text.replace(
          &format!("{}{}", base.display(), separator),
          &format!("{}:", short),
        );
      }
    }
    text
  };
  let mut config_of = |path: &str| -> Result<Option<crate::Settings>, String> {
    let folder = root
      .join(path)
      .parent()
      .map(Path::to_path_buf)
      .unwrap_or(root.clone());
    let config = resolver
      .config_for(&folder)
      .map_err(|e| named(format!("{}: the settings {} holds: {}", path, short, e)))?;
    Ok(crate::Settings::of(&config))
  };
  let mut files = Vec::new();
  let mut tasks = Vec::new();
  for change in changes {
    let name = change.path.rsplit('/').next().unwrap_or(&change.path);
    let link = change.old_mode == LINK || change.new_mode == LINK;
    let found = if change.old_mode == SUBMODULE || change.new_mode == SUBMODULE {
      Found::Unverified("a submodule".into())
    } else if is_format_settings_file(name) {
      // Read now: settings that cannot be read stop the check.
      if change.status != 'D' {
        config_of(&change.path)?;
      }
      Found::Settings
    } else if !change.path.ends_with(".rs") {
      Found::Unverified("not a Rust file".into())
    } else {
      match change.status {
        'A' => Found::Never("added: mto never creates a file".into()),
        'D' => Found::Never("deleted: mto never deletes a file".into()),
        'R' => Found::Never(format!(
          "renamed from {}: mto never renames a file",
          change.from.as_deref().unwrap_or("another path")
        )),
        _ if link => Found::Never("a symbolic link changed: mto never changes a link".into()),
        _ if change.old_mode != change.new_mode => Found::Never(format!(
          "its mode changed from {} to {}: mto never changes a file's mode",
          change.old_mode, change.new_mode
        )),
        'M' => {
          let text = |blob: &str| {
            contents
              .get(blob)
              .and_then(|bytes| String::from_utf8(bytes.clone()).ok())
          };
          match (text(&change.old_blob), text(&change.new_blob)) {
            (Some(before), Some(after)) => {
              tasks.push(Task {
                at: files.len(),
                before,
                after,
                settings: config_of(&change.path)?,
              });
              // Filled in by the checks below.
              Found::Unverified(String::new())
            }
            _ => Found::Unverified("not UTF-8 text".into()),
          }
        }
        other => Found::Unverified(format!("git lists it as changed ({})", other)),
      }
    };
    files.push(Checked {
      path: change.path,
      found,
    });
  }
  warnings.extend(resolver.take_warnings().into_iter().map(named));
  for (at, verdict) in checked(tasks) {
    files[at].found = Found::Verdict(verdict);
  }
  Ok(Verified {
    commit,
    merge: parents.len() > 1,
    files,
    warnings,
  })
}

/// The contents of each of `ids`, in one `git cat-file --batch`.
fn blobs(root: &Path, ids: &[String]) -> Result<HashMap<String, Vec<u8>>, String> {
  let mut wanted: Vec<&String> = ids.iter().collect();
  wanted.sort();
  wanted.dedup();
  let mut found = HashMap::new();
  if wanted.is_empty() {
    return Ok(found);
  }
  let mut child = Command::new("git")
    .arg("-C")
    .arg(root)
    .args(["cat-file", "--batch"])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| format!("git cannot be run ({}); verifying a commit needs it", e))?;
  let mut stdin = child.stdin.take().ok_or("git cat-file takes no input")?;
  let asked: String = wanted.iter().map(|id| format!("{}\n", id)).collect();
  // Written from a thread of its own: git answers while it reads, and both pipes fill.
  let writer = std::thread::spawn(move || stdin.write_all(asked.as_bytes()));
  let mut out = Vec::new();
  child
    .stdout
    .take()
    .ok_or("git cat-file gives no output")?
    .read_to_end(&mut out)
    .map_err(|e| format!("git cat-file: {}", e))?;
  let _ = writer.join();
  let _ = child.wait();
  let mut at = 0;
  for id in wanted {
    let Some(end) = out[at..].iter().position(|b| *b == b'\n') else {
      break;
    };
    let head = String::from_utf8_lossy(&out[at..at + end]).to_string();
    at += end + 1;
    // `ID TYPE SIZE`, then the contents and a line break; `ID missing` for none.
    let Some(size) = head.split(' ').nth(2).and_then(|s| s.parse::<usize>().ok()) else {
      continue;
    };
    let body = out.get(at..at + size).unwrap_or_default().to_vec();
    at += size + 1;
    found.insert(id.clone(), body);
  }
  Ok(found)
}

/// The files the commit changed from `parent`, with their modes and contents' ids, renames
/// found.
fn changes(root: &Path, parent: &str, commit: &str) -> Result<Vec<Change>, String> {
  let listed = git(
    root,
    &[
      "diff-tree",
      "-r",
      "-z",
      "--raw",
      "-M",
      "--no-commit-id",
      parent,
      commit,
    ],
  )?;
  let mut fields = listed.split('\0');
  let mut changes = Vec::new();
  while let Some(head) = fields.next() {
    let Some(head) = head.strip_prefix(':') else {
      continue;
    };
    let parts: Vec<&str> = head.split(' ').collect();
    let [old_mode, new_mode, old_blob, new_blob, status] = parts[..] else {
      continue;
    };
    let status = status.chars().next().unwrap_or('M');
    let first = fields.next().unwrap_or_default().to_string();
    let (path, from) = if matches!(status, 'R' | 'C') {
      (fields.next().unwrap_or_default().to_string(), Some(first))
    } else {
      (first, None)
    };
    changes.push(Change {
      old_mode: old_mode.to_string(),
      new_mode: new_mode.to_string(),
      old_blob: old_blob.to_string(),
      new_blob: new_blob.to_string(),
      status,
      path,
      from,
    });
  }
  Ok(changes)
}

/// Each task's verdict, the tasks spread over the machine's cores.
fn checked(tasks: Vec<Task>) -> Vec<(usize, Verdict)> {
  let next = AtomicUsize::new(0);
  let found = Mutex::new(Vec::with_capacity(tasks.len()));
  let workers = std::thread::available_parallelism()
    .map(|n| n.get())
    .unwrap_or(1)
    .min(tasks.len());
  std::thread::scope(|scope| {
    for _ in 0..workers {
      let worker = || {
        while let Some(task) = tasks.get(next.fetch_add(1, Ordering::Relaxed)) {
          let verdict = match &task.settings {
            Some(settings) => verdict(&task.before, &task.after, settings),
            None => Verdict::LeftAlone("layout and formatting are both off here".into()),
          };
          found
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((task.at, verdict));
        }
      };
      let _ = std::thread::Builder::new()
        .stack_size(crate::STACK)
        .spawn_scoped(scope, worker);
    }
  });
  found.into_inner().unwrap_or_else(|e| e.into_inner())
}

/// The full id of the commit `rev` names; an error in words for one that names none, or
/// names both a branch and a tag.
fn commit_named(root: &Path, rev: &str) -> Result<String, String> {
  let output = run(
    root,
    &[
      "rev-parse",
      "--verify",
      "--end-of-options",
      &format!("{}^{{commit}}", rev),
    ],
  )?;
  let said = String::from_utf8_lossy(&output.stderr);
  if !output.status.success() {
    // An ambiguous short id: git lists the candidates.
    if said.contains("ambiguous") {
      return Err(format!("`{}` is ambiguous: {}", rev, said.trim()));
    }
    return Err(format!("no commit is named `{}` here", rev));
  }
  if said.contains("ambiguous") {
    return Err(format!(
      "`{}` names both a branch and a tag; say which: refs/heads/{} or refs/tags/{}",
      rev, rev, rev
    ));
  }
  Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// What `git ARGS` prints in `dir`, or why it failed; paths that are not UTF-8 are read
/// with the bytes that are not replaced.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
  let output = run(dir, args)?;
  if !output.status.success() {
    return Err(format!(
      "git {}: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr).trim()
    ));
  }
  Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// The path inside the repository a link at `path` to `target` names; `None` for one
/// outside it.
fn linked(path: &str, target: &str) -> Option<String> {
  if target.starts_with('/') {
    return None;
  }
  let mut parts: Vec<&str> = path.split('/').collect();
  parts.pop();
  for part in target.split('/') {
    match part {
      "" | "." => {}
      ".." => {
        parts.pop()?;
      }
      part => parts.push(part),
    }
  }
  Some(parts.join("/"))
}

/// `git ARGS` run in `dir`.
fn run(dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
  Command::new("git")
    .args(args)
    .current_dir(dir)
    .output()
    .map_err(|e| format!("git cannot be run ({}); verifying a commit needs it", e))
}

/// Every file the commit holds: its path, with its mode and contents' id.
fn tree(root: &Path, commit: &str) -> Result<HashMap<String, (String, String)>, String> {
  let listed = git(root, &["ls-tree", "-r", "-z", "--full-tree", commit])?;
  Ok(
    listed
      .split('\0')
      .filter_map(|entry| {
        let (head, path) = entry.split_once('\t')?;
        let mut parts = head.split(' ');
        let mode = parts.next()?.to_string();
        let blob = parts.nth(1)?.to_string();
        Some((path.to_string(), (mode, blob)))
      })
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A link among the settings is followed inside the commit, never out of it.
  #[test]
  fn test_a_link_names_a_path_inside_the_commit() {
    assert_eq!(linked("mto.toml", "cfg.toml").as_deref(), Some("cfg.toml"));
    assert_eq!(
      linked("a/b/rustfmt.toml", "../../shared/rustfmt.toml").as_deref(),
      Some("shared/rustfmt.toml")
    );
    assert_eq!(
      linked("a/mto.toml", "./x/../y.toml").as_deref(),
      Some("a/y.toml")
    );
    assert_eq!(linked("mto.toml", "../outside.toml"), None);
    assert_eq!(linked("mto.toml", "/etc/mto.toml"), None);
  }
}
