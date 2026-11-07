# Run statistics

`src/stats` keeps the ledger `mto stats` reads: counts of what each run that
formats did, never a name. `src/cli/stats.rs` decides whether a run is
counted, gathers its line when it ends, and shows the ledger. The guide's
[Run statistics](../guide/stats.md) says what is kept and how to turn it
off; this page is how it is kept.

## A run's line

`cli::stats::start` runs once the resolver is read, for a run that formats
files or reads stdin: if counting is on, it notes the ledger, the project's
hash and the time. `cli::stats::finish` runs inside the run's `finish`, after
the summary is printed: it sums the files' `Counts` (bytes and lines read,
lines inserted and removed by the same diff `--check` shows, items layout
moved) and outcomes, and hands the `Run` to `Ledger::record`.

The line is tab-separated, one field each: milliseconds, version, system,
mode, context, edition, exit code, project hash (`-` for none), the twelve
counts, wall time in milliseconds, worker threads, and layout as `0` or `1`.
It is appended to `runs/YYYY-MM-DD.tsv` (the UTC day) in one write, so runs
at once each keep their line whole. Every file starts `# mto stats 1`; a line
of another shape is passed over when read.

Counting is on when `stats.enabled` is `on`, or `auto` with
`history.enabled` on and `CI` unset, and `MTINDO_NO_STATS` is unset, and
there is a home folder (`cli::stats::enabled`). `--no-stats` skips `start`.

The project is `history::project_root` of the current folder (the nearest
folder holding `.git`, else `Cargo.toml`), hashed with SHA-256 after the
`salt` file's contents and cut to 12 hex digits. The salt is a new history
id, made once and written by hard link from a temporary file, so two runs
making it at once read back the same one.

## Tiers and folding

A tier's line is a `Total`: period, mode, version, runs, the twelve counts
summed, wall time summed, the longest run, the run times as a histogram, the
runs by context, the runs with layout on, and the set of project hashes.
Everything in it adds up, so a fold is a sum and a year's line is exactly
its months'.

The histogram has 40 buckets: bucket `i` holds a run up to
`600000^(i/39)` milliseconds, about 1.41 times the one before, from 1 ms to
ten minutes, the last holding anything longer. A percentile is the bound of
the bucket it falls in, no more than the longest run, so it reads to within
a bucket's width wherever it is taken.

`Ledger::fold`, after each record, takes `.lock` (made only if absent; one
older than a minute is a run's that died, and removed) and folds what has
aged, as of the run's day:

1. each `runs/` day at least 7 days old, summed by mode and version into
   `days/YYYY-MM.tsv`;
2. each `days/` month whose last day is at least 90 days old into
   `months/YYYY.tsv`;
3. each `months/` year whose last day is at least 730 days old into
   `years.tsv`.

A fold reads the target, adds the period's totals, sorts, writes it whole
under a temporary name and renames it into place; only then is the source
file removed. A target already holding the period is left as it is and the
source removed: a fold cut short between the rename and the removal counts
nothing twice. A run finding the lock held skips folding; the next one
does it.

## Reading

`Ledger::read` sums `runs/` lines into day totals, adds `days/`, and reads
`months/` and `years.tsv` as they are. `mto stats --from` keeps a total
whose period starts on or after the day given, so a month or year begun
before it is left out whole. `--by day` lists only days; older periods are
named in a note.

## Where it lives

`stats/` beside `history/` in the state folder ([Where the files
live](writes.md#where-the-files-live)).
