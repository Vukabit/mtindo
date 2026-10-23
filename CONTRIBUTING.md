# Contributing to mtindo

## Running the checks

`cargo xtask test` runs every gate and test suite and reports each as passed
or failed; tasks outside the chosen groups are listed as skipped, with how to
run them. A chosen task whose tool is missing fails instead of being skipped,
and every task runs under a time limit, so a green run means every chosen task
ran and passed. `cargo xtask test -h` lists every group, task, flag and
environment variable. Each run is logged, with every task's full output, to
`.dev/logs/test.log`, and each green run is added to
`.dev/logs/last-green.tsv` with the exact tree it ran on. While it runs, the
machine is kept awake (`caffeinate` on macOS, `systemd-inhibit` on Linux,
`SetThreadExecutionState` on Windows), and a fuzz task fails if the machine
slept anyway, since sleep would cut its fuzzing short.

Every `cargo xtask` command leaves a run record in `.dev/runs/` (its tree,
start, end, how it ended, and its memory and CPU over time); `--detach` runs any
command in the background. `.dev/` is machine-local and not tracked by git.

```bash
# The hub: every screen, job, incident, metric, profile, log and command
# in the terminal, laid out by .hub.toml (xtask/hub/SPEC.md)
cargo xtask tui
# Where the project stands: v0.1.0 gates, running jobs, latest runs, incidents
cargo xtask status
# The latest runs; one run's record; a detached run's output; stop one
cargo xtask runs [list | show ID | tail ID | cancel ID]
```

```bash
# Gates and all tests (the default)
cargo xtask test
# Also the real-crate check (network), fuzzing (nightly) and benchmarks
cargo xtask test --full
# Only the tasks that failed when they last ran (.dev/logs/last-test.tsv)
cargo xtask test --failed
# Show the plan without running it
cargo xtask test --list
# Regenerate tests/fixtures expected outputs; review the diff before committing
cargo xtask test --bless
```

The other development commands are also in `xtask`, a development-only crate
in this workspace written in Rust like the rest. Its dependencies (`clap`,
`libc`, and the hub's terminal libraries) never reach the shipped `mtindo`
crate, which `cargo xtask gates` holds to none:

```bash
# No runtime dependencies, layering, docs width, capability statuses,
# metrics papers matching their records
cargo xtask gates
# Regenerate src/text/unicode tables from the Unicode Character Database
cargo xtask unicode
# Compare mto's lexer with rustc's own on .rs files
cargo xtask lexdiff [PATH...]
# Parse .rs files with their crate's edition and report syntax errors
cargo xtask parsecheck [PATH...]
# Re-read rustfmt's options and flags (local binaries; latest stable and
# nightly when online) into compat/rustfmt-surface.txt
cargo xtask capabilities refresh
# Add new items to compat/capabilities.txt as missing; drop stale ones
cargo xtask capabilities sync
# How much of rustfmt mto supports, and how rustfmt's own tests fare
cargo xtask capabilities report [--markdown]
# Check out the pinned real-world corpus (compat/corpus.txt), then check
# that every file round-trips, parses and lexes like rustc; --baseline
# records parsing speed in compat/baseline.txt
cargo xtask corpus check
# Time rustfmt stable and nightly (as cargo fmt runs them) and mto on the
# corpus, by wall clock and CPU time, as a table; fails if mto is below
# twice rustfmt's throughput
cargo xtask corpus bench [--markdown]
# How much of the Rust style mto reproduces: files and lines matching
# rustfmt on its own tests, the corpus and the corpus with scrambled
# whitespace; fails below compat/compatibility.txt, --update records
cargo xtask diff [--update] [--markdown]
# Regenerate the metrics papers in docs/metrics/ from the recorded data
# (compat/ and its history, .dev/ when present); runs no suite. --check
# only compares, as the metrics-docs gate does
cargo xtask docs metrics [--check]
# Move every corpus pin to its repository's current HEAD
cargo xtask corpus update
# Time rustfmt, rustfmt nightly and mto feature by feature on rustfmt's
# own test inputs, and mto's layout settings one by one, drawn live;
# --markdown for a shareable table
cargo xtask race [--rounds N] [--filter TEXT]
# Regenerate src/options/rustfmt.rs, rustfmt's settings table, from the
# releases pinned in compat/rustfmt-surface.txt; --check only compares
cargo xtask settings [--check]
# Regenerate docs/config/, the settings reference and mto.toml's JSON
# Schema, from the settings table; --check only compares (the config-docs
# task of cargo xtask test fails while it is stale)
cargo xtask docs config [--check]
# Check the TOML parser against the pinned toml-test conformance suite
# (compat/toml-test.txt): every valid file decodes to the expected
# values and every invalid one is refused
cargo xtask toml-test
# Fuzz one target (default 60 s); what it finds is filed as an incident,
# its memory over the run kept under .dev/runs/
cargo xtask fuzz run TARGET [--time SECONDS] [--sanitizer address|none]
# Every target side by side for 24 hours, each fuzzing on past its finds
# and keeping every input it fails on (--restart stops a target at a find
# and starts it again instead); the machine kept awake (display too on a
# terminal), drawn live on a terminal, a line per target every 30 minutes
# elsewhere. Start it from your own terminal, not from a tool that may reap it
cargo xtask fuzz soak [--time SECONDS] [--targets A,B] [--restart]
# What the fuzz corpora reach, per module of src/, with an HTML report in
# fuzz/coverage/html/ (a nightly with llvm-tools-preview)
cargo xtask fuzz coverage [TARGET...]
# mto against the standard formatter on scrambled corpus files and
# generated source; each way they disagree shrunk and filed once
cargo xtask fuzz diff [--time SECONDS] [--seed N] [--replay FILE]
# The scrambled-pair suite: corpus files drawn by a fresh seed (printed),
# items shuffled and whitespace scrambled, each checked token for token;
# mto and the standard formatter scored against each other, and the
# crates that compile offline built scrambled, after mto and after the
# standard formatter. A crate mto breaks is filed as a compile break, one
# the scrambler breaks as a scrambler bug; --seed replays a run
cargo xtask pairs [--seed N] [--crates NAME,...] [--files N] \
  [--no-compile] [--keep DIR]
# Every failure kept with its story and filed once (fuzz crashes, mto's
# output failing the meaning check or changing on a second run, panics,
# files mto changes that the standard formatter leaves alone, real crates
# that stop compiling); the board is .dev/incidents/INCIDENTS.md
cargo xtask incidents [list | show ID | mark ID STATUS | replay ID]
# Replay every fuzz incident at this tree; shrink one; keep a fixed one's
# inputs as a test in tests/fuzz_regressions/
cargo xtask incidents recheck | minimize ID | promote ID
# Where mto's memory and time go, per function: every allocation and free
# with its stack, CPU time sampled, hot and cold spots, the call tree
# (macOS and Linux; default: the corpus's serde)
cargo xtask memst [PATH...] [--view overview|functions|hot|cold|tree|sites]
# Every number the runs measure, as series with their trend; the ratchet
# (compat/performance.txt) speed, memory and the build guard may not fall
# behind
cargo xtask metrics [list | show NAME | ratchet [--update]]
# What the build guard costs and how few checks a narrowing takes: in mto
# on the corpus with planted culprits, and on real crates with cargo check
cargo xtask doctor bench [--trials N]
# What writing costs on this machine: generated files written with each
# way of syncing (safety.sync), history on and off; kept as io.* metrics
cargo xtask io bench [--files N]
# Every command, gate and exit code
cargo xtask help
```
