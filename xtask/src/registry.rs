//! Every file under records/ and every generated file, wherever it lives: what it holds,
//! what writes it, what reads it and where it is shown. records/README.md and
//! docs/generated/README.md show their part of this table; `cargo xtask docs` writes it
//! there and `--check` (the `docs` gate's run) fails when a table is stale or a file
//! under records/ or docs/generated/ is not listed.

use std::fs;
use std::path::Path;

pub const ENTRIES: &[Entry] = &[
  Entry {
    path: "records/pins/corpus.txt",
    kind: Kind::Pins,
    holds: "the real-world repositories, each at a commit",
    written_by: "hand, `cargo xtask corpus update`",
    read_by: "corpus, diff, pairs, speed",
    shown_in: "CREDITS.md, metrics pages",
  },
  Entry {
    path: "records/pins/toml-test.txt",
    kind: Kind::Pins,
    holds: "the TOML conformance suite, at a commit",
    written_by: "hand",
    read_by: "toml-test",
    shown_in: "—",
  },
  Entry {
    path: "records/upstream/rustfmt-surface.txt",
    kind: Kind::Snapshot,
    holds: "the standard formatter's options, values and flags, per release",
    written_by: "`cargo xtask capabilities refresh`",
    read_by: "capabilities gate, settings, diff",
    shown_in: "config reference",
  },
  Entry {
    path: "records/policy/capabilities.txt",
    kind: Kind::Policy,
    holds: "how far mto supports each of the standard formatter's items",
    written_by: "hand (`capabilities sync` adds new items)",
    read_by: "capabilities gate, src/options/support.rs",
    shown_in: "config reference, `mto config options`",
  },
  Entry {
    path: "records/policy/disagreements.txt",
    kind: Kind::Policy,
    holds: "test pairs where another answer key disagrees with the suite at stable",
    written_by: "hand, `cargo xtask probe --scan --record` (auto-recorded)",
    read_by: "probe",
    shown_in: "—",
  },
  Entry {
    path: "records/policy/exceptions.txt",
    kind: Kind::Policy,
    holds: "outputs of the standard formatter the meaning check flags, and excused pairs",
    written_by: "hand, `cargo xtask diff` (auto-recorded)",
    read_by: "diff",
    shown_in: "meaning page",
  },
  Entry {
    path: "records/results/compatibility.txt",
    kind: Kind::Result,
    holds: "files and lines matching the standard formatter, per set",
    written_by: "`cargo xtask diff --update`",
    read_by: "diff (the ratchet), release, capabilities report",
    shown_in: "compatibility page, README",
  },
  Entry {
    path: "records/results/ratchet.txt",
    kind: Kind::Result,
    holds: "speed and memory limits, for one named machine",
    written_by: "`cargo xtask metrics ratchet --update`",
    read_by: "metrics ratchet",
    shown_in: "performance page",
  },
  Entry {
    path: "records/results/baseline.txt",
    kind: Kind::Result,
    holds: "lexer and parser speed on the corpus",
    written_by: "`cargo xtask corpus check --baseline`",
    read_by: "metrics pages",
    shown_in: "performance page",
  },
  Entry {
    path: "records/results/history.txt",
    kind: Kind::Result,
    holds: "each tracked record as it stood at every change",
    written_by: "`cargo xtask docs`",
    read_by: "metrics pages",
    shown_in: "the pages' history tables",
  },
  Entry {
    path: "records/archive/2026-10-optimization.txt",
    kind: Kind::Archive,
    holds: "every figure before the October optimization rounds",
    written_by: "—",
    read_by: "`cargo xtask metrics compare`",
    shown_in: "—",
  },
  Entry {
    path: "docs/generated/manual.md",
    kind: Kind::Page,
    holds: "the manual, as `mto --man` prints it",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "src/cli/manual.rs",
  },
  Entry {
    path: "docs/generated/commands.md",
    kind: Kind::Page,
    holds: "every xtask command and its arguments",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "xtask's command table",
  },
  Entry {
    path: "docs/generated/config/README.md",
    kind: Kind::Page,
    holds: "the settings reference's index",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "src/options, records/policy/capabilities.txt",
  },
  Entry {
    path: "docs/generated/config/mto.toml.md",
    kind: Kind::Page,
    holds: "mto.toml's settings",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "src/options (the settings table)",
  },
  Entry {
    path: "docs/generated/config/rustfmt.toml.md",
    kind: Kind::Page,
    holds: "rustfmt.toml's settings, as mto honors them",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "src/options, records/policy/capabilities.txt",
  },
  Entry {
    path: "docs/generated/config/config.toml.md",
    kind: Kind::Page,
    holds: "the per-user config.toml's settings",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "src/options (the settings table)",
  },
  Entry {
    path: "docs/generated/config/mto.schema.json",
    kind: Kind::Page,
    holds: "mto.toml's JSON Schema, for editors",
    written_by: "`cargo xtask docs`",
    read_by: "editors",
    shown_in: "src/options (the settings table)",
  },
  Entry {
    path: "docs/generated/metrics/README.md",
    kind: Kind::Page,
    holds: "the metrics pages' index",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "records/results/*",
  },
  Entry {
    path: "docs/generated/metrics/compatibility.md",
    kind: Kind::Page,
    holds: "compatibility with the standard formatter",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "records/results/compatibility.txt, history",
  },
  Entry {
    path: "docs/generated/metrics/meaning.md",
    kind: Kind::Page,
    holds: "the meaning check and its exceptions",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "records/policy/exceptions.txt, history",
  },
  Entry {
    path: "docs/generated/metrics/performance.md",
    kind: Kind::Page,
    holds: "speed and memory",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "records/results/ratchet.txt, baseline.txt; .dev/metrics",
  },
  Entry {
    path: "docs/generated/metrics/fuzzing.md",
    kind: Kind::Page,
    holds: "fuzz targets and kept regression inputs",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "fuzz/, tests/fuzz_regressions, history; .dev",
  },
  Entry {
    path: "docs/generated/metrics/layout.md",
    kind: Kind::Page,
    holds: "layout's checks",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "the suites; .dev",
  },
  Entry {
    path: "docs/generated/metrics/compile-safety.md",
    kind: Kind::Page,
    holds: "real crates still compiling after mto",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "records/pins/corpus.txt; .dev",
  },
  Entry {
    path: "docs/generated/metrics/scrambled-pairs.md",
    kind: Kind::Page,
    holds: "scrambled corpus pairs",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: ".dev/metrics",
  },
  Entry {
    path: "docs/generated/metrics/dictionary.md",
    kind: Kind::Page,
    holds: "what each metric means",
    written_by: "`cargo xtask docs`",
    read_by: "people",
    shown_in: "xtask/src/papers/dictionary.rs",
  },
  Entry {
    path: "src/options/rustfmt.rs",
    kind: Kind::Code,
    holds: "the standard formatter's settings, from its pinned releases",
    written_by: "`cargo xtask settings`",
    read_by: "mto",
    shown_in: "the standard formatter's sources (records/upstream)",
  },
  Entry {
    path: "src/options/support.rs",
    kind: Kind::Code,
    holds: "how far mto honors each option and value",
    written_by: "`cargo xtask capabilities sync`",
    read_by: "mto (`config options`, warnings)",
    shown_in: "records/policy/capabilities.txt",
  },
  Entry {
    path: "src/text/unicode/ident_tables.rs",
    kind: Kind::Code,
    holds: "identifier character classes",
    written_by: "`cargo xtask unicode`",
    read_by: "mto",
    shown_in: "the Unicode Character Database",
  },
  Entry {
    path: "src/text/unicode/punctuation_tables.rs",
    kind: Kind::Code,
    holds: "other punctuation, where comments may break",
    written_by: "`cargo xtask unicode`",
    read_by: "mto",
    shown_in: "the Unicode Character Database",
  },
  Entry {
    path: "src/text/unicode/width_tables.rs",
    kind: Kind::Code,
    holds: "display widths",
    written_by: "`cargo xtask unicode`",
    read_by: "mto",
    shown_in: "the Unicode Character Database",
  },
];

const END: &str = "<!-- end registry -->";

/// The two READMEs, each with its table.
const READMES: &[(&str, &[Kind])] = &[
  (
    "records/README.md",
    &[
      Kind::Pins,
      Kind::Snapshot,
      Kind::Policy,
      Kind::Result,
      Kind::Archive,
    ],
  ),
  ("docs/generated/README.md", &[Kind::Page, Kind::Code]),
];

const START: &str = "<!-- registry: written by cargo xtask docs from xtask/src/registry.rs -->";

/// What a file is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
  /// What is fetched, at which commit.
  Pins,
  /// What an upstream offers, read from its sources.
  Snapshot,
  /// Kept by hand: what mto supports, what is excused.
  Policy,
  /// Measured; may only improve (the ratchets), or the history of what was measured.
  Result,
  /// Kept as it was; nothing writes it.
  Archive,
  /// A page for people, under docs/generated/.
  Page,
  /// Source code generated into src/.
  Code,
}

impl Kind {
  fn name(self) -> &'static str {
    match self {
      Kind::Pins => "pins",
      Kind::Snapshot => "snapshot",
      Kind::Policy => "policy",
      Kind::Result => "result",
      Kind::Archive => "archive",
      Kind::Page => "page",
      Kind::Code => "code",
    }
  }
}

pub struct Entry {
  pub path: &'static str,
  pub kind: Kind,
  pub holds: &'static str,
  pub written_by: &'static str,
  pub read_by: &'static str,
  /// For a page or code: what it is written from; for a record: where it is shown.
  pub shown_in: &'static str,
}

/// Writes the READMEs' tables, or with `check` reports what is stale or unlisted; the
/// problems found.
pub fn run(root: &Path, check: bool) -> Vec<String> {
  let mut problems: Vec<String> = unlisted(root)
    .into_iter()
    .map(|f| format!("{}: not in xtask/src/registry.rs; list it there", f))
    .collect();
  for (readme, kinds) in READMES {
    let path = root.join(readme);
    let written = fs::read_to_string(&path).unwrap_or_default();
    let fresh = match spliced(&written, kinds) {
      Ok(fresh) => fresh,
      Err(e) => {
        problems.push(format!("{}: {}", readme, e));
        continue;
      }
    };
    if fresh == written {
      continue;
    }
    if check {
      problems.push(format!(
        "{}: its table is out of date; run `cargo xtask docs`",
        readme
      ));
    } else if let Err(e) = fs::write(&path, fresh) {
      problems.push(format!("{}: {}", readme, e));
    } else {
      println!("wrote {}", readme);
    }
  }
  problems
}

/// A README with its table between the markers, the rest as written.
fn spliced(written: &str, kinds: &[Kind]) -> Result<String, String> {
  let start = written.find(START).ok_or("no registry marker")?;
  let end = written.find(END).ok_or("no end marker")?;
  Ok(format!(
    "{}{}\n\n{}\n{}",
    &written[..start],
    START,
    table(kinds),
    &written[end..]
  ))
}

fn table(kinds: &[Kind]) -> String {
  let pages = kinds.contains(&Kind::Page);
  let mut out = String::new();
  for e in ENTRIES.iter().filter(|e| kinds.contains(&e.kind)) {
    let shown = e
      .path
      .strip_prefix("docs/generated/")
      .or_else(|| e.path.strip_prefix("records/"))
      .unwrap_or(e.path);
    let link = if e.kind == Kind::Code {
      format!("`{}`", e.path)
    } else {
      format!("[{}]({})", shown, shown)
    };
    // Each part named, but the empty ones (`—`).
    let parts: Vec<String> = if pages {
      vec![format!("from {}", e.shown_in)]
    } else {
      [
        ("written by", e.written_by),
        ("read by", e.read_by),
        ("shown in", e.shown_in),
      ]
      .iter()
      .filter(|(_, v)| *v != "—")
      .map(|(k, v)| format!("{} {}", k, v))
      .collect()
    };
    let mut text = format!("**{}** ({}): {}.", link, e.kind.name(), e.holds);
    if !parts.is_empty() {
      let joined = parts.join("; ");
      let mut chars = joined.chars();
      let first: String = chars
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
      text.push_str(&format!(" {}{}.", first, chars.as_str()));
    }
    out.push_str(&crate::papers::wrap(&text, 80, "- ", "  "));
  }
  out
}

/// Files under records/ and docs/generated/ that the registry does not list.
fn unlisted(root: &Path) -> Vec<String> {
  let mut found = Vec::new();
  for dir in ["records", "docs/generated"] {
    let mut stack = vec![root.join(dir)];
    while let Some(here) = stack.pop() {
      let Ok(entries) = fs::read_dir(&here) else {
        continue;
      };
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
          stack.push(path);
          continue;
        }
        let rel = path
          .strip_prefix(root)
          .unwrap_or(&path)
          .to_string_lossy()
          .replace('\\', "/");
        let readme = READMES.iter().any(|(r, _)| *r == rel);
        if !readme && !ENTRIES.iter().any(|e| e.path == rel) {
          found.push(rel);
        }
      }
    }
  }
  found.sort();
  found
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_table_replaces_what_is_between_the_markers() {
    let written = format!("# Title\n\nIntro.\n\n{}\nold\n{}\n\nAfter.\n", START, END);
    let fresh = spliced(&written, &[Kind::Archive]).unwrap();
    assert!(fresh.starts_with("# Title\n\nIntro.\n\n"));
    assert!(fresh.contains("2026-10-optimization.txt"));
    assert!(!fresh.contains("old"));
    assert!(fresh.ends_with(&format!("{}\n\nAfter.\n", END)));
  }

  #[test]
  fn test_the_repository_lists_every_record_and_page() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    assert_eq!(unlisted(root), Vec::<String>::new());
    for e in ENTRIES {
      assert!(
        root.join(e.path).exists(),
        "{} is listed but missing",
        e.path
      );
    }
  }
}
