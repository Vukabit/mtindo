---
paper: "layout"
title: "Layout"
status: "planned"
generated: "2026-10-15T21:59:29.408Z"
commit: "44f55b3-dirty.3b716a01"
command: "none yet"
sources: []
seed: "none"
metrics: {}
---

# Layout

## Abstract

<!-- generated: abstract -->
No data is recorded for this paper yet. Section 2 describes the planned method;
section 3 will hold the results once a command records them.
<!-- end generated -->

## 1 Question

Layout is mto's own feature: it reorders a file's items into a predictable
order without changing anything within them. How often does it change a
real file, by how much, does it keep every file's meaning and settle in one
run, and does the standard formatter leave its result alone?

## 2 Method

Planned as a recorded measurement; the checks already exist.

`cargo xtask corpus check` runs on every corpus file today and fails on any
violation of these:

- the parsed tree prints back the file byte for byte;
- everything layout would write with the default settings passes the layout
  meaning check, and laying out the result again changes nothing;
- for every file layout changes, the standard formatter formats the result
  and layout runs again: it must not reorder anything the formatter wrote.
  Differences only in blank lines are counted apart.

Only failures are reported; the counts behind them are not kept. The planned
measurement would record, over the corpus and with the default settings:
the files parsed, the files layout changes, the items moved per changed
file, the files layout declines (syntax errors, a file marked to skip), the
files where a second layout changes something (the target is zero), and the
files where the standard formatter and layout disagree, with blank lines
counted apart. The counts would be kept in a tracked file and committed, so
that this page can be generated from them and checked like the others.

## 3 Results

<!-- generated: results -->
No data is recorded yet.
<!-- end generated -->

## 4 Threats to validity

- The corpus is mostly well-ordered code by experienced authors; how much
  layout changes there says little about code written in another order.
- The default settings are one configuration of many; the real-world compile
  check (see the compile-safety paper) covers two more mixes.
- Agreement with the standard formatter is measured with its settings for
  the crate's edition; another style edition sorts imports differently.

## 5 Reproduction

```bash
# The corpus checks as they stand: round trip, lexing, the layout meaning
# check, stability, and agreement with the standard formatter
cargo xtask corpus check
```

No command records the planned figures yet. When one does, it will be listed
here with the command that regenerates this page, `cargo xtask docs
metrics`.

## 6 Changelog

- 2026-10-15: skeleton; the method is planned and no data is recorded.
