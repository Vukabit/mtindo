---
paper: "performance"
title: "Performance"
status: "measured"
generated: "2026-10-20T05:13:14.860Z"
commit: "83b8cf8-dirty.bdac3593"
command: "cargo xtask metrics ratchet --update"
sources:
  - compat/performance.txt
  - compat/baseline.txt
  - compat/optimization-baseline.txt
machine: "macos aarch64, 10 cores"
seed: "fixed in code for the doctor bench's draws; none elsewhere"
metrics:
  corpus.lex.mb_per_s: 363.5
  corpus.lex_parse.mb_per_s: 57.6
limits:
  speed.speedup.stable: 31.39
  speed.speedup.nightly: 32.08
  speed.mto.laid_out.mb_per_s: 292.5
  speed.mto.as_fetched.mb_per_s: 150.8
  speed.layout.one_thread.mb_per_s: 75.48
  memst.serde.allocs: 1857406
  memst.serde.bytes: 420472888
  memst.serde.peak_live: 36052563
  doctor.held.cost.x: 1.248
  doctor.narrow.n10.k1.checks: 9.7
  doctor.narrow.n100.k1.checks: 15.5
  doctor.narrow.n100.k3.checks: 49.6
  doctor.narrow.found_rate: 1
  io.write.batch.history.ms_per_file: 0.737
  io.write.batch.plain.ms_per_file: 0.277
  perf.scaling.one_thread.mb_per_s: 9.702
  perf.latency.check.median_ms: 2.105
  perf.growth.items.exponent: 1.049
  perf.growth.nesting.exponent: 1.18
  perf.growth.line.exponent: 0.988
  perf.growth.chain.exponent: 0.976
  perf.growth.match.exponent: 1.016
---

# Performance

## Abstract

<!-- generated: abstract -->
On the recording machine (macos aarch64, 10 cores), the ratchet holds 22
figures. At their recorded values, mto lays out and formats 9.702 MB/s on one
thread, checks a laid-out corpus 31.39 times as fast as the standard formatter's
stable release, and holds at most 36.1 MB of memory live at once while laying
out and formatting serde.
<!-- end generated -->

## 1 Question

How fast is mto, how much memory does it use, and how does its cost grow
with the size and shape of the code it reads? A formatter runs on every
save and in every CI build, so its speed against the standard formatter,
its start-up time, its scaling over cores and its memory all matter, and
none of them may quietly get worse.

## 2 Method

Each figure is measured by a command that keeps it as a series of
measurements on the machine that ran it:

- `cargo xtask corpus bench`: the whole corpus, end to end. The standard
  formatter's stable and nightly releases run as `cargo fmt --all --check`
  runs them, one repository at a time; the release `mto --check` runs on a
  copy of the corpus already laid out (what CI sees) and on the corpus as
  fetched. mto may use every core; each of its figures is the median of
  three runs. Layout alone is also timed in process on one thread.
- `cargo xtask perf`: one small file through the release binary 200 times
  (median and 99th percentile, start-up included); every 8th corpus file
  laid out and formatted in process on 1, 2, 4 and 8 threads (the fastest
  of three interleaved passes); and generated files of doubling size,
  growing in one of five ways, with the slope of log time against log size
  as the growth exponent.
- `cargo xtask memst`: the corpus's serde repository laid out and formatted
  under an allocation recorder: allocations, bytes and peak live bytes.
- `cargo xtask doctor bench`: what the build guard costs (shaping with
  remembered holds over without) and how many builds a narrowing takes to
  find planted culprits, over seeded draws so the figures repeat.
- `cargo xtask io bench`: a generated 600-file project rewritten by the
  release binary with each way of syncing, history on and off.
- `cargo xtask corpus check --baseline`: lexing and parsing speed on one
  thread, kept in compat/baseline.txt.

Before timing, every bench waits up to two minutes for a quiet machine (at
most 20% of the processor busy with other work, and not throttled). A
timing taken while the machine stayed busy is kept but never judged.

`cargo xtask metrics ratchet --update` writes the latest value of each held
series to compat/performance.txt with a tolerance and a direction: a later
measurement may be worse than the recorded value by the tolerance at most,
or it is filed as a regression incident. Timings depend on the machine, so
the record names its machine and its limits are checked only there, counts
such as allocations included. compat/optimization-baseline.txt keeps the
figures as they stood before the optimization work, for comparison.

## 3 Results

Every figure the ratchet holds: its value before the optimization work
(where measured), the value recorded as its limit, and the slack a new
measurement is allowed in the worse direction.

<!-- generated: results -->
| series | before | limit | slack | better |
|---|---:|---:|---:|---|
| `speed.speedup.stable` | 19.48 | 31.39 | 15% | higher |
| `speed.speedup.nightly` | 18.02 | 32.08 | 15% | higher |
| `speed.mto.laid_out.mb_per_s` | 120 | 292.5 | 15% | higher |
| `speed.mto.as_fetched.mb_per_s` | 65.63 | 150.8 | 15% | higher |
| `speed.layout.one_thread.mb_per_s` | 45.3 | 75.48 | 15% | higher |
| `memst.serde.allocs` | 3200300 | 1857406 | 1% | lower |
| `memst.serde.bytes` | 534467036 | 420472888 | 1% | lower |
| `memst.serde.peak_live` | 33128643 | 36052563 | 25% | lower |
| `doctor.held.cost.x` | 1.271 | 1.248 | 25% | lower |
| `doctor.narrow.n10.k1.checks` | 9.7 | 9.7 | 10% | lower |
| `doctor.narrow.n100.k1.checks` | 15.5 | 15.5 | 10% | lower |
| `doctor.narrow.n100.k3.checks` | 49.6 | 49.6 | 10% | lower |
| `doctor.narrow.found_rate` | - | 1 | 0% | higher |
| `io.write.batch.history.ms_per_file` | 13.19 | 0.737 | 50% | lower |
| `io.write.batch.plain.ms_per_file` | - | 0.277 | 50% | lower |
| `perf.scaling.one_thread.mb_per_s` | 5.731 | 9.702 | 15% | higher |
| `perf.latency.check.median_ms` | 6.469 | 2.105 | 50% | lower |
| `perf.growth.items.exponent` | 1.057 | 1.049 | 20% | lower |
| `perf.growth.nesting.exponent` | - | 1.18 | 20% | lower |
| `perf.growth.line.exponent` | 0.989 | 0.988 | 20% | lower |
| `perf.growth.chain.exponent` | 0.976 | 0.976 | 20% | lower |
| `perf.growth.match.exponent` | 1.256 | 1.016 | 20% | lower |

Parsing speed on one thread, release build, on macos aarch64, 10 cores, over
10731 files, 90345059 bytes (compat/corpus.txt):

| stage | MB/s |
|---|---:|
| lexer | 363.5 |
| lexer and parser | 57.6 |
<!-- end generated -->

### History

Three of the recorded limits at every commit that changed the record,
newest first. The first row is the record as it stands; a commit whose
record equals it is not repeated.

<!-- generated: history -->
| date | commit | one thread MB/s | speedup | serde peak MB |
|---|---|---:|---:|---:|
| current |  | 9.702 | 31.39 | 36.1 |
<!-- end generated -->

### Latest measurements on the generating machine

This part is read from the metric series of the machine that generated the
page. They are not tracked, so the staleness gate does not check it.

<!-- generated: latest -->
The latest measurement of each figure on the machine that generated this page
(macos aarch64, 10 cores); the limits apply only on the recording machine, and a
timing taken while the machine was busy is not judged:

| series | latest | within | date |
|---|---:|---|---|
| `speed.speedup.stable` | 16.06 | no | 2026-10-19 |
| `speed.speedup.nightly` | 16.36 | no | 2026-10-19 |
| `speed.mto.laid_out.mb_per_s` | 136.93 | no | 2026-10-19 |
| `speed.mto.as_fetched.mb_per_s` | 131.84 | yes | 2026-10-19 |
| `speed.layout.one_thread.mb_per_s` | 71.42 | yes | 2026-10-19 |
| `memst.serde.allocs` | 2.45M | no | 2026-10-20 |
| `memst.serde.bytes` | 674.78M | no | 2026-10-20 |
| `memst.serde.peak_live` | 55.21M | no | 2026-10-20 |
| `doctor.held.cost.x` | 1.25 | yes | 2026-10-13 |
| `doctor.narrow.n10.k1.checks` | 9.70 | yes | 2026-10-13 |
| `doctor.narrow.n100.k1.checks` | 15.50 | yes | 2026-10-13 |
| `doctor.narrow.n100.k3.checks` | 49.60 | yes | 2026-10-13 |
| `doctor.narrow.found_rate` | 1 | yes | 2026-10-13 |
| `io.write.batch.history.ms_per_file` | 0.97 | yes | 2026-10-16 |
| `io.write.batch.plain.ms_per_file` | 0.41 | yes | 2026-10-16 |
| `perf.scaling.one_thread.mb_per_s` | 4.48 | no | 2026-10-16 |
| `perf.latency.check.median_ms` | 2.23 | yes | 2026-10-16 |
| `perf.growth.items.exponent` | 0.46 | yes | 2026-10-16 |
| `perf.growth.nesting.exponent` | 1.58 | no | 2026-10-16 |
| `perf.growth.line.exponent` | 0.89 | yes | 2026-10-16 |
| `perf.growth.chain.exponent` | 1.87 | no | 2026-10-16 |
| `perf.growth.match.exponent` | 1.01 | yes | 2026-10-16 |
<!-- end generated -->

## 4 Threats to validity

- **One machine.** Every timed limit was recorded on the machine the front
  matter names. Another machine gives other numbers, and the ratchet does
  not judge them. Ratios (speedups) and counts carry over best, and even
  ratios depend on the number of cores.
- **Noise.** Timings move by up to about 14% with other work on the
  machine, peak live memory by a quarter with how threads interleave. The
  tolerances are set from what the figures do on an unchanged tree, so a
  regression smaller than its tolerance passes unnoticed.
- **Recorded limits are not one run.** Each limit is the latest value when
  the record was updated, not an average, and the figures of one update may
  come from runs on different days.
- **The corpus decides the mix.** Speeds over the corpus weigh each
  repository by its size; code unlike the corpus (generated code, very long
  files) may run at another speed. The growth exponents cover size, for
  five generated shapes only.
- **Start-up is part of latency.** The latency figure includes starting the
  process, as an editor's format-on-save sees it, so it depends on the
  operating system as much as on mto.
- **Before figures were partly measured differently.** Some figures before
  the optimization work were wall times rounded up by the way the runner
  polled its children; compat/optimization-baseline.txt marks those
  `[polled]`, and they compare loosely.

## 5 Reproduction

The corpus must be fetched once (network). Timed commands should run on a
quiet machine; each waits for one.

```bash
# Fetch the corpus and check it; --baseline records parsing speed
cargo xtask corpus check --baseline
# Speed against the standard formatter, and layout on one thread
cargo xtask corpus bench
# Latency, thread scaling and growth with size
cargo xtask perf
# Memory on serde
cargo xtask memst
# The build guard's cost, and writing files
cargo xtask doctor bench
cargo xtask io bench
# Every series against the record; then record the latest as the limits
cargo xtask metrics ratchet
cargo xtask metrics ratchet --update
# Regenerate this page
cargo xtask docs metrics
```

## 6 Changelog

- 2026-10-15: first version, generated from the ratchet, the parsing
  baseline and the figures before optimization.
