//! The edition of the package a folder belongs to, read from `Cargo.toml` as `cargo fmt`
//! reads it: the nearest manifest with a `[package]`, whose `edition` may be inherited from
//! the workspace (`edition.workspace = true`). A package with no edition is 2015.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::toml::{self, Table, Value};

/// Where an edition was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Edition {
    pub edition: String,
    /// The manifest that sets it (the workspace's, when inherited).
    pub path: PathBuf,
    pub line: usize,
}

/// The edition for files in `dir`; `None` outside a package. An error is a manifest that
/// cannot be read; the caller warns and carries on without an edition.
pub(super) fn edition_for(dir: &Path) -> Result<Option<Edition>, String> {
    for folder in dir.ancestors() {
        let path = folder.join("Cargo.toml");
        let Some(manifest) = read(&path)? else {
            continue;
        };
        let Some(package) = manifest.entry("package") else {
            continue;
        };
        let Some(table) = package.value.as_table() else {
            continue;
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
                    workspace_edition(folder)?.ok_or_else(|| {
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
        return Ok(Some(found));
    }
    Ok(None)
}

/// The parsed manifest at `path`, or `None` if there is none.
fn read(path: &Path) -> Result<Option<Table>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    toml::parse(&text)
        .map(Some)
        .map_err(|e| format!("{}:{}:{}: {}", path.display(), e.line, e.column, e.message))
}

/// `workspace.package.edition` of the nearest workspace manifest above `package_dir`.
fn workspace_edition(package_dir: &Path) -> Result<Option<Edition>, String> {
    for folder in package_dir.ancestors() {
        let path = folder.join("Cargo.toml");
        let Some(manifest) = read(&path)? else {
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
