# Run statistics

`mto stats` shows what mto has done for you to date: how many runs, files
and lines, what changed, how long runs took and how they were run. It reads
a small ledger on your own machine that each run adds to. Nothing in it
leaves the machine, and nothing in it names anything.

```bash
mto stats                      # the totals
mto stats --from 7d --by day  # a line per day this week
mto stats --by version         # how each mto version did
mto stats --json               # the same, for scripts
mto stats forget               # remove it all
```

```text
since 2026-10-02 · 1,502 runs · mto 0.1.0 → 0.2.0-dev

formatted   3.5 GB · 228,978 files · 112.0 M lines · 2,965 files changed
changes     23,233 lines added · 19,159 removed · 4,375 items moved
time        8 m 47 s in all · median run 464 ms · p95 900 ms · longest 900 ms
stdin       531 formats · median 22 ms · p95 30 ms
by mode     check 50% · write 14% · stdin 35%
from        terminal 57% · editor 35% · cargo 8%
projects    4 · layout on in 100% of runs
problems    0 writes refused · 0 files with syntax errors · 0 unreadable
```

`--by` takes `day`, `month`, `year`, `version` or `mode`; `--from` takes
`7d`, `4w`, `12h` or a day such as `2026-10-01`.

## What is kept

For each run that formats files (writing, `--check`, `--dry-run`, stdin):

* when it ended, the mto version (`0.2.0`, or `0.2.0-dev` for a build not
  released), the system (`macos-aarch64`), the edition, the exit code;
* how it was run: writing, check, dry run or stdin, and from a terminal, a
  pipe, CI, an editor (stdin with `--stdin-filepath`) or `cargo mto`;
* how many files it took and what came of them (changed, unchanged, left
  alone, with syntax errors, unreadable, refused by the verifier);
* the bytes and lines it read, the lines it added and removed, the items
  layout moved;
* how long it took, its worker threads, and whether layout was on;
* the project it ran in, as a hash: its folder's path with a salt made once
  on this machine, cut to 12 hex digits. It tells your projects apart
  without naming one, and the same project has another hash on another
  machine.

## What is never kept

Paths, file or folder names, anything a file holds, the command's arguments,
your user or host name, git remotes, commits, crate names. The version is
kept without the commit it was built from.

## Turning it off

Counting is set in the [user config](configuration.md#the-user-config):

```toml
[stats]
enabled = "auto"  # or "on", "off"
```

`auto`, the default, counts while history is on (`history.enabled`) and
never on CI (`CI` set): turning history off for your privacy turns this off
too. `--no-history` for one run does not. `on` counts always, CI included;
`off` never. `--no-stats` leaves one run out, and `MTINDO_NO_STATS=1` every
run it is set for. `mto stats` says when counting is off, and why.

## Where it lives, and how big it gets

The ledger is kept in `~/.local/state/mtindo/stats` (`$XDG_STATE_HOME` is
honored; `%LOCALAPPDATA%\mtindo\stats` on Windows), beside history:

| File | Holds | Kept |
|---|---|---|
| `runs/2026-11-07.tsv` | a line per run, that day (UTC) | 7 days |
| `days/2026-11.tsv` | a line per day, mode and version | 90 days |
| `months/2026.tsv` | a line per month, mode and version | 2 years |
| `years.tsv` | a line per year, mode and version | for good |
| `salt` | what project hashes are made with | until forgotten |

A line older than its file keeps is added into the one above, so nothing is
counted twice and the totals never change. Run times are kept as counts in
40 time ranges, so a median or a 95th percentile still reads from a month or
a year, to within about 40%. A month or year begun before `--from` is left
out of what it shows.

A run's line is about 250 bytes and a total's about 600, so the ledger holds
about 1.75 KB for each run a day of the last week, plus about 230 KB, plus
about 5 KB a year:

| Runs a day | Largest file | All of it |
|---|---|---|
| 20 (now and then) | 5 KB | about 270 KB |
| 300 (format on save) | 75 KB | about 0.75 MB |
| 5,000 (all day, a large project) | 1.25 MB | about 9 MB |

Folding happens at the end of a run, after what it printed. A run never
fails because of the ledger: a write it cannot make is passed over.
`mto stats forget` removes the folder, the salt with it, so projects get new
hashes after.

How the ledger is written and folded: [run statistics
design](../design/stats.md).
