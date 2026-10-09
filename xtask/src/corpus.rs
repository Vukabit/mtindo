//! `cargo xtask corpus`: the pinned corpus of real Rust repositories.
//!
//! - `update` moves every pin in `compat/corpus.txt` to the repository's current HEAD.
//! - `fetch [--with-rust]` checks each pinned commit out, shallow, into
//!   `compat/cache/corpus/NAME/` (ignored by git), several at a time. A checkout already at
//!   its pin is kept, after undoing any edits made to it.
//! - `check [--with-rust] [--baseline]` fetches what is missing, then runs the checks in
//!   `xtask-tools` (which links mtindo) in a release build.
//!
//! Everything is logged to `logs/corpus.log`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::thread;

use crate::common::log::Logger;
use crate::common::process;

/// The marker that records which commit a checkout holds.
const MARKER: &str = ".mtindo-commit";

/// How many repositories are fetched at once.
const PARALLEL_FETCHES: usize = 8;

const PINS_PATH: &str = "compat/corpus.txt";

#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub name: String,
    pub url: String,
    pub commit: String,
    pub opt_in: bool,
}

pub fn cache_dir(root: &Path) -> PathBuf {
    root.join("compat").join("cache").join("corpus")
}

/// Parses `compat/corpus.txt`; comments and blank lines are skipped.
pub fn parse_pins(text: &str) -> Result<Vec<Pin>, String> {
    let mut pins = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let opt_in = match words.get(3) {
            None => false,
            Some(&"opt-in") => true,
            Some(other) => {
                return Err(format!("{}:{}: unexpected `{}`", PINS_PATH, idx + 1, other));
            }
        };
        if words.len() < 3 || words.len() > 4 {
            return Err(format!(
                "{}:{}: expected NAME URL COMMIT [opt-in]",
                PINS_PATH,
                idx + 1
            ));
        }
        let commit = words[2];
        if commit != "-" && !(commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit())) {
            return Err(format!(
                "{}:{}: `{}` is not a full commit hash",
                PINS_PATH,
                idx + 1,
                commit
            ));
        }
        if pins.iter().any(|p: &Pin| p.name == words[0]) {
            return Err(format!(
                "{}:{}: `{}` is listed twice",
                PINS_PATH,
                idx + 1,
                words[0]
            ));
        }
        pins.push(Pin {
            name: words[0].to_string(),
            url: words[1].to_string(),
            commit: commit.to_string(),
            opt_in,
        });
    }
    Ok(pins)
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let log = Logger::new(root, "corpus");
    log.info(&format!("cargo xtask corpus {}", args.join(" ")));
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let with_rust = words.contains(&"--with-rust");
    let code = match words.first().copied() {
        Some("update") if words.len() == 1 => update(root, &log),
        Some("fetch") if words[1..].iter().all(|w| *w == "--with-rust") => {
            fetch(root, with_rust, &log)
        }
        Some("check")
            if words[1..]
                .iter()
                .all(|w| *w == "--with-rust" || *w == "--baseline") =>
        {
            match fetch(root, with_rust, &log) {
                0 => check(root, &words[1..], with_rust, &log),
                code => code,
            }
        }
        _ => {
            eprintln!(
                "Usage: cargo xtask corpus update | fetch [--with-rust] | check [--with-rust] [--baseline]"
            );
            2
        }
    };
    log.info(&format!("exit code {}", code));
    ExitCode::from(code)
}

fn check(root: &Path, flags: &[&str], with_rust: bool, log: &Logger) -> u8 {
    let pins = match load_pins(root) {
        Ok(pins) => pins,
        Err(e) => {
            println!("{}", e);
            return 1;
        }
    };
    let mut args: Vec<String> = [
        "run",
        "--quiet",
        "--release",
        "--package",
        "xtask-tools",
        "--",
        "corpus-check",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if flags.contains(&"--baseline") {
        args.push("--baseline".to_string());
    }
    for pin in pins.iter().filter(|p| with_rust || !p.opt_in) {
        args.push(format!(
            "{}={}",
            pin.name,
            cache_dir(root).join(&pin.name).display()
        ));
    }
    log.info("running xtask-tools corpus-check (release build)");
    let status = std::process::Command::new("cargo")
        .args(&args)
        .current_dir(root)
        .status();
    match status {
        Ok(status) => process::exit_code(status).clamp(0, 255) as u8,
        Err(e) => {
            println!("cannot run cargo: {}", e);
            1
        }
    }
}

/// Checks out one pin, or keeps (and cleans) a checkout already at it.
fn checkout(root: &Path, pin: &Pin, log: &Logger) -> Result<&'static str, String> {
    let dir = cache_dir(root).join(&pin.name);
    let marker = fs::read_to_string(dir.join(MARKER)).unwrap_or_default();
    if marker.trim() == pin.commit {
        let changed = git(&dir, &["status", "--porcelain", "--untracked-files=no"])?;
        if changed.trim().is_empty() {
            return Ok("kept");
        }
        log.warning(&format!(
            "{}: modified checkout restored:\n{}",
            pin.name,
            changed.trim_end()
        ));
        git(&dir, &["checkout", "--quiet", "--", "."])?;
        return Ok("restored");
    }
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("cannot clear {}: {}", dir.display(), e))?;
    }
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    let steps: [&[&str]; 4] = [
        &["init", "--quiet"],
        &["remote", "add", "origin", &pin.url],
        &["fetch", "--quiet", "--depth", "1", "origin", &pin.commit],
        &[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "FETCH_HEAD",
        ],
    ];
    for step in steps {
        log.info(&format!("{}: git {}", pin.name, step.join(" ")));
        git(&dir, step)?;
    }
    fs::write(dir.join(MARKER), format!("{}\n", pin.commit)).map_err(|e| e.to_string())?;
    Ok("fetched")
}

fn fetch(root: &Path, with_rust: bool, log: &Logger) -> u8 {
    let pins = match load_pins(root) {
        Ok(pins) => pins,
        Err(e) => {
            println!("{}", e);
            return 1;
        }
    };
    let unpinned: Vec<&str> = pins
        .iter()
        .filter(|p| p.commit == "-")
        .map(|p| p.name.as_str())
        .collect();
    if !unpinned.is_empty() {
        println!(
            "not pinned yet: {}; run `cargo xtask corpus update`",
            unpinned.join(", ")
        );
        return 1;
    }
    let queue: Mutex<Vec<&Pin>> = Mutex::new(
        pins.iter()
            .filter(|p| with_rust || !p.opt_in)
            .rev()
            .collect(),
    );
    let results: Mutex<Vec<(String, Result<&'static str, String>)>> = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..PARALLEL_FETCHES {
            scope.spawn(|| {
                loop {
                    let next = queue.lock().map(|mut q| q.pop()).unwrap_or(None);
                    let Some(pin) = next else {
                        break;
                    };
                    let result = checkout(root, pin, log);
                    if let Ok(mut all) = results.lock() {
                        all.push((pin.name.clone(), result));
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_default();
    results.sort_by(|a, b| a.0.cmp(&b.0));
    let mut failed = 0;
    let mut counts = std::collections::BTreeMap::new();
    for (name, result) in results.iter() {
        match result {
            Ok(what) => *counts.entry(*what).or_insert(0) += 1,
            Err(e) => {
                failed += 1;
                println!("{}: {}", name, e);
                log.error(&format!("{}: {}", name, e));
            }
        }
    }
    let summary: Vec<String> = counts
        .iter()
        .map(|(what, n)| format!("{} {}", n, what))
        .collect();
    println!(
        "corpus fetch: {}{}",
        summary.join(", "),
        if failed > 0 {
            format!(", {} failed", failed)
        } else {
            String::new()
        }
    );
    if failed > 0 { 1 } else { 0 }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (code, output) = process::capture(dir, "git", &owned, &[]);
    if code != 0 {
        return Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            output.trim()
        ));
    }
    Ok(output)
}

fn load_pins(root: &Path) -> Result<Vec<Pin>, String> {
    let text = fs::read_to_string(root.join(PINS_PATH))
        .map_err(|e| format!("cannot read {}: {}", PINS_PATH, e))?;
    parse_pins(&text)
}

/// Moves every pin to its repository's current HEAD, keeping the file's comments and order.
fn update(root: &Path, log: &Logger) -> u8 {
    let path = root.join(PINS_PATH);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            println!("cannot read {}: {}", PINS_PATH, e);
            return 1;
        }
    };
    if let Err(e) = parse_pins(&text) {
        println!("{}", e);
        return 1;
    }
    let mut out = String::new();
    let mut moved = 0;
    let mut failed = 0;
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if line.trim().is_empty() || line.trim_start().starts_with('#') || words.len() < 3 {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let (code, output) = process::capture(
            root,
            "git",
            &[
                "ls-remote".to_string(),
                words[1].to_string(),
                "HEAD".to_string(),
            ],
            &[],
        );
        let head = output.split_whitespace().next().unwrap_or("");
        if code != 0 || head.len() != 40 {
            failed += 1;
            println!("{}: cannot read HEAD ({})", words[0], output.trim());
            log.error(&format!(
                "{}: ls-remote failed: {}",
                words[0],
                output.trim()
            ));
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if head != words[2] {
            moved += 1;
            log.info(&format!("{}: {} -> {}", words[0], words[2], head));
        }
        let mut new_words = words.clone();
        new_words[2] = head;
        out.push_str(&new_words.join(" "));
        out.push('\n');
    }
    if let Err(e) = fs::write(&path, out) {
        println!("cannot write {}: {}", PINS_PATH, e);
        return 1;
    }
    println!(
        "corpus update: {} pin(s) moved{}",
        moved,
        if failed > 0 {
            format!(", {} failed", failed)
        } else {
            String::new()
        }
    );
    if failed > 0 { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pins() {
        let text = "# comment\n\nanyhow https://x/anyhow.git 0123456789abcdef0123456789abcdef01234567\nrust https://x/rust.git - opt-in\n";
        let pins = parse_pins(text).unwrap();
        assert_eq!(pins.len(), 2);
        assert!(!pins[0].opt_in && pins[1].opt_in);
        assert_eq!(pins[1].commit, "-");
        assert!(
            parse_pins("a u abc\n")
                .unwrap_err()
                .contains("not a full commit hash")
        );
        assert!(
            parse_pins("a u - maybe\n")
                .unwrap_err()
                .contains("unexpected `maybe`")
        );
        assert!(
            parse_pins("a u -\na v -\n")
                .unwrap_err()
                .contains("listed twice")
        );
    }

    #[test]
    fn test_the_tracked_pin_list_parses() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let pins = load_pins(root).unwrap();
        assert!(pins.len() >= 45, "{}", pins.len());
        assert!(pins.iter().any(|p| p.name == "rustfmt" && !p.opt_in));
        assert!(pins.iter().any(|p| p.name == "rust" && p.opt_in));
    }
}
