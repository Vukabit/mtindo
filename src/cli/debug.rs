//! `mto debug units`, `moves` and `settle`: what layout sees in a file and why, where each
//! unit went and by which rule, and whether the output is safe and settled. The views are
//! for people and are not stable; `--json` gives the same as a versioned contract.

use crate::layout::Inspection;
use crate::settle::{Outcome, Settle, Tally};
use crate::text::color::{paint, tone};

/// `mto debug moves FILE`: each unit that moves, where to, and the rule that put it there.
pub fn moves(seen: &Inspection, file: &str, on: bool) -> String {
  if seen.moves.is_empty() {
    return format!("{}: nothing moves\n", file);
  }
  let mut out = format!(
    "{}: {} unit(s) move, the fewest out of order; the rest keep their order\n",
    file,
    seen.moves.len()
  );
  for moved in &seen.moves {
    let to = match &moved.before {
      Some((kind, name)) => format!("before {} {}", kind, name),
      None => "to the end".to_string(),
    };
    out.push_str(&format!(
      "  {} {}  {}  {}\n      {}\n",
      moved.kind,
      paint(on, tone::ID, &moved.name),
      paint(on, tone::QUIET, &format!("line {}", moved.line)),
      to,
      paint(on, tone::KEY, moved.rule.says())
    ));
  }
  out
}

/// One file's line in `mto debug settle`.
pub fn settled(file: &str, settle: &Settle, on: bool) -> String {
  let moves = match settle.moves {
    0 => String::new(),
    1 => " · 1 move".to_string(),
    n => format!(" · {} moves", n),
  };
  let what = match &settle.outcome {
    Outcome::AsWritten => paint(on, tone::QUIET, "as written"),
    Outcome::Settled => format!("{}{}", paint(on, tone::GOOD, "changed · settles"), moves),
    Outcome::LeftAlone(why) => paint(on, tone::QUIET, &format!("left alone: {}", why)),
    Outcome::CheckFailed(said) => paint(on, "1;31", &format!("check FAILED: {}", said)),
    Outcome::Panicked(said) => paint(on, "1;31", &format!("PANICKED: {}", said)),
    Outcome::Unsettled { run } => paint(
      on,
      tone::WARN,
      &format!("changed{} · run {} changes it again", moves, run),
    ),
  };
  format!("{}  {}\n", file, what)
}

/// The closing count of `mto debug settle`.
pub fn tally(tally: &Tally) -> String {
  format!(
    "{} file(s): {} changed and settled, {} as written, {} left alone, {} check failed, \
     {} panicked, {} not settled; {} move(s)\n",
    tally.files,
    tally.settled,
    tally.as_written,
    tally.left_alone,
    tally.check_failed,
    tally.panicked,
    tally.unsettled,
    tally.moves
  )
}

/// `mto debug units FILE`: a table per container (the file, then each inline module).
pub fn units(seen: &Inspection, file: &str, on: bool) -> String {
  let mut out = String::new();
  for container in &seen.containers {
    let indent = "  ".repeat(container.depth);
    let title = if container.path.is_empty() {
      format!("{} (edition {})", file, seen.edition.year())
    } else {
      container.path.clone()
    };
    out.push_str(&format!("{}{}\n", indent, paint(on, tone::HEADING, &title)));
    if let Some((from, to)) = container.header {
      out.push_str(&format!(
        "{}  {}\n",
        indent,
        paint(
          on,
          tone::QUIET,
          &format!("header, lines {}-{}: kept first", from, to)
        )
      ));
    }
    let rows: Vec<[String; 5]> = container
      .units
      .iter()
      .enumerate()
      .map(|(at, unit)| {
        let lines = if unit.lines.0 == unit.lines.1 {
          unit.lines.0.to_string()
        } else {
          format!("{}-{}", unit.lines.0, unit.lines.1)
        };
        [
          at.to_string(),
          lines,
          unit.kind.to_string(),
          if unit.visibility.is_empty() {
            unit.name.clone()
          } else {
            format!("{} {}", unit.visibility, unit.name)
          },
          unit.notes.join("; "),
        ]
      })
      .collect();
    let width = |column: usize, heading: &str| {
      rows
        .iter()
        .map(|row| row[column].chars().count())
        .chain([heading.len()])
        .max()
        .unwrap_or(0)
    };
    let widths = [
      width(0, "#"),
      width(1, "LINES"),
      width(2, "KIND"),
      width(3, "NAME").min(40),
    ];
    let cell = |text: &str, width: usize| format!("{:<width$}", shorten(text, width));
    out.push_str(&format!(
      "{}  {}\n",
      indent,
      paint(
        on,
        tone::HEADING,
        &format!(
          "{}  {}  {}  {}  NOTES",
          cell("#", widths[0]),
          cell("LINES", widths[1]),
          cell("KIND", widths[2]),
          cell("NAME", widths[3])
        )
      )
    ));
    for row in &rows {
      let line = format!(
        "{}  {}  {}  {}  {}",
        paint(on, tone::ID, &cell(&row[0], widths[0])),
        paint(on, tone::QUIET, &cell(&row[1], widths[1])),
        cell(&row[2], widths[2]),
        cell(&row[3], widths[3]),
        row[4]
      );
      out.push_str(&format!("{}  {}\n", indent, line.trim_end()));
    }
    if container.tail {
      out.push_str(&format!(
        "{}  {}\n",
        indent,
        paint(
          on,
          tone::QUIET,
          "tail: comments after the last item, kept last"
        )
      ));
    }
  }
  out
}

/// `text` cut to `width` characters, ending in `...` when cut.
fn shorten(text: &str, width: usize) -> String {
  if text.chars().count() <= width || width < 4 {
    return text.to_string();
  }
  let kept: String = text.chars().take(width - 3).collect();
  format!("{}...", kept)
}
