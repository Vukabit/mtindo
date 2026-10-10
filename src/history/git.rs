//! The git guard: which files have changes git has not committed. Used only when history is
//! off, so that a run with no undo of its own never overwrites work git cannot give back.
//!
//! git is the one program mto may run, and only if it is installed. Repositories are found by
//! looking for `.git` upwards from each file; each is asked once, with
//! `git status --porcelain -z`, which lists modified, staged and untracked files.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What the guard found.
#[derive(Debug, Default)]
pub struct Dirty {
    /// Files with uncommitted changes, made absolute with symlinks resolved.
    pub files: HashSet<PathBuf>,
    /// Set when git could not be run: the guard was skipped, and this says why.
    pub skipped: Option<String>,
}

/// The files among `files` (absolute, symlinks resolved) that git sees uncommitted changes
/// in. Files outside any repository are never dirty.
pub fn dirty(files: &[PathBuf]) -> Dirty {
    let mut out = Dirty::default();
    let mut roots: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();
    let mut wanted: HashSet<PathBuf> = HashSet::new();
    for file in files {
        let Some(folder) = file.parent() else {
            continue;
        };
        if let Some(root) = repository_of(folder, &mut roots) {
            wanted.insert(root);
        }
    }
    let mut wanted: Vec<PathBuf> = wanted.into_iter().collect();
    wanted.sort();
    for root in wanted {
        match status(&root) {
            Ok(paths) => out.files.extend(paths),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                out.skipped = Some("git is not installed".into());
                return out;
            }
            Err(e) => {
                out.skipped = Some(format!("git status failed in {}: {}", root.display(), e));
                return out;
            }
        }
    }
    out
}

/// Reads `git status --porcelain=v1 -z`: `XY PATH`, each ended by NUL; a rename or copy is
/// followed by its old path, which is skipped.
fn parse_status(root: &Path, out: &[u8]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut fields = out.split(|b| *b == 0);
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        let path = String::from_utf8_lossy(&field[3..]).to_string();
        paths.push(root.join(path));
        if matches!(field[0], b'R' | b'C') {
            fields.next();
        }
    }
    paths
}

/// The repository `folder` is in, remembered per folder on the way up.
fn repository_of(folder: &Path, roots: &mut HashMap<PathBuf, Option<PathBuf>>) -> Option<PathBuf> {
    let mut seen = Vec::new();
    let mut current = Some(folder);
    let found = loop {
        let Some(here) = current else {
            break None;
        };
        if let Some(known) = roots.get(here) {
            break known.clone();
        }
        seen.push(here.to_path_buf());
        if here.join(".git").exists() {
            break Some(here.to_path_buf());
        }
        current = here.parent();
    };
    for folder in seen {
        roots.insert(folder, found.clone());
    }
    found
}

/// The files `git status` lists in the repository at `root`.
fn status(root: &Path) -> io::Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(parse_status(root, &output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_lists_changed_staged_untracked_and_renamed_files() {
        let out = b" M src/a.rs\0A  src/b.rs\0?? new.rs\0R  c.rs\0old.rs\0";
        let paths = parse_status(Path::new("/repo"), out);
        assert_eq!(
            paths,
            [
                PathBuf::from("/repo/src/a.rs"),
                PathBuf::from("/repo/src/b.rs"),
                PathBuf::from("/repo/new.rs"),
                PathBuf::from("/repo/c.rs"),
            ]
        );
    }
}
