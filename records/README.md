# Records

The data mtindo's development tools read and check. Every number a page under
[docs/generated/](../docs/generated/) shows comes from a file here, and the
gates and ratchets hold the tree to them. Nothing machine-specific lives here
but the ratchet's limits, which name the one machine they apply to.

- **pins:** what is fetched, and at which commit.
- **snapshot:** what an upstream offers, read from its sources; never edited.
- **policy:** kept by hand (with a little help from the commands named).
- **result:** measured; the ratchets may only improve, and
  [history.txt](results/history.txt) keeps every record as it stood at each
  change.
- **archive:** kept as it was.

Every file has a header saying its format. Downloads (the corpus, rustfmt's
sources, cached answers) live in `.dev/cache/`, outside the tracked tree.

<!-- registry: written by cargo xtask docs from xtask/src/registry.rs -->

- **[pins/corpus.txt](pins/corpus.txt)** (pins): the real-world repositories,
  each at a commit. Written by hand, `cargo xtask corpus update`; read by
  corpus, diff, pairs, speed; shown in CREDITS.md, metrics pages.
- **[pins/toml-test.txt](pins/toml-test.txt)** (pins): the TOML conformance
  suite, at a commit. Written by hand; read by toml-test.
- **[upstream/rustfmt-surface.txt](upstream/rustfmt-surface.txt)** (snapshot):
  the standard formatter's options, values and flags, per release. Written by
  `cargo xtask capabilities refresh`; read by capabilities gate, settings, diff;
  shown in config reference.
- **[policy/capabilities.txt](policy/capabilities.txt)** (policy): how far mto
  supports each of the standard formatter's items. Written by hand
  (`capabilities sync` adds new items); read by capabilities gate,
  src/options/support.rs; shown in config reference, `mto config options`.
- **[policy/disagreements.txt](policy/disagreements.txt)** (policy): test pairs
  where another answer key disagrees with the suite at stable. Written by hand,
  `cargo xtask probe --scan --record` (auto-recorded); read by probe.
- **[policy/exceptions.txt](policy/exceptions.txt)** (policy): outputs of the
  standard formatter the meaning check flags, and excused pairs. Written by
  hand, `cargo xtask diff` (auto-recorded); read by diff; shown in meaning page.
- **[results/compatibility.txt](results/compatibility.txt)** (result): files and
  lines matching the standard formatter, per set. Written by `cargo xtask diff
  --update`; read by diff (the ratchet), release, capabilities report; shown in
  compatibility page, README.
- **[results/instructions.txt](results/instructions.txt)** (result):
  instructions the release mto runs on a fixed workload, on Linux. Written by
  `cargo xtask perf instructions --update`, from CI's counts; read by perf
  instructions (CI).
- **[results/ratchet.txt](results/ratchet.txt)** (result): speed and memory
  limits, for one named machine. Written by `cargo xtask metrics ratchet
  --update`; read by metrics ratchet; shown in performance page.
- **[results/baseline.txt](results/baseline.txt)** (result): lexer and parser
  speed on the corpus. Written by `cargo xtask corpus check --baseline`; read by
  metrics pages; shown in performance page.
- **[results/history.txt](results/history.txt)** (result): each tracked record
  as it stood at every change. Written by `cargo xtask docs`; read by metrics
  pages; shown in the pages' history tables.
- **[archive/2026-10-optimization.txt](archive/2026-10-optimization.txt)**
  (archive): every figure before the October optimization rounds. Read by `cargo
  xtask metrics compare`.

<!-- end registry -->
