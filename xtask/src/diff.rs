//! `cargo xtask diff`: how much of the Rust style mto reproduces (stage 3, F1). Fetches the
//! corpus and the standard formatter's pinned test suite, then runs the harness in
//! `xtask-tools` (which links mtindo), comparing against `compat/compatibility.txt`.
//! `--update` records new numbers; `--markdown` prints a shareable report.

use std::path::Path;
use std::process::{Command, ExitCode};

use crate::capabilities::{fetch as surface_fetch, load_surface};
use crate::common::log::Logger;
use crate::common::process;
use crate::corpus;

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let log = Logger::new(root, "diff");
    log.info(&format!("cargo xtask diff {}", args.join(" ")));
    if let Some(other) = args.iter().find(|a| *a != "--update" && *a != "--markdown") {
        eprintln!("unexpected argument `{}`", other);
        eprintln!("Usage: cargo xtask diff [--update] [--markdown]");
        return ExitCode::from(2);
    }
    if corpus::fetch(root, false, &log) != 0 {
        return ExitCode::from(1);
    }
    let tests = match tests_dir(root, &log) {
        Ok(dir) => dir,
        Err(e) => {
            println!("{}", e);
            return ExitCode::from(1);
        }
    };
    let reference = match Command::new("rustfmt")
        .arg("--version")
        .env("RUSTUP_AUTO_INSTALL", "0")
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => {
            println!("the standard formatter (rustfmt) could not be run");
            return ExitCode::from(1);
        }
    };
    let pins = match corpus::load_pins(root) {
        Ok(pins) => pins,
        Err(e) => {
            println!("{}", e);
            return ExitCode::from(1);
        }
    };
    let mut forwarded: Vec<String> = vec![
        "diff".into(),
        "--tests".into(),
        tests.display().to_string(),
        "--reference".into(),
        reference,
        "--cache".into(),
        root.join("compat/cache/diff").display().to_string(),
        "--record".into(),
        root.join("compat/compatibility.txt").display().to_string(),
        "--meaning-log".into(),
        root.join("logs/diff-meaning.txt").display().to_string(),
        "--exceptions".into(),
        root.join("compat/meaning-exceptions.txt")
            .display()
            .to_string(),
    ];
    forwarded.extend(args.iter().cloned());
    for pin in pins.iter().filter(|p| !p.opt_in) {
        forwarded.push(format!(
            "{}={}",
            pin.name,
            corpus::cache_dir(root).join(&pin.name).display()
        ));
    }
    log.info("running xtask-tools diff (release build)");
    let status = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "--release",
            "--package",
            "xtask-tools",
            "--",
        ])
        .args(&forwarded)
        .current_dir(root)
        .status();
    match status {
        Ok(status) => ExitCode::from(process::exit_code(status).clamp(0, 255) as u8),
        Err(e) => {
            println!("cannot run cargo: {}", e);
            ExitCode::from(1)
        }
    }
}

/// The pinned stable release's `tests` folder, from the capabilities cache.
fn tests_dir(root: &Path, log: &Logger) -> Result<std::path::PathBuf, String> {
    let surface = load_surface(root)?;
    let description = surface
        .sources
        .iter()
        .find(|(label, _)| label == "stable")
        .map(|(_, d)| d.clone())
        .ok_or("no stable release in compat/rustfmt-surface.txt; run `cargo xtask capabilities refresh` online")?;
    let commit = description
        .split([' ', ','])
        .find(|w| w.len() == 40 && w.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or("the stable source names no commit")?
        .to_string();
    let checkout = surface_fetch::cached_checkout(root, &commit, log)?
        .ok_or("the standard formatter's tests are not cached; run `cargo xtask capabilities refresh` online")?;
    Ok(checkout.join("tests"))
}
