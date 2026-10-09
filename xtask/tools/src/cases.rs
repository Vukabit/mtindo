//! Runs rustfmt's own test cases through mto, for the report only.
//!
//! A case is a file under `tests/target/`; its input is the file of the same path under
//! `tests/source/`, or the target itself when there is none (rustfmt then checks that
//! formatting changes nothing). Cases that set options with `// rustfmt-KEY: VALUE` lines
//! are counted but not run until mto reads rustfmt's options. A case passes when mto's
//! output equals the target byte for byte; a panic counts as a failure.

use std::fs;
use std::panic;
use std::path::{Path, PathBuf};

use mtindo::{LayoutConfig, try_reorganize};

#[derive(Debug, Default, PartialEq)]
pub struct CaseSummary {
    pub total: usize,
    pub configured: usize,
    pub passed: usize,
    pub unreadable: usize,
}

pub fn run(rustfmt_dir: &Path) -> CaseSummary {
    let target_dir = rustfmt_dir.join("tests").join("target");
    let source_dir = rustfmt_dir.join("tests").join("source");
    let mut targets = Vec::new();
    collect(&target_dir, &mut targets);
    targets.sort();
    let mut summary = CaseSummary::default();
    let config = LayoutConfig::default();
    for target_path in targets {
        summary.total += 1;
        let Ok(target) = fs::read_to_string(&target_path) else {
            summary.unreadable += 1;
            continue;
        };
        if target.lines().any(|line| line.starts_with("// rustfmt-")) {
            summary.configured += 1;
            continue;
        }
        let relative = target_path
            .strip_prefix(&target_dir)
            .unwrap_or(&target_path);
        let input = match fs::read_to_string(source_dir.join(relative)) {
            Ok(text) => text,
            Err(_) => target.clone(),
        };
        let output = panic::catch_unwind(|| try_reorganize(&input, &config, false));
        if let Ok(Ok(text)) = output
            && text == target
        {
            summary.passed += 1;
        }
    }
    summary
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cases_are_counted_and_compared() {
        let dir = std::env::temp_dir().join(format!("xtask-cases-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let target = dir.join("tests").join("target");
        let source = dir.join("tests").join("source");
        fs::create_dir_all(target.join("nested")).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(target.join("same.rs"), "fn a() {}\n").unwrap();
        fs::write(target.join("nested").join("x.rs"), "fn b() {}\n").unwrap();
        fs::write(source.join("same.rs"), "fn a() {}\n").unwrap();
        fs::write(
            target.join("configured.rs"),
            "// rustfmt-max_width: 50\nfn c() {}\n",
        )
        .unwrap();
        fs::write(target.join("changed.rs"), "fn d() {}\n").unwrap();
        fs::write(source.join("changed.rs"), "fn   d() {}\n").unwrap();
        let summary = run(&dir);
        assert_eq!(summary.total, 4);
        assert_eq!(summary.configured, 1);
        assert_eq!(summary.passed, 2);
        fs::remove_dir_all(&dir).unwrap();
    }
}
