//! The files a crate root reaches through `mod name;`, found as the compiler and the standard
//! formatter find them, so that `mto src/lib.rs` covers the files formatting `src/lib.rs`
//! would.
//!
//! A file passed in owns its folder: `mod a;` there is `a.rs` or `a/mod.rs` beside it. A
//! module file `a.rs` owns `a/`, and `a/mod.rs` owns `a/` too. Inline modules add their name
//! to the folder (`mod a { mod b; }` finds `a/b.rs`). `#[path = "..."]` is relative to the
//! folder of the file it is written in (or of the inline module it sits in), and a file
//! reached that way owns its own folder. A module whose file cannot be found is reported and
//! skipped.
//!
//! As the standard formatter does, every `#[cfg_attr(..., path = "...")]` alternative is
//! followed, the items inside `cfg_if! { ... }` are looked into, and a module marked
//! `#[rustfmt::skip]` is not entered.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::layout::parse_any_edition;
use crate::syntax::ast::{Item, ItemKind};
use crate::syntax::{SyntaxKind, SyntaxNode};

/// Every file `root` reaches, `root` first, each once; and what could not be found.
pub fn module_files(root: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut problems = Vec::new();
    let mut seen = HashSet::new();
    let owned = root.parent().unwrap_or(Path::new("")).to_path_buf();
    let mut stack = vec![(root.to_path_buf(), owned)];
    while let Some((file, owned)) = stack.pop() {
        let key = fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
        if !seen.insert(key) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            problems.push(format!("cannot read {}", file.display()));
            continue;
        };
        files.push(file.clone());
        let (_, parsed) = parse_any_edition(&text);
        let mut found = Vec::new();
        let here = file.parent().unwrap_or(Path::new("")).to_path_buf();
        declared(&parsed.syntax_node(), &owned, &here, &mut found);
        // Pushed in reverse, so files come out in the order they are declared.
        for (name, candidates, child_owned) in found.into_iter().rev() {
            match candidates.iter().find(|c| c.is_file()) {
                Some(path) => {
                    let owned = child_owned.unwrap_or_else(|| owned_by(path));
                    stack.push((path.clone(), owned));
                }
                None => problems.push(format!(
                    "{}: no file for `mod {};` (looked for {})",
                    file.display(),
                    name,
                    candidates
                        .iter()
                        .map(|c| c.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
    }
    (files, problems)
}

/// The paths of `#[cfg_attr(condition, path = "...")]` attributes.
fn cfg_attr_paths(item: &Item) -> Vec<String> {
    item.attrs()
        .iter()
        .filter(|a| a.path() == "cfg_attr")
        .filter_map(|a| {
            let text = a.syntax().to_string();
            let mut rest = text.as_str();
            // `path = "..."`, as a word of its own.
            while let Some(at) = rest.find("path") {
                let before = rest[..at].chars().last();
                let after = rest[at + 4..].trim_start();
                rest = &rest[at + 4..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                if let Some(value) = after.strip_prefix('=') {
                    let value = value.trim_start().strip_prefix('"')?;
                    return Some(value[..value.find('"')?].to_string());
                }
            }
            None
        })
        .collect()
}

/// The text inside each `{ ... }` branch of a `cfg_if!` or `cfg_select!` call.
fn cfg_if_blocks(call: &SyntaxNode) -> Vec<String> {
    let Some(tree) = call
        .children()
        .into_iter()
        .find(|c| c.kind() == SyntaxKind::TokenTree)
    else {
        return Vec::new();
    };
    tree.children()
        .into_iter()
        .filter(|c| c.kind() == SyntaxKind::TokenTree)
        .filter_map(|block| {
            let text = block.to_string();
            let inner = text.strip_prefix('{')?.strip_suffix('}')?;
            Some(inner.to_string())
        })
        .collect()
}

/// The out-of-line modules declared in `container`: (name, candidate files, the folder the
/// file owns when it is not the usual one). `owned` is the folder `mod a;` looks in;
/// `here` is where `#[path]` is relative to.
fn declared(
    container: &SyntaxNode,
    owned: &Path,
    here: &Path,
    found: &mut Vec<(String, Vec<PathBuf>, Option<PathBuf>)>,
) {
    for node in container.children() {
        let Some(item) = Item::cast(node.clone()) else {
            continue;
        };
        if item.kind() == ItemKind::MacroCall && is_cfg_if(&node) {
            for block in cfg_if_blocks(&node) {
                let (_, parsed) = parse_any_edition(&block);
                declared(&parsed.syntax_node(), owned, here, found);
            }
            continue;
        }
        if item.kind() != ItemKind::Module {
            continue;
        }
        if item.attrs().iter().any(|a| a.is_rustfmt_skip()) {
            continue;
        }
        let Some(name) = item.name() else {
            continue;
        };
        let name = name.trim_start_matches("r#").to_string();
        let path = path_attr(&item);
        let alternatives = cfg_attr_paths(&item);
        let body = node
            .children()
            .into_iter()
            .find(|c| c.kind() == SyntaxKind::ItemList);
        match body {
            Some(body) => {
                let inner = owned.join(path.as_deref().unwrap_or(&name));
                let here = match &path {
                    Some(p) => here.join(p),
                    None => inner.clone(),
                };
                declared(&body, &inner, &here, found);
            }
            None if path.is_none() && !alternatives.is_empty() => {
                for p in alternatives {
                    let file = here.join(&p);
                    let owns = file.parent().unwrap_or(Path::new("")).to_path_buf();
                    found.push((name.clone(), vec![file], Some(owns)));
                }
                // When no condition holds, the usual file is the module, as the standard
                // formatter assumes too; it is taken only if it exists.
                let usual = [
                    owned.join(format!("{}.rs", name)),
                    owned.join(&name).join("mod.rs"),
                ];
                if usual.iter().any(|f| f.is_file()) {
                    found.push((name, usual.to_vec(), None));
                }
            }
            None => match path {
                Some(p) => {
                    let file = here.join(&p);
                    let owns = file.parent().unwrap_or(Path::new("")).to_path_buf();
                    found.push((name, vec![file], Some(owns)));
                }
                None => found.push((
                    name.clone(),
                    vec![
                        owned.join(format!("{}.rs", name)),
                        owned.join(&name).join("mod.rs"),
                    ],
                    None,
                )),
            },
        }
    }
}

/// Whether a macro call is `cfg_if!` or `cfg_select!` (with or without a path), whose
/// branches hold items.
fn is_cfg_if(call: &SyntaxNode) -> bool {
    call.children()
        .into_iter()
        .find(|c| c.kind() == SyntaxKind::Path)
        .is_some_and(|path| {
            let text = path.to_string();
            let name = text.trim().rsplit("::").next().unwrap_or("");
            name == "cfg_if" || name == "cfg_select"
        })
}

/// The folder a module file owns: its own for `mod.rs`, otherwise one named after it.
fn owned_by(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new(""));
    match path.file_stem().and_then(|s| s.to_str()) {
        Some("mod") | None => parent.to_path_buf(),
        Some(stem) => parent.join(stem),
    }
}

/// The value of a `#[path = "..."]` attribute.
fn path_attr(item: &Item) -> Option<String> {
    item.attrs()
        .iter()
        .find(|a| a.path() == "path")
        .and_then(|a| {
            let text = a.syntax().to_string();
            let start = text.find('"')? + 1;
            let end = start + text[start..].find('"')?;
            Some(text[start..end].to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modules_are_followed_as_rustc_finds_them() {
        let dir = std::env::temp_dir().join(format!("mtindo-modules-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let write = |path: &str, text: &str| {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "src/lib.rs",
            "mod a;\nmod b;\nmod inline { mod c; }\n#[path = \"other/x.rs\"]\nmod x;\nmod missing;\n#[rustfmt::skip]\nmod skipped;\n#[cfg_attr(unix, path = \"u.rs\")]\n#[cfg_attr(not(unix), path = \"w.rs\")]\nmod sys;\ncfg_if::cfg_if! {\n    if #[cfg(unix)] { mod e; } else { mod f; }\n}\ncfg_select! {\n    unix => { mod g; }\n    _ => {}\n}\n",
        );
        write("src/skipped.rs", "");
        write("src/u.rs", "");
        write("src/w.rs", "");
        write("src/e.rs", "");
        write("src/f.rs", "");
        write("src/g.rs", "");
        write("src/sys.rs", "");
        write("src/a.rs", "mod deep;\n");
        write("src/a/deep.rs", "");
        write("src/b/mod.rs", "mod sub;\n");
        write("src/b/sub.rs", "");
        write("src/inline/c.rs", "");
        write("src/other/x.rs", "mod y;\n");
        write("src/other/y.rs", "");
        let (files, problems) = module_files(&dir.join("src/lib.rs"));
        let names: Vec<String> = files
            .iter()
            .map(|f| f.strip_prefix(&dir).unwrap().display().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "src/lib.rs",
                "src/a.rs",
                "src/a/deep.rs",
                "src/b/mod.rs",
                "src/b/sub.rs",
                "src/inline/c.rs",
                "src/other/x.rs",
                "src/other/y.rs",
                "src/u.rs",
                "src/w.rs",
                "src/sys.rs",
                "src/e.rs",
                "src/f.rs",
                "src/g.rs",
            ]
        );
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].contains("no file for `mod missing;`"),
            "{:?}",
            problems
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
