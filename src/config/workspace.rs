//! Cargo packages and their targets, read from the manifests as `cargo fmt` finds them, with
//! no call to Cargo: mto stays std-only and works offline.
//!
//! - **Packages:** the workspace's members (`workspace.members`, with `*` and `?` in path
//!   parts, minus `workspace.exclude`, plus the root package), or the lone package.
//! - **Which ones** ([`Selection`]): by default, every member when run from the workspace
//!   root, else the package whose manifest is the one given or found; `-p NAME` picks members
//!   by name; `--all` takes every member and, recursively, local path dependencies outside
//!   the workspace.
//! - **Targets**, as Cargo discovers them: the library (`[lib] path` or `src/lib.rs`), binaries
//!   (`src/main.rs`, `src/bin/*.rs`, `src/bin/*/main.rs`, `[[bin]]`), examples, tests and
//!   benches (their folder's `*.rs` and `*/main.rs`, and `[[example]]`, `[[test]]`,
//!   `[[bench]]`), and the build script (`package.build` or `build.rs`). `autobins = false` and
//!   its kin turn discovery off; `build = false` drops the build script.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::toml::{self, Table, Value};

/// Which packages to take, as `cargo fmt` chooses them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    /// The package of the manifest, or every member from the workspace root.
    Root,
    /// Members by name (`-p`).
    Packages(Vec<String>),
    /// Every member, and local path dependencies outside the workspace (`--all`).
    All,
}

/// A package: its name, manifest folder and parsed manifest.
struct Package {
    name: String,
    dir: PathBuf,
    manifest: Table,
}

/// A target's root file and its kind (`lib`, `bin`, `example`, `test`, `bench`,
/// `custom-build`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    pub path: PathBuf,
    pub kind: &'static str,
}

/// The targets to lay out, given the manifest (`--manifest-path`, else the nearest
/// `Cargo.toml` above `start`).
pub fn targets(
    manifest: Option<&Path>,
    start: &Path,
    selection: &Selection,
) -> Result<Vec<Target>, String> {
    let manifest = match manifest {
        Some(path) => path.to_path_buf(),
        None => start
            .ancestors()
            .map(|dir| dir.join("Cargo.toml"))
            .find(|path| path.is_file())
            .ok_or_else(|| {
                format!(
                    "could not find Cargo.toml in {} or any parent folder",
                    start.display()
                )
            })?,
    };
    let manifest = fs::canonicalize(&manifest)
        .map_err(|e| format!("cannot read {}: {}", manifest.display(), e))?;
    let dir = manifest.parent().unwrap_or(Path::new("")).to_path_buf();
    let (root, members) = workspace(&dir)?;
    let chosen: Vec<&Package> = match selection {
        Selection::Root => {
            if members.len() == 1 || dir == root {
                members.iter().collect()
            } else {
                members.iter().filter(|p| p.dir == dir).collect()
            }
        }
        Selection::Packages(names) => {
            let mut chosen = Vec::new();
            for name in names {
                let package = members.iter().find(|p| &p.name == name).ok_or_else(|| {
                    format!("package `{}` is not a member of the workspace", name)
                })?;
                chosen.push(package);
            }
            chosen
        }
        Selection::All => members.iter().collect(),
    };
    let mut found = BTreeSet::new();
    for package in &chosen {
        found.extend(package_targets(package));
    }
    if *selection == Selection::All {
        let mut seen: BTreeSet<PathBuf> = members.iter().map(|p| p.dir.clone()).collect();
        let mut queue: Vec<PathBuf> = members.iter().flat_map(path_dependencies).collect();
        while let Some(dep) = queue.pop() {
            let Ok(dep) = fs::canonicalize(&dep) else {
                continue;
            };
            if !seen.insert(dep.clone()) {
                continue;
            }
            if let Some(package) = read_package(&dep)? {
                found.extend(package_targets(&package));
                queue.extend(path_dependencies(&package));
            }
        }
    }
    if found.is_empty() {
        return Err("found no targets to lay out".to_string());
    }
    Ok(found.into_iter().collect())
}

/// The folders a member pattern names; `*` and `?` match within one path part.
fn expand(root: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    for part in pattern.split('/').filter(|p| !p.is_empty() && *p != ".") {
        let mut next = Vec::new();
        for dir in &dirs {
            if part.contains(['*', '?']) {
                let mut matched: Vec<PathBuf> = fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter(|e| wildcard(part, &e.file_name().to_string_lossy()))
                    .map(|e| e.path())
                    .collect();
                matched.sort();
                next.extend(matched);
            } else if part == ".." {
                next.push(dir.parent().unwrap_or(dir).to_path_buf());
            } else {
                next.push(dir.join(part));
            }
        }
        dirs = next;
    }
    dirs.into_iter()
        .filter_map(|d| fs::canonicalize(&d).ok())
        .collect()
}

/// The member folders `workspace.members` names, minus `workspace.exclude`.
fn members(root: &Path, workspace: &Table) -> Result<Vec<PathBuf>, String> {
    let strings = |key: &str| -> Vec<String> {
        workspace
            .get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut excluded = BTreeSet::new();
    for pattern in strings("exclude") {
        excluded.extend(expand(root, &pattern));
    }
    let mut found = Vec::new();
    for pattern in strings("members") {
        for dir in expand(root, &pattern) {
            if dir.join("Cargo.toml").is_file() && !excluded.contains(&dir) && !found.contains(&dir)
            {
                found.push(dir);
            }
        }
    }
    Ok(found)
}

/// The package's targets, as Cargo discovers them.
fn package_targets(package: &Package) -> Vec<Target> {
    let dir = &package.dir;
    let manifest = &package.manifest;
    let section = |key: &str| manifest.get("package").and_then(Value::as_table)?.get(key);
    let auto = |key: &str| section(key).and_then(Value::as_bool).unwrap_or(true);
    let mut targets = Vec::new();
    let mut add = |path: PathBuf, kind: &'static str| {
        if path.is_file() && !targets.iter().any(|t: &Target| t.path == path) {
            targets.push(Target { path, kind });
        }
    };

    // The library.
    let lib = manifest.get("lib").and_then(Value::as_table);
    match lib.and_then(|l| l.get("path")).and_then(Value::as_str) {
        Some(path) => add(dir.join(path), "lib"),
        None => {
            if auto("autolib") {
                add(dir.join("src/lib.rs"), "lib");
            }
        }
    }

    // Binaries, examples, tests and benches: explicit entries, then discovered files.
    let kinds: [(&str, &str, &'static str, &str); 4] = [
        ("bin", "src/bin", "bin", "autobins"),
        ("example", "examples", "example", "autoexamples"),
        ("test", "tests", "test", "autotests"),
        ("bench", "benches", "bench", "autobenches"),
    ];
    for (key, folder, kind, discover) in kinds {
        for entry in manifest
            .get(key)
            .and_then(Value::as_array)
            .unwrap_or_default()
        {
            let Some(table) = entry.as_table() else {
                continue;
            };
            match table.get("path").and_then(Value::as_str) {
                Some(path) => add(dir.join(path), kind),
                None => {
                    if let Some(name) = table.get("name").and_then(Value::as_str) {
                        add(dir.join(folder).join(format!("{}.rs", name)), kind);
                        add(dir.join(folder).join(name).join("main.rs"), kind);
                        if kind == "bin" && name == package.name {
                            add(dir.join("src/main.rs"), kind);
                        }
                    }
                }
            }
        }
        if !auto(discover) {
            continue;
        }
        if kind == "bin" {
            add(dir.join("src/main.rs"), kind);
        }
        let mut found: Vec<PathBuf> = Vec::new();
        for entry in fs::read_dir(dir.join(folder))
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                found.push(path.join("main.rs"));
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
        found.sort();
        for path in found {
            add(path, kind);
        }
    }

    // The build script.
    match section("build") {
        Some(Value::String(path)) => add(dir.join(path), "custom-build"),
        Some(Value::Boolean(false)) => {}
        _ => add(dir.join("build.rs"), "custom-build"),
    }
    targets
}

/// The folders of the package's `path = "..."` dependencies, in every dependency table.
fn path_dependencies(package: &Package) -> Vec<PathBuf> {
    let mut tables: Vec<&Table> = Vec::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        tables.extend(package.manifest.get(key).and_then(Value::as_table));
    }
    if let Some(targets) = package.manifest.get("target").and_then(Value::as_table) {
        for entry in targets.entries() {
            for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
                tables.extend(
                    entry
                        .value
                        .as_table()
                        .and_then(|t| t.get(key))
                        .and_then(Value::as_table),
                );
            }
        }
    }
    tables
        .into_iter()
        .flat_map(|t| t.entries())
        .filter_map(|e| e.value.as_table()?.get("path")?.as_str())
        .map(|p| package.dir.join(p))
        .collect()
}

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

/// The package whose manifest is in `dir`, if it has a `[package]`.
fn read_package(dir: &Path) -> Result<Option<Package>, String> {
    let Some(manifest) = read(&dir.join("Cargo.toml"))? else {
        return Ok(None);
    };
    let Some(name) = manifest
        .get("package")
        .and_then(Value::as_table)
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(None);
    };
    Ok(Some(Package {
        name,
        dir: dir.to_path_buf(),
        manifest,
    }))
}

/// Whether `name` matches `pattern`, where `*` is any run of characters and `?` is one.
fn wildcard(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}

/// The workspace root and its member packages, for the package or workspace in `dir`.
fn workspace(dir: &Path) -> Result<(PathBuf, Vec<Package>), String> {
    // The nearest manifest with `[workspace]`, this one included, that lists `dir`.
    for root in dir.ancestors() {
        let Some(manifest) = read(&root.join("Cargo.toml"))? else {
            continue;
        };
        let Some(table) = manifest.get("workspace").and_then(Value::as_table) else {
            continue;
        };
        let members = members(root, table)?;
        let mut packages = Vec::new();
        if manifest.get("package").is_some() {
            packages.extend(read_package(root)?);
        }
        for member in members {
            if member != root {
                packages.extend(read_package(&member)?);
            }
        }
        if root == dir || packages.iter().any(|p| p.dir == dir) {
            return Ok((root.to_path_buf(), packages));
        }
    }
    let package = read_package(dir)?
        .ok_or_else(|| format!("{} has no [package]", dir.join("Cargo.toml").display()))?;
    Ok((dir.to_path_buf(), vec![package]))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(files: &[(&str, &str)]) -> Dir {
            let root = std::env::temp_dir().join(format!(
                "mtindo-workspace-{}-{}",
                std::process::id(),
                files.len()
            ));
            let _ = fs::remove_dir_all(&root);
            for (path, text) in files {
                let path = root.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            }
            Dir(fs::canonicalize(root).unwrap())
        }

        fn names(&self, targets: &[Target]) -> Vec<String> {
            targets
                .iter()
                .map(|t| {
                    format!(
                        "{} {}",
                        t.kind,
                        t.path.strip_prefix(&self.0).unwrap().display()
                    )
                })
                .collect()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_targets_of_a_workspace() {
        let dir = Dir::new(&[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"crates/skip\"]\n",
            ),
            (
                "crates/a/Cargo.toml",
                "[package]\nname = \"a\"\nbuild = false\n[[bin]]\nname = \"tool\"\npath = \"tools/tool.rs\"\n[dependencies]\nhelper = { path = \"../../helper\" }\n",
            ),
            ("crates/a/src/lib.rs", ""),
            ("crates/a/src/main.rs", ""),
            ("crates/a/src/bin/extra.rs", ""),
            ("crates/a/src/bin/multi/main.rs", ""),
            ("crates/a/tools/tool.rs", ""),
            ("crates/a/build.rs", ""),
            ("crates/a/examples/demo.rs", ""),
            ("crates/a/tests/it.rs", ""),
            (
                "crates/b/Cargo.toml",
                "[package]\nname = \"b\"\nautobins = false\n",
            ),
            ("crates/b/src/main.rs", ""),
            ("crates/b/build.rs", ""),
            ("crates/skip/Cargo.toml", "[package]\nname = \"skip\"\n"),
            ("crates/skip/src/lib.rs", ""),
            ("helper/Cargo.toml", "[package]\nname = \"helper\"\n"),
            ("helper/src/lib.rs", ""),
        ]);
        let all = targets(None, &dir.0, &Selection::Root).unwrap();
        assert_eq!(
            dir.names(&all),
            // In path order; `skip` is excluded, `a`'s build script is off, `b` has no
            // discovered binaries.
            [
                "example crates/a/examples/demo.rs",
                "bin crates/a/src/bin/extra.rs",
                "bin crates/a/src/bin/multi/main.rs",
                "lib crates/a/src/lib.rs",
                "bin crates/a/src/main.rs",
                "test crates/a/tests/it.rs",
                "bin crates/a/tools/tool.rs",
                "custom-build crates/b/build.rs",
            ]
        );
        // From inside a member, only that member; -p by name; --all adds path dependencies.
        let inside = targets(None, &dir.0.join("crates/b/src"), &Selection::Root).unwrap();
        assert_eq!(dir.names(&inside), ["custom-build crates/b/build.rs"]);
        let picked = targets(None, &dir.0, &Selection::Packages(vec!["b".into()])).unwrap();
        assert_eq!(picked, inside);
        assert!(
            targets(None, &dir.0, &Selection::Packages(vec!["nope".into()]))
                .unwrap_err()
                .contains("`nope` is not a member")
        );
        let everything = targets(None, &dir.0, &Selection::All).unwrap();
        assert!(
            dir.names(&everything)
                .contains(&"lib helper/src/lib.rs".to_string())
        );
    }

    #[test]
    fn test_wildcards() {
        assert!(wildcard("*", "anything"));
        assert!(wildcard("a?c*", "abcdef"));
        assert!(!wildcard("a?c", "ac"));
        assert!(wildcard("*-sys", "foo-sys"));
        assert!(!wildcard("*-sys", "foo-sy"));
    }
}
