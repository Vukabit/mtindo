//! The data dictionary: every front-matter field and metric a paper may carry, defined once.
//! `docs/generated/metrics/dictionary.md` is rendered from these tables, and a paper's front matter
//! may hold nothing they do not define.

/// The front-matter fields. A dotted name is a key under the field its first part names.
pub const FIELDS: &[Entry] = &[
  Entry {
    name: "paper",
    unit: "text",
    definition: "The paper's id: its file name under docs/generated/metrics/ without `.md`.",
    measured: "Written by `cargo xtask docs metrics`.",
    values: &[],
  },
  Entry {
    name: "title",
    unit: "text",
    definition: "The paper's title, as its first heading shows it.",
    measured: "Written by `cargo xtask docs metrics`.",
    values: &[],
  },
  Entry {
    name: "status",
    unit: "text",
    definition: "`measured` when the paper's figures come from recorded data; `planned` \
      when the repository records no data for it yet and the paper only describes the \
      planned method.",
    measured: "Written by `cargo xtask docs metrics`.",
    values: &[],
  },
  Entry {
    name: "generated",
    unit: "UTC timestamp (ISO 8601)",
    definition: "When `cargo xtask docs metrics` last changed the page. A regeneration \
      that changes nothing else keeps it, and the staleness gate does not compare it.",
    measured: "The clock of the machine that ran the command.",
    values: &[],
  },
  Entry {
    name: "commit",
    unit: "tree identity",
    definition: "The tree the page was last changed from: the short commit hash, followed \
      by `-dirty.` and the first 8 hex digits of a SHA-256 of the uncommitted changes when \
      there were any. Like `generated`, kept when nothing else changes and not compared \
      by the gate.",
    measured: "`git` in the checkout that ran the command.",
    values: &[],
  },
  Entry {
    name: "command",
    unit: "command line",
    definition: "The command that measures the data the paper reads and records it where \
      the paper finds it. The paper's generator never runs it.",
    measured: "Fixed per paper.",
    values: &[],
  },
  Entry {
    name: "sources",
    unit: "list of paths",
    definition: "The tracked files and folders the front matter and the checked regions \
      are read from, with their git history. The staleness gate depends on these alone.",
    measured: "Fixed per paper.",
    values: &[],
  },
  Entry {
    name: "toolchain",
    unit: "map of versions",
    definition: "The versions of the tools the data was measured with, as the records \
      keep them. A tool whose version the records do not keep is not listed.",
    measured: "Read from the records named in `sources`.",
    values: &[],
  },
  Entry {
    name: "toolchain.reference_formatter",
    unit: "version",
    definition: "The version of the standard formatter the compatibility record was made \
      against: the `reference` line of compat/compatibility.txt without the program's \
      name.",
    measured: "`cargo xtask diff --update`, which asks the formatter for its version.",
    values: &[],
  },
  Entry {
    name: "machine",
    unit: "text",
    definition: "The machine the timed figures were recorded on: operating system, \
      architecture and logical cores. Timed limits apply only on that machine.",
    measured: "The `machine` line of compat/performance.txt, written by `cargo xtask \
      metrics ratchet --update`.",
    values: &[],
  },
  Entry {
    name: "corpus_fingerprint",
    unit: "`sha256:` and 16 hex digits",
    definition: "The start of the SHA-256 of the corpus pin list: one `NAME COMMIT` line, \
      ending in a line feed, for each repository of compat/corpus.txt that is not opt-in, \
      in file order. Equal fingerprints mean the same corpus files.",
    measured: "Computed from compat/corpus.txt by `cargo xtask docs metrics`.",
    values: &[],
  },
  Entry {
    name: "seed",
    unit: "text",
    definition: "How the measurement's randomness is seeded: a number, a rule that \
      derives one per input, or `none` when the measurement uses no randomness.",
    measured: "Fixed per paper, from what the measuring code does.",
    values: &[],
  },
  Entry {
    name: "metrics",
    unit: "map of figures",
    definition: "The paper's figures, each by a name defined under Metrics below.",
    measured: "Read from the records named in `sources`.",
    values: &[],
  },
  Entry {
    name: "limits",
    unit: "map of figures",
    definition: "The performance ratchet: for each series named, the value \
      compat/performance.txt holds it to. A new measurement may be worse than this by \
      the series' tolerance at most (see the performance paper).",
    measured: "`cargo xtask metrics ratchet --update`, from the latest measurements.",
    values: &[],
  },
];

/// The ways `cargo xtask perf growth` grows a generated file.
pub const GROWTHS: &[&str] = &["items", "nesting", "line", "chain", "match"];

/// The metrics, by the names front matter uses for them.
pub const METRICS: &[Entry] = &[
  Entry {
    name: "compat.<set>.files",
    unit: "percent",
    definition: "Files whose output from mto equals the answer byte for byte, as a share \
      of the files scored in the set: `files_matched` over `files_scored`, times 100, \
      rounded to two decimals.",
    measured: "`cargo xtask diff`; recorded in compat/compatibility.txt by `--update`. The \
      same figure is kept as the series of this name.",
    values: SETS,
  },
  Entry {
    name: "compat.<set>.files_matched",
    unit: "files",
    definition: "Files of the set whose output from mto equals the answer exactly.",
    measured: "`cargo xtask diff`; recorded in compat/compatibility.txt by `--update`.",
    values: SETS,
  },
  Entry {
    name: "compat.<set>.files_scored",
    unit: "files",
    definition: "Files of the set that were scored: every input the standard formatter \
      answered. The inputs it refused are counted in `skipped` instead.",
    measured: "`cargo xtask diff`; recorded in compat/compatibility.txt by `--update`.",
    values: SETS,
  },
  Entry {
    name: "compat.<set>.lines",
    unit: "percent",
    definition: "The answers' lines that mto's outputs keep, as a share of all the \
      answers' lines: `lines_matched` over `lines_total`, times 100, rounded to two \
      decimals.",
    measured: "`cargo xtask diff`; recorded in compat/compatibility.txt by `--update`. The \
      same figure is kept as the series of this name.",
    values: SETS,
  },
  Entry {
    name: "compat.<set>.lines_matched",
    unit: "lines",
    definition: "Summed over the scored files: the length of the longest common \
      subsequence of the lines of mto's output and the lines of the answer (every line of \
      a file that matches). A line counts wherever it sits, as long as the lines counted \
      keep their order.",
    measured: "`cargo xtask diff`; recorded in compat/compatibility.txt by `--update`.",
    values: SETS,
  },
  Entry {
    name: "compat.<set>.lines_total",
    unit: "lines",
    definition: "Summed over the scored files: the answer's lines, counted as the pieces \
      of its text between line feeds, so that a file ending in a line feed counts one \
      empty last line.",
    measured: "`cargo xtask diff`; recorded in compat/compatibility.txt by `--update`.",
    values: SETS,
  },
  Entry {
    name: "compat.<set>.skipped",
    unit: "files",
    definition: "Inputs of the set the standard formatter refused (it exited non-zero). \
      They are left out of every other figure, and the record holds this count down: a \
      run that refuses more fails.",
    measured: "`cargo xtask diff`; recorded in compat/compatibility.txt by `--update`. The \
      same figure is kept as the series of this name.",
    values: SETS,
  },
  Entry {
    name: "compat.corpus.repositories",
    unit: "repositories",
    definition: "Repositories of compat/corpus.txt that the corpus sets read: every pin \
      that is not opt-in.",
    measured: "Counted from compat/corpus.txt by `cargo xtask docs metrics`.",
    values: &[],
  },
  Entry {
    name: "meaning.exceptions.<set>",
    unit: "files",
    definition: "Entries of compat/meaning-exceptions.txt for the set: files where the \
      meaning check flags the standard formatter's own output, each listed with its \
      reason. A flagged file not listed fails `cargo xtask diff`.",
    measured: "Written by hand after `cargo xtask diff` reports a flagged file.",
    values: SETS,
  },
  Entry {
    name: "meaning.exceptions.total",
    unit: "files",
    definition: "Entries of compat/meaning-exceptions.txt over every set.",
    measured: "Written by hand after `cargo xtask diff` reports a flagged file.",
    values: &[],
  },
  Entry {
    name: "meaning.exceptions.unmodelled",
    unit: "files",
    definition: "Entries whose reason says the rewrite is `equivalent` or `not modelled`: \
      the formatter kept the meaning in a way the check cannot see yet.",
    measured: "Classified from each entry's reason by `cargo xtask docs metrics`.",
    values: &[],
  },
  Entry {
    name: "meaning.exceptions.other",
    unit: "files",
    definition: "Every other entry: the formatter changes the meaning or writes invalid \
      code, or a test pair's expected file differs from its input beyond formatting.",
    measured: "Classified from each entry's reason by `cargo xtask docs metrics`.",
    values: &[],
  },
  Entry {
    name: "fuzz.targets",
    unit: "targets",
    definition: "Fuzz targets: the `.rs` files directly in fuzz/fuzz_targets/.",
    measured: "Counted by `cargo xtask docs metrics`.",
    values: &[],
  },
  Entry {
    name: "fuzz.regressions",
    unit: "inputs",
    definition: "Inputs found by fuzzing and kept as regression tests: the files in the \
      target folders under tests/fuzz_regressions/. Each runs through its target's checks \
      in `cargo test --test fuzz_regressions`.",
    measured: "Kept by `cargo xtask incidents promote`; counted by `cargo xtask docs \
      metrics`.",
    values: &[],
  },
  Entry {
    name: "corpus.lex.mb_per_s",
    unit: "MB/s (10^6 bytes per second)",
    definition: "Corpus bytes over seconds for mto's lexer alone, release build, one \
      thread.",
    measured: "`cargo xtask corpus check --baseline`; recorded in compat/baseline.txt.",
    values: &[],
  },
  Entry {
    name: "corpus.lex_parse.mb_per_s",
    unit: "MB/s (10^6 bytes per second)",
    definition: "Corpus bytes over seconds for mto's lexer and parser together, release \
      build, one thread.",
    measured: "`cargo xtask corpus check --baseline`; recorded in compat/baseline.txt.",
    values: &[],
  },
  Entry {
    name: "speed.speedup.stable",
    unit: "times",
    definition: "mto's throughput on the laid-out corpus \
      (`speed.mto.laid_out.mb_per_s`) over the standard formatter's, stable release, run \
      as `cargo fmt --all --check` runs it, one repository at a time; by the wall clock. \
      The formatter's bytes are those of the files it says it formats.",
    measured: "`cargo xtask corpus bench`.",
    values: &[],
  },
  Entry {
    name: "speed.speedup.nightly",
    unit: "times",
    definition: "As `speed.speedup.stable`, against the standard formatter's nightly \
      release.",
    measured: "`cargo xtask corpus bench`.",
    values: &[],
  },
  Entry {
    name: "speed.mto.laid_out.mb_per_s",
    unit: "MB/s (10^6 bytes per second)",
    definition: "Corpus bytes over wall seconds for the release `mto --check` on a copy of \
      the corpus already laid out, using every core: the median of three runs.",
    measured: "`cargo xtask corpus bench`.",
    values: &[],
  },
  Entry {
    name: "speed.mto.as_fetched.mb_per_s",
    unit: "MB/s (10^6 bytes per second)",
    definition: "As `speed.mto.laid_out.mb_per_s`, on the corpus as fetched, where most \
      files would change and each is also verified and diffed.",
    measured: "`cargo xtask corpus bench`.",
    values: &[],
  },
  Entry {
    name: "speed.layout.one_thread.mb_per_s",
    unit: "MB/s (10^6 bytes per second)",
    definition: "Corpus bytes over seconds for layout alone, in process, on one thread.",
    measured: "`cargo xtask corpus bench`.",
    values: &[],
  },
  Entry {
    name: "memst.serde.allocs",
    unit: "allocations",
    definition: "Allocation calls while mto lays out and formats the corpus's serde \
      repository (layout and formatting on, no configuration from outside), under the \
      memst recorder.",
    measured: "`cargo xtask memst`.",
    values: &[],
  },
  Entry {
    name: "memst.serde.bytes",
    unit: "bytes",
    definition: "Bytes requested by the allocations of `memst.serde.allocs`, summed.",
    measured: "`cargo xtask memst`.",
    values: &[],
  },
  Entry {
    name: "memst.serde.peak_live",
    unit: "bytes",
    definition: "The most bytes allocated and not yet freed at any moment of the run of \
      `memst.serde.allocs`.",
    measured: "`cargo xtask memst`.",
    values: &[],
  },
  Entry {
    name: "doctor.held.cost.x",
    unit: "times",
    definition: "The time to lay out and format every file of five corpus crates (serde, \
      tokio, clap, syn, regex) while holding back two moves and one run of changed lines, \
      as remembered holds do, over the time without holds; one thread, in process.",
    measured: "`cargo xtask doctor bench`.",
    values: &[],
  },
  Entry {
    name: "doctor.narrow.n10.k1.checks",
    unit: "checks",
    definition: "The mean number of builds a narrowing takes to find the moves to hold \
      back, over seeded draws (`--trials`, 10 by default) of 10 corpus files with 1 move \
      planted as the culprit. The build is a pretend one that breaks while a culprit's \
      item comes before the item it was moved in front of.",
    measured: "`cargo xtask doctor bench`.",
    values: &[],
  },
  Entry {
    name: "doctor.narrow.n100.k1.checks",
    unit: "checks",
    definition: "As `doctor.narrow.n10.k1.checks`, with 100 files and 1 culprit.",
    measured: "`cargo xtask doctor bench`.",
    values: &[],
  },
  Entry {
    name: "doctor.narrow.n100.k3.checks",
    unit: "checks",
    definition: "As `doctor.narrow.n10.k1.checks`, with 100 files and 3 culprits.",
    measured: "`cargo xtask doctor bench`.",
    values: &[],
  },
  Entry {
    name: "doctor.narrow.found_rate",
    unit: "fraction (0 to 1)",
    definition: "The share of every narrowing measured that ended holding back no more \
      than the planted culprits.",
    measured: "`cargo xtask doctor bench`.",
    values: &[],
  },
  Entry {
    name: "io.write.batch.history.ms_per_file",
    unit: "milliseconds",
    definition: "Wall time per file for the release `mto` to rewrite every file of a \
      generated 600-file project with `safety.sync = \"batch\"` and history on: the whole \
      run over the file count, the median of three fresh runs.",
    measured: "`cargo xtask io bench`.",
    values: &[],
  },
  Entry {
    name: "io.write.batch.plain.ms_per_file",
    unit: "milliseconds",
    definition: "As `io.write.batch.history.ms_per_file`, with history off.",
    measured: "`cargo xtask io bench`.",
    values: &[],
  },
  Entry {
    name: "perf.scaling.one_thread.mb_per_s",
    unit: "MB/s (10^6 bytes per second)",
    definition: "Bytes over seconds for every 8th corpus file laid out and formatted in \
      process by one thread: the fastest of three passes.",
    measured: "`cargo xtask perf scaling`.",
    values: &[],
  },
  Entry {
    name: "perf.latency.check.median_ms",
    unit: "milliseconds",
    definition: "The median wall time of 200 runs of the release `mto --check FILE` on one \
      small file, process start-up included.",
    measured: "`cargo xtask perf latency`.",
    values: &[],
  },
  Entry {
    name: "perf.growth.<kind>.exponent",
    unit: "exponent (1 is linear)",
    definition: "How time grows with size for generated files growing one way: the slope \
      of log time against log size over the three largest of five sizes, each double the \
      last. `items` adds items, `nesting` nests deeper, `line` lengthens one line, `chain` \
      lengthens a method chain and `match` adds match arms.",
    measured: "`cargo xtask perf growth`.",
    values: GROWTHS,
  },
];

/// The input sets of `cargo xtask diff`, by the keys its record uses.
pub const SETS: &[&str] = &[
  "tests",
  "tests-with-settings",
  "corpus",
  "corpus-unformatted",
];

/// One defined name. A name holding a placeholder such as `<set>` stands for one name per
/// value in `values`.
pub struct Entry {
  pub name: &'static str,
  pub unit: &'static str,
  pub definition: &'static str,
  /// Where the figure is measured and recorded, or what writes the field.
  pub measured: &'static str,
  pub values: &'static [&'static str],
}

/// Whether `name` is a field (a dotted one is a key under a field).
pub fn is_field(name: &str) -> bool {
  defined(FIELDS, name)
}

/// Whether `name` is a metric.
pub fn is_metric(name: &str) -> bool {
  defined(METRICS, name)
}

/// Every name an entry stands for: its own, or one per value of its placeholder.
pub fn names(entry: &Entry) -> Vec<String> {
  match placeholder(entry.name) {
    Some(hole) => entry
      .values
      .iter()
      .map(|value| entry.name.replace(hole, value))
      .collect(),
    None => vec![entry.name.to_string()],
  }
}

/// The `<...>` an entry's name holds, if any.
pub fn placeholder(name: &str) -> Option<&str> {
  let start = name.find('<')?;
  let end = name[start..].find('>')? + start;
  Some(&name[start..=end])
}

fn defined(entries: &[Entry], name: &str) -> bool {
  entries
    .iter()
    .any(|entry| names(entry).iter().any(|n| n == name))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_every_name_is_defined_once_and_in_full() {
    let mut seen = std::collections::BTreeSet::new();
    for entry in FIELDS.iter().chain(METRICS) {
      assert_eq!(
        placeholder(entry.name).is_some(),
        !entry.values.is_empty(),
        "{}",
        entry.name
      );
      for name in names(entry) {
        assert!(seen.insert(name.clone()), "{} is defined twice", name);
      }
      for text in [entry.unit, entry.definition, entry.measured] {
        assert!(!text.trim().is_empty(), "{}", entry.name);
        assert!(!text.contains("  "), "{}: a doubled space", entry.name);
      }
    }
  }

  #[test]
  fn test_placeholders_expand_to_their_values_only() {
    assert!(is_metric("compat.corpus-unformatted.lines"));
    assert!(is_metric("perf.growth.match.exponent"));
    assert!(!is_metric("compat.corpus-scrambled.lines"));
    assert!(!is_metric("compat.<set>.lines"));
    assert!(is_field("toolchain.reference_formatter"));
    assert!(!is_field("toolchain.rustc"));
  }
}
