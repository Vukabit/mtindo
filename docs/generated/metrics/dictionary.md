# Data dictionary

Every front-matter field and every figure of the [metrics papers](README.md),
defined once: its unit, its exact definition, and where it is measured and
recorded. A paper's front matter may hold only what is defined here; the
generator refuses to write anything else. This list is generated from the
same definitions the generator checks against.

Percentages are rounded to two decimals; MB means 10^6 bytes. The input sets
of the compatibility measurement are named as its record names them, so
`corpus-unformatted` is the set the papers call corpus-scrambled: corpus
files with the whitespace between their tokens scrambled.

## Front-matter fields

<!-- generated: fields -->
### `paper`

- Unit: text.
- Definition: The paper's id: its file name under docs/generated/metrics/
  without `.md`.
- Measured: Written by `cargo xtask docs`.

### `title`

- Unit: text.
- Definition: The paper's title, as its first heading shows it.
- Measured: Written by `cargo xtask docs`.

### `status`

- Unit: text.
- Definition: `measured` when the paper's figures come from recorded data;
  `planned` when the repository records no data for it yet and the paper only
  describes the planned method.
- Measured: Written by `cargo xtask docs`.

### `generated`

- Unit: UTC timestamp (ISO 8601).
- Definition: When `cargo xtask docs` last changed the page. A regeneration that
  changes nothing else keeps it, and the staleness gate does not compare it.
- Measured: The clock of the machine that ran the command.

### `commit`

- Unit: tree identity.
- Definition: The tree the page was last changed from: the short commit hash,
  followed by `-dirty.` and the first 8 hex digits of a SHA-256 of the
  uncommitted changes when there were any. Like `generated`, kept when nothing
  else changes and not compared by the gate.
- Measured: `git` in the checkout that ran the command.

### `command`

- Unit: command line.
- Definition: The command that measures the data the paper reads and records it
  where the paper finds it. The paper's generator never runs it.
- Measured: Fixed per paper.

### `sources`

- Unit: list of paths.
- Definition: The tracked files and folders the front matter and the checked
  regions are read from, with their git history. The staleness gate depends on
  these alone.
- Measured: Fixed per paper.

### `toolchain`

- Unit: map of versions.
- Definition: The versions of the tools the data was measured with, as the
  records keep them. A tool whose version the records do not keep is not listed.
- Measured: Read from the records named in `sources`.

### `toolchain.reference_formatter`

- Unit: version.
- Definition: The version of the standard formatter the compatibility record was
  made against: the `reference` line of records/results/compatibility.txt
  without the program's name.
- Measured: `cargo xtask compare -- --update`, which asks the formatter for its
  version.

### `machine`

- Unit: text.
- Definition: The machine the timed figures were recorded on: operating system,
  architecture and logical cores. Timed limits apply only on that machine.
- Measured: The `machine` line of records/results/ratchet.txt, written by `cargo
  xtask metrics ratchet --update`.

### `corpus_fingerprint`

- Unit: `sha256:` and 16 hex digits.
- Definition: The start of the SHA-256 of the corpus pin list: one `NAME COMMIT`
  line, ending in a line feed, for each repository of records/pins/corpus.txt
  that is not opt-in, in file order. Equal fingerprints mean the same corpus
  files.
- Measured: Computed from records/pins/corpus.txt by `cargo xtask docs`.

### `seed`

- Unit: text.
- Definition: How the measurement's randomness is seeded: a number, a rule that
  derives one per input, or `none` when the measurement uses no randomness.
- Measured: Fixed per paper, from what the measuring code does.

### `metrics`

- Unit: map of figures.
- Definition: The paper's figures, each by a name defined under Metrics below.
- Measured: Read from the records named in `sources`.

### `limits`

- Unit: map of figures.
- Definition: The performance ratchet: for each series named, the value
  records/results/ratchet.txt holds it to. A new measurement may be worse than
  this by the series' tolerance at most (see the performance paper).
- Measured: `cargo xtask metrics ratchet --update`, from the latest
  measurements.
<!-- end generated -->

## Metrics

<!-- generated: metrics -->
### `compat.<set>.files`

- Unit: percent.
- Definition: Files whose output from mto equals the answer byte for byte, as a
  share of the files scored in the set: `files_matched` over `files_scored`,
  times 100, rounded to two decimals.
- Measured: `cargo xtask compare`; recorded in records/results/compatibility.txt
  by `--update`. The same figure is kept as the series of this name.
- `<set>` is one of `tests`, `tests-with-settings`, `corpus`,
  `corpus-unformatted`.

### `compat.<set>.files_matched`

- Unit: files.
- Definition: Files of the set whose output from mto equals the answer exactly.
- Measured: `cargo xtask compare`; recorded in records/results/compatibility.txt
  by `--update`.
- `<set>` is one of `tests`, `tests-with-settings`, `corpus`,
  `corpus-unformatted`.

### `compat.<set>.files_scored`

- Unit: files.
- Definition: Files of the set that were scored: every input the standard
  formatter answered. The inputs it refused are counted in `skipped` instead.
- Measured: `cargo xtask compare`; recorded in records/results/compatibility.txt
  by `--update`.
- `<set>` is one of `tests`, `tests-with-settings`, `corpus`,
  `corpus-unformatted`.

### `compat.<set>.lines`

- Unit: percent.
- Definition: The answers' lines that mto's outputs keep, as a share of all the
  answers' lines: `lines_matched` over `lines_total`, times 100, rounded to two
  decimals.
- Measured: `cargo xtask compare`; recorded in records/results/compatibility.txt
  by `--update`. The same figure is kept as the series of this name.
- `<set>` is one of `tests`, `tests-with-settings`, `corpus`,
  `corpus-unformatted`.

### `compat.<set>.lines_matched`

- Unit: lines.
- Definition: Summed over the scored files: the length of the longest common
  subsequence of the lines of mto's output and the lines of the answer (every
  line of a file that matches). A line counts wherever it sits, as long as the
  lines counted keep their order.
- Measured: `cargo xtask compare`; recorded in records/results/compatibility.txt
  by `--update`.
- `<set>` is one of `tests`, `tests-with-settings`, `corpus`,
  `corpus-unformatted`.

### `compat.<set>.lines_total`

- Unit: lines.
- Definition: Summed over the scored files: the answer's lines, counted as the
  pieces of its text between line feeds, so that a file ending in a line feed
  counts one empty last line.
- Measured: `cargo xtask compare`; recorded in records/results/compatibility.txt
  by `--update`.
- `<set>` is one of `tests`, `tests-with-settings`, `corpus`,
  `corpus-unformatted`.

### `compat.<set>.skipped`

- Unit: files.
- Definition: Inputs of the set the standard formatter refused (it exited
  non-zero). They are left out of every other figure, and the record holds this
  count down: a run that refuses more fails.
- Measured: `cargo xtask compare`; recorded in records/results/compatibility.txt
  by `--update`. The same figure is kept as the series of this name.
- `<set>` is one of `tests`, `tests-with-settings`, `corpus`,
  `corpus-unformatted`.

### `compat.corpus.repositories`

- Unit: repositories.
- Definition: Repositories of records/pins/corpus.txt that the corpus sets read:
  every pin that is not opt-in.
- Measured: Counted from records/pins/corpus.txt by `cargo xtask docs`.

### `meaning.exceptions.<set>`

- Unit: files.
- Definition: Entries of records/policy/exceptions.txt for the set: files where
  the meaning check flags the standard formatter's own output, each listed with
  its reason. A flagged file not listed fails `cargo xtask compare`.
- Measured: Written by hand after `cargo xtask compare` reports a flagged file.
- `<set>` is one of `tests`, `tests-with-settings`, `corpus`,
  `corpus-unformatted`.

### `meaning.exceptions.total`

- Unit: files.
- Definition: Entries of records/policy/exceptions.txt over every set.
- Measured: Written by hand after `cargo xtask compare` reports a flagged file.

### `meaning.exceptions.unmodelled`

- Unit: files.
- Definition: Entries whose reason says the rewrite is `equivalent` or `not
  modelled`: the formatter kept the meaning in a way the check cannot see yet.
- Measured: Classified from each entry's reason by `cargo xtask docs`.

### `meaning.exceptions.other`

- Unit: files.
- Definition: Every other entry: the formatter changes the meaning or writes
  invalid code, or a test pair's expected file differs from its input beyond
  formatting.
- Measured: Classified from each entry's reason by `cargo xtask docs`.

### `fuzz.targets`

- Unit: targets.
- Definition: Fuzz targets: the `.rs` files directly in fuzz/fuzz_targets/.
- Measured: Counted by `cargo xtask docs`.

### `fuzz.regressions`

- Unit: inputs.
- Definition: Inputs found by fuzzing and kept as regression tests: the files in
  the target folders under tests/fuzz_regressions/. Each runs through its
  target's checks in `cargo test --test fuzz_regressions`.
- Measured: Kept by `cargo xtask incidents promote`; counted by `cargo xtask
  docs`.

### `corpus.lex.mb_per_s`

- Unit: MB/s (10^6 bytes per second).
- Definition: Corpus bytes over seconds for mto's lexer alone, release build,
  one thread.
- Measured: `cargo xtask corpus check --baseline`; recorded in
  records/results/baseline.txt.

### `corpus.lex_parse.mb_per_s`

- Unit: MB/s (10^6 bytes per second).
- Definition: Corpus bytes over seconds for mto's lexer and parser together,
  release build, one thread.
- Measured: `cargo xtask corpus check --baseline`; recorded in
  records/results/baseline.txt.

### `speed.speedup.stable`

- Unit: times.
- Definition: mto's throughput on the laid-out corpus
  (`speed.mto.laid_out.mb_per_s`) over the standard formatter's, stable release,
  run as `cargo fmt --all --check` runs it, one repository at a time; by the
  wall clock. The formatter's bytes are those of the files it says it formats.
- Measured: `cargo xtask speed throughput`.

### `speed.speedup.nightly`

- Unit: times.
- Definition: As `speed.speedup.stable`, against the standard formatter's
  nightly release.
- Measured: `cargo xtask speed throughput`.

### `speed.mto.laid_out.mb_per_s`

- Unit: MB/s (10^6 bytes per second).
- Definition: Corpus bytes over wall seconds for the release `mto --check` on a
  copy of the corpus already laid out, using every core: the median of three
  runs.
- Measured: `cargo xtask speed throughput`.

### `speed.mto.as_fetched.mb_per_s`

- Unit: MB/s (10^6 bytes per second).
- Definition: As `speed.mto.laid_out.mb_per_s`, on the corpus as fetched, where
  most files would change and each is also verified and diffed.
- Measured: `cargo xtask speed throughput`.

### `speed.layout.one_thread.mb_per_s`

- Unit: MB/s (10^6 bytes per second).
- Definition: Corpus bytes over seconds for layout alone, in process, on one
  thread.
- Measured: `cargo xtask speed throughput`.

### `memst.serde.allocs`

- Unit: allocations.
- Definition: Allocation calls while mto lays out and formats the corpus's serde
  repository (layout and formatting on, no configuration from outside), under
  the memst recorder. From 2026-10-25 the whole process is recorded, from the
  start of `main` (its reading of arguments and settings too), not only the
  pipeline: about 1% more.
- Measured: `cargo xtask memst`, with memst, which is not yet published.

### `memst.serde.bytes`

- Unit: bytes.
- Definition: Bytes requested by the allocations of `memst.serde.allocs`,
  summed.
- Measured: `cargo xtask memst`, with memst, which is not yet published.

### `memst.serde.peak_live`

- Unit: bytes.
- Definition: The most bytes allocated and not yet freed at any moment of the
  run of `memst.serde.allocs`.
- Measured: `cargo xtask memst`, with memst, which is not yet published.

### `doctor.held.cost.x`

- Unit: times.
- Definition: The time to lay out and format every file of five corpus crates
  (serde, tokio, clap, syn, regex) while holding back two moves and one run of
  changed lines, as remembered holds do, over the time without holds; one
  thread, in process.
- Measured: `cargo xtask speed doctor`.

### `doctor.narrow.n10.k1.checks`

- Unit: checks.
- Definition: The mean number of builds a narrowing takes to find the moves to
  hold back, over seeded draws (`--trials`, 10 by default) of 10 corpus files
  with 1 move planted as the culprit. The build is a pretend one that breaks
  while a culprit's item comes before the item it was moved in front of.
- Measured: `cargo xtask speed doctor`.

### `doctor.narrow.n100.k1.checks`

- Unit: checks.
- Definition: As `doctor.narrow.n10.k1.checks`, with 100 files and 1 culprit.
- Measured: `cargo xtask speed doctor`.

### `doctor.narrow.n100.k3.checks`

- Unit: checks.
- Definition: As `doctor.narrow.n10.k1.checks`, with 100 files and 3 culprits.
- Measured: `cargo xtask speed doctor`.

### `doctor.narrow.found_rate`

- Unit: fraction (0 to 1).
- Definition: The share of every narrowing measured that ended holding back no
  more than the planted culprits.
- Measured: `cargo xtask speed doctor`.

### `io.write.batch.history.ms_per_file`

- Unit: milliseconds.
- Definition: Wall time per file for the release `mto` to rewrite every file of
  a generated 600-file project with `safety.sync = "batch"` and history on: the
  whole run over the file count, the median of three fresh runs.
- Measured: `cargo xtask speed io`.

### `io.write.batch.plain.ms_per_file`

- Unit: milliseconds.
- Definition: As `io.write.batch.history.ms_per_file`, with history off.
- Measured: `cargo xtask speed io`.

### `perf.scaling.one_thread.mb_per_s`

- Unit: MB/s (10^6 bytes per second).
- Definition: Bytes over seconds for every 8th corpus file laid out and
  formatted in process by one thread: the fastest of three passes. From
  2026-10-25 with the allocator the mto binaries install (`cache_alloc`), as mto
  runs; before, with the system's, about 12 to 17% slower.
- Measured: `cargo xtask speed scaling`.

### `perf.latency.check.median_ms`

- Unit: milliseconds.
- Definition: The median wall time of 200 runs of the release `mto --check FILE`
  on one small file, process start-up included.
- Measured: `cargo xtask speed latency`.

### `perf.growth.<kind>.exponent`

- Unit: exponent (1 is linear).
- Definition: How time grows with size for generated files growing one way: the
  slope of log time against log size over the three largest of five sizes, each
  double the last. `items` adds items, `nesting` nests deeper, `line` lengthens
  one line, `chain` lengthens a method chain and `match` adds match arms.
- Measured: `cargo xtask speed growth`.
- `<kind>` is one of `items`, `nesting`, `line`, `chain`, `match`.
<!-- end generated -->
