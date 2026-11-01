//! `records/policy/capabilities.txt`: mto's status for every item, kept by hand.
//!
//! A line is `KIND NAME STATUS [# NOTE]`, sorted by kind then name, in fixed columns; the
//! comment block at the top is kept as written. `cargo xtask sync rustfmt` rewrites the
//! file in canonical form.

use super::surface::{KIND_WIDTH, Kind, NAME_WIDTH};

pub const DEFAULT_HEADER: &str = "\
# What mto supports of rustfmt's surface (records/upstream/rustfmt-surface.txt), plus mto's
# own features. One line per item: KIND NAME STATUS [# NOTE]. Checked by
# `cargo xtask gates capabilities`; `cargo xtask sync rustfmt` adds new items
# as missing, drops stale ones and restores the canonical layout.
#
# supported    works as in rustfmt; the note names the test that shows it
# partial      works in part; the note says which part
# missing      not done yet
# not-planned  deliberately left out; the note says why
";

pub const STATUSES: &[Status] = &[
  Status::Supported,
  Status::Partial,
  Status::Missing,
  Status::NotPlanned,
];

const STATUS_WIDTH: usize = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
  Supported,
  Partial,
  Missing,
  NotPlanned,
}

impl Status {
  pub fn name(self) -> &'static str {
    match self {
      Status::Supported => "supported",
      Status::Partial => "partial",
      Status::Missing => "missing",
      Status::NotPlanned => "not-planned",
    }
  }

  fn parse(text: &str) -> Option<Status> {
    STATUSES.iter().copied().find(|s| s.name() == text)
  }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
  pub kind: Kind,
  pub name: String,
  pub status: Status,
  pub note: String,
  pub line_no: usize,
}

#[derive(Debug, Default)]
pub struct Manifest {
  pub header: Vec<String>,
  pub lines: Vec<Line>,
}

impl Manifest {
  /// Parses the file, collecting every malformed line rather than stopping at the first.
  pub fn parse(text: &str) -> (Manifest, Vec<String>) {
    let mut manifest = Manifest::default();
    let mut errors = Vec::new();
    let mut in_header = true;
    for (idx, raw) in text.lines().enumerate() {
      let line_no = idx + 1;
      let trimmed = raw.trim();
      if in_header && (trimmed.starts_with('#') || (trimmed.is_empty() && idx == 0)) {
        manifest.header.push(raw.trim_end().to_string());
        continue;
      }
      in_header = false;
      if trimmed.is_empty() || trimmed.starts_with('#') {
        continue;
      }
      let (fields, note) = match trimmed.split_once('#') {
        Some((fields, note)) => (fields, note.trim()),
        None => (trimmed, ""),
      };
      let words: Vec<&str> = fields.split_whitespace().collect();
      if words.len() != 3 {
        errors.push(format!(
          "records/policy/capabilities.txt:{}: expected KIND NAME STATUS",
          line_no
        ));
        continue;
      }
      let Some(kind) = Kind::parse(words[0]) else {
        errors.push(format!(
          "records/policy/capabilities.txt:{}: unknown kind `{}`",
          line_no, words[0]
        ));
        continue;
      };
      let Some(status) = Status::parse(words[2]) else {
        errors.push(format!(
          "records/policy/capabilities.txt:{}: unknown status `{}` (use {})",
          line_no,
          words[2],
          STATUSES
            .iter()
            .map(|s| s.name())
            .collect::<Vec<_>>()
            .join(", ")
        ));
        continue;
      };
      manifest.lines.push(Line {
        kind,
        name: words[1].to_string(),
        status,
        note: note.to_string(),
        line_no,
      });
    }
    (manifest, errors)
  }

  pub fn render(&self) -> String {
    let mut out = String::new();
    if self.header.is_empty() {
      out.push_str(DEFAULT_HEADER);
    } else {
      for line in self.header.iter() {
        out.push_str(line);
        out.push('\n');
      }
    }
    let mut lines: Vec<&Line> = self.lines.iter().collect();
    lines.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    let mut previous = None;
    for line in lines {
      if previous != Some(line.kind) {
        out.push('\n');
        previous = Some(line.kind);
      }
      let mut text = format!(
        "{:<kw$}{:<nw$} {:<sw$}",
        line.kind.name(),
        line.name,
        line.status.name(),
        kw = KIND_WIDTH,
        nw = NAME_WIDTH,
        sw = STATUS_WIDTH
      );
      if line.note.is_empty() {
        text = text.trim_end().to_string();
      } else {
        text.push_str("# ");
        text.push_str(&line.note);
      }
      out.push_str(&text);
      out.push('\n');
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_empty_header_gets_the_default() {
    let (manifest, _) = Manifest::parse("option a missing\n");
    assert!(manifest.render().starts_with(DEFAULT_HEADER));
  }

  #[test]
  fn test_parse_and_render_canonical_form() {
    let text = "# heading\n\nflag rustfmt:--check supported # tests/domain_8_cli.rs\noption b missing\noption a not-planned   #  why not\n";
    let (manifest, errors) = Manifest::parse(text);
    assert!(errors.is_empty(), "{:?}", errors);
    assert_eq!(manifest.header, vec!["# heading".to_string()]);
    assert_eq!(manifest.lines.len(), 3);
    assert_eq!(manifest.lines[2].note, "why not");
    let rendered = manifest.render();
    let option_a = rendered.find("option  a").unwrap();
    let option_b = rendered.find("option  b").unwrap();
    let flag = rendered.find("flag    rustfmt:--check").unwrap();
    assert!(option_a < option_b && option_b < flag);
    assert!(rendered.contains("missing\n"));
    assert!(rendered.contains("not-planned  # why not\n"));
    let (again, _) = Manifest::parse(&rendered);
    assert_eq!(again.render(), rendered);
  }

  #[test]
  fn test_parse_reports_every_bad_line() {
    let (manifest, errors) =
      Manifest::parse("option a done\nthing b missing\noption c\noption d missing\n");
    assert_eq!(errors.len(), 3, "{:?}", errors);
    assert!(errors[0].contains(":1: unknown status `done`"));
    assert!(errors[1].contains(":2: unknown kind `thing`"));
    assert!(errors[2].contains(":3: expected KIND NAME STATUS"));
    assert_eq!(manifest.lines.len(), 1);
  }
}
