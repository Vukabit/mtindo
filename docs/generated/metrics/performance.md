---
paper: "performance"
title: "Performance"
status: "measured"
generated: "2026-11-21T20:04:42.654Z"
commit: "f651df4-dirty.7fdf2600"
command: "cargo xtask metrics ratchet --update"
sources:
  - records/results/ratchet.txt
  - records/results/baseline.txt
  - records/archive/2026-10-optimization.txt
machine: "macos aarch64, 10 cores"
seed: "fixed in code for the doctor bench's draws; none elsewhere"
metrics:
  corpus.lex.mb_per_s: 363.5
  corpus.lex_parse.mb_per_s: 57.6
limits:
  speed.speedup.stable: 16.52
  speed.speedup.nightly: 16.66
  speed.mto.laid_out.mb_per_s: 154.7
  speed.mto.as_fetched.mb_per_s: 152.8
  speed.layout.one_thread.mb_per_s: 78.05
  memst.serde.allocs: 1202646
  memst.serde.bytes: 360736587
  memst.serde.peak_live: 40377018
  doctor.held.cost.x: 1.082
  doctor.narrow.n10.k1.checks: 9.7
  doctor.narrow.n100.k1.checks: 13.6
  doctor.narrow.n100.k3.checks: 49.5
  doctor.narrow.found_rate: 1
  io.write.batch.history.ms_per_file: 0.353
  io.write.batch.plain.ms_per_file: 0.151
  perf.scaling.one_thread.mb_per_s: 8.916
  perf.latency.check.median_ms: 2.2
  perf.growth.items.exponent: 1.058
  perf.growth.nesting.exponent: 1.415
  perf.growth.line.exponent: 0.965
  perf.growth.chain.exponent: 0.964
  perf.growth.match.exponent: 1.002
---

# Performance

## Abstract

<!-- generated: abstract -->
On the recording machine (macos aarch64, 10 cores), the ratchet holds 22
figures. At their recorded values, mto lays out and formats 8.916 MB/s on one
thread, checks a laid-out corpus 16.52 times as fast as the standard formatter's
stable release, and holds at most 40.4 MB of memory live at once while laying
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
  memst is not yet published, so only its maintainers can take these.
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
| `speed.speedup.stable` | 19.48 | 16.52 | 15% | higher |
| `speed.speedup.nightly` | 18.02 | 16.66 | 15% | higher |
| `speed.mto.laid_out.mb_per_s` | 120 | 154.7 | 15% | higher |
| `speed.mto.as_fetched.mb_per_s` | 65.63 | 152.8 | 15% | higher |
| `speed.layout.one_thread.mb_per_s` | 45.3 | 78.05 | 15% | higher |
| `memst.serde.allocs` | 3200300 | 1202646 | 1% | lower |
| `memst.serde.bytes` | 534467036 | 360736587 | 1% | lower |
| `memst.serde.peak_live` | 33128643 | 40377018 | 25% | lower |
| `doctor.held.cost.x` | 1.271 | 1.082 | 25% | lower |
| `doctor.narrow.n10.k1.checks` | 9.7 | 9.7 | 10% | lower |
| `doctor.narrow.n100.k1.checks` | 15.5 | 13.6 | 10% | lower |
| `doctor.narrow.n100.k3.checks` | 49.6 | 49.5 | 10% | lower |
| `doctor.narrow.found_rate` | - | 1 | 0% | higher |
| `io.write.batch.history.ms_per_file` | 13.19 | 0.353 | 50% | lower |
| `io.write.batch.plain.ms_per_file` | - | 0.151 | 50% | lower |
| `perf.scaling.one_thread.mb_per_s` | 5.731 | 8.916 | 15% | higher |
| `perf.latency.check.median_ms` | 6.469 | 2.2 | 50% | lower |
| `perf.growth.items.exponent` | 1.057 | 1.058 | 20% | lower |
| `perf.growth.nesting.exponent` | - | 1.415 | 20% | lower |
| `perf.growth.line.exponent` | 0.989 | 0.965 | 20% | lower |
| `perf.growth.chain.exponent` | 0.976 | 0.964 | 20% | lower |
| `perf.growth.match.exponent` | 1.256 | 1.002 | 20% | lower |

Parsing speed on one thread, release build, on macos aarch64, 10 cores, over
10731 files, 90345059 bytes (records/pins/corpus.txt):

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
| current |  | 8.916 | 16.52 | 40.4 |
| 2026-10-30 | a35ea46 | 9.103 | 20.28 | 31.5 |
| 2026-10-29 | 6c7b908 | 8.449 | 17.03 | 31.5 |
| 2026-10-13 | b69b461 | 9.702 | 31.39 | 36.1 |
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
| `speed.speedup.stable` | 16.52 | yes | 2026-11-21 |
| `speed.speedup.nightly` | 16.66 | yes | 2026-11-21 |
| `speed.mto.laid_out.mb_per_s` | 154.71 | yes | 2026-11-21 |
| `speed.mto.as_fetched.mb_per_s` | 152.83 | yes | 2026-11-21 |
| `speed.layout.one_thread.mb_per_s` | 78.05 | yes | 2026-11-21 |
| `memst.serde.allocs` | 1.20M | yes | 2026-11-21 |
| `memst.serde.bytes` | 360.74M | yes | 2026-11-21 |
| `memst.serde.peak_live` | 40.38M | yes | 2026-11-21 |
| `doctor.held.cost.x` | 1.08 | yes | 2026-11-21 |
| `doctor.narrow.n10.k1.checks` | 9.70 | yes | 2026-11-21 |
| `doctor.narrow.n100.k1.checks` | 13.60 | yes | 2026-11-21 |
| `doctor.narrow.n100.k3.checks` | 49.50 | yes | 2026-11-21 |
| `doctor.narrow.found_rate` | 1 | yes | 2026-11-21 |
| `io.write.batch.history.ms_per_file` | 0.35 | yes | 2026-11-21 |
| `io.write.batch.plain.ms_per_file` | 0.15 | yes | 2026-11-21 |
| `perf.scaling.one_thread.mb_per_s` | 8.92 | yes | 2026-11-21 |
| `perf.latency.check.median_ms` | 2.20 | yes | 2026-11-21 |
| `perf.growth.items.exponent` | 1.06 | yes | 2026-11-21 |
| `perf.growth.nesting.exponent` | 1.42 | yes | 2026-11-21 |
| `perf.growth.line.exponent` | 0.96 | yes | 2026-11-21 |
| `perf.growth.chain.exponent` | 0.96 | yes | 2026-11-21 |
| `perf.growth.match.exponent` | 1.00 | yes | 2026-11-21 |
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
