//! The edition of the package a folder belongs to, read from `Cargo.toml` as `cargo fmt`
//! reads it: the nearest manifest with a `[package]` (the resolver walks up the folders), whose `edition` may be inherited from
//! the workspace (`edition.workspace = true`). A package with no edition is 2015.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::Committed;
use crate::config::toml::{self, Table, Value};

/// Where an edition was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Edition {
  pub edition: String,
  /// The manifest that sets it (the workspace's, when inherited).
  pub path: PathBuf,
  pub line: usize,
}

/// Every manifest read in a run, each read and parsed once: a workspace's root manifest is
/// asked for by each of its packages that inherits its edition.
#[derive(Debug, Default)]
pub(super) struct Manifests {
  read: HashMap<PathBuf, Result<Option<Rc<Table>>, String>>,
  /// The manifests a commit holds, read in place of the working tree's inside it.
  committed: Option<Rc<Committed>>,
}

impl Manifests {
  /// Manifests read from `committed` inside its tree.
  pub(super) fn committed(committed: Rc<Committed>) -> Manifests {
    Manifests {
      read: HashMap::new(),
      committed: Some(committed),
    }
  }

  /// The parsed manifest at `path`, or `None` if there is none.
  fn get(&mut self, path: &Path) -> Result<Option<Rc<Table>>, String> {
    if let Some(found) = self.read.get(path) {
      return found.clone();
    }
    let found = match &self.committed {
      Some(committed) if committed.holds(path) => {
        parse(path, committed.text(path)).map(|table| table.map(Rc::new))
      }
      Some(_) => Ok(None),
      None => read(path).map(|table| table.map(Rc::new)),
    };
    self.read.insert(path.to_path_buf(), found.clone());
    found
  }
}

/// The edition of the package whose manifest is `path` (in `folder`); `None` when there is
/// no manifest there or it has no `[package]`.
pub(super) fn manifest_edition(
  manifests: &mut Manifests,
  path: &Path,
  folder: &Path,
) -> Result<Option<Edition>, String> {
  let path = path.to_path_buf();
  let Some(manifest) = manifests.get(&path)? else {
    return Ok(None);
  };
  let Some(package) = manifest.entry("package") else {
    return Ok(None);
  };
  let Some(table) = package.value.as_table() else {
    return Ok(None);
  };
  let found = match table.entry("edition") {
    None => Edition {
      edition: "2015".to_string(),
      path,
      line: package.line,
    },
    Some(entry) => match &entry.value {
      Value::String(edition) => Edition {
        edition: edition.clone(),
        path,
        line: entry.line,
      },
      Value::Table(t) if t.get("workspace").and_then(Value::as_bool) == Some(true) => {
        workspace_edition(manifests, folder)?.ok_or_else(|| {
          format!(
            "{}: `edition.workspace = true`, but no workspace above it sets \
                             `workspace.package.edition`",
            path.display()
          )
        })?
      }
      other => {
        return Err(format!(
          "{}:{}: `package.edition` must be a string, not a {}",
          path.display(),
          entry.line,
          other.type_name()
        ));
      }
    },
  };
  Ok(Some(found))
}

/// The manifest `text` read from `path`, parsed; `None` when there is no text.
fn parse(path: &Path, text: Option<&str>) -> Result<Option<Table>, String> {
  let Some(text) = text else {
    return Ok(None);
  };
  toml::parse(text)
    .map(Some)
    .map_err(|e| format!("{}:{}:{}: {}", path.display(), e.line, e.column, e.message))
}

/// The parsed manifest at `path`, or `None` if there is none.
fn read(path: &Path) -> Result<Option<Table>, String> {
  if !path.is_file() {
    return Ok(None);
  }
  let text =
    fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
  parse(path, Some(&text))
}

/// `workspace.package.edition` of the nearest workspace manifest above `package_dir`.
fn workspace_edition(
  manifests: &mut Manifests,
  package_dir: &Path,
) -> Result<Option<Edition>, String> {
  for folder in package_dir.ancestors() {
    let path = folder.join("Cargo.toml");
    let Some(manifest) = manifests.get(&path)? else {
      continue;
    };
    if let Some(workspace) = manifest.get("workspace").and_then(Value::as_table) {
      let entry = workspace
        .get("package")
        .and_then(Value::as_table)
        .and_then(|p| p.entry("edition"));
      return Ok(entry.and_then(|e| {
        Some(Edition {
          edition: e.value.as_str()?.to_string(),
          path,
          line: e.line,
        })
      }));
    }
  }
  Ok(None)
}
