//! `cargo xtask race`: times rustfmt (stable), rustfmt (nightly) and mto on the same inputs,
//! feature by feature, with the results drawn live.
//!
//! - **Rows** are features: every `KEY=VALUE` that rustfmt's own test inputs set with
//!   `// rustfmt-KEY: VALUE` lines (from the pinned stable checkout that `capabilities
//!   refresh` caches), plus `(defaults)` for inputs that set nothing, a process start-up
//!   row, and a real-code row (the largest source files of the pinned corpus, at the
//!   defaults). A row's time is the total for the inputs that set that feature.
//! - **rustfmt's test-harness lines are followed, not timed as features:**
//!   `// rustfmt-config: FILE` loads `tests/config/FILE` (its settings count as the input's
//!   features), and `target`, `stable` and `unstable` only steer rustfmt's own test runner.
//! - **Repeatable, not exact:** inputs and tool versions are pinned and recorded; every input
//!   runs through the three tools in turn (rotating who goes first each round), so drift
//!   on the machine hits all of them alike; one warm-up round is discarded; each cell is
//!   the median of the rounds with its spread, and a spread over 5% is marked `~`.
//! - **Cells:** a time; `unstable` when the option (or value) is nightly-only according to
//!   `compat/rustfmt-surface.txt` (rustfmt's own warning only decides for keys the surface
//!   does not list, and is logged when the two disagree); `missing` when
//!   `compat/capabilities.txt` says mto does not support it yet. An mto time carries ✓ or
//!   ✗ for whether its output matches rustfmt's byte for byte; the speed-up is only shown
//!   with ✓.
//! - **Times are net of start-up:** every input is a separate run fed through stdin, so a
//!   row's total includes one process start per input; the start-up row measures that cost
//!   and it is subtracted (`--gross` shows totals). A net time under 5% of the total is
//!   shown as `≈0`: the row measures start-up, not formatting, and gets no speed-up.
//! - `file_lines` inputs are left out: that option can only be given on rustfmt's command
//!   line, not in a config file.
//!
//! **Live view:** on a terminal the table is redrawn in place, at most every 150 ms. Every
//! frame is fitted to the terminal's current size (read each frame, so resizing is safe):
//! lines are cut to the width, colour codes never split, the feature column shrinks first,
//! and the rows shown are capped to the height. Frames are drawn from the top-left corner
//! and the rest of the screen is cleared, so a resize can never leave stale lines behind.
//! `--live` forces the live view, `--size COLSxROWS[,COLSxROWS...]` forces sizes (cycled
//! frame by frame, to replay resizes), and `--frames DIR` writes every frame whose text
//! changed, exactly as drawn (`NNNN.raw`) and as plain text (`NNNN.txt`).
//!
//! The machine is kept awake while it runs; details go to `logs/race.log`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

use crate::capabilities::manifest::{Manifest, Status};
use crate::capabilities::surface::Kind;
use crate::capabilities::{MANIFEST_PATH, fetch, load_surface};
use crate::common::awake::KeepAwake;
use crate::common::color::color_enabled;
use crate::common::log::Logger;
use crate::common::process;

/// Columns besides the feature name in the compact layout: " " + 10 + " " + 10 + " " + 14.
const COMPACT_COLUMNS: usize = 37;

/// How many of the corpus's largest source files make up the real-code row.
const DEFAULT_REAL_FILES: usize = 100;

const DEFAULT_ROUNDS: usize = 3;

const DEFAULTS_ROW: &str = "(defaults, no options)";

/// Columns besides the feature name: " " + files 5 + "  " + 14 + " " + 14 + " " + mto 20.
const FIXED_COLUMNS: usize = 58;

/// Keys in rustfmt's test headers that steer its test runner rather than set options.
const HARNESS_DIRECTIVES: &[&str] = &["target", "stable", "unstable"];

/// Lines of the live view that are not table rows: title, note, progress, blank, header,
/// rule.
const LIVE_CHROME: usize = 6;

/// The most rows the live view shows; fewer when the terminal is short.
const LIVE_ROWS: usize = 14;

/// Below this share of the total, a net time is reported as start-up only.
const NET_FLOOR: f64 = 0.05;

const NOISY_PERCENT: f64 = 5.0;

const REAL_ROW: &str = "real code (largest corpus files)";

const STARTUP_ROW: &str = "process start-up (per run)";

const TOOLS: [Tool; 3] = [Tool::Stable, Tool::Nightly, Tool::Mto];

/// What one tool gives for one row.
#[derive(Clone, Debug, PartialEq)]
enum Cell {
    Time {
        median: f64,
        /// The median less one process start per input (equal to `median` with `--gross`).
        net: f64,
        spread: f64,
        matches: Option<bool>,
    },
    Unstable,
    Missing,
    /// The tool exited with an error on at least one of the row's inputs.
    Failed,
    Measuring,
    Queued,
}

/// How the table is laid out for a given width.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// Every column, with spreads: 100 columns or more, or no limit.
    Full,
    /// No `files` column and no spreads: 64-99 columns.
    Compact,
    /// The feature name on one line, the three times indented on the next: under 64.
    Stacked,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Tool {
    Stable,
    Nightly,
    Mto,
}

/// Writes frames to `--frames DIR` whenever their text changes.
struct FrameLog {
    dir: PathBuf,
    count: usize,
    last: String,
}

/// One input: its text, the features it exercises (its rows) and the config file written
/// for every setting it runs with.
struct Input {
    name: String,
    text: String,
    settings: Vec<(String, String)>,
    config: Option<PathBuf>,
    /// From the corpus, for the real-code row, rather than from rustfmt's tests.
    real: bool,
}

struct Options {
    rounds: usize,
    filter: String,
    markdown: bool,
    /// Force the live view on (`--live`), even when stdout is not a terminal.
    live: bool,
    /// Forced terminal sizes, cycled frame by frame (`--size`).
    sizes: Vec<(usize, usize)>,
    /// Where to write every changed frame (`--frames`).
    frames: Option<PathBuf>,
    /// Show totals instead of times net of start-up (`--gross`).
    gross: bool,
    /// How many corpus files make up the real-code row (`--real N`; 0 leaves it out).
    real: usize,
}

struct Progress {
    round: usize,
    rounds: usize,
    done: usize,
    total: usize,
    elapsed: u64,
    current: Option<usize>,
    /// Inputs measured most recently, newest first, to fill the live window.
    recent: Vec<usize>,
}

struct Race {
    inputs: Vec<Input>,
    /// Row label -> indexes of the inputs in it, in table order.
    rows: Vec<(String, Vec<usize>)>,
    /// samples[tool][input] = seconds per measured round.
    samples: BTreeMap<Tool, Vec<Vec<f64>>>,
    startup: BTreeMap<Tool, Vec<f64>>,
    /// Last output of each tool for each input, to compare mto with rustfmt.
    outputs: BTreeMap<Tool, Vec<String>>,
    /// Whether stable rustfmt accepts each row's feature, from rustfmt's surface; rows the
    /// surface does not know are absent.
    stable_rows: BTreeMap<String, bool>,
    /// Settings stable rustfmt refused as nightly-only, from its own warnings.
    refused: BTreeSet<String>,
    /// Inputs each tool exited with an error on. An input either rustfmt fails on is left
    /// out of every row, so all three columns still time the same inputs; an mto failure
    /// shows as `error`.
    failed: BTreeMap<Tool, BTreeSet<usize>>,
    /// Inputs left out because rustfmt failed on them.
    excluded: BTreeSet<usize>,
    /// Rows mto supports according to the capabilities manifest.
    mto_rows: BTreeSet<String>,
    versions: BTreeMap<Tool, String>,
    /// Totals instead of net times (`--gross`).
    gross: bool,
}

/// The space a frame has to fit in: (columns, rows); `None` means unlimited.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Screen {
    cols: Option<usize>,
    rows: Option<usize>,
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let log = Logger::new(root, "race");
    log.info(&format!("cargo xtask race {}", args.join(" ")));
    let options = match parse_args(args) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("{}", e);
            eprintln!(
                "Usage: cargo xtask race [--rounds N] [--filter TEXT] [--real N] [--gross] [--markdown] [--live] [--size COLSxROWS[,...]] [--frames DIR]"
            );
            return ExitCode::from(2);
        }
    };
    let mut race = match prepare(root, &options, &log) {
        Ok(race) => race,
        Err(e) => {
            println!("{}", e);
            log.error(&e);
            return ExitCode::from(1);
        }
    };
    let keep_awake = KeepAwake::start();
    log.info(&format!(
        "keeping the machine awake with {}",
        keep_awake.how()
    ));
    let live = (options.live || std::io::stdout().is_terminal()) && !options.markdown;
    let color = color_enabled(false) && !options.markdown;
    let started = Instant::now();
    measure(root, &mut race, &options, live, color, &log);
    drop(keep_awake);
    // Cross-check: rustfmt refusing a setting the surface calls stable means one of them is
    // wrong, so it is logged for a look.
    for label in race.refused.iter() {
        if race.stable_rows.get(label) == Some(&true) {
            log.warning(&format!(
                "stable rustfmt refused `{}`, which compat/rustfmt-surface.txt calls stable",
                label
            ));
        }
    }
    let elapsed = started.elapsed().as_secs();
    let final_screen = Screen {
        cols: screen_for(&options, 0).cols,
        rows: None,
    };
    let table = render(&race, &options, None, color, final_screen);
    print!("{}", table);
    let summary = summary(&race, elapsed, &options);
    print!("{}", summary);
    log.info(&format!(
        "result:\n{}{}",
        render(
            &race,
            &options,
            None,
            false,
            Screen {
                cols: None,
                rows: None
            }
        ),
        summary
    ));
    ExitCode::SUCCESS
}

/// A row's cell for one tool. A row's time per round is the sum of its inputs' times; the
/// net time takes off one process start per input.
fn cell(race: &Race, label: &str, members: &[usize], tool: Tool, current: Option<usize>) -> Cell {
    if label == STARTUP_ROW {
        return match stats(&race.startup[&tool]) {
            Some((median, spread)) => Cell::Time {
                median,
                net: median,
                spread,
                matches: None,
            },
            None => Cell::Queued,
        };
    }
    if tool == Tool::Stable && !stable_runs(race, label) {
        return Cell::Unstable;
    }
    if tool == Tool::Mto && !race.mto_rows.contains(label) {
        return Cell::Missing;
    }
    if members.iter().any(|i| race.failed[&tool].contains(i)) {
        return Cell::Failed;
    }
    let samples = &race.samples[&tool];
    let rounds = members.iter().map(|i| samples[*i].len()).min().unwrap_or(0);
    if current.is_some_and(|c| members.contains(&c)) && rounds == 0 {
        return Cell::Measuring;
    }
    if rounds == 0 {
        return Cell::Queued;
    }
    let per_round: Vec<f64> = (0..rounds)
        .map(|r| members.iter().map(|i| samples[*i][r]).sum())
        .collect();
    let (median, spread) = stats(&per_round).unwrap_or((0.0, 0.0));
    let net = if race.gross {
        median
    } else {
        let startup = stats(&race.startup[&tool]).map(|(m, _)| m).unwrap_or(0.0);
        (median - startup * members.len() as f64).max(0.0)
    };
    let matches = (tool == Tool::Mto).then(|| {
        let reference = if stable_runs(race, label) {
            Tool::Stable
        } else {
            Tool::Nightly
        };
        members
            .iter()
            .all(|i| race.outputs[&Tool::Mto][*i] == race.outputs[&reference][*i])
    });
    Cell::Time {
        median,
        net,
        spread,
        matches,
    }
}

/// A cell as text, with the speed-up against rustfmt for a matching mto time.
fn cell_text(cell: &Cell, baseline: Option<(f64, f64)>) -> String {
    match cell {
        Cell::Time {
            median,
            net,
            spread,
            ..
        } => {
            let noisy = if *spread > NOISY_PERCENT { "~" } else { "" };
            format!(
                "{} ±{:.0}%{}{}",
                time_text(*median, *net),
                spread,
                noisy,
                verdict(cell, baseline)
            )
        }
        Cell::Unstable => "unstable".to_string(),
        Cell::Missing => "missing".to_string(),
        Cell::Failed => "error".to_string(),
        Cell::Measuring => "⠹".to_string(),
        Cell::Queued => "…".to_string(),
    }
}

/// A cell for the compact and stacked layouts: no spread, the noisy mark kept.
fn cell_text_short(cell: &Cell, baseline: Option<(f64, f64)>) -> String {
    match cell {
        Cell::Time {
            median,
            net,
            spread,
            ..
        } => {
            let noisy = if *spread > NOISY_PERCENT { "~" } else { "" };
            format!(
                "{}{}{}",
                time_text(*median, *net),
                noisy,
                verdict(cell, baseline)
            )
        }
        other => cell_text(other, None),
    }
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

/// Like [`collect`], without `.git` and `target` folders.
fn collect_skipping(dir: &Path, files: &mut Vec<PathBuf>) {
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
            collect_skipping(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

/// Draws one live frame from the top-left corner, fitted to `screen`, and records it.
/// Each line clears to its end and the rest of the screen is cleared after the last line,
/// so nothing from a bigger or differently wrapped earlier frame survives.
fn draw(
    race: &Race,
    options: &Options,
    progress: &Progress,
    color: bool,
    screen: Screen,
    first: bool,
    frames: Option<&mut FrameLog>,
) {
    let text = render(race, options, Some(progress), color, screen);
    let mut out = String::new();
    out.push_str(if first { "\x1b[H\x1b[2J" } else { "\x1b[H" });
    for line in text.lines() {
        out.push_str(line);
        out.push_str("\x1b[K\n");
    }
    out.push_str("\x1b[J");
    print!("{}", out);
    let _ = std::io::stdout().flush();
    if let Some(log) = frames {
        let plain = strip_ansi(&text);
        if plain != log.last {
            log.count += 1;
            let size = match (screen.cols, screen.rows) {
                (Some(c), Some(r)) => format!("{}x{}", c, r),
                _ => "unlimited".to_string(),
            };
            let name = format!("{:04}", log.count);
            let _ = fs::write(log.dir.join(format!("{}.raw", name)), &out);
            let _ = fs::write(
                log.dir.join(format!("{}.txt", name)),
                format!("# frame {}, screen {}\n{}", log.count, size, plain),
            );
            log.last = plain;
        }
    }
}

/// The edition of the crate a file belongs to: `edition = "YEAR"` in the nearest
/// `Cargo.toml` that sets one (workspace ones included), else 2015, rustfmt's default.
fn edition_for(file: &Path) -> String {
    let mut dir = file.parent();
    while let Some(current) = dir {
        if let Ok(manifest) = fs::read_to_string(current.join("Cargo.toml")) {
            for line in manifest.lines() {
                if let Some(value) = line.trim().strip_prefix("edition") {
                    let year = value
                        .trim()
                        .trim_start_matches('=')
                        .trim()
                        .trim_matches('"');
                    if ["2015", "2018", "2021", "2024"].contains(&year) {
                        return year.to_string();
                    }
                }
            }
            if manifest.contains("[package]") && !manifest.contains("edition.workspace") {
                return "2015".to_string();
            }
        }
        dir = current.parent();
    }
    "2015".to_string()
}

/// Takes every input either rustfmt failed on out of all rows (rows left empty go too).
fn exclude_failures(race: &mut Race) {
    let failed: BTreeSet<usize> = race.failed[&Tool::Stable]
        .union(&race.failed[&Tool::Nightly])
        .copied()
        .collect();
    if failed.is_empty() {
        return;
    }
    race.excluded.extend(failed.iter().copied());
    for (_, members) in race.rows.iter_mut() {
        members.retain(|i| !failed.contains(i));
    }
    race.rows.retain(|(_, members)| !members.is_empty());
}

/// Every line cut to fit `cols` columns, never splitting a colour code, and reset after a
/// cut so colour cannot leak. One column is kept free, so no terminal wraps the line.
fn fit(text: &str, cols: usize) -> String {
    let limit = cols.saturating_sub(1);
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, newline) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        let mut visible = 0;
        let mut cut = false;
        let mut chars = body.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                out.push(ch);
                for next in chars.by_ref() {
                    out.push(next);
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
            if visible == limit {
                cut = true;
                break;
            }
            out.push(ch);
            visible += 1;
        }
        if cut && body.contains('\x1b') {
            out.push_str("\x1b[0m");
        }
        out.push_str(newline);
    }
    out
}

/// The settings an input runs with, following rustfmt's test harness: `config` loads a
/// file from `tests/config/` first (header lines then override it), and the runner-only
/// keys are dropped. `None` when a named config file is missing.
fn harness_settings(text: &str, config_dir: &Path) -> Option<Vec<(String, String)>> {
    let headers = settings_of(text);
    let mut settings: Vec<(String, String)> = Vec::new();
    for (key, value) in headers.iter() {
        if key == "config" {
            let file = fs::read_to_string(config_dir.join(value)).ok()?;
            for (k, v) in toml_settings(&file) {
                settings.retain(|(existing, _)| *existing != k);
                settings.push((k, v));
            }
        }
    }
    for (key, value) in headers {
        if key == "config" || HARNESS_DIRECTIVES.contains(&key.as_str()) {
            continue;
        }
        settings.retain(|(existing, _)| *existing != key);
        settings.push((key, value));
    }
    Some(settings)
}

/// The `count` largest `.rs` files under `src/` folders of the corpus, largest first (ties
/// by path), skipping test data, so formatting rather than start-up dominates.
fn largest_sources(corpus: &Path, count: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_skipping(corpus, &mut files);
    let mut sized: Vec<(u64, PathBuf)> = files
        .into_iter()
        .filter(|p| {
            let text = p.display().to_string();
            text.contains("/src/") && !text.contains("/test_data/") && !text.contains("/fixtures/")
        })
        .filter_map(|p| Some((fs::metadata(&p).ok()?.len(), p)))
        .collect();
    sized.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    sized.into_iter().take(count).map(|(_, p)| p).collect()
}

fn measure(root: &Path, race: &mut Race, options: &Options, live: bool, color: bool, log: &Logger) {
    let todo = needed(race);
    let total_rounds = options.rounds + 1;
    let mut frame = 0;
    let mut last_draw = Instant::now();
    let mut frames = options.frames.as_ref().and_then(|dir| {
        fs::create_dir_all(dir).ok()?;
        Some(FrameLog {
            dir: dir.clone(),
            count: 0,
            last: String::new(),
        })
    });
    let started = Instant::now();
    for round in 0..total_rounds {
        let warm_up = round == 0;
        // Start-up cost: an empty input through each tool.
        for tool in rotated(round) {
            let begin = Instant::now();
            let _ = run_tool(root, tool, "", None);
            if !warm_up && let Some(times) = race.startup.get_mut(&tool) {
                times.push(begin.elapsed().as_secs_f64());
            }
        }
        for (done, &idx) in todo.iter().enumerate() {
            if race.excluded.contains(&idx) {
                continue;
            }
            for tool in rotated(round + done) {
                if tool == Tool::Mto && !mto_runs(race, idx) {
                    continue;
                }
                let input = &race.inputs[idx];
                let begin = Instant::now();
                let (code, output, stderr) =
                    run_tool_full(root, tool, &input.text, input.config.as_deref());
                let seconds = begin.elapsed().as_secs_f64();
                if warm_up && tool == Tool::Stable {
                    for key in refused_settings(&stderr) {
                        race.refused.insert(key);
                    }
                }
                if code != Some(0) {
                    let first_new = race.failed.get_mut(&tool).is_some_and(|f| f.insert(idx));
                    if first_new {
                        log.warning(&format!(
                            "{} failed on {} (exit {:?}): {}",
                            tool_name(tool),
                            race.inputs[idx].name,
                            code,
                            stderr
                                .lines()
                                .find(|l| !l.trim().is_empty())
                                .unwrap_or("")
                                .trim()
                        ));
                    }
                }
                if let Some(outputs) = race.outputs.get_mut(&tool) {
                    outputs[idx] = output;
                }
                if !warm_up && let Some(samples) = race.samples.get_mut(&tool) {
                    samples[idx].push(seconds);
                }
            }
            if live && last_draw.elapsed().as_millis() >= 150 {
                let progress = Progress {
                    recent: todo[..done].iter().rev().take(64).copied().collect(),
                    round,
                    rounds: total_rounds,
                    done: done + 1,
                    total: todo.len(),
                    elapsed: started.elapsed().as_secs(),
                    current: Some(idx),
                };
                let screen = screen_for(options, frame);
                draw(
                    race,
                    options,
                    &progress,
                    color,
                    screen,
                    frame == 0,
                    frames.as_mut(),
                );
                frame += 1;
                last_draw = Instant::now();
            }
        }
        if !live {
            println!(
                "{} {}/{} done ({}s)",
                if warm_up { "warm-up round" } else { "round" },
                if warm_up { 0 } else { round },
                options.rounds,
                started.elapsed().as_secs()
            );
        }
        exclude_failures(race);
        log.info(&format!("round {} of {} done", round, options.rounds));
    }
    if live && frame > 0 {
        // Clear the live view; the final table is printed from the top.
        print!("\x1b[H\x1b[2J");
        let _ = std::io::stdout().flush();
    }
}

fn milliseconds(seconds: f64) -> String {
    let ms = seconds * 1000.0;
    if ms >= 100.0 {
        format!("{:.0} ms", ms)
    } else if ms >= 10.0 {
        format!("{:.1} ms", ms)
    } else {
        format!("{:.2} ms", ms)
    }
}

fn mto_binary(root: &Path) -> PathBuf {
    root.join("target").join("release").join("mto")
}

fn mto_runs(race: &Race, idx: usize) -> bool {
    race.rows
        .iter()
        .any(|(label, members)| race.mto_rows.contains(label) && members.contains(&idx))
}

/// Compares labels with numbers by value: `max_width=20` before `max_width=160`.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        n.push(c);
                        it.next();
                    }
                    n
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (mt, nt) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                // By value: fewer significant digits first, then digit by digit, then
                // fewer leading zeros.
                let order = mt
                    .len()
                    .cmp(&nt.len())
                    .then(mt.cmp(nt))
                    .then(m.len().cmp(&n.len()));
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Inputs to measure: those in at least one shown row that some tool runs.
fn needed(race: &Race) -> Vec<usize> {
    let mut set = BTreeSet::new();
    for (_, members) in race.rows.iter() {
        set.extend(members.iter().copied());
    }
    set.into_iter().collect()
}

fn paint(color: bool, code: &str, text: &str) -> String {
    if color {
        format!("\x1b[{}m{}\x1b[0m", code, text)
    } else {
        text.to_string()
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        rounds: DEFAULT_ROUNDS,
        filter: String::new(),
        markdown: false,
        live: false,
        sizes: Vec::new(),
        frames: None,
        gross: false,
        real: DEFAULT_REAL_FILES,
    };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--rounds" if idx + 1 < args.len() => {
                options.rounds =
                    args[idx + 1]
                        .parse()
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| {
                            format!("--rounds needs a positive number, got `{}`", args[idx + 1])
                        })?;
                idx += 1;
            }
            "--filter" if idx + 1 < args.len() => {
                options.filter = args[idx + 1].clone();
                idx += 1;
            }
            "--markdown" => options.markdown = true,
            "--live" => options.live = true,
            "--size" if idx + 1 < args.len() => {
                for size in args[idx + 1].split(',') {
                    let parsed = size
                        .split_once('x')
                        .and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)))
                        .filter(|(c, r)| *c >= 20 && *r >= 8);
                    match parsed {
                        Some(size) => options.sizes.push(size),
                        None => {
                            return Err(format!(
                                "--size wants COLSxROWS (at least 20x8), got `{}`",
                                size
                            ));
                        }
                    }
                }
                idx += 1;
            }
            "--gross" => options.gross = true,
            "--real" if idx + 1 < args.len() => {
                options.real = args[idx + 1]
                    .parse()
                    .map_err(|_| format!("--real needs a number, got `{}`", args[idx + 1]))?;
                idx += 1;
            }
            "--frames" if idx + 1 < args.len() => {
                options.frames = Some(PathBuf::from(&args[idx + 1]));
                idx += 1;
            }
            other => return Err(format!("Unknown race argument `{}`.", other)),
        }
        idx += 1;
    }
    Ok(options)
}

fn prepare(root: &Path, options: &Options, log: &Logger) -> Result<Race, String> {
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
    let rustfmt_dir = fetch::cached_checkout(root, &commit, log)?
        .ok_or("rustfmt's tests are not cached; run `cargo xtask capabilities refresh` online")?;

    // Versions, and that every tool can run.
    let mut versions = BTreeMap::new();
    for tool in TOOLS {
        if tool == Tool::Mto {
            continue;
        }
        let (code, out) = run_tool(root, tool, "", None);
        let version = rustfmt_version(root, tool);
        if code.is_none() || version.is_empty() {
            return Err(format!(
                "{} could not be run: {}",
                tool_name(tool),
                out.trim()
            ));
        }
        versions.insert(tool, version);
    }
    println!("building mto (release)...");
    let (code, out) = process::capture(
        root,
        "cargo",
        &["build", "--quiet", "--release", "--bin", "mto"].map(String::from),
        &[],
    );
    if code != 0 {
        return Err(format!("cannot build mto:\n{}", out));
    }
    let head = process::capture(
        root,
        "git",
        &[
            "rev-parse".to_string(),
            "--short".to_string(),
            "HEAD".to_string(),
        ],
        &[],
    )
    .1;
    let (_, mto_version) = process::capture(
        root,
        &mto_binary(root).display().to_string(),
        &["--version".to_string()],
        &[],
    );
    versions.insert(
        Tool::Mto,
        format!(
            "{} ({})",
            mto_version.trim().trim_start_matches("mto "),
            head.trim()
        ),
    );

    // Options whose values are a fixed list are strings in rustfmt's config.
    let string_keys: BTreeSet<String> = surface
        .items
        .keys()
        .filter(|(kind, _)| *kind == Kind::Value)
        .filter_map(|(_, name)| name.split_once('=').map(|(k, _)| k.to_string()))
        .collect();

    // Inputs and their config files.
    let config_dir = std::env::temp_dir().join(format!("mtindo-race-{}", std::process::id()));
    fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let write_config =
        |n: usize, settings: &[(String, String)]| -> Result<Option<PathBuf>, String> {
            if settings.is_empty() {
                return Ok(None);
            }
            let file = config_dir.join(format!("{}.toml", n));
            let body: String = settings
                .iter()
                .map(|(k, v)| toml_line(k, v, &string_keys))
                .collect();
            fs::write(&file, body).map_err(|e| e.to_string())?;
            Ok(Some(file))
        };
    let tests_dir = rustfmt_dir.join("tests");
    let source_dir = tests_dir.join("source");
    let mut paths = Vec::new();
    collect(&source_dir, &mut paths);
    paths.sort();
    let mut inputs = Vec::new();
    let mut skipped_configs = 0;
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(settings) = harness_settings(&text, &tests_dir.join("config")) else {
            skipped_configs += 1;
            continue;
        };
        if settings.iter().any(|(key, _)| key == "file_lines") {
            continue;
        }
        let config = write_config(inputs.len(), &settings)?;
        let name = path
            .strip_prefix(&source_dir)
            .unwrap_or(&path)
            .display()
            .to_string();
        inputs.push(Input {
            name,
            text,
            settings,
            config,
            real: false,
        });
    }
    if skipped_configs > 0 {
        println!(
            "{} inputs left out: their tests/config file is not cached; run `cargo xtask capabilities refresh` online",
            skipped_configs
        );
    }
    // The real-code row: the corpus's largest source files, each with its crate's edition.
    let corpus = root.join("compat").join("cache").join("corpus");
    if options.real > 0 {
        let real = largest_sources(&corpus, options.real);
        if real.is_empty() {
            println!(
                "real-code row left out: the corpus is not fetched; run `cargo xtask corpus fetch`"
            );
        }
        for path in real {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let edition = edition_for(&path);
            let config = write_config(inputs.len(), &[("edition".to_string(), edition)])?;
            let name = path
                .strip_prefix(&corpus)
                .unwrap_or(&path)
                .display()
                .to_string();
            inputs.push(Input {
                name,
                text,
                settings: Vec::new(),
                config,
                real: true,
            });
        }
    }

    // Rows.
    let mut by_row: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, input) in inputs.iter().enumerate() {
        if input.real {
            by_row.entry(REAL_ROW.to_string()).or_default().push(idx);
            continue;
        }
        if input.settings.is_empty() {
            by_row
                .entry(DEFAULTS_ROW.to_string())
                .or_default()
                .push(idx);
        }
        for (key, value) in input.settings.iter() {
            by_row
                .entry(format!("{}={}", key, value))
                .or_default()
                .push(idx);
        }
    }
    let mut rows: Vec<(String, Vec<usize>)> = Vec::new();
    for fixed in [DEFAULTS_ROW, REAL_ROW] {
        if let Some(members) = by_row.remove(fixed) {
            rows.push((fixed.to_string(), members));
        }
    }
    let mut rest: Vec<(String, Vec<usize>)> = by_row.into_iter().collect();
    rest.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    rows.extend(rest);
    if !options.filter.is_empty() {
        rows.retain(|(label, _)| label == DEFAULTS_ROW || label.contains(&options.filter));
    }

    // Which rows stable rustfmt runs, from rustfmt's surface.
    let reference = if surface.has_source("local") {
        "local"
    } else {
        "stable"
    };
    let mut stable_rows = BTreeMap::new();
    for (label, _) in rows.iter() {
        if label == DEFAULTS_ROW || label == REAL_ROW {
            stable_rows.insert(label.clone(), true);
            continue;
        }
        let Some((key, value)) = label.split_once('=') else {
            continue;
        };
        let Some(option) = surface.items.get(&(Kind::Option, key.to_string())) else {
            continue;
        };
        let mut stable = option.stable.contains(reference);
        // A value only the newer releases know as unstable, such as style_edition=2027.
        if let Some(entry) = surface
            .items
            .get(&(Kind::Value, format!("{}={}", key, value)))
            && entry.sources.contains("stable")
            && !entry.stable.contains("stable")
        {
            stable = false;
        }
        stable_rows.insert(label.clone(), stable);
    }

    // Which rows mto supports, from the capabilities manifest.
    let manifest_text = fs::read_to_string(root.join(MANIFEST_PATH)).unwrap_or_default();
    let (manifest, _) = Manifest::parse(&manifest_text);
    let supported: BTreeMap<(Kind, String), Status> = manifest
        .lines
        .iter()
        .map(|l| ((l.kind, l.name.clone()), l.status))
        .collect();
    let usable =
        |status: Option<&Status>| matches!(status, Some(Status::Supported) | Some(Status::Partial));
    let mut mto_rows = BTreeSet::new();
    mto_rows.insert(DEFAULTS_ROW.to_string());
    mto_rows.insert(REAL_ROW.to_string());
    for (label, _) in rows.iter() {
        if let Some((key, value)) = label.split_once('=')
            && (usable(supported.get(&(Kind::Value, format!("{}={}", key, value))))
                || usable(supported.get(&(Kind::Option, key.to_string()))))
        {
            mto_rows.insert(label.clone());
        }
    }
    log.info(&format!(
        "{} inputs, {} rows, rustfmt tests at {}; versions {:?}",
        inputs.len(),
        rows.len(),
        commit,
        versions
    ));
    let count = inputs.len();
    Ok(Race {
        inputs,
        rows,
        samples: TOOLS
            .iter()
            .map(|t| (*t, vec![Vec::new(); count]))
            .collect(),
        startup: TOOLS.iter().map(|t| (*t, Vec::new())).collect(),
        outputs: TOOLS
            .iter()
            .map(|t| (*t, vec![String::new(); count]))
            .collect(),
        stable_rows,
        refused: BTreeSet::new(),
        failed: TOOLS.iter().map(|t| (*t, BTreeSet::new())).collect(),
        excluded: BTreeSet::new(),
        mto_rows,
        versions,
        gross: options.gross,
    })
}

/// The settings stable rustfmt refused, from its
/// "Warning: can't set `key = value`, unstable features are only available in nightly" lines.
fn refused_settings(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter(|line| line.contains("unstable features are only available"))
        .filter_map(|line| line.split('`').nth(1))
        .map(|setting| {
            let (key, value) = setting.split_once('=').unwrap_or((setting, ""));
            format!("{}={}", key.trim(), value.trim())
        })
        .collect()
}

/// The table: every row, or in the live view a window that fits the screen.
fn render(
    race: &Race,
    options: &Options,
    progress: Option<&Progress>,
    color: bool,
    screen: Screen,
) -> String {
    let version = |tool: Tool| {
        race.versions
            .get(&tool)
            .map(String::as_str)
            .unwrap_or("?")
            .to_string()
    };
    let live = progress.is_some();
    let shape = if options.markdown {
        Shape::Full
    } else {
        shape_for(screen)
    };
    let lines_per_row = if shape == Shape::Stacked { 2 } else { 1 };
    // A screen too short for even one row drops the note and the blank line.
    let full_chrome = LIVE_CHROME + if shape == Shape::Stacked { 2 } else { 0 };
    let tight = live
        && screen
            .rows
            .is_some_and(|r| r < full_chrome + 1 + lines_per_row);

    let mut rows: Vec<(&str, &[usize])> = vec![(STARTUP_ROW, &[])];
    rows.extend(race.rows.iter().map(|(l, m)| (l.as_str(), m.as_slice())));
    if let Some(p) = progress {
        // Live: start-up and defaults, the rows of the input being measured, then the rows
        // finished most recently, as many as fit (none on a screen too short for any).
        // The stacked layout has two more lines: a second title line and the value header.
        let chrome = if tight { full_chrome - 2 } else { full_chrome };
        let room = screen
            .rows
            .map(|r| r.saturating_sub(chrome + 1) / lines_per_row)
            .unwrap_or(LIVE_ROWS)
            .min(LIVE_ROWS);
        let mut window: Vec<(&str, &[usize])> = rows.iter().take(2.min(room)).copied().collect();
        let mut wanted: Vec<usize> = p.current.into_iter().collect();
        wanted.extend(p.recent.iter().copied());
        for input in wanted {
            for row in rows.iter().skip(2) {
                if window.len() >= room {
                    break;
                }
                if row.1.contains(&input) && !window.iter().any(|w| w.0 == row.0) {
                    window.push(*row);
                }
            }
        }
        // Room left: rows that already have results, in table order.
        for row in rows.iter().skip(2) {
            if window.len() >= room {
                break;
            }
            let measured = TOOLS
                .iter()
                .any(|t| matches!(cell(race, row.0, row.1, *t, None), Cell::Time { .. }));
            if measured && !window.iter().any(|w| w.0 == row.0) {
                window.push(*row);
            }
        }
        // Keep table order within the window.
        let order = |label: &str| rows.iter().position(|r| r.0 == label).unwrap_or(usize::MAX);
        window.sort_by_key(|r| order(r.0));
        rows = window;
    }

    let current = progress.and_then(|p| p.current);
    let cells: Vec<(String, usize, [String; 3])> = rows
        .iter()
        .map(|(label, members)| {
            let found: Vec<Cell> = TOOLS
                .iter()
                .map(|t| cell(race, label, members, *t, current))
                .collect();
            let baseline = match (&found[0], &found[1]) {
                (Cell::Time { median, net, .. }, _) | (_, Cell::Time { median, net, .. }) => {
                    Some((*median, *net))
                }
                _ => None,
            };
            let text = |cell: &Cell, base: Option<(f64, f64)>| {
                if shape == Shape::Full {
                    cell_text(cell, base)
                } else {
                    cell_text_short(cell, base)
                }
            };
            (
                label.to_string(),
                members.len(),
                [
                    text(&found[0], None),
                    text(&found[1], None),
                    text(&found[2], baseline),
                ],
            )
        })
        .collect();

    let mut out = String::new();
    if options.markdown {
        out.push_str(&format!(
            "# mto feature race\n\nrustfmt {} · nightly {} · mto {}\n\n{}\n\n",
            version(Tool::Stable),
            version(Tool::Nightly),
            version(Tool::Mto),
            if race.gross {
                "Total times, process start-up included."
            } else {
                "Times are net of process start-up (the first row); `≈0` means a row measures start-up, not formatting."
            }
        ));
        out.push_str("| feature | files | rustfmt | nightly+ | mto |\n|---|---:|---:|---:|---:|\n");
        for (label, files, [a, b, c]) in cells.iter() {
            let files = if label == STARTUP_ROW {
                "–".to_string()
            } else {
                files.to_string()
            };
            out.push_str(&format!(
                "| `{}` | {} | {} | {} | {} |\n",
                label, files, a, b, c
            ));
        }
        return out;
    }

    // Title: full versions at full width outside the live view, short ones otherwise.
    let versions = if live || shape != Shape::Full {
        format!(
            "rustfmt {} · nightly {} · mto {}",
            short_version(&version(Tool::Stable)),
            short_version(&version(Tool::Nightly)),
            short_version(&version(Tool::Mto))
        )
    } else {
        format!(
            "rustfmt {} · nightly {} · mto {}",
            version(Tool::Stable),
            version(Tool::Nightly),
            version(Tool::Mto)
        )
    };
    if shape == Shape::Stacked {
        // Narrow: two short lines that fit 40 columns.
        out.push_str(&format!(
            "{}  {}\n{}\n",
            paint(color, "1", "mto feature race"),
            paint(
                color,
                "90",
                &format!("mto {}", short_version(&version(Tool::Mto)))
            ),
            paint(
                color,
                "90",
                &format!(
                    "rustfmt {} · nightly {}",
                    short_version(&version(Tool::Stable)),
                    short_version(&version(Tool::Nightly))
                )
            )
        ));
    } else {
        out.push_str(&format!(
            "{}  {}\n",
            paint(color, "1", "mto feature race"),
            paint(color, "90", &versions)
        ));
    }

    // What the numbers are; short enough for 40 columns.
    let note = if race.gross {
        "total times, start-up included"
    } else {
        "times net of start-up (--gross: totals)"
    };
    if !tight {
        out.push_str(&format!("{}\n", paint(color, "90", note)));
    }

    // Progress: the counts first; the bar gets whatever room is left, up to 20 cells.
    if let Some(p) = progress {
        let stage = if p.round == 0 {
            "warm-up".to_string()
        } else {
            format!("round {}/{}", p.round, p.rounds - 1)
        };
        let counts = format!("{}  {}/{} inputs  {}s", stage, p.done, p.total, p.elapsed);
        let room = screen
            .cols
            .map(|c| c.saturating_sub(counts.chars().count() + 3))
            .unwrap_or(20);
        let width = room.min(20);
        let fraction = (p.round * p.total + p.done) as f64 / (p.rounds * p.total).max(1) as f64;
        let filled = ((fraction * width as f64) as usize).min(width);
        if width >= 5 {
            out.push_str(&format!(
                "{}  {}{}\n",
                counts,
                paint(color, "32", &"█".repeat(filled)),
                paint(color, "90", &"░".repeat(width - filled))
            ));
        } else {
            out.push_str(&format!("{}\n", counts));
        }
    }
    if !tight {
        out.push('\n');
    }

    let tint = |text: &str| -> String {
        let code = if text.contains('✗') || text == "error" {
            "31"
        } else if text.contains('✓') {
            "32"
        } else if text == "missing" || text == "unstable" || text == "…" {
            "90"
        } else {
            ""
        };
        if code.is_empty() {
            text.to_string()
        } else {
            paint(color, code, text)
        }
    };
    let pad = |text: &str, n: usize| -> String {
        format!(
            "{}{}",
            tint(text),
            " ".repeat(n.saturating_sub(text.chars().count()))
        )
    };
    let cut = |label: &str, width: usize| -> String {
        if label.chars().count() <= width {
            return label.to_string();
        }
        let mut shown: String = label.chars().take(width.saturating_sub(1)).collect();
        shown.push('…');
        shown
    };
    let files_text = |label: &str, files: usize| {
        if label == STARTUP_ROW {
            "–".to_string()
        } else {
            files.to_string()
        }
    };

    // The feature column is sized from every row, not just those shown, so columns stay
    // put from frame to frame; it gives up space first when the terminal is narrow.
    let longest = race
        .rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .chain([STARTUP_ROW.chars().count()])
        .max()
        .unwrap_or(10);
    match shape {
        Shape::Full => {
            let mut width = longest.clamp(20, 44);
            if let Some(cols) = screen.cols {
                width = width.min(cols.saturating_sub(FIXED_COLUMNS + 1)).max(12);
            }
            out.push_str(&format!(
                " {:<width$} {:>5}  {:<14} {:<14} {}\n",
                "feature",
                "files",
                "rustfmt",
                "nightly+",
                "mto",
                width = width
            ));
            out.push_str(&format!(
                " {} {}  {} {} {}\n",
                "─".repeat(width),
                "─".repeat(5),
                "─".repeat(14),
                "─".repeat(14),
                "─".repeat(20)
            ));
            for (label, files, [a, b, c]) in cells.iter() {
                out.push_str(&format!(
                    " {:<width$} {:>5}  {} {} {}\n",
                    cut(label, width),
                    files_text(label, *files),
                    pad(a, 14),
                    pad(b, 14),
                    tint(c),
                    width = width
                ));
            }
        }
        Shape::Compact => {
            let cols = screen.cols.unwrap_or(100);
            let width = longest
                .min(cols.saturating_sub(COMPACT_COLUMNS + 1))
                .max(12);
            out.push_str(&format!(
                " {:<width$} {:<10} {:<10} {}\n",
                "feature",
                "rustfmt",
                "nightly+",
                "mto",
                width = width
            ));
            out.push_str(&format!(
                " {} {} {} {}\n",
                "─".repeat(width),
                "─".repeat(10),
                "─".repeat(10),
                "─".repeat(14)
            ));
            for (label, _, [a, b, c]) in cells.iter() {
                out.push_str(&format!(
                    " {:<width$} {} {} {}\n",
                    cut(label, width),
                    pad(a, 10),
                    pad(b, 10),
                    tint(c),
                    width = width
                ));
            }
        }
        Shape::Stacked => {
            let cols = screen.cols.unwrap_or(64);
            out.push_str(" feature\n");
            out.push_str(&format!(
                "   {:<10} {:<10} {}\n",
                "rustfmt", "nightly+", "mto"
            ));
            out.push_str(&format!(
                " {}\n",
                "─".repeat(cols.saturating_sub(2).min(40))
            ));
            for (label, files, [a, b, c]) in cells.iter() {
                let files = files_text(label, *files);
                let suffix = if files == "–" {
                    String::new()
                } else {
                    format!(" ({})", files)
                };
                out.push_str(&format!(
                    " {}{}\n",
                    cut(label, cols.saturating_sub(3 + suffix.chars().count())),
                    paint(color, "90", &suffix)
                ));
                out.push_str(&format!("   {} {} {}\n", pad(a, 10), pad(b, 10), tint(c)));
            }
        }
    }
    match screen.cols {
        Some(cols) => fit(&out, cols),
        None => out,
    }
}

/// The tools in a different order each time, so none always goes first.
fn rotated(n: usize) -> [Tool; 3] {
    let shift = n % 3;
    [TOOLS[shift], TOOLS[(shift + 1) % 3], TOOLS[(shift + 2) % 3]]
}

fn run_tool(root: &Path, tool: Tool, text: &str, config: Option<&Path>) -> (Option<i32>, String) {
    let (code, out, _) = run_tool_full(root, tool, text, config);
    (code, out)
}

/// Runs one tool on one input through stdin: (exit code, stdout, stderr).
fn run_tool_full(
    root: &Path,
    tool: Tool,
    text: &str,
    config: Option<&Path>,
) -> (Option<i32>, String, String) {
    let mut command = match tool {
        Tool::Mto => {
            let mut c = Command::new(mto_binary(root));
            c.arg("-").current_dir(std::env::temp_dir());
            c
        }
        Tool::Stable | Tool::Nightly => {
            let mut c = Command::new("rustfmt");
            if tool == Tool::Nightly {
                c.args(["+nightly", "--unstable-features"]);
            }
            c.args(["--emit", "stdout"]).env("RUSTUP_AUTO_INSTALL", "0");
            if let Some(config) = config {
                c.arg("--config-path").arg(config);
            }
            c
        }
    };
    let child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return (None, String::new(), String::new());
    };
    if let Some(mut stdin) = child.stdin.take() {
        let bytes = text.as_bytes().to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    let code = child.wait().ok().and_then(|s| s.code());
    (code, stdout, stderr)
}

fn rustfmt_version(root: &Path, tool: Tool) -> String {
    let mut args = Vec::new();
    if tool == Tool::Nightly {
        args.push("+nightly".to_string());
    }
    args.push("--version".to_string());
    let env = [("RUSTUP_AUTO_INSTALL".to_string(), "0".to_string())];
    let (code, out) = process::capture(root, "rustfmt", &args, &env);
    if code != 0 {
        return String::new();
    }
    out.trim().trim_start_matches("rustfmt ").to_string()
}

/// The screen to fit frame number `frame` into: a forced size from `--size` (cycled),
/// else the terminal's size, else unlimited.
fn screen_for(options: &Options, frame: usize) -> Screen {
    if !options.sizes.is_empty() {
        let (cols, rows) = options.sizes[frame % options.sizes.len()];
        return Screen {
            cols: Some(cols),
            rows: Some(rows),
        };
    }
    match terminal_size() {
        Some((cols, rows)) => Screen {
            cols: Some(cols),
            rows: Some(rows),
        },
        None => Screen {
            cols: None,
            rows: None,
        },
    }
}

/// The settings a test input asks for, from its `// rustfmt-KEY: VALUE` lines.
fn settings_of(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.strip_prefix("// rustfmt-"))
        .filter_map(|rest| rest.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .filter(|(key, _)| {
            !key.is_empty() && key.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        })
        .collect()
}

fn shape_for(screen: Screen) -> Shape {
    match screen.cols {
        None => Shape::Full,
        Some(c) if c >= 100 => Shape::Full,
        Some(c) if c >= 64 => Shape::Compact,
        Some(_) => Shape::Stacked,
    }
}

/// The version without channel and build details: `1.9.0-stable (88d9e12ae1 ...)` -> `1.9.0`.
fn short_version(version: &str) -> String {
    version
        .split_whitespace()
        .next()
        .unwrap_or(version)
        .trim_end_matches("-stable")
        .trim_end_matches("-nightly")
        .to_string()
}

/// The speed-up of an mto time over rustfmt's, when both are more than start-up.
fn speedup(cell: &Cell, baseline: Option<(f64, f64)>) -> Option<f64> {
    match (cell, baseline) {
        (Cell::Time { median, net, .. }, Some((base_median, base_net)))
            if !startup_bound(*median, *net)
                && !startup_bound(base_median, base_net)
                && *net > 0.0 =>
        {
            Some(base_net / net)
        }
        _ => None,
    }
}

/// Whether stable rustfmt runs a row's feature: from rustfmt's surface when it knows the
/// key, else from stable rustfmt's own warning.
fn stable_runs(race: &Race, label: &str) -> bool {
    match race.stable_rows.get(label) {
        Some(stable) => *stable,
        None => !race.refused.contains(label),
    }
}

/// True when a time is mostly start-up: its net part is under [`NET_FLOOR`] of the total.
fn startup_bound(median: f64, net: f64) -> bool {
    median > 0.0 && net < median * NET_FLOOR
}

/// Median and spread (half the range, as a percentage of the median) of some samples.
fn stats(samples: &[f64]) -> Option<(f64, f64)> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mid = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    };
    let spread = if median > 0.0 {
        (sorted[sorted.len() - 1] - sorted[0]) / 2.0 / median * 100.0
    } else {
        0.0
    };
    Some((median, spread))
}

/// Text without colour and cursor codes.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn summary(race: &Race, elapsed: u64, options: &Options) -> String {
    let mut supported = 0;
    let mut matching = 0;
    let mut speedups = Vec::new();
    let mut noisy = 0;
    let mut errors = 0;
    for (label, members) in race.rows.iter() {
        let cells: Vec<Cell> = TOOLS
            .iter()
            .map(|t| cell(race, label, members, *t, None))
            .collect();
        errors += cells.iter().filter(|c| **c == Cell::Failed).count();
        noisy += cells
            .iter()
            .filter(|c| matches!(c, Cell::Time { spread, .. } if *spread > NOISY_PERCENT))
            .count();
        if let Cell::Time { matches, .. } = &cells[2] {
            supported += 1;
            if *matches == Some(true) {
                matching += 1;
                let base = match (&cells[0], &cells[1]) {
                    (Cell::Time { median, net, .. }, _) | (_, Cell::Time { median, net, .. }) => {
                        Some((*median, *net))
                    }
                    _ => None,
                };
                if let Some(ratio) = speedup(&cells[2], base) {
                    speedups.push(ratio);
                }
            }
        }
    }
    speedups.sort_by(|a, b| a.total_cmp(b));
    let median_speedup = speedups.get(speedups.len() / 2).copied();
    let mut out = String::new();
    let lead = if options.markdown { "\n" } else { "\n " };
    out.push_str(&format!(
        "{}done in {}m{:02}s · {} rows · {} inputs · {} rounds{}\n",
        lead,
        elapsed / 60,
        elapsed % 60,
        race.rows.len(),
        needed(race).len(),
        options.rounds,
        if noisy > 0 {
            format!(
                " · {} noisy cells (~), rerun with --rounds {}",
                noisy,
                options.rounds * 2 + 1
            )
        } else {
            String::new()
        }
    ));
    out.push_str(&format!(
        "{}mto runs {} of {} rows; its output matches rustfmt on {}{}\n",
        if options.markdown { "" } else { " " },
        supported,
        race.rows.len(),
        matching,
        match median_speedup {
            Some(s) => format!("; median speed-up where it matches: {:.1}×", s),
            None => String::new(),
        }
    ));
    if !race.excluded.is_empty() {
        out.push_str(&format!(
            "{}{} inputs left out: rustfmt exited with an error on them (see logs/race.log)\n",
            if options.markdown { "" } else { " " },
            race.excluded.len()
        ));
    }
    if errors > 0 {
        out.push_str(&format!(
            "{}{} cells show `error`: mto exited with an error on their inputs (see logs/race.log)\n",
            if options.markdown { "" } else { " " },
            errors
        ));
    }
    // How much of each tool's time on the default row is just starting the process.
    if let Some((_, members)) = race.rows.iter().find(|(l, _)| l == DEFAULTS_ROW) {
        let mut shares = Vec::new();
        for tool in TOOLS {
            if let (Cell::Time { median: row, .. }, Some((startup, _))) = (
                cell(race, DEFAULTS_ROW, members, tool, None),
                stats(&race.startup[&tool]),
            ) && row > 0.0
            {
                let share = (startup * members.len() as f64 / row * 100.0).min(100.0);
                shares.push(format!("{} {:.0}%", tool_name(tool), share));
            }
        }
        if !shares.is_empty() {
            out.push_str(&format!(
                "{}start-up's share of the default row: {} (small inputs measure start-up more than formatting)\n",
                if options.markdown { "" } else { " " },
                shares.join(", ")
            ));
        }
    }
    out
}

/// The terminal's size from the kernel (stdout, then stderr), else `COLUMNS` and `LINES`.
fn terminal_size() -> Option<(usize, usize)> {
    #[cfg(unix)]
    {
        #[repr(C)]
        struct WinSize {
            rows: u16,
            cols: u16,
            x_pixels: u16,
            y_pixels: u16,
        }
        unsafe extern "C" {
            fn ioctl(fd: i32, request: std::ffi::c_ulong, ...) -> i32;
        }
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
        const TIOCGWINSZ: std::ffi::c_ulong = 0x4008_7468;
        #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
        const TIOCGWINSZ: std::ffi::c_ulong = 0x5413;
        for fd in [1, 2] {
            let mut size = WinSize {
                rows: 0,
                cols: 0,
                x_pixels: 0,
                y_pixels: 0,
            };
            // SAFETY: TIOCGWINSZ writes one `winsize` struct, which `size` is laid out as.
            let ok = unsafe { ioctl(fd, TIOCGWINSZ, &mut size as *mut WinSize) } == 0;
            if ok && size.cols > 0 && size.rows > 0 {
                return Some((size.cols as usize, size.rows as usize));
            }
        }
    }
    let read = |name: &str| std::env::var(name).ok()?.parse::<usize>().ok();
    Some((read("COLUMNS")?, read("LINES")?))
}

/// The shown part of a time: the net milliseconds, or `≈0` when start-up is all of it.
fn time_text(median: f64, net: f64) -> String {
    if startup_bound(median, net) {
        "≈0".to_string()
    } else {
        milliseconds(net)
    }
}

/// A setting as a TOML line. Options whose values are a fixed list (`edition`,
/// `style_edition`, `imports_granularity`...) are strings in rustfmt's config, so their
/// values are always quoted, even `2021`; otherwise numbers, booleans, arrays and
/// already-quoted strings are written as they are and words are quoted.
fn toml_line(key: &str, value: &str, string_keys: &BTreeSet<String>) -> String {
    let bare = !string_keys.contains(key)
        && (value.parse::<f64>().is_ok()
            || value == "true"
            || value == "false"
            || value.starts_with('[')
            || value.starts_with('{'))
        || value.starts_with('"');
    if bare {
        format!("{} = {}\n", key, value)
    } else {
        format!(
            "{} = \"{}\"\n",
            key,
            value.replace('\\', "\\\\").replace('"', "\\\"")
        )
    }
}

/// `key = value` lines of a small TOML file, values unquoted; tables and comments skipped.
/// An array may span several lines; it is joined onto one.
fn toml_settings(text: &str) -> Vec<(String, String)> {
    let mut settings = Vec::new();
    let mut lines = text.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let mut value = value.trim().to_string();
        let open = |v: &str| v.matches('[').count() > v.matches(']').count();
        while open(&value) {
            let Some(more) = lines.next() else {
                break;
            };
            if !more.is_empty() && !more.starts_with('#') {
                value.push(' ');
                value.push_str(more);
            }
        }
        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .map(str::to_string)
            .unwrap_or(value);
        settings.push((key.trim().to_string(), unquoted));
    }
    settings
}

fn tool_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Stable => "rustfmt",
        Tool::Nightly => "rustfmt +nightly",
        Tool::Mto => "mto",
    }
}

/// The mark after an mto time: ✓ with its speed-up, ✓ alone, or ✗.
fn verdict(cell: &Cell, baseline: Option<(f64, f64)>) -> String {
    match cell {
        Cell::Time {
            matches: Some(true),
            ..
        } => match speedup(cell, baseline) {
            Some(ratio) => format!(" ✓ {:.1}×", ratio),
            None => " ✓".to_string(),
        },
        Cell::Time {
            matches: Some(false),
            ..
        } => " ✗".to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNLIMITED: Screen = Screen {
        cols: None,
        rows: None,
    };

    fn opts(markdown: bool) -> Options {
        Options {
            rounds: 3,
            filter: String::new(),
            markdown,
            live: false,
            sizes: Vec::new(),
            frames: None,
            gross: false,
            real: 0,
        }
    }

    fn progress(current: usize) -> Progress {
        Progress {
            round: 1,
            rounds: 4,
            done: 2,
            total: 3,
            elapsed: 7,
            current: Some(current),
            recent: Vec::new(),
        }
    }

    fn small_race() -> Race {
        let input = |name: &str, settings: Vec<(&str, &str)>| Input {
            name: name.to_string(),
            text: String::new(),
            settings: settings
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            config: None,
            real: false,
        };
        let inputs = vec![
            input("a.rs", vec![]),
            input("b.rs", vec![("max_width", "80")]),
            input("c.rs", vec![("imports_granularity", "Crate")]),
        ];
        let mut race = Race {
            rows: vec![
                (DEFAULTS_ROW.to_string(), vec![0]),
                ("imports_granularity=Crate".to_string(), vec![2]),
                ("max_width=80".to_string(), vec![1]),
            ],
            samples: TOOLS.iter().map(|t| (*t, vec![Vec::new(); 3])).collect(),
            startup: TOOLS
                .iter()
                .map(|t| (*t, vec![if *t == Tool::Mto { 0.001 } else { 0.010 }]))
                .collect(),
            outputs: TOOLS.iter().map(|t| (*t, vec![String::new(); 3])).collect(),
            stable_rows: [
                (DEFAULTS_ROW.to_string(), true),
                ("imports_granularity=Crate".to_string(), false),
                ("max_width=80".to_string(), true),
            ]
            .into_iter()
            .collect(),
            refused: BTreeSet::new(),
            failed: TOOLS.iter().map(|t| (*t, BTreeSet::new())).collect(),
            excluded: BTreeSet::new(),
            gross: false,
            mto_rows: [DEFAULTS_ROW.to_string()].into_iter().collect(),
            versions: TOOLS.iter().map(|t| (*t, "1.0".to_string())).collect(),
            inputs,
        };
        for tool in TOOLS {
            race.samples.get_mut(&tool).unwrap()[0].push(if tool == Tool::Mto {
                0.002
            } else {
                0.020
            });
        }
        race
    }

    #[test]
    fn test_cell_text() {
        let time = Cell::Time {
            median: 0.0712,
            net: 0.0612,
            spread: 1.0,
            matches: Some(true),
        };
        assert_eq!(cell_text(&time, Some((0.424, 0.412))), "61.2 ms ±1% ✓ 6.7×");
        let wrong = Cell::Time {
            median: 0.0712,
            net: 0.0612,
            spread: 8.0,
            matches: Some(false),
        };
        assert_eq!(cell_text(&wrong, Some((0.424, 0.412))), "61.2 ms ±8%~ ✗");
        assert_eq!(cell_text(&Cell::Unstable, None), "unstable");
    }

    /// The columns sit at the same place in every frame and at every height, so the table
    /// does not jump when the live window changes which rows it shows.
    #[test]
    fn test_columns_stay_put_between_frames() {
        let race = small_race();
        let header_at = |current: usize, rows: usize| {
            let screen = Screen {
                cols: Some(120),
                rows: Some(rows),
            };
            let text = render(&race, &opts(false), Some(&progress(current)), false, screen);
            let header = text
                .lines()
                .find(|l| l.trim_start().starts_with("feature"))
                .unwrap()
                .to_string();
            header.find("rustfmt").unwrap()
        };
        let first = header_at(1, 40);
        assert_eq!(header_at(2, 40), first);
        assert_eq!(header_at(0, 12), first);
    }

    /// A resize replay: frames drawn at changing sizes each fit their own screen, and a
    /// drawn frame starts at the top-left and clears what is below it.
    #[test]
    fn test_drawn_frames_survive_resizes() {
        let race = small_race();
        let dir = std::env::temp_dir().join(format!("xtask-race-frames-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut log = FrameLog {
            dir: dir.clone(),
            count: 0,
            last: String::new(),
        };
        let sizes = [(120, 40), (50, 10), (90, 20)];
        for (frame, (cols, rows)) in sizes.iter().enumerate() {
            let screen = Screen {
                cols: Some(*cols),
                rows: Some(*rows),
            };
            draw(
                &race,
                &opts(false),
                &progress(1),
                true,
                screen,
                frame == 0,
                Some(&mut log),
            );
        }
        assert_eq!(
            log.count, 3,
            "each size changes the text, so each frame is kept"
        );
        for (n, (cols, rows)) in sizes.iter().enumerate() {
            let raw = fs::read_to_string(dir.join(format!("{:04}.raw", n + 1))).unwrap();
            assert!(raw.starts_with(if n == 0 { "\x1b[H\x1b[2J" } else { "\x1b[H" }));
            assert!(raw.ends_with("\x1b[J"));
            let txt = fs::read_to_string(dir.join(format!("{:04}.txt", n + 1))).unwrap();
            assert!(
                txt.starts_with(&format!("# frame {}, screen {}x{}\n", n + 1, cols, rows)),
                "{}",
                txt
            );
        }
        // An unchanged frame is not written again.
        draw(
            &race,
            &opts(false),
            &progress(1),
            true,
            Screen {
                cols: Some(90),
                rows: Some(20),
            },
            false,
            Some(&mut log),
        );
        assert_eq!(log.count, 3);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// An input rustfmt fails on leaves every row; an mto failure shows `error`.
    #[test]
    fn test_failures_are_never_timed() {
        let mut race = small_race();
        race.failed.get_mut(&Tool::Nightly).unwrap().insert(1);
        exclude_failures(&mut race);
        assert!(race.excluded.contains(&1));
        assert!(
            !race.rows.iter().any(|(label, _)| label == "max_width=80"),
            "the row held only input 1"
        );
        race.failed.get_mut(&Tool::Mto).unwrap().insert(0);
        let members = race.rows[0].1.clone();
        assert_eq!(
            cell(&race, DEFAULTS_ROW, &members, Tool::Mto, None),
            Cell::Failed
        );
        assert!(matches!(
            cell(&race, DEFAULTS_ROW, &members, Tool::Stable, None),
            Cell::Time { .. }
        ));
        let summary = summary(&race, 1, &opts(false));
        assert!(
            summary.contains("1 inputs left out: rustfmt exited with an error"),
            "{}",
            summary
        );
        assert!(
            summary.contains("1 cells show `error`: mto exited"),
            "{}",
            summary
        );
    }

    #[test]
    fn test_final_table_and_markdown() {
        let race = small_race();
        let options = opts(false);
        let text = render(&race, &options, None, false, UNLIMITED);
        let unstable = text
            .lines()
            .find(|l| l.contains("imports_granularity=Crate"))
            .unwrap();
        assert!(
            unstable.contains("unstable") && unstable.contains("missing") && unstable.contains("…"),
            "{}",
            unstable
        );
        let markdown = render(&race, &opts(true), None, false, UNLIMITED);
        assert!(
            markdown.contains("| feature | files | rustfmt | nightly+ | mto |"),
            "{}",
            markdown
        );
        assert!(
            markdown.contains("| `max_width=80` | 1 | … | … | missing |"),
            "{}",
            markdown
        );
        let summary = summary(&race, 75, &opts(false));
        assert!(summary.contains("done in 1m15s"), "{}", summary);
        assert!(
            summary.contains(
                "its output matches rustfmt on 1; median speed-up where it matches: 10.0×"
            ),
            "{}",
            summary
        );
        assert!(
            summary.contains("start-up's share of the default row: rustfmt 50%"),
            "{}",
            summary
        );
    }

    #[test]
    fn test_fit_cuts_by_visible_width_and_resets_colour() {
        assert_eq!(fit("abcdef\nxy\n", 4), "abc\nxy\n");
        let coloured = "\x1b[32m✓ok\x1b[0m tail";
        let cut = fit(coloured, 3);
        assert_eq!(strip_ansi(&cut), "✓o");
        assert!(cut.ends_with("\x1b[0m"), "{:?}", cut);
        assert_eq!(fit("short", 80), "short");
    }

    /// Every frame, at every size, fits: no line wider than the screen minus one column,
    /// no more lines than the screen has, colour codes intact.
    #[test]
    fn test_frames_fit_every_screen_size() {
        let race = small_race();
        for (cols, rows) in [(20, 8), (40, 10), (60, 12), (80, 24), (120, 40), (200, 60)] {
            let screen = Screen {
                cols: Some(cols),
                rows: Some(rows),
            };
            for color in [false, true] {
                let text = render(&race, &opts(false), Some(&progress(1)), color, screen);
                let plain = strip_ansi(&text);
                assert!(
                    plain.lines().count() < rows,
                    "{}x{}: {} lines\n{}",
                    cols,
                    rows,
                    plain.lines().count(),
                    plain
                );
                for line in plain.lines() {
                    assert!(
                        line.chars().count() < cols,
                        "{}x{}: too wide: {:?}",
                        cols,
                        rows,
                        line
                    );
                }
                // A cut never leaves a colour open.
                if color {
                    for line in text.lines() {
                        let opens = line.matches("\x1b[").count() - line.matches("\x1b[0m").count();
                        assert!(
                            opens <= line.matches("\x1b[0m").count(),
                            "{}x{}: colour left open in {:?}",
                            cols,
                            rows,
                            line
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_harness_settings_follow_rustfmts_test_runner() {
        let dir = std::env::temp_dir().join(format!("xtask-race-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("small_tabs.toml"),
            "# comment\nmax_width = 80\nhard_tabs = true\nimports_granularity = \"Crate\"\n",
        )
        .unwrap();
        let text = "// rustfmt-config: small_tabs.toml\n// rustfmt-hard_tabs: false\n// rustfmt-target: hello.rs\n// rustfmt-stable: true\n// rustfmt-unstable: true\nfn f() {}\n";
        let settings = harness_settings(text, &dir).unwrap();
        assert_eq!(
            settings,
            vec![
                ("max_width".to_string(), "80".to_string()),
                ("imports_granularity".to_string(), "Crate".to_string()),
                ("hard_tabs".to_string(), "false".to_string()),
            ]
        );
        assert!(harness_settings("// rustfmt-config: gone.toml\n", &dir).is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_live_view_shows_progress_and_the_current_rows() {
        let race = small_race();
        let options = opts(false);
        let progress = Progress {
            round: 1,
            rounds: 4,
            done: 2,
            total: 3,
            elapsed: 7,
            current: Some(1),
            recent: Vec::new(),
        };
        let text = render(&race, &options, Some(&progress), false, UNLIMITED);
        assert!(text.contains("round 1/3  "), "{}", text);
        assert!(text.contains("2/3 inputs  7s"), "{}", text);
        assert!(text.contains("max_width=80"), "{}", text);
        // Rows the current input is not in are left out of the live window.
        assert!(!text.contains("imports_granularity=Crate"), "{}", text);
        let row = text.lines().find(|l| l.contains("max_width=80")).unwrap();
        assert!(row.contains("⠹"), "{}", row);
        let defaults = text.lines().find(|l| l.contains(DEFAULTS_ROW)).unwrap();
        assert!(
            defaults.contains("10.0 ms") && defaults.contains("1.00 ms ±0% ✓ 10.0×"),
            "{}",
            defaults
        );
    }

    /// The mto column, the point of the tool, is visible at every width.
    #[test]
    fn test_mto_column_survives_every_width() {
        let race = small_race();
        for cols in [40, 50, 63, 64, 80, 99, 100, 140] {
            let screen = Screen {
                cols: Some(cols),
                rows: Some(30),
            };
            let text = render(&race, &opts(false), Some(&progress(0)), false, screen);
            let defaults_values = if cols < 64 {
                // Stacked: the values are on the line after the label.
                let at = text.lines().position(|l| l.contains("(defaults")).unwrap();
                text.lines().nth(at + 1).unwrap().to_string()
            } else {
                text.lines()
                    .find(|l| l.contains("(defaults"))
                    .unwrap()
                    .to_string()
            };
            assert!(
                defaults_values.contains("1.00 ms") && defaults_values.contains('✓'),
                "{} cols: {:?}\n{}",
                cols,
                defaults_values,
                text
            );
            let expected = match cols {
                c if c < 64 => Shape::Stacked,
                c if c < 100 => Shape::Compact,
                _ => Shape::Full,
            };
            assert_eq!(shape_for(screen), expected);
        }
    }

    #[test]
    fn test_refused_settings_come_from_rustfmts_warning() {
        let stderr = "Warning: can't set `imports_granularity = Crate`, unstable features are only available in nightly channel.\nother\n";
        assert_eq!(refused_settings(stderr), vec!["imports_granularity=Crate"]);
    }

    #[test]
    fn test_rotation_changes_who_goes_first() {
        assert_eq!(rotated(0)[0], Tool::Stable);
        assert_eq!(rotated(1)[0], Tool::Nightly);
        assert_eq!(rotated(2)[0], Tool::Mto);
        for n in 0..3 {
            let mut tools = rotated(n).to_vec();
            tools.sort();
            assert_eq!(tools, TOOLS.to_vec());
        }
    }

    #[test]
    fn test_rows_sort_numbers_by_value() {
        let mut labels = vec![
            "max_width=160",
            "max_width=20",
            "max_width=25",
            "chain_width=1",
            "max_width=080",
        ];
        labels.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            labels,
            vec![
                "chain_width=1",
                "max_width=20",
                "max_width=25",
                "max_width=080",
                "max_width=160"
            ]
        );
    }

    #[test]
    fn test_settings_of_reads_header_lines() {
        let text = "// rustfmt-max_width: 80\n// rustfmt-imports_granularity: Crate\n// plain comment\nfn f() {}\n// rustfmt-Bad: x\n";
        assert_eq!(
            settings_of(text),
            vec![
                ("max_width".to_string(), "80".to_string()),
                ("imports_granularity".to_string(), "Crate".to_string())
            ]
        );
    }

    #[test]
    fn test_size_argument() {
        let args: Vec<String> = ["--size", "80x24,40x10", "--live", "--frames", "/tmp/f"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let options = parse_args(&args).unwrap();
        assert_eq!(options.sizes, vec![(80, 24), (40, 10)]);
        assert!(options.live);
        assert_eq!(
            screen_for(&options, 3),
            Screen {
                cols: Some(40),
                rows: Some(10)
            }
        );
        assert!(parse_args(&["--size".to_string(), "10x2".to_string()]).is_err());
    }

    /// A time that is almost all start-up shows `≈0` and earns no speed-up, on either side.
    #[test]
    fn test_startup_bound_times_show_as_zero_without_speedups() {
        let mto = Cell::Time {
            median: 0.0150,
            net: 0.0003,
            spread: 1.0,
            matches: Some(true),
        };
        assert_eq!(cell_text(&mto, Some((0.1, 0.05))), "≈0 ±1% ✓");
        let mto_real = Cell::Time {
            median: 0.0150,
            net: 0.010,
            spread: 1.0,
            matches: Some(true),
        };
        // rustfmt's side is start-up bound, so there is no meaningful ratio either.
        assert_eq!(
            cell_text(&mto_real, Some((0.0150, 0.0002))),
            "10.0 ms ±1% ✓"
        );
        assert_eq!(
            cell_text_short(&mto_real, Some((0.2, 0.1))),
            "10.0 ms ✓ 10.0×"
        );
    }

    #[test]
    fn test_stats_median_and_spread() {
        let (median, spread) = stats(&[0.10, 0.12, 0.11]).unwrap();
        assert!((median - 0.11).abs() < 1e-9);
        assert!((spread - 9.090909).abs() < 1e-3);
        assert!(stats(&[]).is_none());
    }

    #[test]
    fn test_toml_lines() {
        let strings: BTreeSet<String> = ["edition", "style_edition", "imports_granularity"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(toml_line("max_width", "80", &strings), "max_width = 80\n");
        assert_eq!(
            toml_line("hard_tabs", "true", &strings),
            "hard_tabs = true\n"
        );
        assert_eq!(
            toml_line("imports_granularity", "Crate", &strings),
            "imports_granularity = \"Crate\"\n"
        );
        // Fixed-list options are strings, even when the value looks like a number.
        assert_eq!(
            toml_line("edition", "2021", &strings),
            "edition = \"2021\"\n"
        );
        assert_eq!(
            toml_line("style_edition", "2024", &strings),
            "style_edition = \"2024\"\n"
        );
        assert_eq!(
            toml_line("ignore", "[\"a\"]", &strings),
            "ignore = [\"a\"]\n"
        );
        assert_eq!(
            toml_line("edition", "\"2018\"", &strings),
            "edition = \"2018\"\n"
        );
    }

    #[test]
    fn test_toml_settings_join_multi_line_arrays() {
        let text = "# comment\nignore = [\n  \"tests/**/ice.rs\",\n  \"b.rs\"\n]\nmax_width = 80\n[table]\n";
        assert_eq!(
            toml_settings(text),
            vec![
                (
                    "ignore".to_string(),
                    "[ \"tests/**/ice.rs\", \"b.rs\" ]".to_string()
                ),
                ("max_width".to_string(), "80".to_string()),
            ]
        );
    }

    #[test]
    fn test_unstable_comes_from_the_surface_first() {
        let mut race = small_race();
        // The surface says stable; a stray warning does not override it.
        race.refused.insert("max_width=80".to_string());
        assert!(stable_runs(&race, "max_width=80"));
        // Unknown to the surface: the warning decides.
        race.refused.insert("mystery=1".to_string());
        assert!(!stable_runs(&race, "mystery=1"));
        assert!(stable_runs(&race, "other=2"));
        // The surface says unstable.
        assert!(!stable_runs(&race, "imports_granularity=Crate"));
    }
}
