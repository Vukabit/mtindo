use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

pub const GATES: &[Gate] = &[
    (
        "capabilities",
        "every rustfmt option, value and flag, and mto feature, has a status",
        crate::capabilities::check,
    ),
    (
        "deps",
        "mtindo and xtask have no runtime dependencies; xtask/tools only allowed ones",
        check_dependencies,
    ),
    (
        "docs",
        "public Markdown lines are at most 80 columns",
        check_docs,
    ),
    (
        "layering",
        "modules under src/ only use layers below them",
        check_layering,
    ),
];

/// Each manifest and the dependencies it may declare. mtindo is std-only; xtask (the test
/// runner) must build even when mtindo does not, so it may not depend on it either.
const ALLOWED_DEPENDENCIES: &[(&str, &[&str])] = &[
    ("Cargo.toml", &[]),
    ("xtask/Cargo.toml", &[]),
    ("xtask/tools/Cargo.toml", &["mtindo", "ra-ap-rustc_lexer"]),
];

/// The widest a line of public Markdown may be, in characters.
const DOC_LINE_LIMIT: usize = 80;

/// The layers of `src/`, lowest first. A module may only use modules of a lower layer.
/// Modules sharing a layer may not use each other.
const LAYERS: &[(&str, u8)] = &[
    ("text", 0),
    ("syntax", 1),
    ("options", 1),
    ("history", 1),
    ("verify", 2),
    ("layout", 3),
    ("format", 3),
    ("config", 4),
    ("cli", 5),
];

/// Every gate: (name, what it checks, check). `all` runs them in this order.
/// A gate: (name, what it checks, check returning the problems found).
type Gate = (&'static str, &'static str, fn(&Path) -> Vec<String>);

/// Every manifest may only declare the runtime dependencies allowed for it.
pub fn check_dependencies(root: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    for (manifest, allowed) in ALLOWED_DEPENDENCIES {
        let text = match fs::read_to_string(root.join(manifest)) {
            Ok(text) => text,
            Err(e) => {
                problems.push(format!("{}: cannot read: {}", manifest, e));
                continue;
            }
        };
        for entry in runtime_dependencies(&text) {
            let name = entry.rsplit(' ').next().unwrap_or("");
            if !allowed.contains(&name) {
                problems.push(format!(
                    "{}: runtime dependency `{}` is not allowed",
                    manifest, entry
                ));
            }
        }
    }
    problems
}

/// Public Markdown (tracked or new, not git-ignored) keeps lines within the column limit.
pub fn check_docs(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.md",
        ])
        .current_dir(root)
        .output();
    let listing = match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).to_string()
        }
        _ => return vec!["docs: cannot list Markdown files with git".to_string()],
    };
    let mut files: Vec<&str> = listing.lines().filter(|line| !line.is_empty()).collect();
    files.sort();
    files.dedup();
    let mut problems = Vec::new();
    for file in files {
        if let Ok(text) = fs::read_to_string(root.join(file)) {
            problems.extend(long_lines(file, &text));
        }
    }
    problems
}

/// Every module under `src/` that belongs to a layer may only reference lower layers.
pub fn check_layering(root: &Path) -> Vec<String> {
    let src = root.join("src");
    let mut files = Vec::new();
    collect_rust_files(&src, &mut files);
    files.sort();
    let mut problems = Vec::new();
    for file in files.iter() {
        let relative = file.strip_prefix(root).unwrap_or(file);
        let module = match top_module(&src, file) {
            Some(module) => module,
            None => continue,
        };
        if layer_of(&module).is_none() {
            continue;
        }
        if let Ok(content) = fs::read_to_string(file) {
            problems.extend(layer_violations(
                &module,
                &relative.display().to_string(),
                &content,
            ));
        }
    }
    problems
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let which = args.first().map(String::as_str).unwrap_or("all");
    let mut problems = Vec::new();
    if which == "all" {
        for (_, _, check) in GATES {
            problems.extend(check(root));
        }
    } else {
        match GATES.iter().find(|(name, _, _)| *name == which) {
            Some((_, _, check)) => problems.extend(check(root)),
            None => {
                let names: Vec<&str> = GATES.iter().map(|(name, _, _)| *name).collect();
                eprintln!(
                    "Unknown gate `{}`. Expected all, {}.",
                    which,
                    names.join(", ")
                );
                return ExitCode::from(2);
            }
        }
    }
    if problems.is_empty() {
        println!("gates ({}): ok", which);
        ExitCode::SUCCESS
    } else {
        for problem in problems.iter() {
            println!("{}", problem);
        }
        println!("gates ({}): {} problem(s)", which, problems.len());
        ExitCode::from(1)
    }
}

fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

/// The names of `crate::` paths a source file references, with their line numbers.
fn crate_references(content: &str) -> Vec<(usize, String)> {
    let mut references = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        let code = match line.find("//") {
            Some(idx) => &line[..idx],
            None => line,
        };
        let mut rest = code;
        while let Some(idx) = rest.find("crate::") {
            let after = &rest[idx + "crate::".len()..];
            if let Some(group) = after.strip_prefix('{') {
                let close = group.find('}').unwrap_or(group.len());
                for part in group[..close].split(',') {
                    let name = leading_ident(part.trim());
                    if !name.is_empty() {
                        references.push((line_idx + 1, name.to_string()));
                    }
                }
            } else {
                let name = leading_ident(after);
                if !name.is_empty() {
                    references.push((line_idx + 1, name.to_string()));
                }
            }
            rest = after;
        }
    }
    references
}

fn layer_of(module: &str) -> Option<u8> {
    LAYERS
        .iter()
        .find(|(name, _)| *name == module)
        .map(|(_, layer)| *layer)
}

/// Layer violations in one file of `module`.
fn layer_violations(module: &str, path: &str, content: &str) -> Vec<String> {
    let own_layer = match layer_of(module) {
        Some(layer) => layer,
        None => return Vec::new(),
    };
    let mut problems = Vec::new();
    for (line, target) in crate_references(content) {
        if target == module {
            continue;
        }
        let target_layer = match layer_of(&target) {
            Some(layer) => layer,
            None => continue,
        };
        if target_layer >= own_layer {
            problems.push(format!(
                "{}:{}: `{}` (layer {}) may not use `crate::{}` (layer {})",
                path, line, module, own_layer, target, target_layer
            ));
        }
    }
    problems
}

fn leading_ident(text: &str) -> &str {
    let end = text
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(text.len());
    &text[..end]
}

/// Lines of `text` wider than the documentation limit.
fn long_lines(file: &str, text: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.chars().count() > DOC_LINE_LIMIT)
        .map(|(idx, line)| {
            format!(
                "{}:{}: {} columns (limit {})",
                file,
                idx + 1,
                line.chars().count(),
                DOC_LINE_LIMIT
            )
        })
        .collect()
}

/// Runtime dependency entries in a manifest, as "<section> <name>".
fn runtime_dependencies(manifest: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut section = String::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed.to_string();
            for prefix in ["[dependencies.", "[build-dependencies."] {
                if let Some(name) = trimmed.strip_prefix(prefix) {
                    let name = name.trim_end_matches(']');
                    entries.push(format!("{} {}", prefix.trim_end_matches('.'), name));
                }
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let is_runtime = section == "[dependencies]"
            || section == "[build-dependencies]"
            || (section.starts_with("[target.") && section.ends_with(".dependencies]"));
        if is_runtime {
            let name = trimmed.split('=').next().unwrap_or("").trim();
            entries.push(format!("{} {}", section.trim_end_matches(']'), name));
        }
    }
    entries
}

/// The top-level module a file under `src/` belongs to, e.g. `syntax` for `src/syntax/lexer/mod.rs`.
/// Files directly in `src/` other than `<module>.rs` (such as `lib.rs` and `main.rs`) belong to none.
fn top_module(src: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(src).ok()?;
    let mut components = relative.components();
    let first = components.next()?.as_os_str().to_string_lossy().to_string();
    if components.next().is_some() {
        return Some(first);
    }
    let stem = first.strip_suffix(".rs")?;
    if stem == "lib" || stem == "main" {
        None
    } else {
        Some(stem.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comments_and_own_module_are_ignored() {
        let content = "// see crate::cli for the caller\nuse crate::syntax::kind::SyntaxKind;\n";
        assert!(layer_violations("syntax", "src/syntax/parser/mod.rs", content).is_empty());
    }

    #[test]
    fn test_grouped_crate_imports_are_checked() {
        let content = "use crate::{text::LineIndex, config::load};\n";
        let problems = layer_violations("verify", "src/verify/mod.rs", content);
        assert_eq!(
            problems,
            vec!["src/verify/mod.rs:1: `verify` (layer 2) may not use `crate::config` (layer 4)"]
        );
    }

    #[test]
    fn test_higher_layer_use_is_reported() {
        let content = "use crate::cli::run;\n";
        let problems = layer_violations("syntax", "src/syntax/mod.rs", content);
        assert_eq!(
            problems,
            vec!["src/syntax/mod.rs:1: `syntax` (layer 1) may not use `crate::cli` (layer 5)"]
        );
    }

    #[test]
    fn test_long_lines_counts_characters_not_bytes() {
        let exactly = "é".repeat(DOC_LINE_LIMIT);
        let over = "x".repeat(DOC_LINE_LIMIT + 1);
        let text = format!("short\n{}\n{}\n", exactly, over);
        assert_eq!(
            long_lines("README.md", &text),
            vec!["README.md:3: 81 columns (limit 80)"]
        );
    }

    #[test]
    fn test_lower_layer_use_is_allowed() {
        let content = "use crate::text::LineIndex;\nuse crate::syntax::{parse, Edition};\n";
        assert!(layer_violations("layout", "src/layout/order.rs", content).is_empty());
    }

    #[test]
    fn test_runtime_dependencies_allows_dev_dependencies() {
        let manifest = "[dependencies]\n\n[dev-dependencies]\nproptest = \"1\"\n";
        assert!(runtime_dependencies(manifest).is_empty());
    }

    #[test]
    fn test_runtime_dependencies_finds_every_form() {
        let manifest = "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\n\n[dev-dependencies]\nproptest = \"1\"\n\n[dependencies.rand]\nversion = \"0.8\"\n\n[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n\n[build-dependencies]\ncc = \"1\"\n";
        let entries = runtime_dependencies(manifest);
        assert_eq!(
            entries,
            vec![
                "[dependencies serde",
                "[dependencies rand",
                "[target.'cfg(unix)'.dependencies libc",
                "[build-dependencies cc",
            ]
        );
    }

    #[test]
    fn test_same_layer_sibling_is_reported() {
        let content = "fn f() { crate::format::print(); }\n";
        let problems = layer_violations("layout", "src/layout/mod.rs", content);
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn test_top_module_of_files() {
        let src = Path::new("/repo/src");
        assert_eq!(
            top_module(src, Path::new("/repo/src/syntax/lexer/mod.rs")),
            Some("syntax".to_string())
        );
        assert_eq!(
            top_module(src, Path::new("/repo/src/text.rs")),
            Some("text".to_string())
        );
        assert_eq!(top_module(src, Path::new("/repo/src/lib.rs")), None);
        assert_eq!(top_module(src, Path::new("/repo/src/main.rs")), None);
    }
}
