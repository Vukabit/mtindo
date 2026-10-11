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
    (
        "naming",
        "the reference formatter is named only where the name does work",
        check_naming,
    ),
    (
        "planning",
        "comments in src/, tests/ and fuzz/ name no stages, milestones or decision ids",
        check_planning,
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

/// Where the reference formatter's name may appear anywhere: the tooling that runs it as
/// the answer key, its recorded surface and capabilities, the settings table generated from
/// it, and test inputs taken from it.
const NAMING_ALLOWED_PATHS: &[&str] = &[
    "xtask/",
    "compat/",
    "fuzz/",
    "src/options/rustfmt.rs",
    "tests/fixtures/",
];

/// Spellings of the name that do work wherever they appear: its configuration files, the
/// attributes that tell formatters to skip an item, and the generated table's module.
const NAMING_ALLOWED_SPELLINGS: &[&str] = &[
    ".rustfmt.toml",
    "rustfmt.toml",
    "rustfmt::skip",
    "rustfmt_skip",
    "cfg_attr(rustfmt",
    "RUSTFMT_VERSION",
    "rustfmt::SETTINGS",
    "mod rustfmt;",
    "options/rustfmt.rs",
    "\"rustfmt\"",
    "rustfmt-surface.txt",
    "rustfmt::RUSTFMT_VERSION",
    "config/rustfmt/",
];

/// README sections where the name is the point: the credits, and the development commands
/// that run it.
const NAMING_README_SECTIONS: &[&str] = &["## Credits", "## Development"];

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

/// The reference formatter is named only where the name does work (S3-9): elsewhere, prose
/// says "the Rust style" or "the standard formatter".
pub fn check_naming(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .output();
    let listing = match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).to_string()
        }
        _ => return vec!["naming: cannot list files with git".to_string()],
    };
    let mut files: Vec<&str> = listing.lines().filter(|line| !line.is_empty()).collect();
    files.sort();
    files.dedup();
    let mut problems = Vec::new();
    for file in files {
        if NAMING_ALLOWED_PATHS.iter().any(|p| file.starts_with(p)) {
            continue;
        }
        if let Ok(text) = fs::read_to_string(root.join(file)) {
            problems.extend(stray_names(file, &text));
        }
    }
    problems
}

/// The plan lives in docs/internal/: the code says what it does and why, not which stage,
/// milestone or decision brought it. Test inputs under tests/fixtures/ are left alone.
pub fn check_planning(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_rust_files(&root.join("src"), &mut files);
    collect_rust_files(&root.join("tests"), &mut files);
    collect_rust_files(&root.join("fuzz/fuzz_targets"), &mut files);
    files.sort();
    let fixtures = root.join("tests/fixtures");
    let mut problems = Vec::new();
    for file in files.iter().filter(|f| !f.starts_with(&fixtures)) {
        let shown = file
            .strip_prefix(root)
            .unwrap_or(file)
            .display()
            .to_string();
        if let Ok(text) = fs::read_to_string(file) {
            problems.extend(planning_ids(&shown, &text));
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

/// The comment lines of `text` that name a stage ("stage 3"), a milestone ("F4", "M7") or
/// a decision ("S3-9").
fn planning_ids(file: &str, text: &str) -> Vec<String> {
    let is_id = |word: &str| {
        let mut chars = word.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        let rest = chars.as_str();
        let digits = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit());
        let milestone = matches!(first, 'F' | 'M' | 'N') && digits(rest) && rest.len() <= 2;
        let decision = first == 'S'
            && rest
                .split_once('-')
                .is_some_and(|(stage, n)| digits(stage) && digits(n));
        milestone || decision
    };
    let mut problems = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let Some(at) = line.find("//") else {
            continue;
        };
        let comment = &line[at..];
        let lower = comment.to_ascii_lowercase();
        let stage = lower.match_indices("stage ").any(|(i, m)| {
            lower[i + m.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        });
        let id = comment
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .any(is_id);
        if stage || id {
            problems.push(format!(
                "{}:{}: names a stage, milestone or decision; the plan stays in \
                 docs/internal/: {}",
                file,
                idx + 1,
                line.trim()
            ));
        }
    }
    problems
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

/// The lines of `text` that name the reference formatter outside the allowed spellings (and,
/// in the README, outside the allowed sections).
fn stray_names(file: &str, text: &str) -> Vec<String> {
    let readme = file == "README.md";
    let mut allowed_section = false;
    let mut problems = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if readme && line.starts_with("## ") {
            allowed_section = NAMING_README_SECTIONS.contains(&line.trim_end());
        }
        if allowed_section {
            continue;
        }
        // Longest first, so a spelling inside a longer one cannot leave part of it behind.
        let mut spellings = NAMING_ALLOWED_SPELLINGS.to_vec();
        spellings.sort_by_key(|s| std::cmp::Reverse(s.len()));
        let mut rest = line.to_string();
        for spelling in spellings {
            rest = rest.replace(spelling, "");
        }
        if rest.to_ascii_lowercase().contains("rustfmt") {
            problems.push(format!(
                "{}:{}: names the reference formatter; say \"the Rust style\" or \"the \
                 standard formatter\" (S3-9): {}",
                file,
                idx + 1,
                line.trim()
            ));
        }
    }
    problems
}

/// The top-level module a file under `src/` belongs to, e.g. `syntax` for
/// `src/syntax/lexer/mod.rs`. Files directly in `src/` other than `<module>.rs` (such as
/// `lib.rs` and `main.rs`) belong to none.
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
    fn test_naming_allows_only_the_spellings_that_do_work() {
        let text = "// reads rustfmt.toml and .rustfmt.toml\n#[rustfmt::skip]\n\
                    // as rustfmt sorts them\nlet x = Rustfmt;\n";
        let problems = stray_names("src/a.rs", text);
        assert_eq!(problems.len(), 2, "{:?}", problems);
        assert!(problems[0].starts_with("src/a.rs:3:"));
        // In the README, only the credits and development sections may name it.
        let readme = "# t\nuses rustfmt\n## Credits\nthanks to rustfmt\n## Usage\nrustfmt\n";
        let lines: Vec<String> = stray_names("README.md", readme)
            .iter()
            .map(|p| p.split(':').nth(1).unwrap().to_string())
            .collect();
        assert_eq!(lines, ["2", "6"]);
    }

    #[test]
    fn test_planning_ids_are_found_only_in_comments() {
        let text = "// formatting (stage 3)\n// the F4 rules\n// decided in S3-9\n\
                    let f4 = \"M7\";\n// an F12-key, Stage two, S3 and MS-DOS\n";
        let lines: Vec<String> = planning_ids("src/a.rs", text)
            .iter()
            .map(|p| p.split(':').nth(1).unwrap().to_string())
            .collect();
        assert_eq!(lines, ["1", "2", "3"]);
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
