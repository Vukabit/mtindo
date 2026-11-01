//! rustfmt's surface: every option, option value and command-line flag, merged from several
//! sources and stored in `records/upstream/rustfmt-surface.txt`.
//!
//! An item line is `KIND NAME in=SOURCES stable=SOURCES [hidden]`. `in` lists the sources that
//! have the item and `stable` those that call it stable; `all` and `none` abbreviate. Flag
//! names carry their program (`rustfmt:--check`, `cargo-fmt:--all`); value names are
//! `OPTION=VALUE`. The file keeps every reading intact, so an offline refresh can carry the
//! upstream readings forward.

use std::collections::{BTreeMap, BTreeSet};

/// Column widths for item lines, fixed so that a new long name does not realign the file.
pub const KIND_WIDTH: usize = 8;

/// Source labels, in the order they are listed.
pub const LABELS: &[&str] = &["local", "local-nightly", "stable", "nightly"];

pub const NAME_WIDTH: usize = 48;

const HEADER: &str = "\
# rustfmt's options, option values and command-line flags, written by
# `cargo xtask sync rustfmt`; do not edit. `in` lists the sources that have
# an item and `stable` the sources that call it stable. Only release sources know
# whether a value is stable. `hidden` marks options rustfmt leaves out of its own
# listings (deprecated or internal).
";

/// Item kinds, in the order they are listed. `mto` items are mto's own features; they are
/// read from the repository at check time, never stored in the surface file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
  Option,
  Value,
  Flag,
  Mto,
}

impl Kind {
  pub fn name(self) -> &'static str {
    match self {
      Kind::Option => "option",
      Kind::Value => "value",
      Kind::Flag => "flag",
      Kind::Mto => "mto",
    }
  }

  pub fn parse(text: &str) -> Option<Kind> {
    match text {
      "option" => Some(Kind::Option),
      "value" => Some(Kind::Value),
      "flag" => Some(Kind::Flag),
      "mto" => Some(Kind::Mto),
      _ => None,
    }
  }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
  pub sources: BTreeSet<String>,
  pub stable: BTreeSet<String>,
  pub hidden: bool,
}

/// One item as one source reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
  pub kind: Kind,
  pub name: String,
  pub stable: bool,
  pub hidden: bool,
}

impl Found {
  pub fn new(kind: Kind, name: impl Into<String>, stable: bool) -> Found {
    Found {
      kind,
      name: name.into(),
      stable,
      hidden: false,
    }
  }
}

/// Everything one source reports.
#[derive(Clone, Debug)]
pub struct Reading {
  pub label: String,
  pub description: String,
  pub items: Vec<Found>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Surface {
  /// (label, description), in [`LABELS`] order.
  pub sources: Vec<(String, String)>,
  pub items: BTreeMap<(Kind, String), Entry>,
}

impl Surface {
  pub fn from_readings(readings: &[Reading]) -> Surface {
    let mut surface = Surface::default();
    let mut ordered: Vec<&Reading> = readings.iter().collect();
    ordered.sort_by_key(|r| label_rank(&r.label));
    for reading in ordered {
      surface
        .sources
        .push((reading.label.clone(), reading.description.clone()));
      for found in reading.items.iter() {
        let entry = surface
          .items
          .entry((found.kind, found.name.clone()))
          .or_default();
        entry.sources.insert(reading.label.clone());
        if found.stable {
          entry.stable.insert(reading.label.clone());
        }
        entry.hidden |= found.hidden;
      }
    }
    surface
  }

  pub fn has_source(&self, label: &str) -> bool {
    self.sources.iter().any(|(l, _)| l == label)
  }

  pub fn parse(text: &str) -> Result<Surface, String> {
    let mut surface = Surface::default();
    for (idx, line) in text.lines().enumerate() {
      let line_no = idx + 1;
      let trimmed = line.trim();
      if trimmed.is_empty() || trimmed.starts_with('#') {
        continue;
      }
      let mut words = trimmed.split_whitespace();
      let first = words.next().unwrap_or("");
      if first == "source" {
        let label = words.next().unwrap_or("");
        if !LABELS.contains(&label) {
          return Err(format!("line {}: unknown source `{}`", line_no, label));
        }
        let description = words.collect::<Vec<_>>().join(" ");
        surface.sources.push((label.to_string(), description));
        continue;
      }
      let kind =
        Kind::parse(first).ok_or_else(|| format!("line {}: unknown kind `{}`", line_no, first))?;
      let name = words
        .next()
        .ok_or_else(|| format!("line {}: missing name", line_no))?;
      let mut entry = Entry::default();
      for word in words {
        if let Some(list) = word.strip_prefix("in=") {
          entry.sources = surface.expand(list, line_no)?;
        } else if let Some(list) = word.strip_prefix("stable=") {
          entry.stable = surface.expand(list, line_no)?;
        } else if word == "hidden" {
          entry.hidden = true;
        } else {
          return Err(format!("line {}: unexpected `{}`", line_no, word));
        }
      }
      if entry.sources.is_empty() {
        return Err(format!("line {}: `{}` has no sources", line_no, name));
      }
      surface.items.insert((kind, name.to_string()), entry);
    }
    Ok(surface)
  }

  /// What source `label` reported, rebuilt from the merged file.
  pub fn reading(&self, label: &str) -> Option<Reading> {
    let (_, description) = self.sources.iter().find(|(l, _)| l == label)?;
    let items = self
      .items
      .iter()
      .filter(|(_, entry)| entry.sources.contains(label))
      .map(|((kind, name), entry)| Found {
        kind: *kind,
        name: name.clone(),
        stable: entry.stable.contains(label),
        hidden: entry.hidden,
      })
      .collect();
    Some(Reading {
      label: label.to_string(),
      description: description.clone(),
      items,
    })
  }

  pub fn render(&self) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    let label_width = LABELS.iter().map(|l| l.len()).max().unwrap_or(0) + 2;
    for (label, description) in self.sources.iter() {
      out.push_str(&format!(
        "source  {:<width$}{}\n",
        label,
        description,
        width = label_width
      ));
    }
    let mut previous = None;
    for ((kind, name), entry) in self.items.iter() {
      if previous != Some(*kind) {
        out.push('\n');
        previous = Some(*kind);
      }
      let mut line = format!(
        "{:<kw$}{:<nw$} in={} stable={}",
        kind.name(),
        name,
        self.abbreviate(&entry.sources),
        self.abbreviate(&entry.stable),
        kw = KIND_WIDTH,
        nw = NAME_WIDTH
      );
      if entry.hidden {
        line.push_str(" hidden");
      }
      out.push_str(&line);
      out.push('\n');
    }
    out
  }

  fn abbreviate(&self, labels: &BTreeSet<String>) -> String {
    if labels.is_empty() {
      return "none".to_string();
    }
    if labels.len() == self.sources.len() {
      return "all".to_string();
    }
    let mut ordered: Vec<&String> = labels.iter().collect();
    ordered.sort_by_key(|l| label_rank(l));
    ordered
      .iter()
      .map(|l| l.as_str())
      .collect::<Vec<_>>()
      .join(",")
  }

  fn expand(&self, list: &str, line_no: usize) -> Result<BTreeSet<String>, String> {
    match list {
      "none" => Ok(BTreeSet::new()),
      "all" => Ok(self.sources.iter().map(|(l, _)| l.clone()).collect()),
      _ => {
        let mut set = BTreeSet::new();
        for label in list.split(',') {
          if !self.has_source(label) {
            return Err(format!(
              "line {}: `{}` is not a listed source",
              line_no, label
            ));
          }
          set.insert(label.to_string());
        }
        Ok(set)
      }
    }
  }
}

pub fn label_rank(label: &str) -> usize {
  LABELS
    .iter()
    .position(|l| *l == label)
    .unwrap_or(LABELS.len())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn reading(label: &str, items: &[(Kind, &str, bool)]) -> Reading {
    Reading {
      label: label.to_string(),
      description: format!("rustfmt for {}", label),
      items: items
        .iter()
        .map(|(kind, name, stable)| Found::new(*kind, *name, *stable))
        .collect(),
    }
  }

  #[test]
  fn test_parse_rejects_unknown_sources_and_kinds() {
    let text = "source local x\noption a in=stable stable=none\n";
    assert!(
      Surface::parse(text)
        .unwrap_err()
        .contains("not a listed source")
    );
    assert!(Surface::parse("thing a in=all\n").is_err());
    assert!(Surface::parse("source moon x\n").is_err());
  }

  #[test]
  fn test_readings_can_be_rebuilt() {
    let original = reading(
      "stable",
      &[
        (Kind::Option, "a", true),
        (Kind::Option, "b", false),
        (Kind::Flag, "cargo-fmt:--all", true),
      ],
    );
    let surface = Surface::from_readings(&[
      original.clone(),
      reading("local", &[(Kind::Option, "c", true)]),
    ]);
    let parsed = Surface::parse(&surface.render()).unwrap();
    let rebuilt = parsed.reading("stable").unwrap();
    assert_eq!(rebuilt.description, original.description);
    assert_eq!(rebuilt.items, original.items);
    assert!(parsed.reading("nightly").is_none());
  }

  #[test]
  fn test_render_and_parse_round_trip() {
    let mut hidden = Found::new(Kind::Option, "merge_imports", false);
    hidden.hidden = true;
    let mut nightly = reading(
      "nightly",
      &[
        (Kind::Option, "max_width", true),
        (Kind::Value, "group_imports=One", false),
        (Kind::Flag, "rustfmt:--check", true),
      ],
    );
    nightly.items.push(hidden);
    let surface = Surface::from_readings(&[
      nightly,
      reading("local", &[(Kind::Option, "max_width", true)]),
    ]);
    let text = surface.render();
    assert!(text.contains("\nsource  local          rustfmt for local\n"));
    assert!(text.contains("option  max_width"));
    assert!(text.contains(" in=all stable=all\n"));
    assert!(text.contains(" in=nightly stable=none hidden\n"));
    assert_eq!(Surface::parse(&text).unwrap(), surface);
  }
}
