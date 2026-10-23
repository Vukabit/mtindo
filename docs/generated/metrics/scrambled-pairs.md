---
paper: "scrambled-pairs"
title: "Scrambled pairs"
status: "planned"
generated: "2026-10-15T21:59:29.408Z"
commit: "44f55b3-dirty.3b716a01"
command: "none yet"
sources: []
seed: "none"
metrics: {}
---

# Scrambled pairs

## Abstract

<!-- generated: abstract -->
No data is recorded for this paper yet. Section 2 describes the planned method;
section 3 will hold the results once a command records them.
<!-- end generated -->

## 1 Question

Does mto's output depend only on a file's tokens and comments, and not on
how its whitespace happened to be written? Two copies of a file that differ
only in the whitespace between tokens should format to the same text. Unlike
the compatibility paper, this needs no answer from another formatter: mto is
compared with itself, so it can be measured on any code and with any
settings.

## 2 Method

Planned; nothing records it yet.

For every corpus file, two scrambles are made with the scrambler the
compatibility measurement uses, each with its own seed (the seeded variant
differential fuzzing already uses): every run of whitespace between two
tokens is replaced by another run, never empty, with a line break kept after
a line comment. Files holding a carriage return are left out, as in the
scrambled set. Both scrambles are formatted with mto, with the settings it
resolves for the file's folder, and compared:

- **pairs agreeing:** the share of files whose two outputs are equal byte
  for byte;
- **lines shared:** the longest common subsequence of the two outputs'
  lines, over the lines of the first, summed over files;
- **agreeing with the original:** the share of files whose scrambled output
  equals mto's output for the file as fetched.

The seeds would be fixed per file (derived from the file's text, as the
scrambled set's are) and recorded, so a disagreement can be reproduced.
Each figure would be recorded in a tracked file and committed, so that this
page can be generated and checked like the others.

## 3 Results

<!-- generated: results -->
No data is recorded yet.
<!-- end generated -->

## 4 Threats to validity

- Scrambling only moves whitespace between tokens. It does not reach the
  inside of comments or strings, nor attribute and macro forms that are
  whitespace-sensitive by design.
- Agreement is not correctness: two scrambles can agree on output that is
  wrong. The compatibility paper measures correctness against the Rust style.
- Some layouts of the standard style depend on how the input was written
  (for example, blank lines kept between items, or a comment's position).
  Disagreements caused by such rules are expected and would need to be
  told apart from real ones.

## 5 Reproduction

No command records this measurement yet. When one does, it will be listed
here with the command that regenerates this page, `cargo xtask docs
metrics`.

## 6 Changelog

- 2026-10-15: skeleton; the method is planned and no data is recorded.
