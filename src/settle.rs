//! Layer 5: whether mto's output for a file is safe and settled, without writing anything:
//! `mto debug settle` and `cargo xtask corpus try`.
//!
//! A file goes through the run a real one would (layout, formatting and the verifier, with
//! panics caught), then through it again with its own output, as many times as asked. Every
//! outcome is told apart, so none hides another: as written, changed and settled, left
//! alone (and why), the check failing (and which rules), a panic, or a second run changing
//! the output again. Nothing goes to history or the bug reports.

use crate::text::json::Json;
use crate::{Settings, shape, syntax};

/// How one file came through.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
  /// Nothing to change.
  AsWritten,
  /// Changed, and the next runs change nothing.
  Settled,
  /// Not laid out or formatted, for this reason (syntax errors, a duplicate macro).
  LeftAlone(String),
  /// The verifier stopped the write: what it said.
  CheckFailed(String),
  /// Laying the file out panicked: what it said.
  Panicked(String),
  /// Run `run` (from 2) changed what the one before gave.
  Unsettled { run: usize },
}

impl Outcome {
  /// The outcome's name in JSON and in the summary.
  pub fn id(&self) -> &'static str {
    match self {
      Outcome::AsWritten => "as_written",
      Outcome::Settled => "settled",
      Outcome::LeftAlone(_) => "left_alone",
      Outcome::CheckFailed(_) => "check_failed",
      Outcome::Panicked(_) => "panicked",
      Outcome::Unsettled { .. } => "unsettled",
    }
  }
}

/// One file's result.
#[derive(Clone, Debug)]
pub struct Settle {
  pub outcome: Outcome,
  /// The units the first run moved.
  pub moves: usize,
  /// When unsettled: the output of the run before, and of the run that changed it.
  pub texts: Option<(String, String)>,
}

/// A count of each outcome over many files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tally {
  pub files: usize,
  pub as_written: usize,
  pub settled: usize,
  pub left_alone: usize,
  pub check_failed: usize,
  pub panicked: usize,
  pub unsettled: usize,
  pub moves: usize,
}

impl Tally {
  pub fn add(&mut self, settle: &Settle) {
    self.files += 1;
    self.moves += settle.moves;
    match settle.outcome {
      Outcome::AsWritten => self.as_written += 1,
      Outcome::Settled => self.settled += 1,
      Outcome::LeftAlone(_) => self.left_alone += 1,
      Outcome::CheckFailed(_) => self.check_failed += 1,
      Outcome::Panicked(_) => self.panicked += 1,
      Outcome::Unsettled { .. } => self.unsettled += 1,
    }
  }

  /// No check failed, nothing panicked, everything settled.
  pub fn clean(&self) -> bool {
    self.check_failed == 0 && self.panicked == 0 && self.unsettled == 0
  }

  pub fn json(&self) -> Json {
    Json::object()
      .with("files", self.files)
      .with("as_written", self.as_written)
      .with("settled", self.settled)
      .with("left_alone", self.left_alone)
      .with("check_failed", self.check_failed)
      .with("panicked", self.panicked)
      .with("unsettled", self.unsettled)
      .with("moves", self.moves)
  }
}

/// `source` run through mto with `settings`, then `runs - 1` more times over its own output.
pub fn settle(source: &str, settings: &Settings, runs: usize) -> Settle {
  let first = match once(source, settings) {
    Ok(done) => done,
    Err(outcome) => {
      return Settle {
        outcome,
        moves: 0,
        texts: None,
      };
    }
  };
  let moves = first.1;
  if first.0 == source {
    return Settle {
      outcome: Outcome::AsWritten,
      moves,
      texts: None,
    };
  }
  let mut text = first.0;
  for run in 2..=runs.max(2) {
    let next = match once(&text, settings) {
      Ok((next, _)) => next,
      Err(outcome) => {
        return Settle {
          outcome,
          moves,
          texts: None,
        };
      }
    };
    if next != text {
      return Settle {
        outcome: Outcome::Unsettled { run },
        moves,
        texts: Some((text, next)),
      };
    }
    text = next;
  }
  Settle {
    outcome: Outcome::Settled,
    moves,
    texts: None,
  }
}

/// One run over `source`: the text it gives and how many units moved, or how it failed.
fn once(source: &str, settings: &Settings) -> Result<(String, usize), Outcome> {
  let _parses = syntax::ParseCache::open();
  let shaped = match crate::quietly(true, || {
    // Nothing held back: the build guard plays no part here.
    shape(source, settings, &Default::default())
  }) {
    Err((said, _)) => return Err(Outcome::Panicked(said)),
    Ok(Err(reason)) => return Err(Outcome::LeftAlone(reason)),
    Ok(Ok(shaped)) => shaped,
  };
  if shaped.skipped {
    return Err(Outcome::LeftAlone(
      "the file skips itself (`#![rustfmt::skip]`)".to_string(),
    ));
  }
  let checked = crate::quietly(true, || {
    crate::verified_all(
      source,
      &shaped.laid_out,
      &shaped.text,
      shaped.relaid.as_ref(),
      shaped.edition,
      settings,
      "<settle>",
    )
  });
  match checked {
    Err((said, _)) => Err(Outcome::Panicked(said)),
    Ok(Err(bug)) => Err(Outcome::CheckFailed(bug.title())),
    Ok(Ok(())) => Ok((shaped.text, shaped.moves.len())),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::options::layout::LayoutConfig;

  fn layout_only() -> Settings {
    Settings {
      layout: Some(LayoutConfig::default()),
      format: None,
      edition: syntax::Edition::E2021,
    }
  }

  #[test]
  fn test_each_outcome_is_told_apart() {
    let settings = layout_only();
    let laid = "fn a() {}\n\nfn b() {}\n";
    assert_eq!(settle(laid, &settings, 2).outcome, Outcome::AsWritten);
    let moved = settle("fn b() {}\n\nfn a() {}\n", &settings, 3);
    assert_eq!((moved.outcome.clone(), moved.moves), (Outcome::Settled, 1));
    assert!(matches!(
      settle("fn a( {\n", &settings, 2).outcome,
      Outcome::LeftAlone(_)
    ));
    let mut tally = Tally::default();
    tally.add(&moved);
    tally.add(&settle(laid, &settings, 2));
    assert_eq!((tally.files, tally.settled, tally.as_written), (2, 1, 1));
    assert!(tally.clean());
  }
}
