//! Parser fixtures: each `tests/fixtures/syntax/**/<case>.rs` is parsed and its tree dump,
//! followed by any errors, must equal `<case>.tree`. Cases under `ok/` must have no errors;
//! cases under `err/` must have at least one. A first-line `// edition: YEAR` comment picks
//! the edition (default 2021). `MTINDO_BLESS=1` rewrites the `.tree` files.

use std::fs;
use std::path::{Path, PathBuf};

use mtindo::syntax::tree::debug_dump;
use mtindo::syntax::{Edition, parse};

fn check(path: &Path, bless: bool) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let parse = parse(&text, edition_of(&text));
    let tree = parse.syntax_node();
    if tree.text() != text {
        return Err("the tree does not reproduce the input".to_string());
    }
    let mut dump = debug_dump(&tree);
    for error in parse.errors() {
        dump.push_str(&format!("error@{}: {}\n", error.range, error.message));
    }
    let in_err = path.components().any(|c| c.as_os_str() == "err");
    if in_err && parse.errors().is_empty() {
        return Err("expected errors, found none".to_string());
    }
    if !in_err && !parse.errors().is_empty() {
        return Err(format!(
            "unexpected errors:\n{}",
            dump.lines()
                .filter(|l| l.starts_with("error@"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let expected_path = path.with_extension("tree");
    if bless {
        fs::write(&expected_path, &dump).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let expected = fs::read_to_string(&expected_path)
        .map_err(|_| "missing .tree file (run with MTINDO_BLESS=1)".to_string())?;
    if dump != expected {
        return Err(format!(
            "tree differs from {}:\n{}",
            expected_path.display(),
            dump
        ));
    }
    Ok(())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn edition_of(text: &str) -> Edition {
    text.lines()
        .next()
        .and_then(|line| line.strip_prefix("// edition:"))
        .and_then(Edition::from_year)
        .unwrap_or(Edition::E2021)
}

#[test]
fn test_parser_fixtures() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/syntax");
    let bless = std::env::var("MTINDO_BLESS").is_ok();
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no parser fixtures in {}",
        root.display()
    );
    let failures: Vec<String> = files
        .iter()
        .filter_map(|path| {
            check(path, bless)
                .err()
                .map(|e| format!("{}: {}", path.strip_prefix(&root).unwrap().display(), e))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} parser fixture(s) failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
