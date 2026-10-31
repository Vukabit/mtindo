---
paper: "compatibility"
title: "Compatibility with the standard formatter"
status: "measured"
generated: "2026-10-31T23:39:02.063Z"
commit: "a536de7-dirty.e560aa14"
command: "cargo xtask diff --update"
sources:
  - compat/compatibility.txt
  - compat/corpus.txt
toolchain:
  reference_formatter: "1.10.0-stable (58cf8f9c35 2026-10-13)"
corpus_fingerprint: "sha256:0d26c0baa5577082"
seed: "FNV-1a of each file's text (scrambled set only)"
metrics:
  compat.tests.files: 98.91
  compat.tests.files_matched: 455
  compat.tests.files_scored: 460
  compat.tests.lines: 99.88
  compat.tests.lines_matched: 11192
  compat.tests.lines_total: 11205
  compat.tests.skipped: 0
  compat.tests-with-settings.files: 76.74
  compat.tests-with-settings.files_matched: 429
  compat.tests-with-settings.files_scored: 559
  compat.tests-with-settings.lines: 92.10
  compat.tests-with-settings.lines_matched: 20313
  compat.tests-with-settings.lines_total: 22055
  compat.tests-with-settings.skipped: 0
  compat.corpus.files: 99.97
  compat.corpus.files_matched: 9358
  compat.corpus.files_scored: 9361
  compat.corpus.lines: 100.00
  compat.corpus.lines_matched: 2653089
  compat.corpus.lines_total: 2653185
  compat.corpus.skipped: 6
  compat.corpus-unformatted.files: 99.13
  compat.corpus-unformatted.files_matched: 9267
  compat.corpus-unformatted.files_scored: 9348
  compat.corpus-unformatted.lines: 99.93
  compat.corpus-unformatted.lines_matched: 2774589
  compat.corpus-unformatted.lines_total: 2776485
  compat.corpus-unformatted.skipped: 17
  compat.corpus.repositories: 44
---

# Compatibility with the standard formatter

## Abstract

<!-- generated: abstract -->
Against the standard formatter 1.10.0-stable (58cf8f9c35 2026-10-13), mto's
output equals the expected output for 455 of 460 test files (98.91%) and for
9358 of 9361 corpus files (99.97%). On whitespace-scrambled corpus files it
matches 9267 of 9348 files (99.13%) and keeps 99.93% of the expected lines.
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
| tests | 455 of 460 | 98.91 | 11192 of 11205 | 99.88 | 0 |
| tests-with-settings | 429 of 559 | 76.74 | 20313 of 22055 | 92.10 | 0 |
| corpus | 9358 of 9361 | 99.97 | 2653089 of 2653185 | 100.00 | 6 |
| corpus-scrambled | 9267 of 9348 | 99.13 | 2774589 of 2776485 | 99.93 | 17 |
<!-- end generated -->

### History

Lines matching, in percent, for each set at every commit that changed the
record, newest first, and files matching on the scrambled set. The first
row is the record as it stands now; a commit whose record equals it is not
repeated.

<!-- generated: history -->
| date | commit | tests | with settings | corpus | scrambled | scrambled files |
|---|---|---:|---:|---:|---:|---:|
| current |  | 99.88 | 92.10 | 100.00 | 99.93 | 99.13 |
| 2026-10-31 | 74797b0 | 99.88 | 92.10 | 100.00 | 99.93 | 99.13 |
| 2026-10-31 | 90b0ae0 | 99.88 | 91.80 | 100.00 | 99.93 | 99.13 |
| 2026-10-31 | a9d7145 | 99.88 | 91.53 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | d38fd6d | 99.88 | 89.93 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 206a865 | 99.88 | 89.64 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 96110a6 | 99.88 | 89.15 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | a4e6907 | 99.88 | 88.61 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 2d7693f | 99.88 | 88.39 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 1bc695e | 99.88 | 88.39 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 90dcd40 | 99.88 | 88.37 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 3950f87 | 99.88 | 88.32 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 215423d | 99.88 | 88.11 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 48beb5a | 99.88 | 88.04 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 7b28d61 | 99.88 | 87.93 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 1891833 | 99.88 | 87.89 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 76725a8 | 99.88 | 87.67 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 9ddf976 | 99.88 | 87.67 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 00e97b0 | 99.88 | 87.43 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 6cd3a08 | 99.88 | 87.30 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | f246f93 | 99.88 | 87.28 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | ed8af2a | 99.88 | 87.24 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 15f3d7b | 99.88 | 86.94 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | c7c1f09 | 99.88 | 86.81 | 100.00 | 99.92 | 99.12 |
| 2026-10-31 | 7cdbb97 | 99.88 | 86.81 | 100.00 | 99.91 | 99.09 |
| 2026-10-31 | 56736b3 | 99.88 | 86.81 | 99.99 | 99.90 | 99.06 |
| 2026-10-31 | 5ad8239 | 99.88 | 86.79 | 99.99 | 99.90 | 99.06 |
| 2026-10-31 | 14188ad | 99.88 | 86.70 | 99.99 | 99.90 | 99.06 |
| 2026-10-30 | a22d22c | 99.88 | 86.67 | 99.99 | 99.90 | 99.06 |
| 2026-10-30 | a49d00e | 99.88 | 86.64 | 99.99 | 99.90 | 99.06 |
| 2026-10-30 | a3e5879 | 99.88 | 86.64 | 99.99 | 99.90 | 99.06 |
| 2026-10-21 | 89c3628 | 99.88 | 86.64 | 99.99 | 99.90 | 99.09 |
| 2026-10-19 | 33c3f06 | 99.56 | 86.63 | 99.99 | 99.37 | 99.01 |
| 2026-10-18 | 0973b7d | 99.56 | 86.26 | 99.99 | 99.27 | 96.66 |
| 2026-10-18 | 063f2de | 99.55 | 86.25 | 99.99 | 99.24 | 96.12 |
| 2026-10-18 | fab347d | 99.55 | 86.17 | 99.99 | 99.17 | 95.07 |
| 2026-10-17 | 855e3f0 | 99.13 | 86.16 | 99.99 | 99.14 | 94.84 |
| 2026-10-17 | 8449381 | 97.72 | 86.10 | 99.99 | 99.05 | 94.24 |
| 2026-10-17 | fd5de34 | 97.72 | 85.77 | 99.99 | 98.98 | 93.84 |
| 2026-10-17 | 5e09478 | 96.79 | 85.32 | 99.98 | 98.93 | 93.50 |
| 2026-10-16 | d56f24b | 96.26 | 84.68 | 99.98 | 98.93 | 93.46 |
| 2026-10-16 | d958460 | 94.18 | 83.16 | 99.89 | 97.66 | 82.47 |
| 2026-10-16 | 60002f1 | 90.47 | 81.33 | 99.85 | 97.37 | 80.35 |
| 2026-10-15 | 8556e4d | 88.55 | 80.85 | 99.81 | 95.28 | 69.90 |
| 2026-10-15 | f31de09 | 88.55 | 80.82 | 99.81 | 95.22 | 69.17 |
| 2026-10-15 | ddf690b | 87.94 | 80.63 | 99.80 | 95.02 | 67.97 |
| 2026-10-15 | ce01943 | 87.94 | 80.63 | 99.80 | 94.99 | 67.24 |
| 2026-10-15 | 93fb8ac | 87.80 | 80.63 | 99.80 | 94.84 | 65.21 |
| 2026-10-15 | 65e3945 | 87.72 | 80.63 | 99.80 | 94.78 | 63.52 |
| 2026-10-15 | ee77875 | 87.41 | 80.50 | 99.80 | 94.59 | 62.55 |
| 2026-10-15 | 7db95f6 | 86.15 | 80.13 | 99.79 | 94.12 | 60.97 |
| 2026-10-15 | 6d60bfa | 84.28 | 76.93 | 99.75 | 89.09 | 48.43 |
| 2026-10-14 | 8029c08 | 83.85 | 76.58 | 99.74 | 87.33 | 43.93 |
| 2026-10-14 | f446be9 | 83.85 | 76.57 | 99.74 | 87.20 | 40.50 |
| 2026-10-14 | 2aa79d0 | 82.19 | 75.01 | 99.71 | 85.34 | 35.27 |
| 2026-10-14 | ae6ac20 | 81.99 | 74.53 | 99.70 | 83.25 | 34.12 |
| 2026-10-14 | da2bc6c | 81.46 | 74.01 | 99.69 | 80.45 | 31.93 |
| 2026-10-14 | c98dc85 | 81.39 | 73.90 | 99.68 | 80.33 | 31.84 |
| 2026-10-14 | 5b68749 | 81.38 | 73.87 | 99.68 | 80.23 | 31.64 |
| 2026-10-14 | fd1b526 | 81.13 | 73.58 | 99.67 | 78.69 | 29.64 |
| 2026-10-13 | 90cc2e8 | 81.06 | 73.31 | 99.67 | 78.34 | 29.34 |
| 2026-10-11 | 36c3419 | 80.96 | 73.28 | 99.67 | 77.67 | 29.25 |
| 2026-10-11 | 676c866 | 80.96 | 73.28 | 99.67 | 77.67 | 29.25 |
| 2026-10-11 | eb74fef | 80.54 | 69.95 | 99.62 | 75.13 | 25.45 |
| 2026-10-11 | ca69289 | 80.54 | 69.95 | 99.62 | 74.57 | 16.17 |
| 2026-10-11 | 6a8de89 | 80.38 | 69.77 | 99.62 | 70.80 | 11.90 |
| 2026-10-11 | ecded5d | 80.01 | 69.13 | 99.61 | 69.19 | 7.55 |
| 2026-10-11 | cf3cdcb | 79.38 | 68.26 | 99.59 | 56.61 | 2.30 |
| 2026-10-11 | 262fd9e | 79.33 | 67.73 | 99.58 | 56.29 | 2.17 |
| 2026-10-11 | df8f46a | 78.19 | 66.29 | 99.34 | 37.23 | 1.69 |
| 2026-10-11 | 7846861 | 78.18 | 66.21 | 99.33 | 36.52 | 1.33 |
| 2026-10-11 | 1de1315 | 78.18 | 66.22 | 99.33 | 36.40 | 1.33 |
| 2026-10-10 | 4ce637e | 77.48 | 65.26 | 99.31 | 35.22 | 1.33 |
| 2026-10-10 | 9de46a6 | 77.47 | 64.82 | 99.31 | 35.15 | 1.32 |
| 2026-10-10 | dd5acb2 | 76.79 | 64.04 | 99.30 | 34.50 | 1.16 |
| 2026-10-10 | 40cb291 | 74.46 | 62.65 | 99.27 | 28.44 | 0.20 |
<!-- end generated -->

### The last local run

This part is read from the per-file logs of the last `cargo xtask diff` run
on the machine that generated the page (.dev/logs/diff-scores.txt and
.dev/logs/diff-first.txt). Those logs are not tracked, so the staleness gate
does not check this part, and it may describe a newer run than the record.

<!-- generated: last-run -->
From the last `cargo xtask diff` run recorded on the machine that generated this
page (2026-10-31), which need not be the run the record above was made from.
Scored files by the share of their expected lines mto's output keeps:

| set | every line | 99% or more | 90% to 99% | under 90% |
|---|---:|---:|---:|---:|
| tests | 456 | 0 | 1 | 3 |
| tests-with-settings | 432 | 1 | 31 | 95 |
| corpus | 9358 | 0 | 1 | 2 |
| corpus-scrambled | 9267 | 32 | 32 | 17 |

The syntax at the first line where a mismatching file differs, the ten most
common over every set:

| syntax kind | files |
|---|---:|
| `LetStmt` | 29 |
| `Use` | 22 |
| `TokenTree` | 19 |
| `BlockExpr` | 13 |
| `Fn` | 13 |
| `MethodCallExpr` | 11 |
| `MacroRules` | 9 |
| `MatchArm` | 9 |
| `Visibility` | 7 |
| `RecordExpr` | 6 |
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
