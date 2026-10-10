//! `corpus-check`: the stage 1 checks over the pinned corpus (run by `cargo xtask corpus
//! check`, which fetches it first). Every `.rs` file is parsed with its crate's edition, and:
//!
//! - **Round trip:** the tree must print back byte for byte. Required for every file.
//! - **Errors on valid code:** a file with syntax errors fails only if rustfmt (decision
//!   S1-2) parses it; the corpus also holds deliberately invalid test data. rustfmt reads
//!   the file from stdin, so it never follows `mod` declarations.
//! - **Tokens:** mto's lexer must agree with rustc's (`lexdiff`), apart from the documented
//!   expected differences.
//! - **Splitters:** for each file the stage 0 engine processes, its old text scanner and the
//!   new parser must cut the file into the same top-level items, with the same kind, name,
//!   visibility and impl target. Differences are reported by cause, not failed: they are
//!   what stage 2 must know before it swaps the scanner for the parser.
//! - **Verifier:** what today's engine would write is checked by `verify::verify_layout`;
//!   findings are reported by rule but do not fail the check until stage 2's N1.
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

use mtindo::syntax::ast::{Item, ItemKind, SourceFile, Visibility};
use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::{Edition, SyntaxElement, SyntaxKind, parse};
use mtindo::text::source::normalize;
use mtindo::verify::verify_layout;
use mtindo::{
    CodeBlock, ItemKind as OldKind, LayoutConfig, Visibility as OldVisibility, extract_blocks,
    split_header, try_reorganize,
};

use crate::common;
use crate::log::Logger;

const BASELINE_PATH: &str = "compat/baseline.txt";

/// How many examples of each finding are printed; the log has all of them.
const SHOWN: usize = 5;

enum Splitters {
    /// The stage 0 engine refuses the file, so there is nothing to compare.
    NotProcessed,
    /// The file has `// mto: off` regions, which the old scanner keeps as opaque blocks.
    Skipped,
    /// The causes of every disagreement, each with the line it is on and what each side saw.
    Compared(Vec<(String, usize, String)>),
}

#[derive(Default)]
struct Totals {
    files: usize,
    bytes: usize,
    tokens: usize,
    not_utf8: usize,
    lex_time: Duration,
    parse_time: Duration,
    round_trip: Vec<String>,
    with_errors: Vec<(PathBuf, Edition, String)>,
    lexer: Vec<String>,
    lexer_expected: usize,
    split_files: usize,
    split_skipped: usize,
    split_broken: usize,
    split_agree: usize,
    split_causes: BTreeMap<String, Vec<String>>,
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
    // Report-only until stage 2's N1 replaces the old text scanner, whose known mistakes
    // these are; N1's done-when requires zero.
    let _ = section(
        &log,
        "verifier",
        &format!(
            "every change today's engine makes passes the verifier ({} files changed)",
            totals.verified
        ),
        &findings,
    );
    for (rule, examples) in totals.verify_findings.iter() {
        println!("  {:>5}  {}", examples.len(), rule);
    }
    println!(
        "splitters: {} of {} files stage 0 processes agree ({} skipped for `// mto: off`, {} with syntax errors not compared)",
        totals.split_agree, totals.split_files, totals.split_skipped, totals.split_broken
    );
    for (cause, examples) in totals.split_causes.iter() {
        println!("  {:>5}  {}", examples.len(), cause);
        for example in examples.iter().take(2) {
            println!("         e.g. {}", example);
        }
        log.info(&format!(
            "splitters: {} ({}):\n{}",
            cause,
            examples.len(),
            examples.join("\n")
        ));
    }
    println!(
        "speed: lexing {:.1} MB/s, lexing and parsing {:.1} MB/s (one thread, release build)",
        lex_speed, parse_speed
    );
    log.info(&format!(
        "speed: lex {:.2} MB/s, lex+parse {:.2} MB/s",
        lex_speed, parse_speed
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

    // The verifier on what today's engine would write.
    if let Ok(output) = try_reorganize(&text, &LayoutConfig::default(), false)
        && output != text
    {
        totals.verified += 1;
        for violation in verify_layout(&text, &output, edition) {
            totals
                .verify_findings
                .entry(violation.rule.name())
                .or_default()
                .push(format!("{}: {}", shown, violation.detail));
        }
    }

    if !parsed.errors().is_empty() {
        // Item boundaries in broken code are guesses on both sides; not compared.
        totals.split_broken += 1;
    } else {
        compare_into(&text, edition, shown, totals);
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

fn compare_into(text: &str, edition: Edition, shown: &str, totals: &mut Totals) {
    match compare_splitters(text, edition) {
        Splitters::NotProcessed => {}
        Splitters::Skipped => {
            totals.split_files += 1;
            totals.split_skipped += 1;
        }
        Splitters::Compared(causes) => {
            totals.split_files += 1;
            if causes.is_empty() {
                totals.split_agree += 1;
            }
            for (cause, line, detail) in causes {
                let mut example = format!("{}:{}", shown, line);
                if !detail.is_empty() {
                    example.push_str(&format!(" ({})", detail));
                }
                totals.split_causes.entry(cause).or_default().push(example);
            }
        }
    }
}

fn compare_splitters(text: &str, edition: Edition) -> Splitters {
    if try_reorganize(text, &LayoutConfig::default(), false).is_err() {
        return Splitters::NotProcessed;
    }
    let Ok(normalized) = normalize(text) else {
        return Splitters::NotProcessed;
    };
    let source = normalized.text.as_str();
    let (header, body) = split_header(source);
    if body.contains("// mto: off") {
        return Splitters::Skipped;
    }
    let line_of = |offset: usize| source[..offset.min(source.len())].matches('\n').count() + 1;

    // The old scanner's blocks, located in the text, as core spans.
    let mut old = Vec::new();
    let mut cursor = 0;
    for block in extract_blocks(body) {
        let Some(found) = body[cursor..].find(block.raw_content.as_str()) else {
            return Splitters::Compared(vec![(
                "old block not found in the text (scanner bug)".to_string(),
                line_of(header.len() + cursor),
                String::new(),
            )]);
        };
        let start = header.len() + cursor + found;
        cursor += found + block.raw_content.len();
        if let Some((a, b)) = core_span(&block.raw_content, edition) {
            old.push(((start + a, start + b), block));
        }
    }

    // The new parser's items, plus any damaged text it could not make an item of.
    let tree = parse(source, edition).syntax_node();
    let Some(file) = SourceFile::cast(tree.clone()) else {
        return Splitters::Compared(vec![("no source file node".to_string(), 1, String::new())]);
    };
    let mut new: Vec<((usize, usize), Option<Item>)> = Vec::new();
    for child in tree.children() {
        if child.kind() == SyntaxKind::Attr {
            continue;
        }
        let item = Item::cast(child.clone());
        let span = match &item {
            Some(item) => item_core_span(item),
            None => core_span(&child.text(), edition).map(|(a, b)| {
                let start = child.text_range().start as usize;
                (start + a, start + b)
            }),
        };
        if let Some(span) = span {
            new.push((span, item));
        }
    }
    let _ = file;
    // The first line of a span, shortened, to show what each side saw.
    let snippet = |span: (usize, usize)| -> String {
        let text = &source[span.0..span.1];
        let line = text.lines().next().unwrap_or("").trim();
        let short: String = line.chars().take(50).collect();
        if short.len() < line.len() {
            format!("{}...", short)
        } else {
            short
        }
    };

    let mut causes = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < old.len() && j < new.len() {
        let (old_span, block) = &old[i];
        let (new_span, item) = &new[j];
        if old_span == new_span {
            if let Some(item) = item {
                causes.extend(
                    metadata_causes(block, item)
                        .into_iter()
                        .map(|(cause, detail)| (cause, line_of(old_span.0), detail)),
                );
            } else {
                causes.push((
                    "new parser has no item here (damaged text)".to_string(),
                    line_of(old_span.0),
                    snippet(*old_span),
                ));
            }
            i += 1;
            j += 1;
            continue;
        }
        // Boundaries differ: classify by how the spans overlap, then move past both.
        let cause = if old_span.0 == new_span.0 && old_span.1 > new_span.1 {
            "old block spans several items"
        } else if old_span.0 == new_span.0 {
            "old block ends inside an item"
        } else if old_span.0 < new_span.0 {
            "old block starts before the item (non-item text)"
        } else {
            "item starts before the old block"
        };
        causes.push((
            format!("boundaries: {}", cause),
            line_of(old_span.0.min(new_span.0)),
            format!("old `{}`, new `{}`", snippet(*old_span), snippet(*new_span)),
        ));
        let end = old_span.1.max(new_span.1);
        while i < old.len() && old[i].0.0 < end {
            i += 1;
        }
        while j < new.len() && new[j].0.0 < end {
            j += 1;
        }
    }
    if i < old.len() || j < new.len() {
        let at = old
            .get(i)
            .map(|o| o.0.0)
            .or(new.get(j).map(|n| n.0.0))
            .unwrap_or(0);
        causes.push((
            format!(
                "boundaries: {} old block(s) and {} item(s) left over",
                old.len() - i,
                new.len() - j
            ),
            line_of(at),
            String::new(),
        ));
    }
    Splitters::Compared(causes)
}

/// The byte range from a text's first significant token to its last, comments excluded.
fn core_span(text: &str, edition: Edition) -> Option<(usize, usize)> {
    let mut offset = 0;
    let mut first = None;
    let mut last = None;
    for token in tokenize(text, edition) {
        let end = offset + token.len as usize;
        if !token.kind.is_trivia() {
            first.get_or_insert(offset);
            last = Some(end);
        }
        offset = end;
    }
    Some((first?, last?))
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

fn item_core_span(item: &Item) -> Option<(usize, usize)> {
    let mut first = None;
    let mut last = None;
    for element in item.syntax().descendants_with_tokens() {
        if let SyntaxElement::Token(token) = element
            && !token.kind().is_trivia()
        {
            let range = token.text_range();
            first.get_or_insert(range.start as usize);
            last = Some(range.end as usize);
        }
    }
    Some((first?, last?))
}

/// Where the old scanner's view of a block differs from the typed view of the same item:
/// (cause, what each side saw).
fn metadata_causes(block: &CodeBlock, item: &Item) -> Vec<(String, String)> {
    let mut causes = Vec::new();
    let expected_kind = match item.kind() {
        ItemKind::Const => OldKind::ConstDecl,
        ItemKind::Enum => OldKind::EnumDecl,
        ItemKind::Fn => OldKind::FnDecl,
        ItemKind::Impl => OldKind::ImplBlock,
        ItemKind::MacroRules => OldKind::MacroDecl,
        ItemKind::Module => OldKind::ModDecl,
        ItemKind::Static => OldKind::StaticDecl,
        ItemKind::Struct => OldKind::StructDecl,
        ItemKind::Trait => OldKind::TraitDecl,
        ItemKind::Use => OldKind::UseDecl,
        _ => OldKind::Unknown,
    };
    if block.kind != expected_kind {
        let head: String = block
            .raw_content
            .lines()
            .find(|l| !l.trim_start().starts_with("//") && !l.trim_start().starts_with('#'))
            .unwrap_or("")
            .trim()
            .chars()
            .take(50)
            .collect();
        causes.push((
            format!("kind: {:?} read as {:?}", item.kind(), block.kind),
            format!("`{}`", head),
        ));
        return causes;
    }
    // Both sides compared as plain identifiers; the old scanner's `_` means no name.
    let leading_ident = |text: &str| -> String {
        let text = text.strip_prefix("r#").unwrap_or(text);
        let ident: String = text
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if ident == "_" { String::new() } else { ident }
    };
    match item.kind() {
        ItemKind::Impl => {
            let target = item
                .as_impl()
                .and_then(|i| i.self_ty_name())
                .unwrap_or_default();
            if leading_ident(&block.parent_target) != target {
                causes.push((
                    "impl target: differs".to_string(),
                    format!("old {:?}, new {:?}", block.parent_target, target),
                ));
            }
        }
        // The old scanner names no use, and nothing of a kind it does not know.
        _ if item.kind() == ItemKind::Use || expected_kind == OldKind::Unknown => {}
        _ => {
            let name = item.name().map(|n| leading_ident(&n)).unwrap_or_default();
            if leading_ident(&block.identifier) != name {
                causes.push((
                    format!("name: {:?} differs", item.kind()),
                    format!("old {:?}, new {:?}", block.identifier, name),
                ));
            }
        }
    }
    let expected_visibility = match item.visibility() {
        Visibility::Private => OldVisibility::Private,
        Visibility::Pub => OldVisibility::Pub,
        Visibility::PubCrate => OldVisibility::PubCrate,
        Visibility::PubSuper => OldVisibility::PubSuper,
        Visibility::PubSelf | Visibility::PubIn(_) => OldVisibility::Private,
    };
    if block.visibility != expected_visibility {
        causes.push((
            format!(
                "visibility: {:?} read as {:?}",
                item.visibility(),
                block.visibility
            ),
            String::new(),
        ));
    }
    causes
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
