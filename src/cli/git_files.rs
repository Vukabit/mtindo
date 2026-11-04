//! The files a run takes as git sees them: `--changed`, `--staged`, `--since REV` and
//! `--tracked`. The run finds its files as always (ignored folders, settings, modules), and
//! only those git names go on. git is asked once per repository; without git, or for a path
//! in no repository, nothing is run, rather than every file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::history::git::{on_disk, repository_of, status};

/// Whether a file goes on.
#[derive(Debug, PartialEq, Eq)]
pub enum Keep {
  Yes,
  No,
  /// Staged, with changes not staged as well: left alone, so the commit gets what was
  /// staged.
  Unstaged,
}

/// What one repository answered.
#[derive(Debug, Default)]
struct Answer {
  /// The files the run takes, absolute.
  picked: HashSet<PathBuf>,
  /// `--staged`: files with changes not staged as well, left alone.
  unstaged: HashSet<PathBuf>,
  /// `--tracked`: the files git tracks, as git spells them.
  tracked: HashSet<PathBuf>,
  /// The same, as the file system names them: resolved only when a file is not found under
  /// git's spelling, as there may be many.
  tracked_on_disk: OnceLock<HashSet<PathBuf>>,
}

/// What the run asks git for.
#[derive(Clone, Debug, Default)]
pub struct Ask {
  /// `--changed`: files with changes not committed, untracked ones too.
  pub changed: bool,
  /// `--staged`: files staged for the next commit.
  pub staged: bool,
  /// `--since REV`: files changed since where HEAD left REV (their merge base).
  pub since: Option<String>,
  /// `--tracked`: a folder walk takes only the files git tracks.
  pub tracked: bool,
}

impl Ask {
  /// Whether any of the flags was given.
  pub fn any(&self) -> bool {
    self.changed || self.staged || self.since.is_some() || self.tracked
  }

  /// Whether the run is narrowed to what changed (not only to what git tracks).
  fn narrows(&self) -> bool {
    self.changed || self.staged || self.since.is_some()
  }

  /// What a run that found nothing says.
  pub fn nothing(&self) -> &'static str {
    match self {
      Ask { staged: true, .. } => "nothing to do: no Rust file is staged, as git sees it",
      Ask { since: Some(_), .. } => {
        "nothing to do: no Rust file changed since then, as git sees it"
      }
      Ask { changed: true, .. } => "nothing to do: no Rust file changed, as git sees it",
      _ => "nothing to do: git tracks no Rust file here",
    }
  }
}

/// git's answers for the repositories a run's paths are in.
#[derive(Debug, Default)]
pub struct Selection {
  ask: Ask,
  answers: HashMap<PathBuf, Answer>,
  roots: HashMap<PathBuf, Option<PathBuf>>,
}

impl Selection {
  /// Asks git about the repository of each of `paths` (absolute, symlinks resolved); an
  /// error when one is in no repository or git cannot answer.
  pub fn ask(ask: Ask, paths: &[PathBuf]) -> Result<Selection, String> {
    let mut selection = Selection {
      ask,
      ..Selection::default()
    };
    for path in paths {
      let folder = if path.is_dir() {
        path.as_path()
      } else {
        path.parent().unwrap_or(Path::new("."))
      };
      let Some(root) = repository_of(folder, &mut selection.roots) else {
        return Err(format!(
          "{} is not in a git repository; {} asks git which files to take",
          path.display(),
          selection.flag()
        ));
      };
      if !selection.answers.contains_key(&root) {
        let answer = answer(&selection.ask, &root)?;
        selection.answers.insert(root, answer);
      }
    }
    Ok(selection)
  }

  /// Whether the file at `real` (absolute, symlinks resolved) goes on; `walked` when a
  /// folder walk found it rather than the command line naming it, or a file named bringing
  /// it as a module.
  pub fn keeps(&self, real: &Path, walked: bool) -> Keep {
    // The innermost repository holding it.
    let answer = self
      .answers
      .iter()
      .filter(|(root, _)| real.starts_with(root))
      .max_by_key(|(root, _)| root.as_os_str().len())
      .map(|(_, answer)| answer);
    let Some(answer) = answer else {
      return Keep::No;
    };
    if self.ask.tracked
      && walked
      && !answer.tracked.contains(real)
      && !answer
        .tracked_on_disk
        .get_or_init(|| answer.tracked.iter().cloned().map(on_disk).collect())
        .contains(real)
    {
      return Keep::No;
    }
    if !self.ask.narrows() {
      return Keep::Yes;
    }
    if answer.unstaged.contains(real) {
      Keep::Unstaged
    } else if answer.picked.contains(real) {
      Keep::Yes
    } else {
      Keep::No
    }
  }

  /// The flag that asked, for messages.
  fn flag(&self) -> &'static str {
    match &self.ask {
      Ask { staged: true, .. } => "--staged",
      Ask { since: Some(_), .. } => "--since",
      Ask { changed: true, .. } => "--changed",
      _ => "--tracked",
    }
  }
}

/// The folder a run with a git flag and no path takes: the repository `here` is in, named
/// from `here` (`.`, `..`, `../..`); `None` outside any repository.
pub fn repository_folder(here: &Path) -> Option<PathBuf> {
  let real = crate::real_path(here).ok()?;
  let root = repository_of(&real, &mut HashMap::new())?;
  let up = real.strip_prefix(&root).ok()?.components().count();
  Some(match up {
    0 => PathBuf::from("."),
    _ => (0..up).map(|_| "..").collect(),
  })
}

/// What the repository at `root` answers for `ask`.
fn answer(ask: &Ask, root: &Path) -> Result<Answer, String> {
  // As git spells them; `named` resolves each as the file system names it.
  let listed = |args: &[&str]| -> Result<HashSet<PathBuf>, String> {
    Ok(
      git(root, args)?
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(|p| root.join(p))
        .collect(),
    )
  };
  let named = |args: &[&str]| -> Result<HashSet<PathBuf>, String> {
    Ok(listed(args)?.into_iter().map(on_disk).collect())
  };
  let mut answer = Answer::default();
  let base = match &ask.since {
    Some(rev) => {
      let base =
        git(root, &["merge-base", rev, "HEAD"]).map_err(|e| format!("--since {}: {}", rev, e))?;
      Some(base.trim().to_string())
    }
    None => None,
  };
  if ask.staged {
    let mut args = vec![
      "diff",
      "--cached",
      "--name-only",
      "-z",
      "--diff-filter=ACMR",
    ];
    if let Some(base) = &base {
      args.push(base);
    }
    answer.picked = named(&args)?;
    answer.unstaged = named(&["diff", "--name-only", "-z"])?
      .intersection(&answer.picked)
      .cloned()
      .collect();
  } else if let Some(base) = &base {
    answer.picked = named(&["diff", "--name-only", "-z", "--diff-filter=ACMR", base])?;
    answer.picked.extend(named(&[
      "ls-files",
      "--others",
      "--exclude-standard",
      "-z",
    ])?);
  } else if ask.changed {
    answer.picked = status(root)
      .map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "git is not installed".to_string(),
        _ => format!("git status in {}: {}", root.display(), e),
      })?
      .into_iter()
      .collect();
  }
  if ask.tracked {
    answer.tracked = listed(&["ls-files", "-z"])?;
  }
  Ok(answer)
}

/// What `git ARGS` prints in `dir`, or why it failed.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
  let output = Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(args)
    .output()
    .map_err(|e| match e.kind() {
      std::io::ErrorKind::NotFound => "git is not installed".to_string(),
      _ => format!("git cannot be run: {}", e),
    })?;
  if !output.status.success() {
    return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
  }
  Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
