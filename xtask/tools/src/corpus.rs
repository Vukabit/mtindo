//! `corpus-check`: the stage 1 checks over the pinned corpus (run by `cargo xtask corpus
//! check`, which fetches it first). Every `.rs` file is parsed with its crate's edition, and:
//!
//! - **Round trip:** the tree must print back byte for byte. Required for every file.
//! - **Errors on valid code:** a file with syntax errors fails only if rustfmt (decision
//!   S1-2) parses it; the corpus also holds deliberately invalid test data. rustfmt reads
//!   the file from stdin, so it never follows `mod` declarations.
//! - **Tokens:** mto's lexer must agree with rustc's (`lexdiff`), apart from the documented
//!   expected differences.
//! - **Verifier:** everything layout would write (default settings) must pass
//!   `verify::verify_layout`, and laying out the result again must change nothing.
//! - **Speed:** bytes per second for lexing alone and for lexing plus parsing, single
//!   threaded; `--baseline` records them in `compat/baseline.txt`.
//!
//! Details of every finding go to `logs/corpus.log`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::{Edition, parse};
use mtindo::verify::verify_layout;
use mtindo::{LayoutConfig, try_reorganize};

use crate::common;
use crate::log::Logger;

const BASELINE_PATH: &str = "compat/baseline.txt";

/// How many examples of each finding are printed; the log has all of them.
const SHOWN: usize = 5;

#[derive(Default)]
struct Totals {
    files: usize,
    bytes: usize,
    tokens: usize,
    not_utf8: usize,
    lex_time: Duration,
    parse_time: Duration,
    layout_time: Duration,
    verify_time: Duration,
    round_trip: Vec<String>,
    with_errors: Vec<(PathBuf, Edition, String)>,
    lexer: Vec<String>,
    lexer_expected: usize,
    /// Files the stage 0 engine changes, and the verifier's findings on them by rule.
    verified: usize,
    verify_findings: BTreeMap<&'static str, Vec<String>>,
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let log = Logger::new(root, "corpus");
    let mut baseline = false;
    let mut repos = Vec::new();
    for arg in args {
        if arg == "--baseline" {
            baseline = true;
        } else if let Some((name, dir)) = arg.split_once('=') {
            repos.push((name.to_string(), PathBuf::from(dir)));
        }
    }
    let mut totals = Totals::default();
    for (name, dir) in repos.iter() {
        let mut files = Vec::new();
        collect(dir, &mut files);
        files.sort();
        for file in files {
            check_file(
                &file,
                &format!("{}/{}", name, display(&file, dir)),
                &mut totals,
            );
        }
    }

    // Files with syntax errors fail only if rustfmt parses them.
    let mut valid_with_errors = Vec::new();
    let mut rejected_by_both = 0;
    for (path, edition, message) in totals.with_errors.iter() {
        let shown = display(path, &root.join("compat").join("cache").join("corpus"));
        match rustfmt_accepts(path, *edition) {
            Some(true) => valid_with_errors.push(format!("{}: {}", shown, message)),
            Some(false) => rejected_by_both += 1,
            None => {
                valid_with_errors.push(format!("{}: {} (rustfmt could not be run)", shown, message))
            }
        }
    }

    let speed = |time: Duration| {
        let secs = time.as_secs_f64();
        if secs == 0.0 {
            0.0
        } else {
            totals.bytes as f64 / secs / 1_000_000.0
        }
    };
    let lex_speed = speed(totals.lex_time);
    let parse_speed = speed(totals.parse_time);
    let layout_speed = speed(totals.layout_time);
    let verify_speed = speed(totals.verify_time);

    println!(
        "corpus: {} repositories, {} files, {:.1} MB, {} tokens{}",
        repos.len(),
        totals.files,
        totals.bytes as f64 / 1_000_000.0,
        totals.tokens,
        if totals.not_utf8 > 0 {
            format!(" ({} not UTF-8, skipped)", totals.not_utf8)
        } else {
            String::new()
        }
    );
    let mut failed = false;
    failed |= section(
        &log,
        "round trip",
        "every file prints back exactly",
        &totals.round_trip,
    );
    failed |= section(
        &log,
        "valid code",
        &format!(
            "no syntax errors in files rustfmt parses ({} files with errors are invalid for rustfmt too)",
            rejected_by_both
        ),
        &valid_with_errors,
    );
    failed |= section(
        &log,
        "tokens",
        &format!(
            "mto's lexer agrees with rustc's ({} expected differences)",
            totals.lexer_expected
        ),
        &totals.lexer,
    );
    let findings: Vec<String> = totals
        .verify_findings
        .iter()
        .flat_map(|(rule, examples)| examples.iter().map(move |e| format!("{}: {}", rule, e)))
        .collect();
    failed |= section(
        &log,
        "verifier",
        &format!(
            "every change layout makes passes the verifier and is stable ({} files changed)",
            totals.verified
        ),
        &findings,
    );
    for (rule, examples) in totals.verify_findings.iter() {
        println!("  {:>5}  {}", examples.len(), rule);
    }
    println!(
        "speed: lexing {:.1} MB/s, lexing and parsing {:.1} MB/s (one thread, release build)",
        lex_speed, parse_speed
    );
    println!(
        "       layout {:.1} MB/s, verifier {:.1} MB/s (every file; the verifier on changed files)",
        layout_speed, verify_speed
    );
    log.info(&format!(
        "speed: lex {:.2} MB/s, lex+parse {:.2} MB/s, layout {:.2} MB/s, verify {:.2} MB/s",
        lex_speed, parse_speed, layout_speed, verify_speed
    ));

    if baseline {
        let text = format!(
            "# Speed baseline, written by `cargo xtask corpus check --baseline` (release build,\n# one thread). Numbers depend on the machine; compare runs on the same one.\nmachine    {} {}, {} cores\ncorpus     {} files, {} bytes (compat/corpus.txt)\nlex        {:.1} MB/s\nlex+parse  {:.1} MB/s\n",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            totals.files,
            totals.bytes,
            lex_speed,
            parse_speed
        );
        match fs::write(root.join(BASELINE_PATH), text) {
            Ok(()) => println!("wrote {}", BASELINE_PATH),
            Err(e) => {
                println!("cannot write {}: {}", BASELINE_PATH, e);
                failed = true;
            }
        }
    }
    if failed {
        println!("corpus check: failed; details in logs/corpus.log");
        ExitCode::from(1)
    } else {
        println!("corpus check: ok");
        ExitCode::SUCCESS
    }
}

/// `corpus-speed NAME=DIR...`: layout's single-threaded speed over every file (read time
/// excluded), printed as `layout-mb-per-s VALUE` for `cargo xtask corpus bench`.
pub fn speed(args: &[String]) -> ExitCode {
    let mut files = Vec::new();
    for arg in args {
        if let Some((_, dir)) = arg.split_once('=') {
            collect(Path::new(dir), &mut files);
        }
    }
    files.sort();
    let mut bytes = 0usize;
    let mut time = Duration::ZERO;
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        bytes += text.len();
        let started = Instant::now();
        let _ = try_reorganize(&text, &LayoutConfig::default(), false);
        time += started.elapsed();
    }
    let speed = bytes as f64 / time.as_secs_f64().max(1e-9) / 1_000_000.0;
    println!("layout-mb-per-s {:.2}", speed);
    ExitCode::SUCCESS
}

fn check_file(path: &Path, shown: &str, totals: &mut Totals) {
    let Ok(bytes) = fs::read(path) else {
        return;
    };
    let Ok(text) = String::from_utf8(bytes) else {
        totals.not_utf8 += 1;
        return;
    };
    let edition = common::edition_for(path, &text);
    totals.files += 1;
    totals.bytes += text.len();

    let started = Instant::now();
    let tokens = tokenize(&text, edition);
    totals.lex_time += started.elapsed();
    totals.tokens += tokens.len();

    let started = Instant::now();
    let parsed = parse(&text, edition);
    totals.parse_time += started.elapsed();

    let tree = parsed.syntax_node();
    if tree.text() != text {
        totals.round_trip.push(shown.to_string());
    }
    if let Some(error) = parsed.errors().first() {
        let line = text[..(error.range.start as usize).min(text.len())]
            .matches('\n')
            .count()
            + 1;
        totals.with_errors.push((
            path.to_path_buf(),
            edition,
            format!("line {}: {}", line, error.message),
        ));
    }

    let (unexplained, expected, _) = crate::lexdiff::differences(&text, edition);
    totals.lexer_expected += expected.len();
    for difference in unexplained {
        totals.lexer.push(format!("{}: {}", shown, difference));
    }

    // The verifier on what layout would write.
    let started = Instant::now();
    let laid_out = try_reorganize(&text, &LayoutConfig::default(), false);
    totals.layout_time += started.elapsed();
    if let Ok(output) = laid_out
        && output != text
    {
        totals.verified += 1;
        let started = Instant::now();
        let violations = verify_layout(&text, &output, edition);
        totals.verify_time += started.elapsed();
        for violation in violations {
            totals
                .verify_findings
                .entry(violation.rule.name())
                .or_default()
                .push(format!("{}: {}", shown, violation.detail));
        }
        // A second run must change nothing.
        if try_reorganize(&output, &LayoutConfig::default(), false).as_deref()
            != Ok(output.as_str())
        {
            totals
                .verify_findings
                .entry("not-stable")
                .or_default()
                .push(format!("{}: a second run changes the output", shown));
        }
    }
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == "target" {
            continue;
        }
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

fn display(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn edition_year(edition: Edition) -> &'static str {
    match edition {
        Edition::E2015 => "2015",
        Edition::E2018 => "2018",
        Edition::E2021 => "2021",
        Edition::E2024 => "2024",
    }
}

/// Whether rustfmt parses the file: `None` if it cannot be run. Lexing and parsing errors
/// count (`error: ...`, `error[E0758]: ...`); rustfmt's own formatting failures
/// (`error[internal]: ...`) do not.
fn rustfmt_accepts(path: &Path, edition: Edition) -> Option<bool> {
    let text = fs::read(path).ok()?;
    let mut child = Command::new("rustfmt")
        .args(["--edition", edition_year(edition), "--emit", "stdout"])
        .env("RUSTUP_AUTO_INSTALL", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&text);
    });
    let output = child.wait_with_output().ok()?;
    let _ = writer.join();
    let stderr = String::from_utf8_lossy(&output.stderr);
    Some(
        !stderr
            .lines()
            .any(|line| line.starts_with("error") && !line.starts_with("error[internal]")),
    )
}

/// Prints one required check; returns true if it failed.
fn section(log: &Logger, name: &str, what: &str, failures: &[String]) -> bool {
    if failures.is_empty() {
        println!("{}: ok, {}", name, what);
        return false;
    }
    println!(
        "{}: {} file(s) fail; required: {}",
        name,
        failures.len(),
        what
    );
    for failure in failures.iter().take(SHOWN) {
        println!("  {}", failure);
    }
    if failures.len() > SHOWN {
        println!("  ... {} more in logs/corpus.log", failures.len() - SHOWN);
    }
    log.error(&format!(
        "{} failures ({}):\n{}",
        name,
        failures.len(),
        failures.join("\n")
    ));
    true
}
