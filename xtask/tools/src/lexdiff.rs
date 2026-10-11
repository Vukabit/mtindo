//! Compares mto's lexer with rustc's own (`ra-ap-rustc_lexer`) on real source files.
//!
//! rustc's lexer does not know about editions, so mto is run for one edition (default 2021)
//! and the two must agree on every token boundary and on a coarse token class.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mtindo::syntax::lexer::tokenize;
use mtindo::syntax::{Edition, SyntaxKind};
use ra_ap_rustc_lexer as rustc;

/// Directories compared when no paths are given.
const DEFAULT_PATHS: &[&str] = &["src", "tests", "benches", "xtask/src", "fuzz/fuzz_targets"];

/// How many differences to print per file.
const SHOWN_PER_FILE: usize = 3;

struct Span {
  start: usize,
  end: usize,
  class: String,
}

/// The unexplained and the expected differences between the two lexers on one file, and
/// how many tokens mto produced.
pub fn differences(text: &str, edition: Edition) -> (Vec<String>, Vec<String>, usize) {
  let ours = mto_spans(text, edition);
  let theirs = rustc_spans(text);
  let (unexplained, expected) = compare(&ours, &theirs, text, edition);
  (unexplained, expected, ours.len())
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
  let mut edition = Edition::E2021;
  let mut paths = Vec::new();
  let mut idx = 0;
  while idx < args.len() {
    match args[idx].as_str() {
      "--edition" if idx + 1 < args.len() => {
        edition = match Edition::from_year(&args[idx + 1]) {
          Some(edition) => edition,
          None => {
            eprintln!("Unknown edition `{}`.", args[idx + 1]);
            return ExitCode::from(2);
          }
        };
        idx += 1;
      }
      path => paths.push(PathBuf::from(path)),
    }
    idx += 1;
  }
  if paths.is_empty() {
    for path in DEFAULT_PATHS {
      paths.push(root.join(path));
    }
  }

  let mut files = Vec::new();
  for path in paths.iter() {
    collect(path, &mut files);
  }
  files.sort();

  let mut compared = 0;
  let mut tokens = 0;
  let mut differing = 0;
  let mut expected_lines: Vec<String> = Vec::new();
  for file in files.iter() {
    let text = match fs::read_to_string(file) {
      Ok(text) => text,
      Err(_) => continue,
    };
    compared += 1;
    let (ours, theirs) = (mto_spans(&text, edition), rustc_spans(&text));
    tokens += theirs.len();
    let (differences, known) = compare(&ours, &theirs, &text, edition);
    for line in known.iter() {
      expected_lines.push(format!("{}: {}", file.display(), line));
    }
    if !differences.is_empty() {
      differing += 1;
      println!("{}:", file.display());
      for line in differences.iter().take(SHOWN_PER_FILE) {
        println!("  {}", line);
      }
      if differences.len() > SHOWN_PER_FILE {
        println!("  ... {} more", differences.len() - SHOWN_PER_FILE);
      }
    }
  }
  if !expected_lines.is_empty() {
    println!("expected differences (rustc's lexer ignores editions and nightly gates):");
    for line in expected_lines.iter() {
      println!("  {}", line);
    }
  }
  println!(
    "lexdiff (edition {}): {} files, {} tokens, {} file(s) differ, {} expected difference(s)",
    edition.year(),
    compared,
    tokens,
    differing,
    expected_lines.len()
  );
  if differing == 0 {
    ExitCode::SUCCESS
  } else {
    ExitCode::from(1)
  }
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
  if path.is_file() {
    if path.extension().is_some_and(|ext| ext == "rs") {
      files.push(path.to_path_buf());
    }
    return;
  }
  let entries = match fs::read_dir(path) {
    Ok(entries) => entries,
    Err(_) => return,
  };
  for entry in entries.flatten() {
    let child = entry.path();
    let name = entry.file_name().to_string_lossy().to_string();
    // Skip build output (a `target` next to a Cargo.toml), but not test directories that
    // happen to be called `target`, such as rustfmt's expected outputs.
    let is_build_dir = name == "target" && path.join("Cargo.toml").exists();
    if is_build_dir || name == ".git" {
      continue;
    }
    if child.is_dir() {
      collect(&child, files);
    } else if child.extension().is_some_and(|ext| ext == "rs") {
      files.push(child);
    }
  }
}

/// Every boundary or class mismatch, with the line it is on, split into unexplained
/// differences and expected ones (see [`expected_difference`]).
fn compare(
  ours: &[Span],
  theirs: &[Span],
  text: &str,
  edition: Edition,
) -> (Vec<String>, Vec<String>) {
  let mut differences = Vec::new();
  let mut expected = Vec::new();
  let mut last_was_expected = false;
  let (mut i, mut j) = (0, 0);
  while i < ours.len() && j < theirs.len() {
    let (a, b) = (&ours[i], &theirs[j]);
    if a.start == b.start && a.end == b.end && a.class == b.class {
      i += 1;
      j += 1;
      continue;
    }
    let line = text[..a.start.min(b.start)].matches('\n').count() + 1;
    let description = format!(
      "line {}: mto {} {:?}, rustc {} {:?}",
      line,
      a.class,
      &text[a.start..a.end],
      b.class,
      &text[b.start..b.end]
    );
    match expected_difference(&b.class, &text[b.start..b.end], edition) {
      Some(reason) => {
        expected.push(format!("{} (expected: {})", description, reason));
        last_was_expected = true;
      }
      None => {
        differences.push(description);
        last_was_expected = false;
      }
    }
    // Resynchronise where both lexers next start a token at the same byte, so one root
    // cause is reported once.
    i += 1;
    j += 1;
    while let (Some(x), Some(y)) = (ours.get(i), theirs.get(j)) {
      if x.start == y.start {
        break;
      }
      if x.start < y.start {
        i += 1;
      } else {
        j += 1;
      }
    }
  }
  if i < ours.len() || j < theirs.len() {
    let line = format!(
      "token counts differ at the end: mto {} left, rustc {} left",
      ours.len() - i,
      theirs.len() - j
    );
    // Running out of shared boundaries right after an expected difference is part of it.
    if last_was_expected {
      expected.push(format!("{} (follows the expected difference above)", line));
    } else {
      differences.push(line);
    }
  }
  (differences, expected)
}

/// Why rustc's edition-agnostic lexer is expected to differ here, if it is: it always produces
/// tokens that only exist in later editions or on nightly, and its parser splits them again.
fn expected_difference(
  rustc_class: &str,
  rustc_text: &str,
  edition: Edition,
) -> Option<&'static str> {
  match rustc_class {
    "forced-keyword" => Some("nightly raw keyword `k#name`; mto lexes a reserved prefix"),
    "guarded-string-prefix" if edition < Edition::E2024 => Some("`#\"` is only reserved from 2024"),
    "guarded-string-prefix" => Some("both reject `#\"`; mto marks the `#` and lexes the string"),
    "lifetime" if edition < Edition::E2021 && rustc_text.starts_with("'r#") => {
      Some("raw lifetimes exist from 2021")
    }
    "literal"
      if edition < Edition::E2021
        && (rustc_text.starts_with("c\"") || rustc_text.starts_with("cr")) =>
    {
      Some("C strings exist from 2021")
    }
    _ => None,
  }
}

fn mto_class(kind: SyntaxKind) -> String {
  match kind {
    SyntaxKind::Whitespace => "whitespace".to_string(),
    SyntaxKind::Frontmatter => "frontmatter".to_string(),
    SyntaxKind::LineComment | SyntaxKind::BlockComment => "comment".to_string(),
    SyntaxKind::OuterLineDoc
    | SyntaxKind::InnerLineDoc
    | SyntaxKind::OuterBlockDoc
    | SyntaxKind::InnerBlockDoc => "doc-comment".to_string(),
    SyntaxKind::Ident | SyntaxKind::RawIdent => "ident".to_string(),
    SyntaxKind::Lifetime => "lifetime".to_string(),
    SyntaxKind::Unknown => "unknown".to_string(),
    kind if kind.is_literal() => "literal".to_string(),
    kind => format!("punct {}", kind.name()),
  }
}

/// mto's tokens after the byte-order mark and shebang, which rustc strips before lexing.
fn mto_spans(text: &str, edition: Edition) -> Vec<Span> {
  let mut spans = Vec::new();
  let mut offset = 0;
  for token in tokenize(text, edition) {
    let end = offset + token.len as usize;
    if token.kind != SyntaxKind::Bom && token.kind != SyntaxKind::Shebang {
      spans.push(Span {
        start: offset,
        end,
        class: mto_class(token.kind),
      });
    }
    offset = end;
  }
  spans
}

fn rustc_class(kind: rustc::TokenKind) -> String {
  use rustc::TokenKind as K;
  let punct = |name: &str| format!("punct {}", name);
  match kind {
    K::Whitespace => "whitespace".to_string(),
    K::LineComment { doc_style: None }
    | K::BlockComment {
      doc_style: None, ..
    } => "comment".to_string(),
    K::LineComment { .. } | K::BlockComment { .. } => "doc-comment".to_string(),
    K::Ident | K::RawIdent | K::UnknownPrefix => "ident".to_string(),
    K::Lifetime { .. } | K::RawLifetime => "lifetime".to_string(),
    K::Literal { .. } => "literal".to_string(),
    K::InvalidIdent => "invalid-ident".to_string(),
    K::ForcedKeywordIdent => "forced-keyword".to_string(),
    K::UnknownPrefixLifetime => "unknown-prefix-lifetime".to_string(),
    K::GuardedStrPrefix => "guarded-string-prefix".to_string(),
    K::Frontmatter { .. } => "frontmatter".to_string(),
    K::Unknown => "unknown".to_string(),
    K::Eof => "eof".to_string(),
    K::Semi => punct("SEMICOLON"),
    K::Comma => punct("COMMA"),
    K::Dot => punct("DOT"),
    K::OpenParen => punct("L_PAREN"),
    K::CloseParen => punct("R_PAREN"),
    K::OpenBrace => punct("L_BRACE"),
    K::CloseBrace => punct("R_BRACE"),
    K::OpenBracket => punct("L_BRACK"),
    K::CloseBracket => punct("R_BRACK"),
    K::At => punct("AT"),
    K::Pound => punct("POUND"),
    K::Tilde => punct("TILDE"),
    K::Question => punct("QUESTION"),
    K::Colon => punct("COLON"),
    K::Dollar => punct("DOLLAR"),
    K::Eq => punct("EQ"),
    K::Bang => punct("BANG"),
    K::Lt => punct("L_ANGLE"),
    K::Gt => punct("R_ANGLE"),
    K::Minus => punct("MINUS"),
    K::And => punct("AMP"),
    K::Or => punct("PIPE"),
    K::Plus => punct("PLUS"),
    K::Star => punct("STAR"),
    K::Slash => punct("SLASH"),
    K::Caret => punct("CARET"),
    K::Percent => punct("PERCENT"),
  }
}

/// rustc's tokens, positioned in the original text: the byte-order mark and shebang are
/// skipped first, as rustc itself does.
fn rustc_spans(text: &str) -> Vec<Span> {
  let mut offset = if text.starts_with('\u{feff}') {
    '\u{feff}'.len_utf8()
  } else {
    0
  };
  offset += rustc::strip_shebang(&text[offset..]).unwrap_or(0);
  let mut spans = Vec::new();
  for token in rustc::tokenize(&text[offset..], rustc::FrontmatterAllowed::Yes) {
    let end = offset + token.len as usize;
    spans.push(Span {
      start: offset,
      end,
      class: rustc_class(token.kind),
    });
    offset = end;
  }
  spans
}
