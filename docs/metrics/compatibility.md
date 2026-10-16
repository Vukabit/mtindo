---
paper: "compatibility"
title: "Compatibility with the standard formatter"
status: "measured"
generated: "2026-10-16T02:10:05.133Z"
commit: "47908a6-dirty.a6d8acad"
command: "cargo xtask diff --update"
sources:
  - compat/compatibility.txt
  - compat/corpus.txt
toolchain:
  reference_formatter: "1.9.0-stable (88d9e12ae1 2026-08-18)"
corpus_fingerprint: "sha256:0bfa13269439c574"
seed: "FNV-1a of each file's text (scrambled set only)"
metrics:
  compat.tests.files: 71.21
  compat.tests.files_matched: 329
  compat.tests.files_scored: 462
  compat.tests.lines: 88.55
  compat.tests.lines_matched: 9941
  compat.tests.lines_total: 11227
  compat.tests.skipped: 0
  compat.tests-with-settings.files: 44.72
  compat.tests-with-settings.files_matched: 250
  compat.tests-with-settings.files_scored: 559
  compat.tests-with-settings.lines: 80.82
  compat.tests-with-settings.lines_matched: 17824
  compat.tests-with-settings.lines_total: 22055
  compat.tests-with-settings.skipped: 0
  compat.corpus.files: 95.66
  compat.corpus.files_matched: 9983
  compat.corpus.files_scored: 10436
  compat.corpus.lines: 99.81
  compat.corpus.lines_matched: 2648921
  compat.corpus.lines_total: 2653974
  compat.corpus.skipped: 260
  compat.corpus-unformatted.files: 69.17
  compat.corpus-unformatted.files_matched: 7205
  compat.corpus-unformatted.files_scored: 10416
  compat.corpus-unformatted.lines: 95.22
  compat.corpus-unformatted.lines_matched: 2639854
  compat.corpus-unformatted.lines_total: 2772411
  compat.corpus-unformatted.skipped: 273
  compat.corpus.repositories: 44
---

# Compatibility with the standard formatter

## Abstract

<!-- generated: abstract -->
Against the standard formatter 1.9.0-stable (88d9e12ae1 2026-08-18), mto's
output equals the expected output for 329 of 462 test files (71.21%) and for
9983 of 10436 corpus files (95.66%). On whitespace-scrambled corpus files it
matches 7205 of 10416 files (69.17%) and keeps 95.22% of the expected lines.
<!-- end generated -->

## 1 Question

When mto formats a Rust file, how much of its output is what the standard
formatter writes for the same file with the same settings? mto aims to
reproduce the Rust style exactly, so the answer is measured against the
standard formatter's own output, file by file and line by line, on its test
suite, on real code as published, and on real code with its whitespace
scrambled.

## 2 Method

`cargo xtask diff` builds four input sets, formats every input with mto
(`format = true`) and compares the result with an answer:

- **tests:** the test pairs of the standard formatter's pinned stable release
  (the release compat/rustfmt-surface.txt names; `cargo xtask capabilities
  refresh` caches its source). The input is the pair's source file and the
  answer its expected file. A pair whose input sets no options in its header
  comments belongs to this set.
- **tests-with-settings:** the pairs whose input sets options in header
  comments. mto reads each header as it would read the same setting on its
  command line; a setting it does not know is left out.
- **corpus:** every `.rs` file of the repositories pinned in compat/corpus.txt
  (the opt-in ones excepted), outside `.git` and `target` folders. The answer
  is the standard formatter's output for the file, run from the file's folder
  with the crate's edition and its configuration file, with an empty home
  folder so that no settings of the machine leak in, on the stable channel.
  mto uses the settings it resolves for the same folder, without the options
  only nightly honors.
- **corpus-scrambled** (`corpus-unformatted` in the record): the same files
  with every run of whitespace between two tokens replaced by another run
  (spaces, tabs, line breaks), chosen by a generator seeded with an FNV-1a
  hash of the file's text, so a file always scrambles the same way. Tokens,
  strings and comments are untouched, a run never becomes empty, and a run
  after a line comment keeps a line break. Files holding a carriage return
  are left out. The answer is the standard formatter's output for the
  scrambled text.

Each scored file counts two ways. It **matches** when mto's output equals the
answer byte for byte. Its **lines matched** are the length of the longest
common subsequence of the two texts' lines (all of them for a match), and
its **lines** are the answer's lines, the pieces between line feeds. A set's
shares are summed matches over scored files and summed lines matched over
summed lines.

An input the standard formatter refuses (it exits non-zero) is **skipped**:
left out of the shares and counted on its own. An input it could not run on
at all (it failed to start, was killed or passed its 120-second limit), and
an input mto panics on, fails the run instead, so that neither can pass for
a number. Answers are cached by input, edition, configuration file and
formatter version, so a rerun with an unchanged corpus only runs mto.

`cargo xtask diff --update` writes the four sets' numbers and the reference
version to compat/compatibility.txt. Without `--update` the run compares
with that record and fails when a share falls below it, when more inputs are
skipped, or when the reference version differs. The record can only ratchet
upward, and every change to it is a commit, which is where the history below
comes from.

## 3 Results

The record as it stands (shares in percent):

<!-- generated: results -->
| set | files matching | % | lines matching | % | skipped |
|---|---:|---:|---:|---:|---:|
| tests | 329 of 462 | 71.21 | 9941 of 11227 | 88.55 | 0 |
| tests-with-settings | 250 of 559 | 44.72 | 17824 of 22055 | 80.82 | 0 |
| corpus | 9983 of 10436 | 95.66 | 2648921 of 2653974 | 99.81 | 260 |
| corpus-scrambled | 7205 of 10416 | 69.17 | 2639854 of 2772411 | 95.22 | 273 |
<!-- end generated -->

### History

Lines matching, in percent, for each set at every commit that changed the
record, newest first, and files matching on the scrambled set. The first
row is the record as it stands now; a commit whose record equals it is not
repeated.

<!-- generated: history -->
| date | commit | tests | with settings | corpus | scrambled | scrambled files |
|---|---|---:|---:|---:|---:|---:|
| current |  | 88.55 | 80.82 | 99.81 | 95.22 | 69.17 |
| 2026-10-15 | 47908a6 | 87.94 | 80.63 | 99.80 | 95.02 | 67.97 |
| 2026-10-15 | 0623726 | 87.94 | 80.63 | 99.80 | 94.99 | 67.24 |
| 2026-10-15 | 1ee0fd5 | 87.80 | 80.63 | 99.80 | 94.84 | 65.21 |
| 2026-10-15 | 0cfb09d | 87.72 | 80.63 | 99.80 | 94.78 | 63.52 |
| 2026-10-15 | acd6789 | 87.41 | 80.50 | 99.80 | 94.59 | 62.55 |
| 2026-10-15 | a8c8d39 | 86.15 | 80.13 | 99.79 | 94.12 | 60.97 |
| 2026-10-15 | 44f55b3 | 84.28 | 76.93 | 99.75 | 89.09 | 48.43 |
| 2026-10-14 | 070b4db | 83.85 | 76.58 | 99.74 | 87.33 | 43.93 |
| 2026-10-14 | 6bbbb37 | 83.85 | 76.57 | 99.74 | 87.20 | 40.50 |
| 2026-10-14 | 56cdfee | 82.19 | 75.01 | 99.71 | 85.34 | 35.27 |
| 2026-10-14 | f28bba7 | 81.99 | 74.53 | 99.70 | 83.25 | 34.12 |
| 2026-10-14 | 296655b | 81.46 | 74.01 | 99.69 | 80.45 | 31.93 |
| 2026-10-14 | cfbc4f8 | 81.39 | 73.90 | 99.68 | 80.33 | 31.84 |
| 2026-10-14 | f96926d | 81.38 | 73.87 | 99.68 | 80.23 | 31.64 |
| 2026-10-14 | bfffcc5 | 81.13 | 73.58 | 99.67 | 78.69 | 29.64 |
| 2026-10-13 | 0b22b81 | 81.06 | 73.31 | 99.67 | 78.34 | 29.34 |
| 2026-10-11 | 7bb39cb | 80.96 | 73.28 | 99.67 | 77.67 | 29.25 |
| 2026-10-11 | b117865 | 80.96 | 73.28 | 99.67 | 77.67 | 29.25 |
| 2026-10-11 | 2a809d0 | 80.54 | 69.95 | 99.62 | 75.13 | 25.45 |
| 2026-10-11 | 0769733 | 80.54 | 69.95 | 99.62 | 74.57 | 16.17 |
| 2026-10-11 | a8e8cd0 | 80.38 | 69.77 | 99.62 | 70.80 | 11.90 |
| 2026-10-11 | d392a23 | 80.01 | 69.13 | 99.61 | 69.19 | 7.55 |
| 2026-10-11 | 16d27bc | 79.38 | 68.26 | 99.59 | 56.61 | 2.30 |
| 2026-10-11 | 4f6e097 | 79.33 | 67.73 | 99.58 | 56.29 | 2.17 |
| 2026-10-11 | 57e7c5f | 78.19 | 66.29 | 99.34 | 37.23 | 1.69 |
| 2026-10-11 | fb34927 | 78.18 | 66.21 | 99.33 | 36.52 | 1.33 |
| 2026-10-11 | 19124da | 78.18 | 66.22 | 99.33 | 36.40 | 1.33 |
| 2026-10-10 | 2bed100 | 77.48 | 65.26 | 99.31 | 35.22 | 1.33 |
| 2026-10-10 | d9e04ec | 77.47 | 64.82 | 99.31 | 35.15 | 1.32 |
| 2026-10-10 | 32ee249 | 76.79 | 64.04 | 99.30 | 34.50 | 1.16 |
| 2026-10-10 | b75fdb4 | 74.46 | 62.65 | 99.27 | 28.44 | 0.20 |
<!-- end generated -->

### The last local run

This part is read from the per-file logs of the last `cargo xtask diff` run
on the machine that generated the page (.dev/logs/diff-scores.txt and
.dev/logs/diff-first.txt). Those logs are not tracked, so the staleness gate
does not check this part, and it may describe a newer run than the record.

<!-- generated: last-run -->
From the last `cargo xtask diff` run recorded on the machine that generated this
page (2026-10-16), which need not be the run the record above was made from.
Scored files by the share of their expected lines mto's output keeps:

| set | every line | 99% or more | 90% to 99% | under 90% |
|---|---:|---:|---:|---:|
| tests | 331 | 1 | 24 | 106 |
| tests-with-settings | 258 | 0 | 48 | 253 |
| corpus | 9991 | 43 | 119 | 283 |
| corpus-scrambled | 7312 | 573 | 1575 | 956 |

The syntax at the first line where a mismatching file differs, the ten most
common over every set:

| syntax kind | files |
|---|---:|
| `LetStmt` | 413 |
| `TokenTree` | 292 |
| `Fn` | 264 |
| `MatchArm` | 205 |
| `CallExpr` | 199 |
| `Const` | 161 |
| `BlockExpr` | 160 |
| `MacroExpr` | 143 |
| `Attr` | 137 |
| `Visibility` | 137 |
<!-- end generated -->

## 4 Threats to validity

- **The answer is one release.** Every number is against the version in the
  front matter. Another release formats some constructs differently, and the
  record refuses a run against another version until it is updated, so a
  change of release shows as a step in the history, not as drift.
- **The version is read from the program on the path.** The record names
  the version the formatter on the path reports, while the corpus answers
  come from the stable channel. On a machine whose default toolchain is not
  stable the two can differ.
- **The test expectations are not all formatting.** Some expected files
  differ from their inputs beyond whitespace (see the meaning paper), and
  some test inputs are invalid on purpose. They stay in the sets; they cap
  what mto can match there.
- **Settings mto does not know are dropped.** In tests-with-settings, a
  header mto cannot read is left out, so mto formats that pair with fewer
  settings than its answer had. The set's share partly measures settings
  coverage.
- **Lines matched is generous.** A line counts when it appears in order, not
  where it should be: an output with one item moved can keep nearly all its
  lines. Files matching is the strict measure; read both.
- **Skipped inputs are excluded.** Inputs the standard formatter refuses
  never count against mto, and mto's behavior on them is not scored. The
  record only lets their number shrink.
- **The corpus is a sample.** It is a few dozen popular repositories
  (`compat.corpus.repositories`), pinned to commits. Their style is mostly
  the standard formatter's already, which is why the corpus set scores
  high. The scrambled set removes that advantage and is the better estimate
  for untidy code.
- **Scrambling is one family of mess.** It changes only whitespace between
  tokens. Code written badly in other ways (long lines joined by hand,
  comments in odd places) is not covered by it.

## 5 Reproduction

The measurement needs network access once (the corpus and the standard
formatter's test suite are downloaded and cached) and the standard
formatter on the stable channel.

```bash
# Cache the standard formatter's pinned releases and their test suite
cargo xtask capabilities refresh
# Fetch the corpus, score every set, compare with the record
cargo xtask diff
# The same, as Markdown tables with the mismatches by syntax kind
cargo xtask diff --markdown
# Record the new numbers when they are at or above the old ones
cargo xtask diff --update
# Regenerate this page from the record
cargo xtask docs metrics
```

To look at one file, `cargo run -p xtask-tools -- diff-show FILE` prints
where mto's output first parts from the answer, and `--scrambled` does the
same for the file's scrambled form.

## 6 Changelog

- 2026-10-15: first version, generated from the record and its history.
