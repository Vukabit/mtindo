//! The measures: each case scored against its answer (matching files and lines, the
//! meaning check on the answer and on mto's output, a second run of mto), in parallel,
//! a panic in mto kept as a finding about its input.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use mtindo::options::format::FormatConfig;
use mtindo::syntax::{Edition, SyntaxKind, parse};

use super::key::*;
use super::source::*;
use super::*;

/// What is measured of a case.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Measure {
  /// Files and lines matching the answer.
  Match,
  /// The meaning check, on the answer and on mto's output.
  Meaning,
  /// mto's output unchanged by a second run.
  Stable,
  /// Every token where rustc's lexer puts it.
  Tokens,
  /// Parsed without syntax errors, and back to the same text.
  Parses,
}

/// A source's answer key and measures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Plan {
  pub(crate) key: Key,
  pub(crate) measures: &'static [Measure],
}

/// One scored case.
#[derive(Default)]
pub(crate) struct Score {
  /// The answer could not be had: the standard formatter refused the input.
  pub(crate) skipped: bool,
  /// The standard formatter did not run on the input (it could not start, was killed or
  /// passed its time limit): why. Fails the run.
  pub(crate) unanswered: Option<String>,
  /// mto panicked on the input: the panic message. Fails the run.
  pub(crate) panicked: Option<String>,
  pub(crate) matches: bool,
  pub(crate) lines: usize,
  pub(crate) matched_lines: usize,
  /// For a mismatch: the first differing line (1-based) and the syntax kind there.
  pub(crate) first: Option<(usize, String)>,
  /// The meaning check (S3-3) run on the answer itself: what it reports, if anything.
  pub(crate) meaning: Option<String>,
  /// What is wrong with mto's own output: the meaning check fails on it, or a second run
  /// changes it (S3-3, S3-4).
  pub(crate) own: Option<String>,
  /// The answer is the input as written, and mto changed it: a file mto breaks.
  pub(crate) broke: bool,
  /// The standard formatter's answer changes when it runs on it again: the answer scored is
  /// where its runs settle, which mto, stable, can match (a file to do better on).
  pub(crate) unsettled: bool,
  /// Not scored, as the case said: (what, why).
  pub(crate) skip: Option<(&'static str, String)>,
  /// The standard formatter refused the input, or did not run on it: what it said.
  pub(crate) refused: Option<String>,
  /// Against rustc's lexer: the differences unexplained, those expected, and its tokens.
  pub(crate) tokens: Option<(Vec<String>, Vec<String>, usize)>,
  /// The syntax errors, as `LINE:COLUMN: MESSAGE`.
  pub(crate) errors: Vec<String>,
}

/// How many lines `got` shares with `want` in order: the length of their longest common
/// subsequence, by Myers' O(ND) diff over line ids.
pub fn common_lines<'a>(got: &[&'a str], want: &[&'a str]) -> usize {
  let mut ids: BTreeMap<&'a str, u32> = BTreeMap::new();
  let mut id = |line: &'a str| -> u32 {
    let next = ids.len() as u32;
    *ids.entry(line).or_insert(next)
  };
  let a: Vec<u32> = got.iter().map(|l| id(l)).collect();
  let b: Vec<u32> = want.iter().map(|l| id(l)).collect();
  let (n, m) = (a.len() as isize, b.len() as isize);
  let max = (n + m) as usize;
  let offset = max as isize;
  let mut v = vec![0isize; 2 * max + 2];
  for d in 0..=max as isize {
    let mut k = -d;
    while k <= d {
      let index = (k + offset) as usize;
      let mut x = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
        v[index + 1]
      } else {
        v[index - 1] + 1
      };
      let mut y = x - k;
      while x < n && y < m && a[x as usize] == b[y as usize] {
        x += 1;
        y += 1;
      }
      v[index] = x;
      if x >= n && y >= m {
        return ((n + m - d) / 2) as usize;
      }
      k += 2;
    }
  }
  0
}

/// The kind of the outermost syntax node that starts where line `line` (0-based) of `text`
/// starts its text, an expression statement standing for its expression; `end of file` past
/// the end. A line starting inside a node (a continuation) gives the innermost node holding
/// it.
pub fn kind_at_line(text: &str, line: usize, edition: Edition) -> String {
  let mut offset = 0;
  for (i, l) in text.split('\n').enumerate() {
    if i == line {
      offset += l.len() - l.trim_start().len();
      break;
    }
    offset += l.len() + 1;
  }
  if offset >= text.len() {
    return "end of file".to_string();
  }
  let mut node = parse(text, edition).syntax_node();
  loop {
    let child = node.children().into_iter().find(|c| {
      let range = c.text_range();
      (range.start as usize) <= offset && offset < range.end as usize
    });
    let Some(child) = child else {
      return format!("{:?}", node.kind());
    };
    if child.text_range().start as usize == offset {
      // The outermost node starting here; a statement is named by its expression.
      if child.kind() == SyntaxKind::ExprStmt
        && let Some(expr) = child.children().into_iter().next()
      {
        return format!("{:?}", expr.kind());
      }
      return format!("{:?}", child.kind());
    }
    node = child;
  }
}

/// mto's formatting of `text`, as `format = true` runs it.
pub fn mto_output(text: &str, edition: Edition, config: &FormatConfig) -> String {
  mtindo::format::file::format_file(text, edition, config)
}

/// The text a panic carried.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
  payload
    .downcast_ref::<&str>()
    .map(|s| s.to_string())
    .or_else(|| payload.downcast_ref::<String>().cloned())
    .unwrap_or_else(|| "a panic without a message".to_string())
}

/// The standard formatter's answer for a case, from the cache or a run.
pub(crate) fn reference_output(case: &Case, options: &Options, home: &Path) -> Answer {
  standard_output(
    &case.input,
    case.edition,
    &case.dir,
    &options.reference,
    &options.cache,
    home,
  )
}

pub(crate) fn score(case: &Case, options: &Options, home: &Path) -> Score {
  if let Some((what, why)) = &case.skip {
    return Score {
      skip: Some((what, why.clone())),
      ..Score::default()
    };
  }
  let plan = case.source.plan();
  let measured = |m: Measure| plan.measures.contains(&m);
  match plan.key {
    Key::RustcLexer => {
      return Score {
        tokens: Some(lexer::against_rustc(&case.input, case.edition)),
        ..Score::default()
      };
    }
    Key::Nothing => return parsed(case),
    Key::Standard { cached: false, .. } => return fresh(case, options, home),
    Key::Expected | Key::Standard { cached: true, .. } => {}
  }
  let settle = matches!(plan.key, Key::Standard { settle: true, .. });
  let mut unsettled = false;
  let answer = match &case.expected {
    Some(expected) => expected.clone(),
    None => match reference_output(case, options, home) {
      // Run again on its own output until it settles (a few times at most): that is the
      // answer, the first one being one the standard formatter itself would change.
      Answer::Text(text) => {
        let mut text = text;
        for _ in 0..if settle { 3 } else { 0 } {
          let again = standard_output(
            &text,
            case.edition,
            &case.dir,
            &options.reference,
            &options.cache,
            home,
          );
          match again {
            Answer::Text(next) if next != text => {
              unsettled = true;
              text = next;
            }
            _ => break,
          }
        }
        text
      }
      Answer::Refused(_) => {
        return Score {
          skipped: true,
          ..Score::default()
        };
      }
      Answer::NotRun(why) => {
        return Score {
          unanswered: Some(why),
          ..Score::default()
        };
      }
    },
  };
  let meaning = if !measured(Measure::Meaning) {
    None
  } else {
    let found = mtindo::verify::format::verify_format(&case.input, &answer, case.edition);
    (!found.is_empty()).then(|| {
      found
        .iter()
        .map(|v| format!("{}: {}", v.rule.name(), v.detail))
        .collect::<Vec<_>>()
        .join("; ")
    })
  };
  let output = mto_output(&case.input, case.edition, &case.format);
  let own = {
    let found = if measured(Measure::Meaning) {
      mtindo::verify::format::verify_format(&case.input, &output, case.edition)
    } else {
      Vec::new()
    };
    if !found.is_empty() {
      Some(
        found
          .iter()
          .map(|v| format!("{}: {}", v.rule.name(), v.detail))
          .collect::<Vec<_>>()
          .join("; "),
      )
    } else if measured(Measure::Stable) && mto_output(&output, case.edition, &case.format) != output
    {
      Some("a second run changes it".to_string())
    } else {
      None
    }
  };
  let expected: Vec<&str> = answer.split('\n').collect();
  let got: Vec<&str> = output.split('\n').collect();
  let matches = output == answer;
  let broke = !matches && case.input == answer;
  let matched_lines = if matches {
    expected.len()
  } else {
    common_lines(&got, &expected)
  };
  let first = (!matches).then(|| {
    let line = expected
      .iter()
      .zip(&got)
      .position(|(a, b)| a != b)
      .unwrap_or(expected.len().min(got.len()));
    (line + 1, kind_at_line(&answer, line, case.edition))
  });
  Score {
    skipped: false,
    unanswered: None,
    panicked: None,
    matches,
    lines: expected.len(),
    matched_lines,
    first,
    meaning,
    own,
    broke,
    unsettled,
    ..Score::default()
  }
}

pub(crate) fn score_all(cases: &[Case], options: &Options, home: &Path) -> Vec<Score> {
  let next = AtomicUsize::new(0);
  // Only a long run says how far along it is.
  let progress =
    (cases.len() >= 500).then(|| xtask_core::progress::Progress::new("scoring", cases.len()));
  let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
  let mut scores: Vec<Option<Score>> = (0..cases.len()).map(|_| None).collect();
  let results: Vec<(usize, Score)> = std::thread::scope(|scope| {
    let handles: Vec<_> = (0..threads)
      .map(|_| {
        scope.spawn(|| {
          let mut mine = Vec::new();
          loop {
            let at = next.fetch_add(1, Ordering::Relaxed);
            let Some(case) = cases.get(at) else {
              break;
            };
            // A panic in mto is a finding about that input, not a reason to lose the batch.
            let scored =
              std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| score(case, options, home)))
                .unwrap_or_else(|payload| Score {
                  panicked: Some(panic_message(&*payload)),
                  ..Score::default()
                });
            mine.push((at, scored));
            if let Some(progress) = &progress {
              progress.tick();
            }
          }
          mine
        })
      })
      .collect();
    handles
      .into_iter()
      .flat_map(|h| h.join().unwrap_or_default())
      .collect()
  });
  if let Some(progress) = &progress {
    progress.finish();
  }
  for (at, score) in results {
    scores[at] = Some(score);
  }
  scores
    .into_iter()
    .map(|score| {
      score.unwrap_or_else(|| Score {
        panicked: Some("its worker thread died before scoring it".to_string()),
        ..Score::default()
      })
    })
    .collect()
}

/// A case of a source held against the standard formatter run fresh, once (a scrambled
/// pair): matching files and lines only, its texts written out when the run asks.
fn fresh(case: &Case, options: &Options, home: &Path) -> Score {
  let write = |side: &str, text: &str| {
    if let Some(out) = &options.out {
      let path = out.join(side).join(&case.group).join(&case.relative);
      if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
      }
      let _ = fs::write(path, text);
    }
  };
  write("scrambled", &case.input);
  let answer = match run_standard(&case.input, edition_year(case.edition), &case.dir, home) {
    Answer::Text(answer) => answer,
    Answer::Refused(why) | Answer::NotRun(why) => {
      write("standard", &case.input);
      return Score {
        refused: Some(why),
        ..Score::default()
      };
    }
  };
  write("standard", &answer);
  let output = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    mto_output(&case.input, case.edition, &case.format)
  })) {
    Ok(output) => output,
    Err(payload) => {
      return Score {
        panicked: Some(panic_message(&*payload)),
        ..Score::default()
      };
    }
  };
  write("mto", &output);
  let want: Vec<&str> = answer.split('\n').collect();
  let got: Vec<&str> = output.split('\n').collect();
  let matches = output == answer;
  Score {
    matches,
    lines: want.len(),
    matched_lines: if matches {
      want.len()
    } else {
      common_lines(&got, &want)
    },
    ..Score::default()
  }
}

/// A case only parsed: its syntax errors, and a parse that does not give back the same
/// text, which is a bug of the parser's (reported as a panic is).
fn parsed(case: &Case) -> Score {
  let result = mtindo::syntax::parse(&case.input, case.edition);
  if result.syntax_node().text() != case.input {
    return Score {
      panicked: Some(format!("{} did not round-trip", case.name)),
      ..Score::default()
    };
  }
  let index = mtindo::text::line_index::LineIndex::new(&case.input);
  Score {
    errors: result
      .errors()
      .iter()
      .map(|error| {
        let position = index.line_col(error.range.start as usize);
        format!(
          "{}:{}: {}",
          position.line + 1,
          position.byte_column + 1,
          error.message
        )
      })
      .collect(),
    ..Score::default()
  }
}
