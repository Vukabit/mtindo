use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use mtindo::{LayoutConfig, parse_toml_config, reorganize_file};

fn compiles(source: &str, scratch: &Path, label: &str) -> Result<(), String> {
    let file = scratch.join(format!("{}.rs", label));
    fs::write(&file, source).unwrap();
    let output = Command::new("rustc")
        .args([
            "--edition",
            "2021",
            "--crate-type",
            "lib",
            "--emit",
            "metadata",
            "-A",
            "warnings",
            "--out-dir",
        ])
        .arg(scratch)
        .arg(&file)
        .output()
        .map_err(|e| format!("cannot run rustc: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn run_fixture(dir: &Path, bless: bool) -> Result<(), String> {
    let input = fs::read_to_string(dir.join("input.rs")).map_err(|e| e.to_string())?;
    let config_path = dir.join("mto.toml");
    let config = if config_path.exists() {
        parse_toml_config(&config_path).map_err(|e| e.to_string())?
    } else {
        LayoutConfig::default()
    };
    let output = reorganize_file(input.clone(), &config, false);
    let second = reorganize_file(output.clone(), &config, false);
    if second != output {
        return Err(format!(
            "not idempotent:\n--- first pass\n{}\n--- second pass\n{}",
            output, second
        ));
    }
    let expected_path = dir.join("expected.rs");
    if bless {
        fs::write(&expected_path, &output).map_err(|e| e.to_string())?;
    } else {
        let expected = fs::read_to_string(&expected_path)
            .map_err(|e| format!("missing expected.rs: {}", e))?;
        if output != expected {
            return Err(format!(
                "output differs from expected.rs:\n--- output\n{}\n--- expected\n{}",
                output, expected
            ));
        }
    }
    if dir.join("COMPILE").exists() {
        let scratch = tempfile::tempdir().unwrap();
        compiles(&input, scratch.path(), "input")
            .map_err(|e| format!("input does not compile:\n{}", e))?;
        compiles(&output, scratch.path(), "output")
            .map_err(|e| format!("output does not compile:\n{}", e))?;
    }
    Ok(())
}

#[test]
fn test_regression_fixtures() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/regressions");
    let bless = std::env::var("MTINDO_BLESS").is_ok();
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no fixtures found in {}", root.display());
    let mut failures = Vec::new();
    for dir in dirs.iter() {
        if let Err(reason) = run_fixture(dir, bless) {
            failures.push(format!(
                "{}: {}",
                dir.file_name().unwrap().to_string_lossy(),
                reason
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} fixture(s) failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
