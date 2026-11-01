//! The sources: where the cases come from (the standard formatter's test pairs, the
//! corpus as fetched and scrambled), each with its input, edition and mto's settings.

use std::fs;
use std::path::{Path, PathBuf};

use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::options::format::{FormatConfig, NewlineStyle};
use mtindo::syntax::{Edition, SyntaxElement, parse};

use super::*;
use crate::common::{edition_for, header_edition};
use crate::unformat::unformat;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Source {
  /// The standard formatter's test pairs with default settings, against their expected
  /// files.
  Tests,
  /// Its test pairs with `// rustfmt-` settings.
  TestsWithSettings,
  /// The corpus as fetched, against the standard formatter's output.
  Corpus,
  /// The corpus with its whitespace scrambled.
  Unformatted,
  /// Corpus files scrambled by a seed, items and whitespace (`crate::scramble`), against
  /// the standard formatter's first output.
  Pairs,
  /// Files whose tokens are held against rustc's own lexer.
  Lexer,
  /// Files only parsed, for the grammar's gaps.
  Parse,
}

impl Source {
  pub(crate) const ALL: [Source; 4] = [
    Source::Tests,
    Source::TestsWithSettings,
    Source::Corpus,
    Source::Unformatted,
  ];

  pub(crate) fn key(self) -> &'static str {
    match self {
      Source::Tests => "tests",
      Source::TestsWithSettings => "tests-with-settings",
      Source::Corpus => "corpus",
      Source::Unformatted => "corpus-unformatted",
      Source::Pairs => "pairs",
      Source::Lexer => "lexer",
      Source::Parse => "parse",
    }
  }

  /// What a case of this source is held against, and what is measured.
  pub(crate) fn plan(self) -> Plan {
    match self {
      Source::Tests | Source::TestsWithSettings => Plan {
        key: Key::Expected,
        measures: &[Measure::Match, Measure::Meaning, Measure::Stable],
      },
      // The corpus's answer is where the standard formatter's runs settle, cached.
      Source::Corpus | Source::Unformatted => Plan {
        key: Key::Standard {
          settle: true,
          cached: true,
        },
        measures: &[Measure::Match, Measure::Meaning, Measure::Stable],
      },
      // A scrambled pair is new every run: its first answer, run fresh.
      Source::Pairs => Plan {
        key: Key::Standard {
          settle: false,
          cached: false,
        },
        measures: &[Measure::Match],
      },
      Source::Lexer => Plan {
        key: Key::RustcLexer,
        measures: &[Measure::Tokens],
      },
      Source::Parse => Plan {
        key: Key::Nothing,
        measures: &[Measure::Parses],
      },
    }
  }

  pub(crate) fn title(self) -> &'static str {
    match self {
      Source::Tests => "test pairs, default settings",
      Source::TestsWithSettings => "test pairs with settings",
      Source::Corpus => "corpus",
      Source::Unformatted => "corpus, unformatted",
      Source::Pairs => "scrambled pairs",
      Source::Lexer => "against rustc's lexer",
      Source::Parse => "parsed",
    }
  }
}

/// One input to score.
pub(crate) struct Case {
  pub(crate) source: Source,
  pub(crate) name: String,
  pub(crate) input: String,
  pub(crate) edition: Edition,
  /// Where the standard formatter runs, for its configuration lookup.
  pub(crate) dir: PathBuf,
  /// The answer, when it is given rather than computed (test pairs).
  pub(crate) expected: Option<String>,
  /// The formatting settings mto uses for the case: its folder's, as mto resolves them.
  pub(crate) format: FormatConfig,
  /// For a scrambled pair: its crate, its path in it, and how many items the scramble
  /// moved.
  pub(crate) group: String,
  pub(crate) relative: PathBuf,
  pub(crate) moved: usize,
  /// Not scored, and why: (what, why), as a scrambled pair's line names it.
  pub(crate) skip: Option<(&'static str, String)>,
}

impl Case {
  /// A case with nothing in it, to fill in.
  pub(crate) fn empty() -> Case {
    Case {
      source: Source::Corpus,
      name: String::new(),
      input: String::new(),
      edition: Edition::E2015,
      dir: PathBuf::new(),
      expected: None,
      format: FormatConfig::default(),
      group: String::new(),
      relative: PathBuf::new(),
      moved: 0,
      skip: None,
    }
  }
}

/// One of the standard formatter's test pairs, as the harness scores it.
pub struct Pair {
  pub input: String,
  /// The expected file, with the line endings the header asks for.
  pub expected: String,
  pub edition: Edition,
  /// The settings mto formats the input with: its header's.
  pub format: FormatConfig,
}

/// Every `.rs` file under `dir`, without `.git` and `target` folders.
pub fn collect_rs(dir: &Path, files: &mut Vec<PathBuf>) {
  let Ok(entries) = fs::read_dir(dir) else {
    return;
  };
  for entry in entries.flatten() {
    let path = entry.path();
    let Ok(kind) = entry.file_type() else {
      continue;
    };
    if kind.is_dir() {
      let name = entry.file_name();
      if name != ".git" && name != "target" {
        collect_rs(&path, files);
      }
    } else if kind.is_file() && path.extension().is_some_and(|e| e == "rs") {
      files.push(path);
    }
  }
}

pub fn edition_year(edition: Edition) -> &'static str {
  match edition {
    Edition::E2015 => "2015",
    Edition::E2018 => "2018",
    Edition::E2021 => "2021",
    _ => "2024",
  }
}

/// Whether a test pair, named relative to the suite's `target` folder, is a module its
/// parent's run never formats, whose expected text is its source left alone: the standard
/// formatter's own test runner leaves these out.
pub fn never_formatted(relative: &Path) -> bool {
  [
    "configs/skip_children/foo/mod.rs",
    "issue-3253/paths/excluded.rs",
  ]
  .iter()
  .any(|child| relative == Path::new(child))
}

/// The pair `relative` names under the suite `tests` (`tests/target/RELATIVE` and its
/// input in `tests/source/`, or the target itself when it has none), whose expected file
/// reads `expected`.
pub fn test_pair(tests: &Path, relative: &Path, expected: String) -> Pair {
  let source_dir = tests.join("source");
  let input = fs::read_to_string(source_dir.join(relative)).unwrap_or(expected.clone());
  let mut format = header_settings(&input, &tests.join("config"));
  // A module beside a crate root is formatted as the root's run reaches it, with the
  // macros and attributes the root's own attributes skip.
  let folder = source_dir.join(relative).parent().map(Path::to_path_buf);
  if let Some(root) = folder.map(|f| f.join("lib.rs"))
    && root != source_dir.join(relative)
    && let Ok(text) = fs::read_to_string(&root)
  {
    (format.crate_skipped_macros, format.crate_skipped_attributes) =
      mtindo::cli::modules::crate_skips(&text);
  }
  // The test runner reads a pair's settings from its header alone, so a pair is 2015 (the
  // default edition, which sets the style edition too) unless the header says otherwise;
  // the crate the suite sits in has nothing to do with it.
  let edition = header_edition(&input).unwrap_or(Edition::E2015);
  // The pairs are stored with `\n` endings whatever the header asks for; the
  // standard formatter's own test runner ignores endings.
  let expected =
    if format.newline_style == NewlineStyle::Windows && !strings_span_lines(&expected, edition) {
      expected.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
      expected
    };
  Pair {
    input,
    expected,
    edition,
    format,
  }
}

/// Every input, in a fixed order: the test pairs, then the corpus as fetched, then unformatted.
pub(crate) fn collect_cases(options: &Options) -> Vec<Case> {
  let mut cases = Vec::new();
  // Settings as mto resolves them for each folder, with no user configuration.
  let mut resolver = Resolver::new(&Overrides::default(), Environment::default()).ok();
  // The corpus is answered by the stable release, which ignores nightly-only settings; mto
  // honors them on every channel, so they are dropped here (S3-2).
  let mut format_for = |dir: &Path| -> FormatConfig {
    resolver
      .as_mut()
      .and_then(|r| r.config_for(dir).ok())
      .map(|c| c.stable_only().format())
      .unwrap_or_default()
  };
  if let Some(tests) = &options.tests {
    let target_dir = tests.join("target");
    let mut targets = Vec::new();
    collect_rs(&target_dir, &mut targets);
    targets.sort();
    for target in targets {
      let Ok(expected) = fs::read_to_string(&target) else {
        continue;
      };
      let relative = target.strip_prefix(&target_dir).unwrap_or(&target);
      if never_formatted(relative) {
        continue;
      }
      let pair = test_pair(tests, relative, expected);
      let set = if pair.input.lines().any(|l| l.starts_with("// rustfmt-")) {
        Source::TestsWithSettings
      } else {
        Source::Tests
      };
      cases.push(Case {
        source: set,
        name: format!("tests/{}", relative.display()),
        edition: pair.edition,
        format: pair.format,
        input: pair.input,
        dir: tests.to_path_buf(),
        expected: Some(pair.expected),
        ..Case::empty()
      });
    }
  }
  let mut corpus = Vec::new();
  for (name, dir) in &options.repos {
    let mut files = Vec::new();
    collect_rs(dir, &mut files);
    files.sort();
    for file in files {
      let Ok(text) = fs::read_to_string(&file) else {
        continue;
      };
      let shown = format!(
        "{}/{}",
        name,
        file.strip_prefix(dir).unwrap_or(&file).display()
      );
      if fixture(&shown) {
        continue;
      }
      corpus.push((file, shown, text));
    }
  }
  for (file, shown, text) in &corpus {
    cases.push(Case {
      source: Source::Corpus,
      name: shown.clone(),
      edition: edition_for(file, text),
      input: text.clone(),
      dir: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
      expected: None,
      format: format_for(file.parent().unwrap_or(Path::new("."))),
      ..Case::empty()
    });
  }
  for (file, shown, text) in &corpus {
    let edition = edition_for(file, text);
    let Some(messy) = unformat(text, edition) else {
      continue;
    };
    cases.push(Case {
      source: Source::Unformatted,
      name: shown.clone(),
      edition,
      input: messy,
      dir: file.parent().unwrap_or(Path::new(".")).to_path_buf(),
      expected: None,
      format: format_for(file.parent().unwrap_or(Path::new("."))),
      ..Case::empty()
    });
  }
  cases
}

/// mto's formatting settings for files in `folder`, as the harness resolves them: the
/// stable ones, since the answer is the stable standard formatter's.
pub(crate) fn config_for(folder: &Path) -> FormatConfig {
  Resolver::new(&Overrides::default(), Environment::default())
    .ok()
    .and_then(|mut r| r.config_for(folder).ok())
    .map(|c| c.stable_only().format())
    .unwrap_or_default()
}

/// The files `seed` draws from the crate in `dir`: `count` of them (all for 0), as paths
/// relative to `dir`, sorted.
pub(crate) fn draw(dir: &Path, name: &str, seed: u64, count: usize) -> Vec<PathBuf> {
  let mut files = Vec::new();
  crate::corpus::collect(dir, &mut files);
  let mut files: Vec<PathBuf> = files
    .into_iter()
    .filter_map(|f| f.strip_prefix(dir).ok().map(Path::to_path_buf))
    .collect();
  files.sort();
  if count > 0 && count < files.len() {
    let mut random = crate::scramble::Random::new(seed ^ fnv(name.as_bytes()));
    random.shuffle(&mut files);
    files.truncate(count);
    files.sort();
  }
  files
}

/// The files of `paths` (folders walked, build output and `.git` passed over), or with
/// none, every file of the repository at `root`, sorted, each a case of `source`. The
/// edition is the one given, else each file's crate's; `fixtures` leaves in the test
/// fixtures, which hold deliberately broken files. A file that cannot be read is a case
/// to skip.
pub(crate) fn file_cases(
  root: &Path,
  paths: &[PathBuf],
  source: Source,
  edition: Option<Edition>,
  fixtures: bool,
) -> Result<Vec<Case>, String> {
  let mut files = Vec::new();
  if paths.is_empty() {
    files.extend(
      crate::common::repo_rust_files(root)?
        .into_iter()
        .filter(|f| fixtures || !f.components().any(|c| c.as_os_str() == "fixtures")),
    );
  }
  for path in paths {
    collect_files(path, &mut files);
  }
  files.sort();
  Ok(
    files
      .into_iter()
      .map(|file| {
        let name = file.display().to_string();
        match fs::read_to_string(&file) {
          Ok(text) => Case {
            source,
            name,
            edition: edition.unwrap_or_else(|| edition_for(&file, &text)),
            input: text,
            ..Case::empty()
          },
          Err(e) => Case {
            source,
            name,
            skip: Some(("unreadable", e.to_string())),
            ..Case::empty()
          },
        }
      })
      .collect(),
  )
}

/// The seed one file is scrambled with: from the run's seed, the crate and the file alone.
pub(crate) fn file_seed(seed: u64, name: &str, relative: &Path) -> u64 {
  let mut key = seed.to_le_bytes().to_vec();
  key.extend(name.as_bytes());
  key.push(0);
  key.extend(relative.to_string_lossy().as_bytes());
  fnv(&key)
}

/// Whether a corpus file, named as the report shows it, is a test fixture, left out of the
/// corpus sets: the standard formatter's own test inputs (scored as test pairs, with their
/// settings) and rust-analyzer's `test_data` folders, written to be odd rather than to be
/// formatted.
pub(crate) fn fixture(shown: &str) -> bool {
  shown.starts_with("rustfmt/tests/")
    || shown.starts_with("rust-analyzer/") && shown.contains("/test_data/")
}

/// The formatting settings a test pair's `// rustfmt-NAME: VALUE` header lines set, resolved
/// as mto resolves `--config NAME=VALUE`: the same widths, heuristics and defaults a user
/// gets. A header mto cannot read leaves the defaults.
pub(crate) fn header_settings(input: &str, config_dir: &Path) -> FormatConfig {
  // Each header mto accepts on its own command line; one it does not know is left out
  // rather than failing the rest.
  let accepted = |pair: &(String, String)| {
    let one = Overrides {
      settings: vec![pair.clone()],
      ..Overrides::default()
    };
    Resolver::new(&one, Environment::default()).is_ok()
  };
  let settings: Vec<(String, String)> = input
    .lines()
    .filter_map(|line| line.strip_prefix("// rustfmt-"))
    .filter_map(|rest| rest.split_once(':'))
    .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
    .filter(accepted)
    .collect();
  // `// rustfmt-config: file.toml` names a file in the suite's `config` folder, read as
  // `--config-path` reads one.
  let config_path = input
    .lines()
    .filter_map(|line| line.strip_prefix("// rustfmt-config:"))
    .map(|name| config_dir.join(name.trim()))
    .next();
  let overrides = Overrides {
    config_path,
    settings,
    ..Overrides::default()
  };
  // An empty folder, so that no file's settings mix in.
  let dir = std::env::temp_dir().join("mtindo-diff-headers");
  let _ = fs::create_dir_all(&dir);
  Resolver::new(&overrides, Environment::default())
    .ok()
    .and_then(|mut r| r.config_for(&dir).ok())
    .map(|c| c.format())
    .unwrap_or_default()
}

/// The scrambled pairs a seed draws: `files` files of each crate (all for 0), each
/// scrambled with a seed of its own, so one crate of a run can be replayed alone. A file
/// that cannot be scrambled is a case to skip, with why.
pub(crate) fn pair_cases(crates: &[(String, PathBuf)], seed: u64, files: usize) -> Vec<Case> {
  let mut cases = Vec::new();
  for (name, dir) in crates {
    if let Some(downloads) = dir.parent().and_then(Path::parent) {
      fence(downloads);
    }
    for relative in draw(dir, name, seed, files) {
      let file = dir.join(&relative);
      let folder = file.parent().unwrap_or(Path::new(".")).to_path_buf();
      let mut case = Case {
        source: Source::Pairs,
        name: relative.display().to_string(),
        dir: folder.clone(),
        group: name.clone(),
        relative: relative.clone(),
        ..Case::empty()
      };
      let Ok(text) = fs::read_to_string(&file) else {
        case.skip = Some(("skipped", "not UTF-8".to_string()));
        cases.push(case);
        continue;
      };
      let edition = edition_for(&file, &text);
      case.edition = edition;
      match crate::scramble::scramble(&text, edition, file_seed(seed, name, &relative)) {
        crate::scramble::Outcome::Scrambled { text, moved } => {
          case.input = text;
          case.moved = moved;
          case.format = config_for(&folder);
        }
        crate::scramble::Outcome::Skipped(why) => case.skip = Some(("skipped", why)),
        crate::scramble::Outcome::Bug(what) => case.skip = Some(("scrambler-bug", what)),
      }
      cases.push(case);
    }
  }
  cases
}

/// Whether a string literal holds a line break: line endings inside it are its contents.
pub(crate) fn strings_span_lines(text: &str, edition: Edition) -> bool {
  parse(text, edition)
    .syntax_node()
    .descendants_with_tokens()
    .iter()
    .any(|e| match e {
      SyntaxElement::Token(t) => t.kind().is_literal() && t.text().contains('\n'),
      SyntaxElement::Node(_) => false,
    })
}

/// The `.rs` files under `path` (or `path` itself), skipping build output (a `target`
/// beside a Cargo.toml, not test folders that happen to be called so, such as the
/// standard formatter's expected outputs) and `.git`.
fn collect_files(path: &Path, files: &mut Vec<PathBuf>) {
  if path.is_file() {
    if path.extension().is_some_and(|ext| ext == "rs") {
      files.push(path.to_path_buf());
    }
    return;
  }
  let Ok(entries) = fs::read_dir(path) else {
    return;
  };
  for entry in entries.flatten() {
    let child = entry.path();
    let name = entry.file_name().to_string_lossy().to_string();
    let is_build_dir = name == "target" && path.join("Cargo.toml").exists();
    if is_build_dir || name == ".git" {
      continue;
    }
    if child.is_dir() {
      collect_files(&child, files);
    } else if child.extension().is_some_and(|ext| ext == "rs") {
      files.push(child);
    }
  }
}

/// FNV-1a.
fn fnv(bytes: &[u8]) -> u64 {
  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  for byte in bytes {
    hash ^= u64::from(*byte);
    hash = hash.wrapping_mul(0x0100_0000_01b3);
  }
  hash
}
