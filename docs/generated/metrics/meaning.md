---
paper: "meaning"
title: "Meaning and stability"
status: "measured"
generated: "2026-11-01T02:35:40.822Z"
commit: "096a91d-dirty.7f47fb1f"
command: "cargo xtask diff"
sources:
  - records/policy/exceptions.txt
  - records/results/compatibility.txt
toolchain:
  reference_formatter: "1.10.0-stable (58cf8f9c35 2026-10-13)"
corpus_fingerprint: "sha256:0d26c0baa5577082"
seed: "FNV-1a of each file's text (scrambled set only)"
metrics:
  meaning.exceptions.tests: 8
  meaning.exceptions.tests-with-settings: 11
  meaning.exceptions.corpus: 1
  meaning.exceptions.corpus-unformatted: 1
  meaning.exceptions.total: 21
  meaning.exceptions.unmodelled: 5
  meaning.exceptions.other: 16
---

# Meaning and stability

## Abstract

<!-- generated: abstract -->
Every output of mto that `cargo xtask diff` scores must keep the input's meaning
and must not change when formatted again, or the run fails. The same meaning
check, run on the standard formatter's own outputs, flags 21 files, each listed
with its reason: 5 are rewrites the check does not model yet, and 16 are the
formatter changing meaning, writing invalid code, or test inputs that differ
from their expected files beyond formatting.
<!-- end generated -->

## 1 Question

Does formatting with mto ever change what a program means, and does
formatting its output again ever change it further? A formatter may only
move whitespace and make rewrites that provably keep the program's meaning,
and its output should be a fixed point: a second run changes nothing. Both
must hold for every input, so the measure here is a count of failures, and
the target is zero.

## 2 Method

**The meaning check** compares an input with a formatted output. Both texts
are parsed and walked into canonical token streams: whitespace and comments
are dropped, doc comments are kept (they are attributes) with each line
trimmed, and every allowed rewrite is resolved to one form, so that a
rewrite in either direction compares equal. The streams must be equal.
Comments are compared on their own, as a multiset with each comment's lines
trimmed, since formatting may re-indent a block comment or move a comment
across a comma; every comment must survive. Adding or removing a byte-order
mark also fails the check. The allowed rewrites are declared one by one in
the meaning check's source, each with the reason it keeps the meaning (an
optional trailing comma, braces around a closure body holding one
expression, and so on).

**Stability** is checked by formatting mto's output a second time with the
same settings: the result must equal the output.

Both run inside `cargo xtask diff` (see the compatibility paper for its
input sets) on every scored input:

- mto's output must pass the meaning check against the input and be
  stable. Any input that fails either fails the run, and is filed as a
  `meaning` or `unstable` incident with the input kept for replay; a panic
  is filed as a `panic` incident. Open incidents of these kinds fail the
  `incidents` gate of `cargo xtask test`.
- The same meaning check runs on the standard formatter's own output. This
  tests the check itself: a flag there is either a rewrite the check does
  not model yet, or a real fault of the standard formatter. Each flagged
  file must be listed in records/policy/exceptions.txt with its reason, or
  the run fails; a listed file no longer flagged is reported for removal.

This paper counts the listed exceptions, which are tracked, and classifies
each by its reason: a reason saying the rewrite is `equivalent` or `not
modelled` counts as a gap in the check, any other as the formatter's fault
or a test input that differs from its expected file beyond formatting.
mto's own failures are not recorded in a tracked file: a run with any
fails, and they live as incidents on the machine that ran it.

## 3 Results

Exceptions listed for the standard formatter's output, by set:

<!-- generated: results -->
| set | listed | not modelled | other |
|---|---:|---:|---:|
| tests | 8 | 0 | 8 |
| tests-with-settings | 11 | 5 | 6 |
| corpus | 1 | 0 | 1 |
| corpus-scrambled | 1 | 0 | 1 |

The reasons given, with how many files give each:

- a static in an extern block loses its initializer (`= "string"`): the
  formatter's issue 6267 (2)
- an attributed match arm body wrapped in a block: equivalent, not modelled (2)
- the formatter drops a `for` without a binder list (invalid input) (2)
- `cfg_select!` arm bodies unwrapped: macro-specific, not modelled (1)
- `imports_granularity = "One"` with renames: not modelled (1)
- `trailing_semicolon` adds `;` after a macro call ending a loop body (1)
- a `;` added after the last arm of a `macro_rules!` inside a macro: equivalent,
  not modelled (1)
- the expected `(|| ..).method()` puts parentheses around a closure, which
  changes what the closure covers (1)
- the expected file differs from its input beyond formatting (1)
- the expected file drops `pub` from a const block item, which changes the
  tokens (1)
- the expected file drops `use self;` (invalid input), which changes the tokens;
  mto keeps it (1)
- the expected file drops a `;` from a `macro_rules!` transcriber, which changes
  its expansion (1)
- the expected file writes two spaces after a tuple field's visibility (`pub
  u64`) after a comment (1)
- the formatter adds parentheses that change what a closure covers (1)
- the formatter drops `pub` from a const block (1)
- the formatter drops `pub` from an impl item (1)
- the formatter drops `use self` (invalid input) (1)
- the formatter removes a `;` from a macro transcriber, which changes its
  expansion (1)
<!-- end generated -->

### History

The listed exceptions at every commit that changed the list, newest first.
The first row is the list as it stands; a commit whose list equals it is not
repeated.

<!-- generated: history -->
| date | commit | listed | not modelled | other |
|---|---|---:|---:|---:|
| current |  | 21 | 5 | 16 |
| 2026-10-31 | ae7586b | 25 | 5 | 20 |
| 2026-10-31 | 2315258 | 30 | 10 | 20 |
| 2026-10-30 | 394bd5e | 31 | 10 | 21 |
| 2026-10-30 | a3e5879 | 31 | 10 | 21 |
| 2026-10-24 | c7fa04f | 29 | 10 | 19 |
| 2026-10-17 | fd5de34 | 32 | 10 | 22 |
| 2026-10-17 | 5e09478 | 31 | 10 | 21 |
| 2026-10-15 | 65e3945 | 97 | 20 | 77 |
| 2026-10-15 | ee77875 | 101 | 22 | 79 |
| 2026-10-10 | 407413b | 107 | 28 | 79 |
<!-- end generated -->

### Incidents on the generating machine

mto's own meaning and stability failures, as the incident records of the
machine that generated the page keep them. They are not tracked, so the
staleness gate does not check this part.

<!-- generated: incidents -->
Incidents filed on the machine that generated this page (`cargo xtask
incidents`), each failure counted once however often it was seen, by kind and
status:

| kind | open | flaky | fixed | wont-fix |
|---|---:|---:|---:|---:|
| `meaning` | 0 | 0 | 151 | 0 |
| `unstable` | 0 | 0 | 348 | 0 |
| `panic` | 0 | 0 | 7 | 0 |
<!-- end generated -->

## 4 Threats to validity

- **The check is only as strict as its model.** It compares tokens after
  resolving the allowed rewrites. A rewrite allowed by mistake would hide a
  real change of meaning in both mto's output and the standard formatter's.
  Each rewrite has its own test, and running the check on the standard
  formatter's output, which is expected to keep meaning, is what keeps the
  rewrites honest: a rewrite it needs and the check lacks shows as a flag.
- **Tokens are not semantics.** Equal token streams mean an equal program
  for the parser, but the check does not expand macros. A rewrite inside a
  macro's input that changes what the macro does would pass if the tokens
  resolved equal. The few rewrites that reach into macro syntax (the
  delimiters of `vec!`, the arms of `macro_rules!`) each state in their
  reason why the expansion stays the same.
- **Refused inputs are not checked.** An input the standard formatter
  refuses is skipped before mto formats it, so mto's meaning and stability
  are not checked on it by this run. The fuzz targets cover mto on arbitrary
  input (see the fuzzing paper).
- **Zero failures is a fact of one run.** A run that finds a failure exits
  with an error, but `--update` still writes the compatibility record, so
  the record alone does not prove a clean run. The incidents gate and the
  local incident table are the evidence.
- **The classification of exceptions is by wording.** A reason is counted as
  a gap in the check by the words `equivalent` or `not modelled`. A reason
  worded otherwise is counted as the formatter's fault.
- **Stability is checked once.** A second run must change nothing; a third
  is not run, since a stable second run makes every later one stable too.

## 5 Reproduction

```bash
# Score every set; fails on any of mto's outputs that changes meaning or is
# not stable, and on any flag of the standard formatter's output that
# records/policy/exceptions.txt does not list
cargo xtask diff
# mto's failures, filed once each, with their inputs
cargo xtask incidents list --kind meaning --kind unstable --kind panic
# Regenerate this page
cargo xtask docs metrics
```

## 6 Changelog

- 2026-10-15: first version, generated from the exception list and its
  history.
